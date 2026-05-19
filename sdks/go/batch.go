package handler

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"runtime/debug"
)

// RunBatch reads JSON-Lines from stdin, dispatches batches to fn, writes
// batch_result envelopes to stdout. Returns when stdin closes or rowforge
// sends shutdown. Calls os.Exit(2) on protocol-level errors (malformed JSON,
// missing init, handler panic, length mismatch).
//
// The handler MUST return exactly len(rows) Results for each batch, indexed
// positionally. A mismatch is treated as a fatal protocol error and the SDK
// exits(2) — rowforge will see HandlerExit and apply the manifest's
// idempotent gating (WORKER_CRASH or WORKER_CRASH_UNSAFE).
func RunBatch(fn BatchHandleFunc, opts ...Option) {
	cfg := config{handlerVersion: "0.0.0"}
	for _, o := range opts {
		o(&cfg)
	}

	scanner := bufio.NewScanner(os.Stdin)
	// Batch envelopes can be much larger than single rows. The pool's
	// configured byte cap is typically 16 MiB; we allow 64 MiB here for
	// headroom (envelope JSON overhead + safety margin).
	scanner.Buffer(make([]byte, 1024*1024), 64*1024*1024)

	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	emit := func(v any) {
		b, err := json.Marshal(v)
		if err != nil {
			fmt.Fprintln(os.Stderr, "rowforge-handler: emit marshal:", err)
			os.Exit(2)
		}
		if _, err := out.Write(b); err != nil {
			fmt.Fprintln(os.Stderr, "rowforge-handler: emit write:", err)
			os.Exit(2)
		}
		_, _ = out.WriteString("\n")
		if err := out.Flush(); err != nil {
			fmt.Fprintln(os.Stderr, "rowforge-handler: emit flush:", err)
			os.Exit(2)
		}
	}

	// 1. Init.
	if !scanner.Scan() {
		// EOF before init — caller closed early; not our problem.
		return
	}
	var init struct {
		Type    string         `json:"type"`
		RunID   string         `json:"run_id"`
		Config  map[string]any `json:"config"`
		Columns []string       `json:"columns"`
	}
	if err := json.Unmarshal(scanner.Bytes(), &init); err != nil {
		fmt.Fprintln(os.Stderr, "rowforge-handler: malformed init:", err)
		os.Exit(2)
	}
	if init.Type != "init" {
		fmt.Fprintf(os.Stderr, "rowforge-handler: expected init, got %q\n", init.Type)
		os.Exit(2)
	}

	// 2. Ready.
	emit(map[string]any{"type": "ready", "handler_version": cfg.handlerVersion})

	// 3. Batch loop.
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var env struct {
			Type string `json:"type"`
			Rows []struct {
				Seq  uint64         `json:"seq"`
				Data map[string]any `json:"data"`
				Meta struct {
					DryRun   bool   `json:"dry_run"`
					RowIndex uint64 `json:"row_index"`
				} `json:"meta"`
			} `json:"rows"`
		}
		if err := json.Unmarshal(line, &env); err != nil {
			fmt.Fprintln(os.Stderr, "rowforge-handler: malformed envelope:", err)
			os.Exit(2)
		}
		if env.Type == "shutdown" {
			return
		}
		if env.Type != "batch" {
			// Forward-compat: ignore unknown envelopes.
			continue
		}

		rows := make([]Row, len(env.Rows))
		for i, r := range env.Rows {
			rows[i] = Row{
				Seq:  r.Seq,
				Data: r.Data,
				Meta: RowMeta{DryRun: r.Meta.DryRun, RowIndex: r.Meta.RowIndex},
			}
		}
		ctx := Context{
			Config:         init.Config,
			HandlerVersion: cfg.handlerVersion,
			RunID:          init.RunID,
		}

		results := safeInvokeBatch(fn, rows, ctx)

		if len(results) != len(rows) {
			fmt.Fprintf(os.Stderr,
				"rowforge-handler: BatchHandleFunc returned %d results for %d rows; exiting\n",
				len(results), len(rows))
			os.Exit(2)
		}

		entries := make([]map[string]any, len(results))
		for i, r := range results {
			if r.success {
				entries[i] = map[string]any{"kind": "result", "data": r.data}
			} else {
				entry := map[string]any{"kind": "error", "code": r.code, "message": r.message}
				if r.data != nil {
					entry["data"] = r.data
				}
				entries[i] = entry
			}
		}
		emit(map[string]any{"type": "batch_result", "results": entries})
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "rowforge-handler: scanner:", err)
		os.Exit(2)
	}
}

// safeInvokeBatch runs fn, recovering panics. On panic, logs the stack to
// stderr and exits(2). This causes rowforge to observe HandlerExit, which
// the pool converts to WORKER_CRASH or WORKER_CRASH_UNSAFE per the
// manifest's idempotent flag.
func safeInvokeBatch(fn BatchHandleFunc, rows []Row, ctx Context) []Result {
	defer func() {
		if r := recover(); r != nil {
			fmt.Fprintf(os.Stderr, "rowforge-handler: batch panic: %v\n%s\n", r, debug.Stack())
			os.Exit(2)
		}
	}()
	return fn(rows, ctx)
}
