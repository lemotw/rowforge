# Part 1 — Overview

## 1.1 Purpose

`rowforge-studio` is a desktop GUI for the `rowforge` CLI. It exists for two
user goals:

1. **Manage executions** — start, observe, cancel, and export the per-row
   batch jobs that `rowforge exec` already supports, without dropping to a
   terminal.
2. **Author handlers** — scaffold, edit, validate, smoke-test, and package
   the handler programs that drive those executions.

The v1 milestone covers goal (1) end to end. Goal (2) is staged across
later milestones and is reflected throughout this spec as anchor points
(types, hooks, file layouts) that v1 must not preclude.

## 1.2 Principles

- **Extension, not wrapper.** `rowforge-studio-core` is an extension of
  `rowforge-core`. Anything also useful to the CLI is pushed down into
  `rowforge-core` and shared. `studio-core` only contains capabilities the
  CLI does not need.
- **No second consumer is designed for.** TUI / web / remote frontends are
  out of scope. The public surface is shaped for the single Tauri app.
- **Source of truth on disk.** Studio is a viewer and a launcher over the
  CLI's on-disk artifacts (SQLite registry, `outcomes.jsonl`, `meta.json`,
  etc.). Studio never invents data the CLI cannot reproduce.
- **Studio and CLI share the same workspace.** They see identical state at
  any moment, modulo Studio's in-memory cache (Part 4).
- **Streaming, not loading.** No projection ever loads a full
  `outcomes.jsonl` into memory. Failed-row browsing is paged; rollups are
  streamed.
- **Tauri-agnostic core.** No `tauri::` types appear in `studio-core`. The
  Tauri layer is thin glue: argument translation, IPC, event emit.

## 1.3 Architecture (at a glance)

```
apps/rowforge-studio (Tauri + React)
        │
        │  thin glue: commands.rs
        ▼
crates/rowforge-studio-core   ← Tauri-agnostic; v1 scope
        │
        │  consumes only public API
        ▼
crates/rowforge-core          ← engine, unchanged + minor lifts
```

The "minor lifts" into `rowforge-core` are deliberate: workspace discovery,
SQLite registry open/migrate, `compute_resolution`, manifest validation,
and on-disk artifact parsing are CLI-and-studio common ground and belong
there. See Part 5 §5.1.

## 1.4 Scope

### In v1
- Execution management: list, show, start, run, attempts, attempt detail,
  cancel, export.
- Live progress display: progress bar + recent-events tail.
- Failed-row inspection: paged, optionally filtered later (Part 4 §4.4).
- Multi-execution concurrency (bounded; Part 3 §3.4).
- Crash recovery for orphaned attempts (Part 3 §3.7).

### Anchored in v1, implemented later
- Handler authoring (manifest editor backing, scaffolding, in-app build,
  smoke test, `pack`). API anchor points are listed in Part 5 §5.4.
- Replay of finished attempts as an event stream (Part 6 §6.4).

### Out of scope
- Multi-workspace registry, remote workspaces, daemon mode.
- Cross-execution analytics, BI views, scheduled runs.
- i18n / theming / accessibility tuning beyond Tauri defaults.
- Sidecar runner process. v1 is in-process (Part 3 §3.1).

## 1.5 Relationship to the CLI

| Concern | CLI | Studio |
|---|---|---|
| Workspace ownership | Read & write | Read & write |
| Schema migrations | Owns | Refuses to open newer schema |
| `outcomes.jsonl` | Writes (append-only) | Reads only |
| `executions.db` | Reads & writes | Reads only |
| `outcomes.idx` (future) | Writes incrementally | Reads; rebuilds if missing |
| Concurrency with CLI | First-writer-wins | Detects external mutation via mtime |

A user may run `rowforge exec run` in a terminal while Studio is open; the
contract is that the next user interaction in Studio shows the new state
(Part 4 §4.5).

## 1.6 Non-goals (called out)

- **No re-implementation of CLI logic.** If a behaviour exists in the CLI,
  Studio routes to it through `rowforge-core`.
- **No Tauri-only abstractions.** If we add an abstraction "to make Tauri
  happy," it lives in the Tauri crate, not in `studio-core`.
- **No premature multi-consumer design.** The day a second frontend
  appears, refactor then.
