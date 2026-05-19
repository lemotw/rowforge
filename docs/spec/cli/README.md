# rowforge — Specification

Status: **Authoritative**. Code defers to this spec; when the two conflict, the spec is correct and the code is wrong, unless an explicit revision entry has been written back into this spec.

Date: 2026-05-18.

This directory is the single authoritative source for what rowforge is, how it works, and what contracts each component (CLI, core, handler, persistence layer, wire protocol) must uphold. It supersedes all earlier spec and plan documents under `docs/archived/`.

## Section Index

| File | Sections | Content |
|---|---|---|
| [part-1-overview.md](part-1-overview.md) | §1 | What rowforge is: one-sentence definition, system view, reader guide, out-of-scope |
| [part-2-model.md](part-2-model.md) | §2-3 | Core concepts (Execution / HandlerInstance / Attempt / RowResolution / Handler), Execution and Attempt state machines, row resolution states |
| [part-3-runtime.md](part-3-runtime.md) | §4-6 | Dispatch pipeline (four tasks + cancel C1-C6), wire protocol (envelope + handshake + error codes + W1-W5), manifest schema |
| [part-4-data.md](part-4-data.md) | §7-9 | Input/output formats (including `outcomes.jsonl` frame format), persistence layout (SQLite + filesystem + hash locking), export and resolution algorithm (R1-R4) |
| [part-5-cli.md](part-5-cli.md) | §10 | CLI surface: `exec` / `pack` / `run`, exit codes, logging, environment variables |
| [part-6-base.md](part-6-base.md) | §11-13 + Conformance | System invariants (I1-I10), design decisions (D1-D15), glossary, delta from current code (informative) |

## Invariants and Decisions Quick Reference

Scan the invariant table in `part-6` before changing code. Frequent references:

| Label | Topic | File |
|---|---|---|
| **I1-I10** | System invariants | [part-6-base.md](part-6-base.md) |
| **C1-C6** | Cancel invariants | [part-3-runtime.md](part-3-runtime.md) §4.4 |
| **W1-W5** | Wire-layer invariants | [part-3-runtime.md](part-3-runtime.md) §5.5 |
| **R1-R4** | Resolution properties | [part-4-data.md](part-4-data.md) §9.1 |
| **D1-D15** | Design decisions | [part-6-base.md](part-6-base.md) |

## Recommended Reading Paths

| Reader | Suggested path |
|---|---|
| New contributor | part-1 → part-2 → part-5 |
| Maintainer changing dispatch | part-3 (§4-5) → part-6 (I + C invariants) |
| Maintainer changing persistence | part-4 (§8-9) → part-6 (I7-I10) |
| Handler author | part-3 (§5-6) → part-4 (§7) |
| AI making code changes | Skim part-2 → part-6 (full invariant list) → the section nearest the change |

## Cross-File Reference Convention

Each file uses `§N.M` for section references internally. Locate the owning file via the table above. Examples:

- `§4.4 C5` lives in part-3-runtime.md
- `§9.1 R4` lives in part-4-data.md
- Labels `I5` / `D8` / `W3` are globally unique and grep-friendly

---

Revision history is in git. When adding sections: update the section index table here, and keep `§N.M` numbering contiguous within each file.
