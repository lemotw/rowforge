# Part V — External Surface

> Corresponds to §10. For the directory index see [README.md](README.md).

---

## 10. CLI interface

```
rowforge <subcommand> ...
```

`ROWFORGE_HOME` overrides the data root (default `~/.rowforge`).

### 10.1 `exec` — Execution lifecycle

#### `exec start --csv <path> [flags]`

Creates a new Execution.

| Flag | Required | Meaning |
|---|---|---|
| `--csv <path>` | Yes | Source file (CSV or JSONL). Copied into the execution directory. |
| `--name <N>` | No | Human-readable label. Shown in `exec list` / `exec show`. |
| `--csv-id <ID>` | No | Logical ID for cross-referencing. Default `csv_unregistered`. |
| `--handler-instance-id <HI>` | No | Pre-bind a HandlerInstance. |

Prints: new `e_<ULID>`, directory path, row count, input hash.

#### `exec list`

Prints all executions in reverse chronological order. Columns: `ID STATE ROWS CREATED NAME`.

#### `exec show <id>`

Prints all fields of a single Execution.

#### `exec set-state <id> <state> [--reason R]`

Manually transitions state. `<state> ∈ {open, iterating, settled, closed, abandoned}`. `--reason` is **required** when `abandoned`. CLOSED and ABANDONED reject subsequent `exec run`.

#### `exec run <id> --handler <dir> [flags]`

Starts a new Attempt.

| Flag | Default | Meaning |
|---|---|---|
| `--handler <dir>` | required | Handler directory (containing `rowforge.yaml`). |
| `--format csv\|jsonl` | (detected) | Override input format detection. |
| `--sample <N>` | — | Dispatch at most N rows. Counted after skips. |
| `--dry-run` | false | Sets `meta.dry_run = true` on each dispatched row. |
| `--retry-failed` | false | Dispatch failure-class rows only (`FailedLast` / `CrashedLast` / `CancelledLast` / `TooLarge`). Skips Resolved and NeverAttempted. Mutually exclusive with `--force`. |
| `--force` | false | Re-dispatch everything (empty skip_seqs). Does not alter canonical SUCCESS (R4). Mutually exclusive with `--retry-failed`. |
| `--workers <N>` | 1 | Worker pool size. Forced to 1 when `runtime.stateful: true`. |
| `--config K=V` | — | Repeatable. Overrides `manifest.config[K].default`. Value is parsed as JSON first; falls back to string on failure. |
| `--field-map K=V` | — | Repeatable. Renames input columns at the boundary. `--field-map email=user_email` maps `data["email"]` to the input's `user_email` column. |
| `--fsync-outcomes` | false | Call `fsync` after each append to `outcomes.jsonl`. |

Behavior summary:

1. Load manifest; compute `manifest_hash`; upsert HandlerInstance.
2. `compute_resolution(<id>)` → build `skip_seqs`:
   - Default (one-shot): all already-Attempted seqs
   - `--retry-failed`: all non-failure-class seqs
   - `--force`: empty set
3. INSERT new Attempt row (state `running`); create attempt directory.
4. Run dispatch pipeline (§4, see [part-3-runtime.md](part-3-runtime.md)), writing to `<attempt>/outcomes.jsonl`.
5. UPDATE attempt to `completed` / `aborted`; advance Execution state if needed.

#### `exec attempts <id>`

Lists all Attempts of an Execution. Columns: `ATTEMPT_ID STATE OK FAILED SOURCE STARTED DIR`.

#### `exec attempt <attempt_id>`

Prints full details of a single Attempt.

#### `exec export <id> [flags]`

Computes resolution and writes the merged bundle (§9, see [part-4-data.md](part-4-data.md)).

| Flag | Default | Meaning |
|---|---|---|
| `--format csv\|jsonl\|both` | `csv` | Output format. |
| `--output-dir DIR` | (timestamp) | Write directly into DIR; skip the `<timestamp>/` subdirectory. |
| `--strict` | false | Exit 3 and write no files when `fully_processed == false`. |

### 10.2 `pack` — Cross-platform compile + bundle

```
rowforge pack --target <TARGET> --handler <DIR> -o <BUNDLE.zip> [additional --handler ...]
```

Cross-compiles the rowforge binary for `<TARGET>` and zips all specified handler directories together. Independent of the Execution model.

Supported targets: `darwin-arm64`, `linux-x86_64`, `linux-aarch64`. Requires `cargo-zigbuild` (`brew install zig && cargo install cargo-zigbuild`).

### 10.3 `run` — Legacy single-shot execution (compatibility)

```
rowforge run --handler <DIR> --input <CSV> --output-dir <DIR> [flags]
```

Command from the pre-execution-model era. Retained for single-shot batches and CI scripts. **Does not integrate with the Execution registry** — writes per-attempt output directly to `<DIR>`, bypassing SQLite entirely. Not recommended for new work; use `exec start` + `exec run` instead.

### 10.4 Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. All dispatched rows succeeded, or no rows were dispatched. |
| 1 | Partial failure (`failed_count > 0`). |
| 2 | Run aborted (startup timeout, all workers failed, or fatal IO). |
| 3 | Argument / config / persistence error. `exec export --strict` also returns this code when the execution is incomplete. |

### 10.5 Logging contract

- All structured logging goes to **stderr** via the `tracing` crate.
- Default level: `INFO`.
- Override with `RUST_LOG` (e.g. `RUST_LOG=rowforge_core=debug`).
- stdout is reserved for human-readable summary output (e.g. `exec list` tables, success counts after `exec run`).
- Errors corresponding to non-zero exit codes are also printed to stderr as `[rowforge] error: <message>`.

### 10.6 Environment variables

| Variable | Purpose |
|---|---|
| `ROWFORGE_HOME` | Override data root. Default `~/.rowforge`. |
| `RUST_LOG` | Set the tracing filter for rowforge's own logging. |

Handler-specific environment variables are declared in `manifest.entry.env` and consumed by the handler subprocess; rowforge does not interpret them.

---

[← README](README.md) · Previous: [Part IV](part-4-data.md) · Next: [Part VI](part-6-base.md)
