// Package main is a rowforge row-mode handler scaffolded by Studio.
//
// Protocol (JSON-Lines over stdin/stdout):
//   1. Read init envelope:   {"type":"init","run_id":"...","config":{...},"columns":[...]}
//   2. Write ready envelope: {"type":"ready","handler_version":"0.1.0"}
//   3. Loop:
//      - Read row envelope:    {"type":"row","seq":N,"data":{...},"meta":{...}}
//      - Write result:         {"type":"result","seq":N,"data":{...}}
//      - Or write error:       {"type":"error","seq":N,"code":"...","message":"..."}
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
	scanner.Buffer(make([]byte, 1024*1024), 16*1024*1024)

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
		}
		if err := json.Unmarshal(line, &msg); err != nil {
			fmt.Fprintln(os.Stderr, "handler: malformed message:", err)
			os.Exit(2)
		}
		if msg.Type == "shutdown" {
			return
		}
		if msg.Type != "row" {
			continue // forward-compat: ignore unknown envelopes
		}

		value, _ := msg.Data["{{primary_field}}"].(string)
		if value == "" {
			emit(map[string]any{
				"type":    "error",
				"seq":     msg.Seq,
				"code":    "MISSING_{{primary_field}}",
				"message": "row has no '{{primary_field}}' field",
			})
			continue
		}

		// TODO: replace this echo with your real handler logic.
		emit(map[string]any{
			"type": "result",
			"seq":  msg.Seq,
			"data": map[string]any{
				"{{primary_field}}":        value,
				"echoed_{{primary_field}}": value,
			},
		})
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "handler: scanner error:", err)
		os.Exit(2)
	}
}
