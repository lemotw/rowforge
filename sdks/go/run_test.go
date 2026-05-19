package handler_test

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// writeHandler writes a tiny main.go using the SDK to a temp dir, returns the dir.
// The dir is set up so `go run .` works (go.mod with a replace pointing at the
// local SDK).
func writeHandler(t *testing.T, body string) string {
	t.Helper()
	dir := t.TempDir()

	// Locate the SDK root: this test file lives in sdks/go/, so up-one is the
	// directory we want for the replace directive.
	sdkRoot, err := filepath.Abs(".")
	if err != nil {
		t.Fatal(err)
	}

	gomod := `module testharness

go 1.22

require github.com/lemotw/rowforge/sdks/go v0.0.0

replace github.com/lemotw/rowforge/sdks/go => ` + sdkRoot + `
`
	if err := os.WriteFile(filepath.Join(dir, "go.mod"), []byte(gomod), 0644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "main.go"), []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	return dir
}

func runHandler(t *testing.T, dir string, stdinMsgs []map[string]any) (envs []map[string]any, stderr string, exitCode int) {
	t.Helper()
	var stdinBuf bytes.Buffer
	enc := json.NewEncoder(&stdinBuf)
	for _, m := range stdinMsgs {
		if err := enc.Encode(m); err != nil {
			t.Fatal(err)
		}
	}
	cmd := exec.Command("go", "run", ".")
	cmd.Dir = dir
	cmd.Stdin = &stdinBuf
	var stdout, stderrBuf bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderrBuf
	err := cmd.Run()
	if exitErr, ok := err.(*exec.ExitError); ok {
		exitCode = exitErr.ExitCode()
	} else if err != nil {
		t.Fatalf("exec failed: %v\nstderr: %s", err, stderrBuf.String())
	}
	stderr = stderrBuf.String()
	for _, line := range strings.Split(stdout.String(), "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		var m map[string]any
		if err := json.Unmarshal([]byte(line), &m); err != nil {
			t.Fatalf("bad envelope %q: %v", line, err)
		}
		envs = append(envs, m)
	}
	return envs, stderr, exitCode
}

const happyMainFull = `package main

import (
	"strings"
	handler "github.com/lemotw/rowforge/sdks/go"
)

func main() {
	handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
		name, _ := row["name"].(string)
		return map[string]any{"upper": strings.ToUpper(name)}, nil
	}, handler.WithVersion("9.9.9"))
}
`

func TestHappyPath(t *testing.T) {
	dir := writeHandler(t, happyMainFull)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"name"}},
		{"type": "row", "seq": 0, "data": map[string]any{"name": "alice"}, "meta": map[string]any{"row_index": 0}},
		{"type": "row", "seq": 1, "data": map[string]any{"name": "bob"}, "meta": map[string]any{"row_index": 1}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if envs[0]["type"] != "ready" || envs[0]["handler_version"] != "9.9.9" {
		t.Fatalf("ready mismatch: %+v", envs[0])
	}
	if envs[1]["type"] != "result" || envs[1]["data"].(map[string]any)["upper"] != "ALICE" {
		t.Fatalf("first result mismatch: %+v", envs[1])
	}
	if envs[2]["data"].(map[string]any)["upper"] != "BOB" {
		t.Fatalf("second result mismatch: %+v", envs[2])
	}
}

const handlerErrorMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
		name, _ := row["name"].(string)
		if name == "" {
			return nil, handler.Error("EMPTY_NAME", "name is empty")
		}
		return map[string]any{"upper": name}, nil
	})
}
`

func TestHandlerError(t *testing.T) {
	dir := writeHandler(t, handlerErrorMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"name"}},
		{"type": "row", "seq": 5, "data": map[string]any{"name": ""}, "meta": map[string]any{}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if envs[1]["type"] != "error" || envs[1]["code"] != "EMPTY_NAME" {
		t.Fatalf("expected EMPTY_NAME error, got %+v", envs[1])
	}
}

const panicMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
		panic("boom")
	})
}
`

func TestPanicBecomesUncaught(t *testing.T) {
	dir := writeHandler(t, panicMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "row", "seq": 0, "data": map[string]any{}, "meta": map[string]any{}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if envs[1]["code"] != "UNCAUGHT" {
		t.Fatalf("expected UNCAUGHT, got %+v", envs[1])
	}
	if !strings.Contains(stderr, "boom") {
		t.Fatalf("stack trace not on stderr: %s", stderr)
	}
}

const ctxMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.Run(func(_ map[string]any, ctx handler.Context) (map[string]any, error) {
		return map[string]any{
			"dry_run":   ctx.DryRun,
			"row_index": ctx.RowIndex,
			"cfg_x":     ctx.Config["x"],
		}, nil
	}, handler.WithVersion("0.7.0"))
}
`

func TestContextPropagates(t *testing.T) {
	dir := writeHandler(t, ctxMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{"x": float64(42)}, "columns": []string{}},
		{"type": "row", "seq": 0, "data": map[string]any{}, "meta": map[string]any{"dry_run": true, "row_index": 7}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero: %d\nstderr: %s", code, stderr)
	}
	d := envs[1]["data"].(map[string]any)
	if d["dry_run"] != true || d["row_index"].(float64) != 7 || d["cfg_x"].(float64) != 42 {
		t.Fatalf("ctx propagation wrong: %+v", d)
	}
}

const handlerErrorWithDataMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
		billid, _ := row["billid"].(string)
		return nil, handler.ErrorWithData("DEMO_FAIL", "always fails",
			map[string]any{"billid": billid})
	})
}
`

// Row-mode counterpart to TestBatchFailureWithDataEmitsDataField: an error
// envelope MUST carry the `data` field when ErrorWithData is used, and MUST
// be byte-identical to Error(...) when constructed without data.
func TestHandlerErrorWithDataEmitsDataField(t *testing.T) {
	dir := writeHandler(t, handlerErrorWithDataMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"billid"}},
		{"type": "row", "seq": 0, "data": map[string]any{"billid": "B7"}, "meta": map[string]any{}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if envs[1]["type"] != "error" || envs[1]["code"] != "DEMO_FAIL" {
		t.Fatalf("expected DEMO_FAIL error, got %+v", envs[1])
	}
	d, ok := envs[1]["data"].(map[string]any)
	if !ok {
		t.Fatalf("expected data field on error envelope, got %+v", envs[1])
	}
	if d["billid"] != "B7" {
		t.Fatalf("expected billid=B7, got %v", d["billid"])
	}
}

func TestHandlerErrorWithoutDataOmitsDataField(t *testing.T) {
	// Sanity: old-style Error(...) callers stay byte-identical (no `data`
	// key on the wire). Mirrors TestHandlerError but asserts absence.
	dir := writeHandler(t, handlerErrorMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"name"}},
		{"type": "row", "seq": 5, "data": map[string]any{"name": ""}, "meta": map[string]any{}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if _, hasData := envs[1]["data"]; hasData {
		t.Fatalf("Error() must not emit data field: %+v", envs[1])
	}
}
