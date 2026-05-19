// Package handler is the rowforge handler SDK for Go.
//
// Write a HandleFunc, pass it to Run, the SDK owns the JSON-Lines stdio loop.
//
//	package main
//
//	import (
//	    handler "github.com/lemotw/rowforge/sdks/go"
//	)
//
//	func main() {
//	    handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
//	        email, _ := row["email"].(string)
//	        if !strings.Contains(email, "@") {
//	            return nil, handler.Error("INVALID_EMAIL", "missing @")
//	        }
//	        return map[string]any{"domain": strings.SplitN(email, "@", 2)[1]}, nil
//	    }, handler.WithVersion("0.1.0"))
//	}
//
// See the protocol spec at docs/superpowers/specs/2026-05-10-rowforge-design.md §6.3.
package handler

// HandleFunc processes one row. Return a non-nil result map on success.
// To surface a protocol-level error, return (nil, Error(code, msg)).
// Any other error is treated as UNCAUGHT.
type HandleFunc func(row map[string]any, ctx Context) (map[string]any, error)

// Context is per-row metadata passed to HandleFunc.
type Context struct {
	DryRun         bool
	RowIndex       uint64
	Config         map[string]any
	HandlerVersion string
	RunID          string
}

// HandlerError surfaces a protocol-level error envelope.
// Construct via Error(code, msg) or ErrorWithData(code, msg, data).
type HandlerError struct {
	Code    string
	Message string
	// Data is the optional handler-supplied domain payload. Keys declared
	// under manifest.schema.failed_output materialize as columns in
	// failed.csv. Nil ≡ omitted from the wire envelope (skip on encode).
	Data map[string]any
}

func (e *HandlerError) Error() string { return e.Code + ": " + e.Message }

// Error constructs a HandlerError. Use it as the error return from HandleFunc.
func Error(code, message string) error {
	return &HandlerError{Code: code, Message: message}
}

// ErrorWithData constructs a HandlerError carrying domain columns to surface
// in failed.csv (per manifest schema.failed_output). Data is OPTIONAL — nil
// is byte-equivalent to Error(code, message) on the wire.
func ErrorWithData(code, message string, data map[string]any) error {
	return &HandlerError{Code: code, Message: message, Data: data}
}

// Option configures Run.
type Option func(*config)

type config struct {
	handlerVersion string
}

// WithVersion sets the handler_version emitted in the ready envelope.
// Default: "0.0.0".
func WithVersion(v string) Option {
	return func(c *config) { c.handlerVersion = v }
}

// Row is one input element of a batch. Used by RunBatch handlers.
//
// Seq is exposed for handler-side logging convenience only. It is NOT
// re-emitted on output: batch_result entries are positional (output[i] is the
// result for input rows[i]).
type Row struct {
	Seq  uint64
	Data map[string]any
	Meta RowMeta
}

// RowMeta carries per-row metadata inside a batch.
type RowMeta struct {
	DryRun   bool
	RowIndex uint64
}

// Result is one positional element of a batch's output. Use Success(),
// Failure(), or FailureWithData() to construct; the underlying fields are
// intentionally unexported.
//
// The `data` field carries two distinct payloads depending on `success`:
//   - success == true:  handler output columns (mapped via schema.output)
//   - success == false: domain context columns (mapped via schema.failed_output)
//
// They're disjoint by construction — only one branch ever reads `data`.
type Result struct {
	success bool
	data    map[string]any
	code    string
	message string
}

// Success constructs a successful Result carrying the given data.
func Success(data map[string]any) Result {
	return Result{success: true, data: data}
}

// Failure constructs an error Result with the given protocol code and message.
func Failure(code, message string) Result {
	return Result{success: false, code: code, message: message}
}

// FailureWithData constructs an error Result carrying domain columns to
// surface in failed.csv (per manifest schema.failed_output). Data is
// OPTIONAL — nil is byte-equivalent to Failure(code, message) on the wire.
func FailureWithData(code, message string, data map[string]any) Result {
	return Result{success: false, code: code, message: message, data: data}
}

// BatchHandleFunc processes a batch of rows. It MUST return exactly len(rows)
// Results, indexed positionally (output[i] is the result for input rows[i]).
// Returning a different length is a fatal protocol error — the SDK will log
// to stderr and exit(2). No seq field is emitted on output; position alone
// determines attribution.
type BatchHandleFunc func(rows []Row, ctx Context) []Result
