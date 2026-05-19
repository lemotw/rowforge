package handler_test

import (
	"strings"
	"testing"
)

// Note: writeHandler and runHandler live in run_test.go; both files share
// the handler_test package so the helpers are visible here.

const batchHappyMain = `package main

import (
	"strings"
	handler "github.com/lemotw/rowforge/sdks/go"
)

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		out := make([]handler.Result, len(rows))
		for i, r := range rows {
			name, _ := r.Data["name"].(string)
			out[i] = handler.Success(map[string]any{"upper": strings.ToUpper(name)})
		}
		return out
	}, handler.WithVersion("1.2.3"))
}
`

func TestBatchHappyPath(t *testing.T) {
	dir := writeHandler(t, batchHappyMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"name"}},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{"name": "alice"}, "meta": map[string]any{"row_index": 0}},
			{"seq": 1, "data": map[string]any{"name": "bob"}, "meta": map[string]any{"row_index": 1}},
			{"seq": 2, "data": map[string]any{"name": "carol"}, "meta": map[string]any{"row_index": 2}},
		}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	if len(envs) != 2 {
		t.Fatalf("expected 2 envelopes (ready, batch_result), got %d: %+v", len(envs), envs)
	}
	if envs[0]["type"] != "ready" || envs[0]["handler_version"] != "1.2.3" {
		t.Fatalf("ready mismatch: %+v", envs[0])
	}
	if envs[1]["type"] != "batch_result" {
		t.Fatalf("expected batch_result, got %+v", envs[1])
	}
	results, ok := envs[1]["results"].([]any)
	if !ok {
		t.Fatalf("results is not an array: %T %+v", envs[1]["results"], envs[1]["results"])
	}
	if len(results) != 3 {
		t.Fatalf("expected 3 results, got %d", len(results))
	}
	expects := []string{"ALICE", "BOB", "CAROL"}
	for i, entry := range results {
		e := entry.(map[string]any)
		if e["kind"] != "result" {
			t.Fatalf("results[%d].kind != result: %+v", i, e)
		}
		if _, hasSeq := e["seq"]; hasSeq {
			t.Fatalf("results[%d] should not carry seq: %+v", i, e)
		}
		got := e["data"].(map[string]any)["upper"]
		if got != expects[i] {
			t.Fatalf("results[%d].data.upper = %v, want %s", i, got, expects[i])
		}
	}
}

const batchPanicMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		panic("kaboom")
	})
}
`

func TestBatchPanic(t *testing.T) {
	dir := writeHandler(t, batchPanicMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{"x": 1}, "meta": map[string]any{}},
		}},
	})
	// `go run .` reports the child's non-zero exit as code 1 (printing
	// "exit status 2" to its own stderr). Either way it must be non-zero
	// and the panic diagnostic must reach stderr.
	if code == 0 {
		t.Fatalf("expected non-zero exit on panic, got 0\nstderr: %s", stderr)
	}
	for _, env := range envs {
		if env["type"] == "batch_result" {
			t.Fatalf("batch_result should not be emitted on panic: %+v", env)
		}
	}
	if !strings.Contains(stderr, "kaboom") {
		t.Fatalf("expected stack with 'kaboom' on stderr, got: %s", stderr)
	}
	if !strings.Contains(stderr, "exit status 2") && !strings.Contains(stderr, "batch panic") {
		t.Fatalf("expected exit-status-2 diagnostic, got: %s", stderr)
	}
}

const batchWrongLengthMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		// Intentionally return one fewer result.
		out := make([]handler.Result, 0, len(rows)-1)
		for i := 0; i < len(rows)-1; i++ {
			out = append(out, handler.Success(map[string]any{"i": i}))
		}
		return out
	})
}
`

func TestBatchWrongLength(t *testing.T) {
	dir := writeHandler(t, batchWrongLengthMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{}, "meta": map[string]any{}},
			{"seq": 1, "data": map[string]any{}, "meta": map[string]any{}},
			{"seq": 2, "data": map[string]any{}, "meta": map[string]any{}},
		}},
	})
	// `go run .` may surface the child's exit(2) as its own exit 1.
	// Either way: must be non-zero and emit the length-mismatch diagnostic.
	if code == 0 {
		t.Fatalf("expected non-zero exit on length mismatch, got 0\nstderr: %s", stderr)
	}
	for _, env := range envs {
		if env["type"] == "batch_result" {
			t.Fatalf("batch_result should not be emitted on length mismatch: %+v", env)
		}
	}
	if !strings.Contains(stderr, "2 results for 3 rows") {
		t.Fatalf("expected length-mismatch diagnostic on stderr, got: %s", stderr)
	}
}

const batchMixedMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		out := make([]handler.Result, len(rows))
		for i, r := range rows {
			if v, _ := r.Data["bad"].(bool); v {
				out[i] = handler.Failure("BAD_INPUT", "input flagged bad")
			} else {
				out[i] = handler.Success(map[string]any{"ok": true})
			}
		}
		return out
	})
}
`

func TestBatchMixedSuccessError(t *testing.T) {
	dir := writeHandler(t, batchMixedMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{"bad": false}, "meta": map[string]any{}},
			{"seq": 1, "data": map[string]any{"bad": true}, "meta": map[string]any{}},
			{"seq": 2, "data": map[string]any{"bad": false}, "meta": map[string]any{}},
		}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	results := envs[1]["results"].([]any)
	if len(results) != 3 {
		t.Fatalf("expected 3 results, got %d", len(results))
	}
	e0 := results[0].(map[string]any)
	e1 := results[1].(map[string]any)
	e2 := results[2].(map[string]any)
	if e0["kind"] != "result" || e0["data"].(map[string]any)["ok"] != true {
		t.Fatalf("results[0] wrong: %+v", e0)
	}
	if e1["kind"] != "error" || e1["code"] != "BAD_INPUT" || e1["message"] != "input flagged bad" {
		t.Fatalf("results[1] wrong: %+v", e1)
	}
	if _, hasData := e1["data"]; hasData {
		t.Fatalf("error entry should not include data field: %+v", e1)
	}
	if e2["kind"] != "result" || e2["data"].(map[string]any)["ok"] != true {
		t.Fatalf("results[2] wrong: %+v", e2)
	}
}

const batchShutdownMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		out := make([]handler.Result, len(rows))
		for i := range rows {
			out[i] = handler.Success(map[string]any{})
		}
		return out
	})
}
`

func TestBatchShutdownExitsClean(t *testing.T) {
	dir := writeHandler(t, batchShutdownMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("expected clean exit on shutdown, got %d\nstderr: %s", code, stderr)
	}
	if len(envs) != 1 || envs[0]["type"] != "ready" {
		t.Fatalf("expected only ready envelope, got: %+v", envs)
	}
}

func TestBatchUnknownEnvelopeIgnored(t *testing.T) {
	dir := writeHandler(t, batchShutdownMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{}},
		{"type": "future_envelope", "anything": "goes"},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{}, "meta": map[string]any{}},
		}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit on forward-compat ignore: %d\nstderr: %s", code, stderr)
	}
	// Expect: ready, batch_result. Unknown envelope must not produce an output.
	if len(envs) != 2 {
		t.Fatalf("expected 2 envelopes after ignoring unknown, got %d: %+v", len(envs), envs)
	}
	if envs[0]["type"] != "ready" {
		t.Fatalf("envs[0] should be ready: %+v", envs[0])
	}
	if envs[1]["type"] != "batch_result" {
		t.Fatalf("envs[1] should be batch_result: %+v", envs[1])
	}
}

const batchFailureWithDataMain = `package main

import handler "github.com/lemotw/rowforge/sdks/go"

func main() {
	handler.RunBatch(func(rows []handler.Row, _ handler.Context) []handler.Result {
		out := make([]handler.Result, len(rows))
		for i, r := range rows {
			billid, _ := r.Data["billid"].(string)
			out[i] = handler.FailureWithData("EMPTY_FAIL_FIELDS", "demo",
				map[string]any{"billid": billid})
		}
		return out
	})
}
`

// FailureWithData attaches a domain payload that surfaces in failed.csv via
// manifest.schema.failed_output. On the wire each error entry MUST carry a
// `data` field; Failure() (no data) MUST NOT — assertion mirrored below.
func TestBatchFailureWithDataEmitsDataField(t *testing.T) {
	dir := writeHandler(t, batchFailureWithDataMain)
	envs, stderr, code := runHandler(t, dir, []map[string]any{
		{"type": "init", "run_id": "r1", "config": map[string]any{}, "columns": []string{"billid"}},
		{"type": "batch", "rows": []map[string]any{
			{"seq": 0, "data": map[string]any{"billid": "B1"}, "meta": map[string]any{}},
			{"seq": 1, "data": map[string]any{"billid": "B2"}, "meta": map[string]any{}},
		}},
		{"type": "shutdown"},
	})
	if code != 0 {
		t.Fatalf("nonzero exit: %d\nstderr: %s", code, stderr)
	}
	results := envs[1]["results"].([]any)
	if len(results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(results))
	}
	for i, want := range []string{"B1", "B2"} {
		e := results[i].(map[string]any)
		if e["kind"] != "error" || e["code"] != "EMPTY_FAIL_FIELDS" {
			t.Fatalf("results[%d] wrong: %+v", i, e)
		}
		dataField, ok := e["data"].(map[string]any)
		if !ok {
			t.Fatalf("results[%d] missing data field: %+v", i, e)
		}
		if dataField["billid"] != want {
			t.Fatalf("results[%d] data.billid = %v, want %s", i, dataField["billid"], want)
		}
	}
}
