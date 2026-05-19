// Package main is a minimal rowforge handler in Go: uppercases the "name"
// field of each input row. Pairs with examples/handlers/python3-uppercase
// to demonstrate the same logic in two SDKs.
//
// Build: `go build -o handler handler.go` (rowforge auto-builds via the
//        manifest's entry.build setting; this is for local testing only).
// Run:   `rowforge exec run <exec_id> --handler examples/handlers/golang-uppercase`
package main

import (
	"strings"

	handler "github.com/lemotw/rowforge/sdks/go"
)

func main() {
	handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
		name, ok := row["name"].(string)
		if !ok || name == "" {
			return nil, handler.Error("EMPTY_NAME", "input 'name' field is missing or empty")
		}
		return map[string]any{"name_upper": strings.ToUpper(name)}, nil
	}, handler.WithVersion("0.1.0"))
}
