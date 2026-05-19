# Part IV — Data and Persistence

> Corresponds to §7-9. For the directory index see [README.md](README.md).

---

## 7. Input / output formats

### 7.1 Input — CSV

- A header row is required.
- Quoting follows RFC 4180.
- Encoding: UTF-8. A leading BOM is consumed and discarded.
- Each row becomes a `data` object keyed by header names.
- Trailing blank lines are silently skipped.

### 7.2 Input — JSONL

- One JSON object per line; no header.
- Each line's top-level keys become the `data` object.
- Blank lines are skipped.
- **Malformed-row tolerance (v3.4)**: Rows that fail to parse (invalid JSON, non-object, etc.) are logged at WARN and skipped — the pipeline is not aborted. Skipped rows appear as `NeverAttempted` in `compute_resolution`; fixing the input is the only way to permanently eliminate them.
  (Decision D13; see `docs/plan/2026-05-16-streaming-dispatch.md` §4)

### 7.3 Format detection

| `--format` flag | Extension | Resolved as |
|---|---|---|
| explicit `csv` | any | CSV |
| explicit `jsonl` | any | JSONL |
| absent | `.csv` | CSV |
| absent | `.jsonl`, `.ndjson` | JSONL |
| absent | other | Rejected with config error (exit 3) |

### 7.4 Per-attempt output — `outcomes.jsonl`

The sole machine-readable output file of an Attempt. Append-only. One JSON object per line. Each line is a `BatchOutcome`:

```jsonc
{"first_seq": 10,
 "seqs":      [10, 11, 12],
 "outcomes": [
   {"type":"success","seq":10,"data":{"is_valid":true},"dur_ms":42},
   {"type":"error","seq":11,"code":"INVALID","message":"missing @",
    "dur_ms":12,"data":{"billid":"B11"}},
   {"type":"crash","seq":12,"worker_id":2,"crash_at_seq":12}
 ]}
```

Field semantics:

| Field | Meaning |
|---|---|
| `first_seq` | The smallest `seq` in this BatchOutcome. Equals `seqs[0]`. Redundant storage; lets readers skip lines without fully parsing the `seqs` array. |
| `seqs` | All `seq` values covered by this BatchOutcome, **strictly increasing within the batch**. Batches arrive in completion order; interleaving across batches is possible. |
| `outcomes[i].type` | One of `success`, `error`, `crash` (snake_case serde tag). Synthesized codes (`STARTUP_FAILED`, `ROW_TOO_LARGE`, `BATCH_PROTOCOL_ERROR`, `MISSING_REQUIRED_INPUT_COLUMN`) are written with `type=error`, the synthesized code in `code`, and `data=null`. Worker crash codes (`WORKER_CRASH`, `WORKER_CRASH_UNSAFE`) are written with `type=crash`. |
| `outcomes[i].seq` | The input seq this outcome corresponds to. Positionally aligned with `seqs`. |
| `outcomes[i].data` (success / error) | Handler-emitted payload. Omitted for synthesized errors and crashes. |
| `outcomes[i].dur_ms` (success / error) | Wall-clock time for dispatching this row (or this row's share within a batch). |
| `outcomes[i].worker_id`, `crash_at_seq` (crash only) | Identifies which worker crashed and at which seq. |

The file's growth is the input to the stall monitor (§4.1, see [part-3-runtime.md](part-3-runtime.md)).

### 7.5 Durability

By default, `outcomes.jsonl` does not call `fsync` after each write — OS page cache is trusted. With `exec run --fsync-outcomes`, rowforge calls `fsync` (or the platform equivalent) after each append, trading throughput for durability against hard crashes.

---

## 8. Persistence layout

### 8.1 Filesystem root

The data root defaults to `~/.rowforge/`; the environment variable `ROWFORGE_HOME` overrides it.

```
$ROWFORGE_HOME/
├── executions.db                   # SQLite registry (§8.2)
└── executions/
    └── <exec_id>/
        ├── manifest.json           # Mirror of executions row (§8.4)
        ├── input.csv | input.jsonl          # hash-locked snapshot (extension matches source)
        ├── input.csv.sha256 | input.jsonl.sha256  # named after the snapshot extension
        ├── attempts/
        │   └── <attempt_id>/
        │       ├── outcomes.jsonl          # §7.4
        │       ├── meta.json               # attempt metadata
        │       ├── input.csv | input.jsonl # per-attempt copy (extension matches source)
        │       └── handler-snapshot/
        └── exports/
            └── <UTC-timestamp>/    # produced by `exec export` (§9)
                ├── success.csv
                ├── failed.csv
                ├── success.jsonl
                ├── failed.jsonl
                └── resolution.json
```

### 8.2 SQLite schema

The registry is the source of truth for Execution / HandlerInstance / Attempt rows. The schema version is recorded in `schema_version`; migrations are applied in `ExecutionStore::open`.

```sql
CREATE TABLE schema_version (version INTEGER NOT NULL);

CREATE TABLE executions (
    id                           TEXT PRIMARY KEY,         -- e_<ULID>
    name                         TEXT,
    input_csv_id                 TEXT NOT NULL,
    input_csv_hash               TEXT NOT NULL,            -- sha256 hex
    input_row_count              INTEGER NOT NULL,
    current_handler_instance_id  TEXT,
    state                        TEXT NOT NULL,            -- open | iterating | settled | closed | abandoned
    created_at                   TEXT NOT NULL,            -- RFC 3339 UTC
    settled_at                   TEXT,
    closed_at                    TEXT,
    abandoned_at                 TEXT,
    abandoned_reason             TEXT
);
CREATE INDEX idx_executions_state      ON executions(state);
CREATE INDEX idx_executions_created_at ON executions(created_at);

CREATE TABLE handler_instances (
    id                  TEXT PRIMARY KEY,                  -- hi_<ULID>
    handler_id          TEXT NOT NULL,                     -- manifest.name
    manifest_hash       TEXT NOT NULL,                     -- sha256 of rowforge.yaml
    source_snapshot_dir TEXT NOT NULL,                     -- canonicalized handler directory path
    binary_hash         TEXT,                              -- reserved; not currently used
    created_at          TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_handler_instances_content
    ON handler_instances(handler_id, manifest_hash, source_snapshot_dir);

CREATE TABLE attempts (
    id                       TEXT PRIMARY KEY,             -- r_<ULID>
    execution_id             TEXT NOT NULL REFERENCES executions(id),
    handler_instance_id      TEXT NOT NULL REFERENCES handler_instances(id),
    parent_attempt_id        TEXT,
    run_type_source          TEXT NOT NULL,                -- "full" | "sampled"
    run_type_sample_size     INTEGER,
    run_type_simulation      TEXT NOT NULL,                -- "real" | "dry"
    state                    TEXT NOT NULL,                -- "running" | "completed" | "aborted"
    success_count            INTEGER NOT NULL DEFAULT 0,
    failed_count             INTEGER NOT NULL DEFAULT 0,
    aborted_reason           TEXT,
    started_at               TEXT NOT NULL,
    ended_at                 TEXT
);
CREATE INDEX idx_attempts_execution    ON attempts(execution_id);
CREATE INDEX idx_attempts_started_at   ON attempts(started_at);
```

Database pragmas: `journal_mode=WAL`, `foreign_keys=ON`.

### 8.3 Identifier scheme

All IDs are ULIDs with a type prefix:

| Layer | Prefix |
|---|---|
| Execution | `e_` |
| HandlerInstance | `hi_` |
| Attempt | `r_` |

ULIDs are time-sortable; chronological order is implicit in the ID.

### 8.4 `manifest.json` mirror

Each Execution directory contains a `manifest.json` mirroring the SQLite `executions` row. Rewritten on every state change. Purpose: if `executions.db` is lost, individual Execution directories remain self-describing and the registry can be reconstructed from the filesystem.

```json
{
  "id": "e_01HX...",
  "name": "refund-2026-05",
  "input_csv_id": "csv_unregistered",
  "input_csv_hash": "sha256:...",
  "input_row_count": 1000,
  "current_handler_instance_id": "hi_01HW...",
  "state": "iterating",
  "created_at": "2026-05-18T10:00:00Z",
  "settled_at": null,
  "closed_at": null,
  "abandoned_at": null,
  "abandoned_reason": null
}
```

### 8.5 Attempt `meta.json`

```json
{
  "run_id": "r_01HX...",
  "execution_id": "e_01HX...",
  "handler_instance_id": "hi_01HW...",
  "parent_attempt_id": null,
  "lifecycle": {
    "terminal_state": "completed",
    "aborted_reason": null,
    "phases_reached": ["initializing","snapshotting","resolving_input",
                       "spawning_workers","dispatching","synthesizing","persisting"],
    "started_at": "2026-05-18T10:05:00Z",
    "ended_at":   "2026-05-18T10:06:42Z"
  },
  "run_type": {
    "source":     {"kind": "sampled", "sample_size": 2},
    "simulation": {"dry_run": false}
  },
  "stats": {
    "success": 2,
    "failed": 0,
    "skipped": 0,
    "not_sampled": 919,
    "avg_dur_ms": 142,
    "by_error_code": {}
  },
  "input_path": "<execution>/input.csv|input.jsonl",
  "input_row_count": 921
}
```

### 8.6 Hash locking and verification

- `input.csv.sha256` (or `input.jsonl.sha256`) is written next to the snapshot during `exec start`.
- Each Attempt re-reads that snapshot path during `INITIALIZING` and confirms the SHA-256 still matches the recorded hash before proceeding. Mismatch → Attempt aborts with `SnapshotFailed`. (Invariant **I1**)
- `manifest_hash` is computed from the live `rowforge.yaml` bytes at each `exec run`. HandlerInstance is content-addressed by `(name, manifest_hash, source_snapshot_dir)`; identical content reuses the same `hi_*` row.

### 8.7 Retention policy

- Aborted Attempts and their `outcomes.jsonl` are retained — they contribute to RowResolution (§9.1).
- HandlerInstance rows are retained indefinitely. This spec provides no cleanup command.
- The `exports/` directory accumulates; the user may delete it manually without affecting the Execution.

---

## 9. Export and resolution algorithm

### 9.1 Resolution derivation

`compute_resolution(execution)` traverses every `completed` or `aborted` Attempt (chronologically) and builds a history for each input seq:

```
def resolve(execution, seq):
    history = []
    for attempt in execution.attempts_chronological:
        if attempt.state == "running":
            continue                       # not terminal; skip
        for outcome in attempt.outcomes_for(seq):
            history.append(outcome)

    if not history:
        return NeverAttempted
    if any(o.is_success() for o in history):
        return Resolved(canonical = earliest_success(history))
    latest = history[-1]
    return classify(latest.code)            # FailedLast | CrashedLast |
                                            # CancelledLast | TooLarge
```

Properties:

| ID | Statement |
|---|---|
| **R1 (Completeness)** | For every seq in `0..input_row_count`, **exactly one** resolution state is produced. |
| **R2 (Monotonicity)** | Resolved is absorbing. Once any Attempt produces a SUCCESS for a seq, subsequent Attempts cannot flip it back (unless `--force`; earliest SUCCESS remains canonical). |
| **R3 (TooLarge permanence)** | A row that exceeds `ROW_HARD_CAP_BYTES` will always exceed it on the same input. TooLarge is treated as permanent unless the input is edited. |
| **R4 (Canonical SUCCESS)** | When multiple Attempts succeed on the same seq, the earliest SUCCESS in time is the canonical export row. |

### 9.2 Export output

`exec export <id>` produces:

```
<execution_dir>/exports/<UTC-timestamp>/
├── success.csv           # one row per Resolved seq
├── failed.csv            # one row per non-Resolved seq
├── success.jsonl         # same content as success.csv, JSONL form
├── failed.jsonl          # same content as failed.csv, JSONL form
└── resolution.json       # counts + completeness block (always written)
```

`--format` controls which files are written (`csv` | `jsonl` | `both`); `resolution.json` is always written. `--output-dir DIR` writes directly into `DIR`, skipping the timestamp subdirectory.

#### Column detection

There is no schema. Columns for `success.csv` / `success.jsonl` are dynamically detected by scanning all contributing `outcomes.jsonl` files for successful outcomes:

1. `seqid` — always the first column.
2. Handler `data` keys — alphabetical order (BTreeSet).
3. Meta columns (only when any contributing manifest has `output.include_meta: true`): `meta_dur_ms`, `meta_handler_ver`.

Columns for `failed.csv` / `failed.jsonl`:

1. `seqid`
2. `errcode`
3. `errmessage`
4. Handler `data` keys (from error envelope) — alphabetical order.
5. Meta columns (when enabled): `meta_handler_ver`, `meta_crash_at_seq`, `meta_crash_worker_id`.

#### Value rendering

| JSON type | CSV rendering | JSONL rendering |
|---|---|---|
| string | as-is | as-is |
| number | `42`, `3.14` | as-is |
| bool | `true` / `false` | as-is |
| null | empty string | explicit `null` |
| object / array | JSON text (`{"k":1}`) | as-is |
| missing key | empty string | explicit `null` |

Rows are sorted by `seqid` ascending.

### 9.3 `--force` semantics

`exec run --force` re-dispatches already-Resolved rows. The new Attempt's outcomes are appended to its own `outcomes.jsonl` as usual. However, `compute_resolution` still uses the **earliest** SUCCESS as the canonical export row (R4). Outcomes produced by a `--force` run appear in Attempt metadata and `resolution.json` history but do not change the content of the canonical `success.csv`.

### 9.4 Completeness block

`resolution.json` always contains:

```json
{
  "execution_id": "e_01...",
  "input_row_count": 1000,
  "counts": {
    "resolved": 800, "failed_last": 50, "crashed_last": 0,
    "cancelled_last": 0, "too_large": 0, "never_attempted": 150
  },
  "by_error_code": {"INVALID": 50},
  "completeness": {
    "fully_processed": false,
    "completion_percent": 85.0,
    "completed_attempts": 1,
    "aborted_attempts": 2,
    "aborted_attempt_ids": ["r_01...", "r_02..."],
    "aborted_reasons": ["stalled", "cancelled"]
  },
  "merged_from_attempts": ["r_01...", "r_02..."]
}
```

`fully_processed == true` is equivalent to `never_attempted == 0 AND aborted_attempts == 0`.

### 9.5 `--strict`

`exec export --strict` exits 3 and writes no files when `fully_processed` is false. The error message carries the same counts as `resolution.json.completeness`.

### 9.6 WARN signals

`exec export` (without `--strict`) prints one WARN log entry when any of the following hold:

- `never_attempted > 0`
- `aborted_attempts > 0`

Export still produces its output files; the WARN is for operator awareness.

---

[← README](README.md) · Previous: [Part III](part-3-runtime.md) · Next: [Part V](part-5-cli.md)
