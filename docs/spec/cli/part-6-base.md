# Part VI — Spec Foundation

> Corresponds to §11-13 + conformance notes. For the directory index see [README.md](README.md).

---

## 11. System invariants

These are the contracts every component must uphold. Treat each entry below as a hard checklist item before changing code — whether as AI or human maintainer.

| ID | Invariant |
|---|---|
| **I1** | `Execution.input_csv_hash` is constant from `exec start` through any terminal state. Every Attempt re-validates it during `INITIALIZING`. |
| **I2** | `Attempt.handler_instance_id` is fixed at creation time and is immutable thereafter. |
| **I3** | HandlerInstance is content-addressed by `(handler_name, manifest_hash, source_snapshot_dir)`. Registering identical content reuses the existing `hi_*`. |
| **I4** | A Worker MUST write each `BatchOutcome` to `outcomes.jsonl` **before** pulling the next batch. (Ordering guarantee: the outcome is durably visible to RowResolution recomputation before the worker proceeds.) |
| **I5** | Resolved is absorbing. Once any Attempt produces a SUCCESS for a seq, that seq remains Resolved in all subsequent Attempts (except under `--force` semantics; the earliest SUCCESS remains canonical, R4). |
| **I6** | In every Attempt, each input row falls in exactly one bucket: `dispatched-with-outcome` (success / error / crash) ∪ `skipped` (already Resolved at Attempt start) ∪ `not_sampled` (excluded by `--sample`) ∪ `too_large` (exceeds `ROW_HARD_CAP_BYTES`) ∪ `cancel-dropped` (in the Accumulator when cancel fired). The sum of bucket sizes equals `input_row_count`. |
| **I7** | `Execution.state` is a cache. Ground truth is RowResolution + lifecycle markers. Code MUST recompute and write back `state` after every Attempt completion and after every user state command. |
| **I8** | The dispatch pipeline only writes what the handler said, plus the synthesized codes in §5.4 (produced because the handler could not or did not emit an outcome). For rows the handler never received and never crashed on, the pipeline MUST NOT fabricate outcomes. |
| **I9** | `outcomes.jsonl` is the sole machine-readable output of an Attempt. All downstream tools (export, resolution) read it. Modifying or deleting it after `PERSISTING` is undefined behavior. |
| **I10** | Aborted Attempts contribute to RowResolution; only `running` Attempts are skipped. |

Subsystem invariants: cancel `C1-C6` (`part-3-runtime.md` §4.4), wire `W1-W5` (`part-3-runtime.md` §5.5), resolution `R1-R4` (`part-4-data.md` §9.1).

## 12. Design decisions

These decisions have been considered and resolved. Reopening requires an explicit rationale.

| ID | Decision | Resolution |
|---|---|---|
| **D1** | Execution creation: implicit on first run vs explicit | Explicit. `exec start` must be called first. |
| **D2** | HandlerInstance storage: per-attempt only vs global registry | Both. The global SQLite row carries the content address; per-attempt `handler-snapshot/` holds the raw source bytes. |
| **D3** | SETTLED → CLOSED auto-transition | Manual only. `exec set-state ... closed`. |
| **D4** | Cross-execution forking | Out of scope. Attempts within one Execution are linear. |
| **D5** | `--force` overriding SUCCESS monotonicity | Permitted. New outcomes are persisted; `exec export` canonical SUCCESS is still the earliest one (R4). |
| **D6** | Manifest output schema declaration | No. Columns are dynamically detected from `outcomes.jsonl` at `exec export` time. |
| **D7** | Enforcing handler error code vocabulary | No. Handlers may emit any string. The codes in §5.4 are rowforge-reserved. |
| **D8** | Synthesizing `CANCELLED` for pending rows on cancel | Not emitted. Presented as `NeverAttempted`; next Attempt re-dispatches automatically. (Cancel invariant C5.) |
| **D9** | Per-attempt `success.csv` / `failed.csv` | None. Only `outcomes.jsonl`. CSV/JSONL is produced by `exec export`. |
| **D10** | Default `outcomes.jsonl` durability | No fsync. Opt in with `--fsync-outcomes`. |
| **D11** | Input format detection | Extension-first; `--format` overrides. Unknown extension rejected with exit 3. |
| **D12** | Resume / FromFailed source | Out of scope for this spec. Today seq-based resume is implicitly provided by `skip_seqs` derived from `compute_resolution`. |
| **D13** | Reader per-row parse error tolerance | WARN + skip + NeverAttempted. Do not abort the pipeline. Applies to both CSV (column-count mismatch) and JSONL (invalid JSON). (v3.4) |
| **D14** | Default retry policy | One-shot: default `exec run` skips all already-Attempted seqs; each row dispatched at most once. Prevents duplicate side effects for non-idempotent handlers. (v3.4) |
| **D15** | `--retry-failed` semantics | Dispatch only `FailedLast` / `CrashedLast` / `CancelledLast` / `TooLarge`. Leave Resolved and NeverAttempted untouched. Mutually exclusive with `--force`. (v3.4) |

## 13. Glossary

| Term | Definition |
|---|---|
| **Attempt** | One physical dispatch of (a subset of) rows from an Execution against one HandlerInstance. ID `r_<ULID>`. Append-only once terminal. |
| **BatchJob** | Internal unit in the Accumulator → Worker channel. Carries 1..=batch_size rows. |
| **BatchOutcome** | One line in `outcomes.jsonl`. Carries the outcomes of one BatchJob. |
| **CancellationToken** | Tokio cancellation primitive shared by all four dispatch tasks. |
| **CSV input** | A `.csv` file, or one explicitly specified with `--format csv`. UTF-8, RFC 4180. |
| **dispatch pipeline** | The four-task streaming pipeline in §4. |
| **Execution** | The scope for one complete processing lifecycle of an input CSV/JSONL. ID `e_<ULID>`. |
| **fsync-outcomes** | Optional durability flag; calls `fsync` after each `outcomes.jsonl` append. |
| **HandlerInstance** | An immutable content-addressed snapshot of a handler. ID `hi_<ULID>`. |
| **JSONL input** | A `.jsonl` / `.ndjson` file, or one explicitly specified with `--format jsonl`. |
| **manifest** | The `rowforge.yaml` file in a handler directory. §6. |
| **manifest_hash** | SHA-256 of the `rowforge.yaml` bytes. Half of HandlerInstance identity. |
| **outcomes.jsonl** | The sole machine-readable output file of each Attempt. §7.4. |
| **RowJob** | Internal unit in the Reader → Accumulator channel. Holds one row. |
| **RowResolution** | The per-row derived state across all completed and aborted Attempts of an Execution. §3.3, §9.1. |
| **run_id** | The `r_<ULID>` of an Attempt. Used in the `init.run_id` field of the wire protocol. |
| **RunType** | The `(Source, Simulation)` pair for an Attempt. §4.5. |
| **seq / seqid** | Zero-based index of a row in `input.csv` / `input.jsonl`. First column of every export CSV. |
| **SharedJsonlWriter** | Append-only writer for `outcomes.jsonl`, shared by all workers within one Attempt. |
| **skip_seqs** | Set of already-Resolved `seq` values at Attempt start. Built by `compute_resolution`. |
| **Stall Monitor** | Pipeline task that cancels the run when `outcomes.jsonl.bytes_written` has not grown for 300 consecutive seconds. |
| **ULID** | The 26-character, time-prefixed identifier used for all rowforge entities. |
| **WAL** | SQLite write-ahead logging mode used by `executions.db`. |

---

## Conformance notes (informative)

This section is **informative** — it records delta between the code and this spec as of `2026-05-18`. The spec is authoritative; deltas should be eliminated by future work, not preserved.

| § | Spec statement | Current code status |
|---|---|---|
| 3.1 | SETTLED is auto-detected when an Attempt ends with `unresolved_count == 0`. | Not implemented; state stays ITERATING until `exec set-state ... settled`. |
| 4.5 | RunType Source ∈ {Full, Sampled}. | Matches. Earlier designs' Resume / FromFailed are explicitly excluded from this spec (D12). |
| 5.4 | `CANCELLED` is reserved and never emitted. | Matches. |
| 8.6 | Each Attempt re-validates `input_csv_hash` during INITIALIZING. | Not enforced; input file is hashed at `exec start` but not re-checked at Attempt start. Risk: external edits to the input file go undetected. |
| 10.3 | `run` is a legacy single-shot command. | Present; not integrated with the Execution registry. Removable once `exec` covers all workflows. |

Eliminating any delta is a code change, not a spec change.

---

[← README](README.md) · Previous: [Part V](part-5-cli.md)
