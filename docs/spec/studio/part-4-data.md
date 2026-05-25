# Part 4 — Data

Describes the on-disk artifacts Studio reads, how projections are derived
from them, caching policy, the optional sidecar index, and schema
versioning. Disk layout produced by the CLI is in
[`../cli/part-4-data.md`](../cli/part-4-data.md); this part references it.

## 4.1 Source artifacts (read-side view)

| Artifact | Source | Studio reads as |
|---|---|---|
| `executions.db` (SQLite) | CLI writes | Read-only registry queries |
| `executions/<e>/manifest.json` | CLI writes | `ExecDetail` mirror; redundant with SQLite |
| `executions/<e>/attempts/<r>/meta.json` | CLI writes at terminal state | `AttemptDetail.stats`, `by_error_code` |
| `executions/<e>/attempts/<r>/outcomes.jsonl` | CLI streams during run | Scanned for `FailedRowPage`, `ExecRollup`, `RowHistory` |
| `executions/<e>/attempts/<r>/handler_log.log` | Studio appends during run (Plan 9) | Logs tab bootstrap via `handler_log_tail`; live tail via `handler_log_subscribe` |
| `executions/<e>/attempts/<r>/handler-snapshot/` | CLI writes at attempt start | Inspected only via "Reveal in Finder" |
| `executions/<e>/exports/<ts>/resolution.json` | CLI writes at export | Read when an export has happened |

Studio never writes to artifacts other than what `start_exec` and
`start_run` produce indirectly (through `rowforge-core`).

## 4.2 Read strategies

For each projection in Part 2:

| Projection | Strategy | Worst-case cost |
|---|---|---|
| `Workspace` | Open SQLite, read `schema_version` | constant |
| `ExecSummary` | One SQLite query + one `meta.json` read for latest attempt | constant per execution |
| `ExecDetail` | One SQLite query + N attempt-summary queries | linear in attempts |
| `AttemptDetail` | One SQLite row + one `meta.json` | constant |
| `ExecRollup` | Streamed fold of every attempt's `outcomes.jsonl` | linear in total outcomes |
| `FailedRowPage` (v1) | Linear scan from `offset` | linear in `offset + limit` |
| `FailedRowPage` (v2, indexed) | Seek via `outcomes.idx` | constant + page IO |
| `RowHistory` | One pass per attempt, only failed rows | linear in failed-row count |

The "streamed fold" is `tokio::task::spawn_blocking` + buffered line
reader; outcomes are parsed lazily (only the fields needed for the
projection are deserialized). No projection ever materializes the full
parsed `outcomes.jsonl` in memory.

## 4.3 Caching

Three tiers, with explicit invalidation:

### Hot — always cached
- `Workspace` and its `schema_version`.
- SQLite connection pool.

Invalidation: process restart only.

### Warm — cached with mtime + TTL
- `ExecSummary` list.
- `ExecDetail` and `AttemptDetail` for **terminal** attempts only.

Invalidation:
- mtime probe against the source artifact before returning a cached
  entry. If stale, drop and re-read.
- TTL ceiling of 30 seconds regardless of mtime, to catch coarse-grained
  mistakes.
- Explicit refresh: user-initiated refresh button, Tauri `WindowEvent::Focused`,
  end-of-run notifications.

### Cold — never cached
- `ExecRollup`.
- `FailedRowPage`.
- `RowHistory`.
- Any in-progress attempt's `AttemptDetail`.

Cost-of-mistake: showing stale counts after a CLI-launched run completed
externally erodes user trust faster than a slow refresh. The mtime
probe is therefore mandatory (not "nice to have") for the warm tier.

Filesystem watchers (`notify` crate, FSEvents/inotify/ReadDirectoryChangesW)
are explicitly **not** used in v1: battery cost on macOS, platform
fragility, and complexity outweigh the benefit when mtime probes already
catch external mutation within one user interaction.

## 4.4 Sidecar index (`outcomes.idx`)

### v1 status
Not required. v1 uses linear scan for `FailedRowPage` and `ExecRollup`.
Linear scan of a 1 GB `outcomes.jsonl` takes ~5–10 seconds on SSD; that
is acceptable on user action with a spinner, but unacceptable on every
UI tick.

### v2 format (reserved here so the format is fixed when it lands)

A fixed-size record file, little-endian, **24 bytes per outcome**, in
the same directory as `outcomes.jsonl`:

```
record (24 B): {
    seq:           u32,
    byte_offset:   u64,    // offset into outcomes.jsonl
    line_offset:   u32,    // 0 if one outcome per line, otherwise position within batch line
    outcome_kind:  u8,     // 0=success 1=error 2=crash 3=too_large
    error_code_id: u16,    // interned per-attempt
    _pad:          u8,
    dur_ms:        u32,
}
trailer (16 B): { magic: b"RFIDX01\0" (8 B), outcomes_jsonl_size_at_finalize: u64 }
companion file: error_codes.txt        // one code per line, line number = id
```

### Ownership and lifecycle
- **Written by `rowforge-core`** during the run (cheap incremental
  append).
- Atomically `rename`d into place at the `PERSISTING` phase.
- A partial / aborted run leaves `outcomes.idx.tmp`; Studio rebuilds on
  next open.
- Magic-byte version pin: `RFIDX01` is v1 of the format; future bumps
  go to `RFIDX02` and old Studio rebuilds.

### Staleness detection
The trailer's `outcomes_jsonl_size_at_finalize` must match the live size
of `outcomes.jsonl`. Mismatch → discard and rebuild.

### When the index is missing
Studio rebuilds it on demand (in a `spawn_blocking` task with a UI
spinner). The rebuild outcome is the same as if the CLI had written it.

### When to argue against
If the GUI's failed-row browser is willing to be **scan-only with no
filter and no seq seek**, no index is needed. The data-flow events
(Part 6) already cover the running-attempt case. v1 takes this
position. v2 lifts it.

## 4.5 External mutation (CLI runs while Studio is open)

Contract: the next user-initiated read in Studio shows the new state.
Mechanism:
- Warm-tier mtime probes catch finished CLI runs.
- Window-focus event triggers a list refresh.
- Studio does not actively poll.

Studio does not attempt to observe a CLI run in progress as a live event
stream. That is the deferred `watch.rs` capability (see Part 6 §6.4).

## 4.6 Schema versioning

Three artifact classes, three contracts:

### SQLite `executions.db`
- Hard pin: `schema_version` must be ≤ Studio's known max.
- Higher → Studio refuses to open with a clear "this workspace was
  written by a newer rowforge; upgrade Studio" error.
- Lower → Studio refuses (no compat shims). Users upgrade core or
  Studio in lockstep.
- Migrations are CLI-owned exclusively.

### JSON metadata (`meta.json`, `manifest.json`, `resolution.json`)
- Tolerant subset reader: `#[serde(default)]` for missing fields,
  unknown fields dropped silently, type mismatches on known fields
  hard-error.
- A `schema_version: Option<u8>` field at the top of each JSON enables
  degraded display when present and indicates a known version.

### `outcomes.jsonl`
- Permissive parsing per line: unknown `type` discriminators are
  synthesized as "unknown outcome" and counted toward `failed_last`
  for safety (never toward `resolved`). Unknown error codes pass
  through as strings.
- Format has been stable since v3.4 (CLI Decision D13).

### `outcomes.idx` (when present)
- Strict magic-byte pin (§4.4). Version mismatch is never fatal — the
  index is rebuildable.

## 4.7 Cross-attempt resolution (live)

`ExecRollup` folds across all attempts using the `RowResolution` rules
defined in [`../cli/part-2-model.md`](../cli/part-2-model.md). The fold
logic lives in `rowforge-core::compute_resolution`; `studio-core`
invokes it through a counts-only entry point that does not materialize
the canonical-success map.

Per-row `RowHistory` is computed on demand by reading each attempt's
`outcomes.jsonl` for matching `seq` values. With the v2 index this is
constant per attempt; without, it is linear.

The matrix view (per-row × per-attempt) is deliberately not built. See
Part 2 §2.3.
