# rowforge-studio — Specification

Companion spec to the `rowforge` CLI (see [`../cli/`](../cli/)). Describes the
desktop GUI (`apps/rowforge-studio`, Tauri + React) and its supporting Rust
crate `rowforge-studio-core`.

繁體中文版本：[`zh-Hant/`](zh-Hant/)。

This spec is **broader than the v1 milestone**. The first milestone is scoped
to execution management; later milestones add handler authoring and richer
observability. Each section calls out what is in v1 vs. deferred.

## Parts

1. [`part-1-overview.md`](part-1-overview.md) — purpose, principles, scope, non-goals, relationship to the CLI
2. [`part-2-model.md`](part-2-model.md) — entities, projections, derived views
3. [`part-3-runtime.md`](part-3-runtime.md) — process model, run state machine, concurrency, cancel, crash recovery
4. [`part-4-data.md`](part-4-data.md) — source artifacts, caching, sidecar index, schema versioning
5. [`part-5-api.md`](part-5-api.md) — `studio-core` API, Tauri commands, errors, settings, versioning
6. [`part-6-observability.md`](part-6-observability.md) — event taxonomy, throughput safety, live vs replay, metrics, multi-run
7. [`part-7-ui.md`](part-7-ui.md) — stack, design language, information architecture, primary flows, state colors, interaction patterns, boundary states
8. [`part-8-handler-authoring.md`](part-8-handler-authoring.md) — handler discovery, edit launcher, scaffold, build, smoke test (supersedes Part 1 §1.4 / Part 5 §5.4 anchors)

## Companion artifact

A short v1 implementation plan is at
[`../../superpowers/specs/2026-05-19-rowforge-studio-mvp-design.md`](../../superpowers/specs/2026-05-19-rowforge-studio-mvp-design.md).
That document is the **MVP design**; this spec is the **target shape**. Where
they disagree (the MVP is narrower), this spec wins for any work outside the
v1 milestone.
