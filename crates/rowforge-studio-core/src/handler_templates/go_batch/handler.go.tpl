// Package main is a rowforge batch-mode handler scaffolded by Studio.
//
// Protocol (JSON-Lines over stdin/stdout):
//   1. Read init envelope:         {"type":"init","run_id":"...","config":{...},"columns":[...]}
//   2. Write ready envelope:       {"type":"ready","handler_version":"0.1.0"}
//   3. Loop:
//      - Read batch envelope:      {"type":"batch","rows":[{"seq":N,"data":{...},"meta":{...}}, ...]}
//      - Write batch_result:       {"type":"batch_result","results":[{"kind":"result","data":{...}}, ...]}
//        Result entries are positional: results[i] maps to rows[i].
//        Error entry: {"kind":"error","code":"...","message":"..."}
//      - On {"type":"shutdown"}: exit cleanly
//   4. Exit on EOF.
//
// Input column:  {{primary_field}}
// Output column: echoed_{{primary_field}}
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1024*1024), 64*1024*1024)

	out := bufio.NewWriter(os.Stdout)
	emit := func(v map[string]any) {
		b, err := json.Marshal(v)
		if err != nil {
			fmt.Fprintln(os.Stderr, "handler: marshal error:", err)
			os.Exit(2)
		}
		out.Write(b)
		out.WriteString("\n")
		out.Flush()
	}

	// 1. Read init.
	if !scanner.Scan() {
		return // EOF before init — caller closed early
	}
	var init struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal(scanner.Bytes(), &init); err != nil || init.Type != "init" {
		fmt.Fprintln(os.Stderr, "handler: expected init envelope")
		os.Exit(2)
	}

	// 2. Ready.
	emit(map[string]any{"type": "ready", "handler_version": "0.1.0"})

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
			} `json:"rows"`
		}
		if err := json.Unmarshal(line, &env); err != nil {
			fmt.Fprintln(os.Stderr, "handler: malformed envelope:", err)
			os.Exit(2)
		}
		if env.Type == "shutdown" {
			return
		}
		if env.Type != "batch" {
			continue // forward-compat: ignore unknown envelopes
		}

		results := make([]map[string]any, len(env.Rows))
		for i, r := range env.Rows {
			value, _ := r.Data["{{primary_field}}"].(string)
			if value == "" {
				results[i] = map[string]any{
					"kind":    "error",
					"code":    "MISSING_{{primary_field}}",
					"message": "row has no '{{primary_field}}' field",
				}
				continue
			}

			// TODO: replace this echo with your real handler logic.
			results[i] = map[string]any{
				"kind": "result",
				"data": map[string]any{
					"{{primary_field}}":        value,
					"echoed_{{primary_field}}": value,
				},
			}
		}
		emit(map[string]any{"type": "batch_result", "results": results})
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "handler: scanner error:", err)
		os.Exit(2)
	}
}
