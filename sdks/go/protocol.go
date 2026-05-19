package handler

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"runtime/debug"
)

// Run reads JSON-Lines from stdin, dispatches to fn, writes results to stdout.
// Returns when stdin closes or rowforge sends shutdown. Calls os.Exit(2) on
// protocol-level errors (malformed JSON, missing init, etc.).
func Run(fn HandleFunc, opts ...Option) {
	cfg := config{handlerVersion: "0.0.0"}
	for _, o := range opts {
		o(&cfg)
	}

	scanner := bufio.NewScanner(os.Stdin)
	// Tolerate big rows (default 64KB buffer is too small for verbose CSV→JSON).
	scanner.Buffer(make([]byte, 1024*1024), 16*1024*1024)

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

	// 3. Row loop.
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var msg struct {
			Type string         `json:"type"`
			Seq  uint64         `json:"seq"`
			Data map[string]any `json:"data"`
			Meta struct {
				DryRun   bool   `json:"dry_run"`
				RowIndex uint64 `json:"row_index"`
			} `json:"meta"`
		}
		if err := json.Unmarshal(line, &msg); err != nil {
			fmt.Fprintln(os.Stderr, "rowforge-handler: malformed message:", err)
			os.Exit(2)
		}

		if msg.Type == "shutdown" {
			return
		}
		if msg.Type != "row" {
			// Forward-compat: ignore unknown envelopes.
			continue
		}

		ctx := Context{
			DryRun:         msg.Meta.DryRun,
			RowIndex:       msg.Meta.RowIndex,
			Config:         init.Config,
			HandlerVersion: cfg.handlerVersion,
			RunID:          init.RunID,
		}

		result, err := safeInvoke(fn, msg.Data, ctx)
		if err != nil {
			var herr *HandlerError
			if errors.As(err, &herr) {
				env := map[string]any{
					"type":    "error",
					"seq":     msg.Seq,
					"code":    herr.Code,
					"message": herr.Message,
				}
				if herr.Data != nil {
					env["data"] = herr.Data
				}
				emit(env)
				continue
			}
			// UNCAUGHT.
			emit(map[string]any{
				"type":    "error",
				"seq":     msg.Seq,
				"code":    "UNCAUGHT",
				"message": err.Error(),
			})
			continue
		}
		if result == nil {
			emit(map[string]any{
				"type":    "error",
				"seq":     msg.Seq,
				"code":    "BAD_RETURN",
				"message": "HandleFunc returned a nil result with no error",
			})
			continue
		}
		emit(map[string]any{"type": "result", "seq": msg.Seq, "data": result})
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "rowforge-handler: scanner:", err)
		os.Exit(2)
	}
}

// safeInvoke runs fn, recovering panics and converting them into errors.
// This implements the spec §6.4 point 5 (panic → error code=UNCAUGHT).
func safeInvoke(fn HandleFunc, row map[string]any, ctx Context) (result map[string]any, err error) {
	defer func() {
		if r := recover(); r != nil {
			fmt.Fprintf(os.Stderr, "rowforge-handler: panic: %v\n%s\n", r, debug.Stack())
			err = fmt.Errorf("panic: %v", r)
		}
	}()
	return fn(row, ctx)
}
