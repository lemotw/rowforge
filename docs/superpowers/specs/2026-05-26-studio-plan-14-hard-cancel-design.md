# Plan 14 — Hard cancel for stuck runs

**Date:** 2026-05-26
**Branch:** `studio-plan-14-hard-cancel`
**Builds on:** Plans 4, 5, 8, 9, 13

## 1. Purpose

Today's cancel is **soft**: the dispatch loop stops sending new rows, the in-flight workers are told to shut down via `Outbound::Shutdown`, and we give them a grace window (`shutdown(Duration::from_secs(grace))`). If the handler ignores stdin EOF, blocks on a network call, deadlocks, or is genuinely runaway, the grace timer eventually fires `Child::kill` — but only after `grace`, and only against the direct child. Sub-processes spawned by the handler (e.g., a Go binary that itself runs `ffmpeg`) survive.

Plan 14 adds a **Hard cancel** action: when the user confirms, the studio:
1. Sends SIGKILL (Unix) / TerminateProcess (Windows) immediately to the worker child AND its entire process group, skipping the grace timer.
2. Marks the attempt as `cancelled` with reason `hard_cancel`.
3. Drains pending rows from accumulator as `cancelled` outcomes.

This closes the multi-plan "stuck-handler" limitation that has accumulated since Plan 4.

## 2. Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Trigger UX | Two-step: existing "Cancel" issues soft cancel; new "Force kill" button appears in confirm dialog after soft cancel has been pending for ≥5s | Avoids accidental hard kills; matches macOS Force Quit pattern |
| Process group | Spawn each worker into its own process group (Unix: `setsid` via `pre_exec`; Windows: `CREATE_NEW_PROCESS_GROUP`) | Required to kill grandchildren; foundation for hard cancel |
| Signal sequence | Hard cancel sends SIGKILL to PGID directly (no SIGTERM dance — soft cancel already tried SIGTERM via stdin shutdown) | Soft is the polite path; hard is the unconditional path |
| Windows | Use `Job Objects` to terminate the worker + descendants atomically | Process groups on Windows are weaker than Unix; Job Objects are the recommended primitive |
| Attempt state | New `cancelled_reason` column on attempts: `null` (soft) / `"hard_cancel"` / `"timeout"` (reserved for future) | Audit trail — UI shows "force-killed" badge |
| Pending row outcomes | Synthesized as `cancelled` status with code `HARD_CANCEL` | Distinct from soft `CANCELLED` so post-mortem can tell which rows died how |
| Concurrent hard cancel | Idempotent — second click is a no-op once attempt is in `cancelling` or `cancelled` state | Avoid duplicate signals |
| Active-run gate elsewhere | Reuse sqlite `has_active_attempt` check from Plan 10 — hard-cancelled attempts release the gate immediately | Lets the user delete / re-run without manual sqlite surgery |

## 3. Backend changes

### 3.1 Process group spawning

In `crates/rowforge-core/src/worker.rs`, the `Command` construction:

**Unix:**
```rust
use std::os::unix::process::CommandExt;
// In WorkerHandle::spawn:
let mut cmd = Command::new(&binary_path);
cmd.args(&args);
unsafe {
    cmd.pre_exec(|| {
        // Detach into a new session so all descendants share our PGID.
        if libc::setsid() == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    });
}
```

After spawn, `worker.pgid = child.id() as i32` (on Unix, after setsid, the pid is the pgid).

**Windows:** use `CommandExt::creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED)`, assign child to a `Job Object` via `AssignProcessToJobObject`, resume. Store `JobHandle` on `WorkerHandle`.

Cross-platform wrapper struct:
```rust
pub(crate) struct ProcessGroup {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: windows::Win32::Foundation::HANDLE,
}

impl ProcessGroup {
    pub fn kill(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let r = unsafe { libc::killpg(self.pgid, libc::SIGKILL) };
            if r == -1 { return Err(std::io::Error::last_os_error()); }
            Ok(())
        }
        #[cfg(windows)]
        {
            unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.job, 137) }
                .map_err(|e| std::io::Error::other(e))
        }
    }
}
```

### 3.2 `WorkerHandle::hard_kill`

```rust
impl WorkerHandle {
    /// Immediately terminate the worker and all descendants.
    /// Does NOT wait for stdin drain or shutdown protocol.
    pub async fn hard_kill(&mut self) -> Result<(), CoreError> {
        if let Some(pg) = &self.process_group {
            pg.kill().map_err(CoreError::Io)?;
        }
        // Belt + suspenders: also kill the direct child handle in case PG spawn failed.
        let _ = self.child.kill().await;
        // Reap to avoid zombies; ignore wait failure.
        let _ = self.child.wait().await;
        Ok(())
    }
}
```

### 3.3 `RunRequest` cancel-mode plumb-through

`CancellationToken` stays as the soft-cancel mechanism. Add a parallel signal for hard cancel:

```rust
pub struct RunRequest {
    // ...existing...
    pub hard_cancel: Arc<AtomicBool>,
}
```

In `pool_streaming.rs` dispatch loop, after detecting `hard_cancel.load(Ordering::Relaxed) == true`:

1. Set the soft `CancellationToken` (so the dispatch loop drops pending rows).
2. For each in-flight worker, call `worker.hard_kill().await`.
3. Synthesize `RowOutcome::Cancelled { code: "HARD_CANCEL", ... }` for any dispatched-but-unanswered seq.
4. Skip the normal `shutdown(grace)` path.
5. Return a `RunStatus::Cancelled { reason: "hard_cancel" }` (extend enum).

### 3.4 `StudioCore::hard_cancel_run`

```rust
impl StudioCore {
    /// Trigger hard cancel for an active run. Idempotent.
    pub fn hard_cancel_run(&self, exec_id: &str, attempt_id: &str) -> Result<(), UiError>;
}
```

Algorithm:
1. Validate IDs via `is_valid_id_component`.
2. Look up `SessionRegistry` entry for `(exec_id, attempt_id)`.
3. If no entry → `UiError::AttemptNotRunning`. (The attempt may have finished already; surface as a friendly no-op.)
4. Otherwise: `session.hard_cancel_flag.store(true, Ordering::Relaxed)` AND `session.soft_cancel_token.cancel()` (belt + suspenders).
5. Return immediately; the dispatch loop picks up the flag and does the actual killing.

### 3.5 Attempt finalization changes

`attempts` table gets a new column:
```sql
ALTER TABLE attempts ADD COLUMN cancelled_reason TEXT NULL;  -- null | "hard_cancel" | "timeout"
```

In `run.rs` finalize path: when `RunStatus::Cancelled { reason }` returns, write `attempts.cancelled_reason = reason` if non-null.

Migration: add a new sqlite migration file under `crates/rowforge-core/migrations/` (or wherever Plan 3 put exec migrations).

### 3.6 ExecSummary / AttemptSummary surface

Both views gain `cancelled_reason: Option<String>`. Re-export through Tauri DTOs.

## 4. Tauri shell

One new command:

```rust
#[tauri::command]
pub fn hard_cancel_run(
    state: State<'_, AppState>,
    execId: String,
    attemptId: String,
) -> Result<(), UiError>;
```

Existing `cancel_run` (soft) is unchanged.

Active-attempt event already exists from Plan 4; no new event needed — soft + hard both transition to `cancelled` state on success.

## 5. React UI

### 5.1 Existing CancelDialog escalation

`apps/rowforge-studio/src/components/CancelRunDialog.tsx` (Plan 4) currently has a single Cancel button. After the user clicks it and the dialog flips into "Cancelling…" state:

- Start a 5-second timer.
- If after 5s the attempt is still `running` (not `cancelled`): show a **"Force kill"** button + warning text.

```
┌─ Cancel run? ─────────────────────────────────────┐
│ Stop dispatching new rows and shut down workers.  │
│                                                   │
│         [Cancel run] [Keep running]               │
└───────────────────────────────────────────────────┘

   ↓ click Cancel run

┌─ Cancelling… ─────────────────────────────────────┐
│ Waiting for workers to finish current rows.       │
│ ▓▓▓▓░░░░░░ 4s                                     │
│                                                   │
│ Still cancelling after 5s? The handler may be     │
│ stuck. Force kill terminates the process and any  │
│ child processes immediately.                      │
│                                                   │
│         [Force kill] [Close]                      │
└───────────────────────────────────────────────────┘
```

Force kill click → confirmation step (typed-confirm pattern from Plan 10 delete):
```
Type "force" to confirm — pending rows will be marked cancelled
and the handler process will receive SIGKILL.
[                ]   [Cancel]   [Force kill]
```

On confirm → `useHardCancelRun.mutate({ execId, attemptId })`.

### 5.2 Hook

```ts
export const useHardCancelRun = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { execId: string; attemptId: string }) =>
      ipc.hard_cancel_run(args),
    onSuccess: (_, { execId }) => {
      qc.invalidateQueries({ queryKey: ["exec_detail", execId] });
      qc.invalidateQueries({ queryKey: ["attempt_detail"] });
    },
  });
};
```

### 5.3 Badge for hard-cancelled attempts

In `AttemptDetail.tsx` header and in `ExecDetail.tsx` AttemptsList rows:
- State `cancelled` + `cancelled_reason === "hard_cancel"` → red "Force-killed" badge instead of yellow "Cancelled".

### 5.4 Smoke test integration

`SmokeTab` from Plan 13 also needs Force kill (a smoke run can hang the same way). Phase 2 follow-on if needed; v1 of Plan 14 only wires the exec attempt path. Document this gap explicitly in the spec.

## 6. CLI

One new subcommand:
```
rowforge attempt hard-cancel <exec_id> <attempt_id>
```

Uses the same `StudioCore::hard_cancel_run` path. Useful for ops when the UI is wedged.

## 7. Out of scope (explicit)

- Per-row hard cancel (always cancels the whole attempt)
- Timeout-based auto hard cancel (the `"timeout"` cancelled_reason value is reserved but not implemented)
- Smoke run hard cancel (Plan 13 surface; deferred to follow-on)
- Hard cancel during build (build is soft-cancelled by killing the build child; sub-build descendants survive — accept this for v1 since builds are short and easy to retry)
- Custom signal selection (always SIGKILL on Unix, always TerminateJobObject on Windows)
- Cleanup of orphaned temp files / sockets the handler may have left behind (handler's responsibility)
- Audit log of "who pressed force kill, when" (the `cancelled_reason` column is sufficient)

## 8. Testing

| Suite | Adds | Notes |
|---|---|---|
| rowforge-core | ~10 | ProcessGroup spawn (Unix); killpg terminates child + grandchild; Windows Job Object (gated behind `#[cfg(windows)]`); hard_kill is idempotent; RunRequest.hard_cancel triggers immediate worker termination; pending rows synthesized as Cancelled with HARD_CANCEL code; soft + hard cancel race resolves to hard |
| rowforge-studio-core | ~4 | hard_cancel_run sets flag; AttemptNotRunning when no session; cancelled_reason persisted to sqlite; sqlite migration applied idempotently |
| studio-shell ipc_contract | ~1 | hard_cancel_run registered |
| vitest | ~6 | CancelDialog 5s timer reveals Force kill; typed-confirm validates "force" exact match; mutation calls hard_cancel_run with correct args; "Force-killed" badge renders when cancelled_reason==hard_cancel; soft cancel still works (no regression); buttons disabled during pending |

Targets:
- cargo: 408 → ~422 (+14)
- vitest: 166 → ~172 (+6)

**Test environment caveat:** the grandchild-kill test spawns a tiny Go fixture (in `tests/fixtures/`) that spawns a sleeper child and writes the child's PID. Test asserts both PIDs are unkillable after `killpg`. Windows tests gated behind `#[cfg(windows)]` will only run on the Windows CI matrix (not Mac dev).

## 9. Spec doc updates

- `docs/spec/studio/part-3-runtime.md`: process group / Job Object section; hard_cancel signal flow; cancelled_reason column
- `docs/spec/studio/part-5-api.md`: new `hard_cancel_run` command + updated `attempt` DTO with `cancelled_reason`
- `docs/spec/studio/part-7-ui.md`: CancelRunDialog escalation flow + Force-killed badge
- `docs/spec/studio/part-9-cli.md` (if exists; otherwise part-5): `attempt hard-cancel` subcommand
- Mirror in zh-Hant
- HUMAN_SMOKE Plan 14: ~20 steps including:
  - Start a run with a deliberately-stuck handler fixture (sleep 600s)
  - Soft cancel → observe "Cancelling…" persists past grace
  - 5s timer reveals Force kill
  - Typed "force" confirms
  - Worker process + child sleep process both gone (verified via `ps`)
  - Attempt shows "Force-killed" badge
  - Pending rows in outcomes.jsonl have code HARD_CANCEL
  - CLI `rowforge attempt hard-cancel` produces identical result

## 10. Acceptance criteria

1. `cargo build && cargo test` clean (Mac/Linux; Windows tests gated)
2. `pnpm tsc -b && pnpm test && pnpm build` clean
3. Sqlite migration adds `cancelled_reason` column without losing existing rows
4. Soft cancel UX unchanged for happy path
5. Force kill button appears 5s after soft cancel is still pending
6. Typed "force" confirmation gate works (Cancel button stays enabled; Force kill disabled until exact match)
7. After force kill: worker process AND its child processes are terminated (verified by spawning a known-grandchild fixture)
8. Attempt finalized with state=cancelled, cancelled_reason="hard_cancel"
9. "Force-killed" red badge renders on AttemptDetail and ExecDetail AttemptsList
10. Pending rows appear in outcomes.jsonl with code HARD_CANCEL and status cancelled
11. CLI `rowforge attempt hard-cancel <exec> <attempt>` works on a running attempt
12. Hard-cancelled attempt releases the Plan 10 active-attempt sqlite gate (delete / re-run / new exec all unblocked)
13. HUMAN_SMOKE Plan 14 walkthrough added
14. Spec docs (part-3 + part-5 + part-7 en + zh-Hant) updated

## 11. Open questions

None at design time. Two intentional follow-ons noted:
- Smoke run force kill (Plan 13 surface)
- Timeout-based auto hard cancel (reserves `cancelled_reason="timeout"` value)
