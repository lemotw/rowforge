# rowforge

Streaming batch processor for tabular data via per-row handlers (any language)
speaking JSON-Lines over stdio.

Feed it a `.csv` / `.jsonl` / `.ndjson` file plus a handler that knows what to do
with one row at a time, and rowforge runs the rows through a worker pool with
durable per-row outcomes, automatic retry semantics, stall detection, and a
single-source-of-truth event log (`outcomes.jsonl`) per attempt.

## Build

```bash
cargo build --release
./target/release/rowforge --help
```

Tests:

```bash
cargo test --workspace        # 155 passed, 1 ignored
```

## Quick start

```bash
# 1. Build the demo handler binary (handler may auto-build via entry.build;
#    this is just to confirm it compiles)
cd examples/handlers/golang-uppercase && go build -o handler handler.go && cd ../../..

# 2. Make an input file
printf 'name\nalice\nbob\ncarol\n' > /tmp/names.csv

# 3. Register the execution (snapshots input + handler)
./target/release/rowforge exec start --csv /tmp/names.csv --name demo
EXEC=$(./target/release/rowforge exec list | head -2 | tail -1 | awk '{print $1}')

# 4. Run the handler against the execution
./target/release/rowforge exec run $EXEC --handler examples/handlers/golang-uppercase

# 5. Export results
./target/release/rowforge exec export $EXEC --output-dir /tmp/exp
cat /tmp/exp/success.csv
```

JSONL input works the same way — auto-detected by extension:

```bash
printf '{"name":"alice"}\n{"name":"bob"}\n' > /tmp/names.jsonl
./target/release/rowforge exec start --csv /tmp/names.jsonl --name demo-jsonl
```

## Execution model

```
Execution           ← logical job, owns one input snapshot
  └─ HandlerInstance ← pinned handler version + manifest hash
       └─ Attempt    ← one run of the pool (outcomes.jsonl is its truth)
            ├─ outcomes.jsonl   ← append-only event log
            ├─ meta.json        ← state + stats + abort reason
            └─ input.{csv|jsonl}
```

Resolution across attempts (success-absorbing, idempotent retry):

| Per-row state | Meaning |
|---|---|
| `Resolved` | succeeded in some attempt; canonical |
| `FailedLast` / `CrashedLast` / `CancelledLast` / `TooLarge` | failed in latest attempt |
| `NeverAttempted` | never dispatched |

## `exec run` retry policy

```
(default)         → dispatch only NeverAttempted rows (one-shot)
--retry-failed    → dispatch only failures (FailedLast/CrashedLast/...)
--force           → dispatch every row (full reset)
--sample N        → pair with any of the above to limit to N rows
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Run completed; all dispatched rows OK |
| 1 | Run completed but some rows failed |
| 2 | Run aborted (stalled / cancelled / startup-fail) |
| 3 | `exec export --strict` saw an incomplete execution |

## Writing a handler

A handler is any executable that:

1. Reads JSON-Lines from stdin, one envelope per line.
2. Sends `{"type":"ready","handler_version":"X"}` once after init.
3. For each `{"type":"row","seq":N,"data":{...},"meta":{...}}` input, replies
   `{"type":"result","seq":N,"data":{...}}` or `{"type":"error","seq":N,"code":"...","message":"..."}`.
4. Exits cleanly on `{"type":"shutdown"}`.

For batch mode the envelope is `{"type":"batch","rows":[...]}` and the reply
is `{"type":"batch_result","results":[...]}` (positional).

Minimal manifest (`rowforge.yaml`):

```yaml
name: my-handler
version: 0.1.0
entry:
  cmd: ["./bin/my-handler"]
  startup_timeout_ms: 10000

runtime:
  mode: row              # or batch
  idempotent: true

required_input: [email]  # fail-fast if any of these columns are missing
```

Working examples:

- [`examples/handlers/golang-uppercase`](examples/handlers/golang-uppercase) — Go via SDK
- [`examples/handlers/python3-uppercase`](examples/handlers/python3-uppercase) — Python via SDK
- [`examples/handlers/raw-protocol-python`](examples/handlers/raw-protocol-python) — Python without SDK (just stdin/stdout JSON-Lines)

## Spec

The formal specification lives under [`docs/spec/2026-05-18-rowforge/`](docs/spec/2026-05-18-rowforge/) — six parts covering overview, execution model, runtime pipeline, data layout, CLI surface, and base conformance.

## License

MIT. See [LICENSE](LICENSE).
