# Plan 14 — Hard cancel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkbox syntax.

**Goal:** Make `CancelMode::Hard` actually terminate the worker child process and its descendants (process group), instead of falling back to soft cancel. Persist `cancelled_reason = "hard_cancel"` on the attempt row. Surface a "Force-killed" badge in the UI.

**Architecture:** Workers spawn into their own POSIX process group (`setsid` via `Command::pre_exec`). `RunRequest` gains a parallel `hard_cancel: Arc<AtomicBool>` flag. When set AND the cancel token fires, the worker loop calls `Worker::hard_kill()` (SIGKILL to the process group) instead of `worker.shutdown(grace)`. A new sqlite migration adds `attempts.cancelled_reason TEXT NULL`. UI reads this column to render a red "Force-killed" badge on cancelled attempts.

**Tech Stack:** Rust (`std::os::unix::process::CommandExt::pre_exec`, `libc::setsid` + `libc::killpg` + `libc::SIGKILL`), Tokio, rusqlite migration, React.

**Design spec:** `docs/superpowers/specs/2026-05-26-studio-plan-14-hard-cancel-design.md`

**Deviations from design:**
- **Windows is out of scope.** Design § 3.1 mentioned Job Objects; this implementation is Unix-only and gates the new behavior behind `#[cfg(unix)]`. Hard cancel on Windows degrades to soft cancel with a tracing::warn (existing behavior). Adding Windows support is a clean follow-on.
- **5s → 10s force-kill reveal threshold.** Design said 5s; the existing `CancelDialog.tsx` already uses 10s (`FORCE_KILL_THRESHOLD_MS = 10_000`). We keep 10s — already shipped, well-tested.
- **Typed confirm token: first 4 chars of exec name** (existing behavior), not the design's `"force"` literal. Existing wins — better friction than a fixed word.
- **No pending-row synthesis** in v1. Rows that were dispatched-but-unanswered when the process group is killed are simply absent from `outcomes.jsonl` (same shape as soft cancel today). The "Force-killed" badge tells the user "rows are missing because we hard-killed". Adding `RowOutcome::Cancelled { code: "HARD_CANCEL" }` synthesis is a clean follow-on if user feedback wants it.
- **Smoke run hard-cancel** explicitly deferred (design § 5.4). Plan 14 only covers exec attempts.

---

## File map

| Path | Role | Action |
|------|------|--------|
| `crates/rowforge-core/src/worker.rs` | Worker spawn + process group + hard_kill | Modify |
| `crates/rowforge-core/src/run.rs` | RunRequest.hard_cancel field | Modify |
| `crates/rowforge-core/src/pool_streaming.rs` | Thread hard_cancel into worker loop | Modify |
| `crates/rowforge-core/src/worker_loop.rs` | Branch on hard_cancel when token fires | Modify |
| `crates/rowforge-core/Cargo.toml` | Add `libc` dep | Modify |
| `crates/rowforge-core/src/execution_store.rs` | MIGRATION_V4: cancelled_reason column; schema 3 → 4 | Modify |
| `crates/rowforge-studio-core/src/session.rs` | Session.hard_cancel: Arc<AtomicBool> | Modify |
| `crates/rowforge-studio-core/src/run.rs` | Wire hard_cancel into RunRequest; CancelMode::Hard sets the flag; finalize cancelled_reason | Modify |
| `crates/rowforge-studio-core/src/attempt_detail.rs` | AttemptDetail.cancelled_reason | Modify |
| `crates/rowforge-studio-core/src/exec_detail.rs` | AttemptSummary.cancelled_reason | Modify |
| `apps/rowforge-studio/src/ipc/types.ts` | AttemptSummary + AttemptDetail TS shape | Modify |
| `apps/rowforge-studio/src/pages/AttemptDetail.tsx` | Force-killed badge | Modify |
| `apps/rowforge-studio/src/pages/ExecDetail.tsx` | Force-killed badge in AttemptsList | Modify |
| `crates/rowforge-cli/src/attempt_hard_cancel_cmd.rs` | New CLI subcommand | **Create** |
| `crates/rowforge-cli/src/main.rs` | Register subcommand | Modify |
| `docs/spec/studio/part-3-runtime.md` (+zh-Hant) | Process group + hard cancel docs | Modify |
| `docs/spec/studio/part-5-api.md` (+zh-Hant) | cancelled_reason on Attempt DTOs | Modify |
| `docs/spec/studio/part-7-ui.md` (+zh-Hant) | Force-killed badge | Modify |
| `apps/rowforge-studio/HUMAN_SMOKE.md` | Plan 14 walkthrough | Modify |

---

## Task 1: Worker process group spawning (Unix)

**Files:**
- Modify: `crates/rowforge-core/src/worker.rs`
- Modify: `crates/rowforge-core/Cargo.toml`
- Test: inline `#[cfg(test)] mod tests` (later tasks add hard_kill tests)

- [ ] **Step 1: Add libc dep**

Edit `crates/rowforge-core/Cargo.toml`. Under `[dependencies]`, add:

```toml
libc = "0.2"
```

Run `cargo build -p rowforge-core` to confirm. No behavior change yet.

- [ ] **Step 2: Add pgid field to Worker struct**

In `crates/rowforge-core/src/worker.rs`, locate the `pub struct Worker { ... }` definition (around line 61). Add a new field at the bottom (before the closing brace):

```rust
    /// Unix process group id of the child. Set on Unix; `None` on Windows.
    /// Used by `Worker::hard_kill` to send SIGKILL to the entire process
    /// group (child + grandchildren). On Unix, equal to `child.id()` after
    /// `setsid()` in `pre_exec`.
    pub(crate) pgid: Option<i32>,
```

- [ ] **Step 3: Install setsid via pre_exec**

Still in `crates/rowforge-core/src/worker.rs`, find the `command.spawn()` block in `Worker::spawn` (around line 97-101). BEFORE the `command.spawn()` call, add the pre_exec hook. The new block:

```rust
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Plan 14: spawn the worker into its own POSIX process group so a
        // hard cancel can SIGKILL the child AND any grandchildren via killpg.
        // Set BEFORE spawn(). Tokio's Command re-exports
        // std::os::unix::process::CommandExt::pre_exec on Unix targets.
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().map_err(CoreError::Io)?;
```

- [ ] **Step 4: Capture pgid after spawn**

Still in `Worker::spawn`, AFTER `let mut child = command.spawn()...?;` and BEFORE the `let stdin = child.stdin.take()...` line, add:

```rust
        #[cfg(unix)]
        let pgid: Option<i32> = child.id().map(|p| p as i32);
        #[cfg(not(unix))]
        let pgid: Option<i32> = None;
```

Then, where `Worker { ... }` is constructed (around line 108), add the new field `pgid` in the struct literal:

```rust
        let mut w = Worker {
            id,
            child,
            stdin,
            stdout,
            handler_version: String::new(),
            log_sink: None,
            pre_ready_log_lines: Vec::new(),
            pgid,
        };
```

- [ ] **Step 5: Build + run existing tests; nothing should regress**

```
cargo test -p rowforge-core --lib worker:: 2>&1 | tail -10
```

Expected: all pre-existing worker tests still PASS. Process group setup is a no-op for happy-path tests.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-core/Cargo.toml \
        crates/rowforge-core/src/worker.rs
git commit -m "rowforge-core: Plan 14 T1 — spawn worker into own process group (Unix)"
```

---

## Task 2: Worker::hard_kill method

**Files:**
- Modify: `crates/rowforge-core/src/worker.rs`
- Test: inline tests in the same file

- [ ] **Step 1: Failing test for hard_kill on a long-sleep handler**

In `crates/rowforge-core/src/worker.rs`, find the existing `#[cfg(test)] mod tests` block (likely at the bottom of the file). Add:

```rust
#[cfg(all(test, unix))]
#[tokio::test(flavor = "multi_thread")]
async fn hard_kill_terminates_child_immediately() {
    // Sleep for 1 hour; if hard_kill works, this completes in <1s.
    let dir = tempfile::tempdir().unwrap();
    let manifest_yaml = r#"name: sleepy
version: "1"
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
"#;
    std::fs::write(dir.path().join("rowforge.yaml"), manifest_yaml).unwrap();
    let py = r#"#!/usr/bin/env python3
import sys, json, time
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "init":
        print(json.dumps({"type": "ready", "handler_version": "1.0"}), flush=True)
        # Now sleep forever; hard_kill must terminate us.
        time.sleep(3600)
"#;
    std::fs::write(dir.path().join("handler.py"), py).unwrap();

    let (manifest, _) = crate::manifest::Manifest::load_from_dir(dir.path()).unwrap();
    let mut worker = Worker::spawn(
        0,
        dir.path(),
        &manifest,
        "test",
        &Default::default(),
        &[],
    )
    .await
    .unwrap();

    let started = std::time::Instant::now();
    worker.hard_kill().await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "hard_kill should be near-instant; took {:?}",
        elapsed
    );
}
```

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-core --lib worker::tests::hard_kill_terminates_child_immediately 2>&1 | tail -5
```

Expected: `hard_kill` method not found.

- [ ] **Step 3: Implement Worker::hard_kill**

In `crates/rowforge-core/src/worker.rs`, near the existing `shutdown` method (around line 370), add:

```rust
    /// Plan 14: terminate the worker AND its entire process group
    /// immediately (SIGKILL). Used by hard cancel. Does NOT send shutdown
    /// or wait for graceful drain.
    ///
    /// Unix: `killpg(pgid, SIGKILL)`. Falls back to `child.kill()` if pgid
    /// is unset (shouldn't happen on Unix after Plan 14 T1, but defensive).
    ///
    /// Non-Unix: behaves like `child.kill()` (no process group support
    /// yet).
    pub async fn hard_kill(&mut self) -> Result<(), CoreError> {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            let r = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            if r == -1 {
                let err = std::io::Error::last_os_error();
                // ESRCH (no such process) is fine — the child may have
                // already exited; just reap and return.
                if err.raw_os_error() != Some(libc::ESRCH) {
                    tracing::warn!(pgid, error = %err, "killpg failed; falling back to child.kill");
                }
            }
        }
        // Belt + suspenders: also signal the direct child handle.
        let _ = self.child.kill().await;
        // Reap to avoid zombies; ignore wait failures.
        let _ = self.child.wait().await;
        Ok(())
    }
```

- [ ] **Step 4: Run test; expect PASS**

```
cargo test -p rowforge-core --lib worker::tests::hard_kill_terminates_child_immediately 2>&1 | tail -5
```

Expected: 1 PASS, elapsed < 2s.

- [ ] **Step 5: Add a test that hard_kill also reaps grandchildren**

Add another test below:

```rust
#[cfg(all(test, unix))]
#[tokio::test(flavor = "multi_thread")]
async fn hard_kill_terminates_grandchild() {
    // Parent forks a long-sleeping child; we verify both are gone after hard_kill.
    let dir = tempfile::tempdir().unwrap();
    let manifest_yaml = r#"name: forky
version: "1"
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
"#;
    std::fs::write(dir.path().join("rowforge.yaml"), manifest_yaml).unwrap();
    let child_pid_file = dir.path().join("child.pid");
    let py = format!(
        r#"#!/usr/bin/env python3
import sys, json, os, time, subprocess
# Spawn a long-sleeping grandchild; write its PID for the test to check.
proc = subprocess.Popen(["sleep", "3600"])
with open(r"{}", "w") as f:
    f.write(str(proc.pid))
    f.flush()
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("type") == "init":
        print(json.dumps({{"type": "ready", "handler_version": "1.0"}}), flush=True)
        time.sleep(3600)
"#,
        child_pid_file.display()
    );
    std::fs::write(dir.path().join("handler.py"), py).unwrap();

    let (manifest, _) = crate::manifest::Manifest::load_from_dir(dir.path()).unwrap();
    let mut worker = Worker::spawn(
        0, dir.path(), &manifest, "test", &Default::default(), &[],
    )
    .await
    .unwrap();
    // Wait briefly for the python handler to spawn the grandchild.
    for _ in 0..50 {
        if child_pid_file.exists() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let grandchild_pid: i32 = std::fs::read_to_string(&child_pid_file)
        .expect("grandchild pid file should exist")
        .trim()
        .parse()
        .unwrap();

    worker.hard_kill().await.unwrap();

    // Give the kernel ~250ms to reap the grandchild. killpg is synchronous
    // but waitpid on an orphaned grandchild can be asynchronous.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Verify grandchild PID is no longer alive. kill(pid, 0) returns 0 if
    // alive, -1 if not. ESRCH means "no such process".
    let alive = unsafe { libc::kill(grandchild_pid, 0) };
    assert_eq!(
        alive, -1,
        "grandchild pid {} should be dead after hard_kill, got alive=0",
        grandchild_pid
    );
}
```

- [ ] **Step 6: Run all worker tests**

```
cargo test -p rowforge-core --lib worker:: 2>&1 | tail -10
```

Expected: hard_kill tests PASS; pre-existing tests still PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/worker.rs
git commit -m "rowforge-core: Plan 14 T2 — Worker::hard_kill via killpg"
```

---

## Task 3: RunRequest.hard_cancel flag plumbing

**Files:**
- Modify: `crates/rowforge-core/src/run.rs` (RunRequest struct + pass to pool_streaming)
- Modify: `crates/rowforge-core/src/pool_streaming.rs` (StreamingPoolConfig + plumb to worker loops)

- [ ] **Step 1: Add field to RunRequest**

In `crates/rowforge-core/src/run.rs`, find the `pub struct RunRequest { ... }` (around line 56). Add a new field at the bottom (before the closing brace):

```rust
    /// Plan 14: when set to `true` AND `cancel` token fires, the pool kills
    /// each worker's process group via `Worker::hard_kill` instead of
    /// `shutdown(grace)`. `None` / `false` means soft cancel (default).
    ///
    /// Always paired with `cancel`: the caller sets `hard_cancel.store(true,
    /// Relaxed)` THEN fires `cancel.cancel()`. The worker loop checks
    /// `hard_cancel.load()` only after observing the cancel token fired.
    pub hard_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
```

- [ ] **Step 2: Find all RunRequest constructors and add hard_cancel: None**

Run:
```
grep -rn "RunRequest {" crates/ apps/ 2>&1 | grep -v target | head -20
```

Each construction site must be updated to include `hard_cancel: None`. Likely sites: CLI exec_cmd, studio-core start_run, tests. Add `hard_cancel: None,` to each struct literal until `cargo build --workspace` is clean.

- [ ] **Step 3: Pass hard_cancel through pool_streaming**

In `crates/rowforge-core/src/pool_streaming.rs`, find the `StreamingPoolConfig` struct. Add a new field:

```rust
    /// Plan 14: when set AND `cancel` token fires, workers receive SIGKILL
    /// to their process group instead of graceful shutdown.
    pub hard_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
```

In `crates/rowforge-core/src/run.rs`, find where `StreamingPoolConfig` is built from `RunRequest` (search for `StreamingPoolConfig {` — likely in the `execute` function around the area that calls `pool_streaming::run`). Add:

```rust
    hard_cancel: req.hard_cancel.clone(),
```

In `pool_streaming.rs::run`, find where each worker loop is spawned (around line 286-303 where `run_worker_loop` is called). Pass `hard_cancel.clone()` through to the worker loop:

```rust
                let cancel_clone = cancel.clone();
                let mode_clone = mode.clone();
                let on_row_done_clone = cfg.on_row_done.clone();
                let hard_cancel_clone = cfg.hard_cancel.clone();

                let h = tokio::spawn(async move {
                    run_worker_loop(
                        worker,
                        mode_clone,
                        idempotent,
                        job_rx_clone,
                        jsonl_clone,
                        grace,
                        Some(cancel_clone),
                        on_row_done_clone,
                        hard_cancel_clone,
                    )
                    .await
                });
```

(Note: this changes `run_worker_loop`'s signature; T4 implements the parameter and consumption.)

- [ ] **Step 4: Build check (will fail at run_worker_loop signature; that's T4)**

```
cargo build -p rowforge-core 2>&1 | tail -10
```

Expected: error about `run_worker_loop` signature mismatch. Defer to T4.

- [ ] **Step 5: For now, update `run_worker_loop`'s declaration to accept the extra parameter as `_unused`**

Open `crates/rowforge-core/src/worker_loop.rs`. Find `pub async fn run_worker_loop` and add a new parameter at the end. T4 will USE it; here we just add the parameter so T3 compiles:

```rust
pub async fn run_worker_loop(
    mut worker: Worker,
    mode: Mode,
    idempotent: bool,
    job_rx: tokio::sync::mpsc::Receiver<RowJob>,  // signature may differ; match what's there
    jsonl: Arc<Mutex<JsonlWriter>>,
    grace: Duration,
    cancel: Option<CancellationToken>,
    on_row_done: Option<OnRowDoneCallback>,
    _hard_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) {
    // existing body unchanged; the new parameter is ignored until T4.
}
```

(Match the exact existing parameter names and types — only the trailing `_hard_cancel` is new.)

- [ ] **Step 6: Build clean**

```
cargo build --workspace 2>&1 | tail -5
cargo test -p rowforge-core 2>&1 | tail -5
```

Expected: clean build; existing rowforge-core tests still pass.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/run.rs \
        crates/rowforge-core/src/pool_streaming.rs \
        crates/rowforge-core/src/worker_loop.rs \
        crates/rowforge-cli/ \
        crates/rowforge-studio-core/src/run.rs
# Add whatever else cargo build identified as needing the hard_cancel: None addition.
git commit -m "rowforge-core: Plan 14 T3 — RunRequest.hard_cancel plumbed through pool_streaming"
```

---

## Task 4: Worker loop branches on hard_cancel when token fires

**Files:**
- Modify: `crates/rowforge-core/src/worker_loop.rs`
- Test: inline integration test in worker_loop.rs OR via pool_streaming.rs tests

- [ ] **Step 1: Read the current shutdown path**

Open `crates/rowforge-core/src/worker_loop.rs` and find where the cancel token's "cancelled" arm leads to worker shutdown. The pattern is something like:

```rust
_ = cancel_clone.cancelled() => {
    let _ = worker.shutdown(grace).await;
    break;
}
```

The exact location and surrounding code may vary; read the file to find the right place. (Search for `worker.shutdown` to locate it.)

- [ ] **Step 2: Add the hard_cancel branch**

Modify the shutdown arm to check `hard_cancel`:

```rust
_ = cancel_clone.cancelled() => {
    let do_hard_kill = _hard_cancel
        .as_ref()
        .map(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false);
    if do_hard_kill {
        tracing::info!(worker = worker.id, "hard cancel: killing process group");
        let _ = worker.hard_kill().await;
    } else {
        let _ = worker.shutdown(grace).await;
    }
    break;
}
```

Rename the parameter from `_hard_cancel` to `hard_cancel` (drop the leading underscore — it's now used):

```rust
    hard_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
```

- [ ] **Step 3: Build + run existing tests**

```
cargo test -p rowforge-core 2>&1 | tail -10
```

Expected: all rowforge-core tests still PASS. Existing soft cancel behavior unchanged (hard_cancel defaults to None / false).

- [ ] **Step 4: Failing integration test for hard cancel**

In `crates/rowforge-core/src/pool_streaming.rs` `#[cfg(test)] mod tests`, find the existing cancel test pattern (search for `cancel` or `CancellationToken` inside `mod tests`). Mirror that pattern for a hard-cancel test. Skeleton:

```rust
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn hard_cancel_kills_workers_immediately() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let tmp = tempfile::tempdir().unwrap();

    // Sleepy handler that responds to row but never to shutdown.
    let manifest_yaml = r#"name: stubborn
version: "1"
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
"#;
    let hdir = tmp.path().join("handler");
    std::fs::create_dir_all(&hdir).unwrap();
    std::fs::write(hdir.join("rowforge.yaml"), manifest_yaml).unwrap();
    let py = r#"#!/usr/bin/env python3
import sys, json, time
for line in sys.stdin:
    msg = json.loads(line)
    t = msg.get("type")
    if t == "init":
        print(json.dumps({"type": "ready", "handler_version": "1.0"}), flush=True)
    elif t == "row":
        time.sleep(3600)   # Stuck forever
    elif t == "shutdown":
        time.sleep(3600)   # IGNORE shutdown signal
"#;
    std::fs::write(hdir.join("handler.py"), py).unwrap();

    let cancel = crate::cancel::CancellationToken::new();
    let hard = std::sync::Arc::new(AtomicBool::new(false));
    let input = tmp.path().join("input.csv");
    std::fs::write(&input, b"id\n1\n2\n3\n").unwrap();
    let out = tmp.path().join("out");

    let req = crate::run::RunRequest {
        run_id: "t".into(),
        parent_run_id: None,
        handler_dir: hdir.clone(),
        input_csv: input,
        output_dir: out,
        workers: 1,
        dry_run: false,
        dry_run_sample: 0,
        row_limit: None,
        skip_seqs: Default::default(),
        field_map: Default::default(),
        config_overrides: Default::default(),
        shutdown_grace: std::time::Duration::from_secs(30),
        on_progress: None,
        on_handler_log: None,
        cancel: Some(cancel.clone()),
        input_format: None,
        fsync_outcomes: false,
        capture_raw_stdout: false,
        only_row_ids: None,
        hard_cancel: Some(hard.clone()),
    };

    let started = std::time::Instant::now();
    let exec_handle = tokio::spawn(crate::run::execute(req));

    // Let init + first row dispatch happen.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Trigger hard cancel.
    hard.store(true, Ordering::Relaxed);
    cancel.cancel();

    // execute must return within shutdown_grace * 0.5 (way before 30s).
    let report = tokio::time::timeout(std::time::Duration::from_secs(10), exec_handle)
        .await
        .expect("run finished within 10s after hard cancel")
        .unwrap()
        .unwrap();
    let elapsed = started.elapsed();
    assert!(report.aborted, "report should be aborted");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "hard cancel should finish under 5s; took {:?}",
        elapsed
    );
}
```

- [ ] **Step 5: Run; expect PASS**

```
cargo test -p rowforge-core --lib pool_streaming::tests::hard_cancel_kills_workers_immediately 2>&1 | tail -10
```

Expected: PASS, elapsed < 5s. If it times out, the worker loop branch isn't taking the hard_kill path — re-check Step 2.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-core/src/worker_loop.rs \
        crates/rowforge-core/src/pool_streaming.rs
git commit -m "rowforge-core: Plan 14 T4 — worker loop branches on hard_cancel"
```

---

## Task 5: Session.hard_cancel + StudioCore wiring

**Files:**
- Modify: `crates/rowforge-studio-core/src/session.rs`
- Modify: `crates/rowforge-studio-core/src/run.rs`

- [ ] **Step 1: Add field to Session struct**

In `crates/rowforge-studio-core/src/session.rs`, locate `pub struct Session { ... }` (around line 19). Add:

```rust
    /// Plan 14: paired with `cancel_token`. When set true BEFORE
    /// `cancel_token.cancel()` fires, the run aborts via SIGKILL to each
    /// worker's process group. When unset, cancel is graceful (soft).
    pub hard_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
```

- [ ] **Step 2: Initialize in start_run + register_fake_session_for_test + tests**

In the same file, find every `Session { ... }` construction. There are at least 3:

1. The real one in `crates/rowforge-studio-core/src/run.rs::start_run` (around line 320; search for `Session {`).
2. `register_fake_session_for_test` in session.rs (around line 176).
3. `fake_session` helpers inside `mod tests` (around line 261 and 277).

Add `hard_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),` to each.

- [ ] **Step 3: Pass hard_cancel into RunRequest in start_run**

In `crates/rowforge-studio-core/src/run.rs`, find where `RunRequest { ... }` is constructed inside `start_run` (search for `RunRequest {` — around line 775 based on prior grep). Add:

```rust
    hard_cancel: Some(session.hard_cancel.clone()),
```

The `session` variable should be in scope at that point (it was created above). If the `RunRequest` is built before the `session` is constructed, hoist `hard_cancel` to a local Arc before both:

```rust
let hard_cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
// ... then use hard_cancel_flag.clone() in both RunRequest and Session
```

- [ ] **Step 4: Wire CancelMode::Hard to set the flag**

In `crates/rowforge-studio-core/src/run.rs`, find the `cancel` method's `Hard` arm (around line 555). Replace:

```rust
CancelMode::Hard => {
    // Hard cancel: fire the token immediately. rowforge-core does
    // not currently expose a SIGKILL handle, so Hard == Soft for
    // now. When pool_streaming gains process-kill plumbing, wire
    // it here.
    session.cancel_token.cancel();
    tracing::warn!(
        handle = %h,
        "Hard cancel requested; rowforge-core has no kill handle yet — \
         falling back to soft cancel (token fire)"
    );
}
```

With:

```rust
CancelMode::Hard => {
    // Plan 14: set hard_cancel FIRST, then fire the token. The worker
    // loop observes the cancel token, then checks hard_cancel.load()
    // and branches to killpg() instead of graceful shutdown.
    session
        .hard_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    session.cancel_token.cancel();
}
```

- [ ] **Step 5: Build clean**

```
cargo build --workspace 2>&1 | tail -5
cargo test -p rowforge-studio-core 2>&1 | tail -10
```

Expected: clean build; existing tests still PASS (the cancel-test that exercised `CancelMode::Hard` previously relied on the tracing::warn fallback; it'll still work because soft cancel still happens — just hard_cancel is now also set).

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-studio-core/src/session.rs \
        crates/rowforge-studio-core/src/run.rs
git commit -m "studio-core: Plan 14 T5 — Session.hard_cancel + CancelMode::Hard now SIGKILLs"
```

---

## Task 6: sqlite MIGRATION_V4 — attempts.cancelled_reason column

**Files:**
- Modify: `crates/rowforge-core/src/execution_store.rs`

- [ ] **Step 1: Failing test for the new column**

In `crates/rowforge-core/src/execution_store.rs` tests module, add:

```rust
#[test]
fn migration_v4_adds_cancelled_reason_column() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ExecutionStore::open(tmp.path()).unwrap();
    let count: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name='cancelled_reason'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1, "cancelled_reason column should exist after migration");
}
```

If `store.conn` isn't pub(crate)-accessible from the test (it's likely private), use a helper or expose a query method. Look at the existing test patterns — there may be a `store.execute_raw` helper or you may need to add a small `pub(crate) fn raw_query_count` helper just for this assertion. The simplest path: open the connection directly via `rusqlite::Connection::open(tmp.path().join("executions.db"))` after `ExecutionStore::open` is called, then query.

```rust
#[test]
fn migration_v4_adds_cancelled_reason_column() {
    let tmp = tempfile::tempdir().unwrap();
    let _store = ExecutionStore::open(tmp.path()).unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("executions.db")).unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name='cancelled_reason'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
```

Confirm the exact DB filename by reading `ExecutionStore::open` (likely `executions.db` under `home`).

- [ ] **Step 2: Verify FAIL**

```
cargo test -p rowforge-core --lib migration_v4_adds_cancelled_reason_column 2>&1 | tail -5
```

Expected: assertion fails (count is 0).

- [ ] **Step 3: Add MIGRATION_V4**

In `crates/rowforge-core/src/execution_store.rs`, around line 776 (where `MIGRATION_V3` is defined), add after it:

```rust
const MIGRATION_V4: &str = "
ALTER TABLE attempts ADD COLUMN cancelled_reason TEXT;
";
```

Bump the schema constant at line 22:

```rust
const SCHEMA_VERSION: i64 = 4;
```

Update the migrate() match block (around line 228-261) to handle the new version transition:

```rust
        match current {
            None => {
                self.conn.execute_batch(MIGRATION_V1)?;
                self.conn.execute_batch(MIGRATION_V2)?;
                self.conn.execute_batch(MIGRATION_V3)?;
                self.conn.execute_batch(MIGRATION_V4)?;
                self.conn.execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )?;
            }
            Some(1) => {
                self.conn.execute_batch(MIGRATION_V2)?;
                self.conn.execute_batch(MIGRATION_V3)?;
                self.conn.execute_batch(MIGRATION_V4)?;
                self.conn
                    .execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
            }
            Some(2) => {
                self.conn.execute_batch(MIGRATION_V3)?;
                self.conn.execute_batch(MIGRATION_V4)?;
                self.conn
                    .execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
            }
            Some(3) => {
                self.conn.execute_batch(MIGRATION_V4)?;
                self.conn
                    .execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            // ... existing too-new / unsupported branches unchanged
        }
```

- [ ] **Step 4: Run test; expect PASS**

```
cargo test -p rowforge-core --lib migration_v4 2>&1 | tail -5
```

Expected: PASS.

- [ ] **Step 5: Add migration-from-v3 upgrade test**

To verify the upgrade path (not just fresh install), add another test:

```rust
#[test]
fn migration_v4_upgrades_from_v3_db() {
    let tmp = tempfile::tempdir().unwrap();
    // First create a v3 DB by manually setting version=3 after fresh open
    // (then re-opening should trigger the V4 upgrade).
    {
        let _store = ExecutionStore::open(tmp.path()).unwrap();
        let conn = rusqlite::Connection::open(tmp.path().join("executions.db")).unwrap();
        // Drop the v4-only column to simulate a pre-Plan-14 DB.
        // Actually easier: rewrite schema_version to 3 and drop the column.
        // SQLite's ALTER TABLE doesn't support DROP COLUMN in older versions,
        // so for the test we just blow away the column by rebuilding.
        // Simplest: set schema_version to 3 and run open again — it should
        // re-apply V4 idempotently. But ALTER TABLE ADD will fail if column
        // exists. Test the path more directly by inspecting what migrate()
        // does on Some(3).
        conn.execute("UPDATE schema_version SET version = 3", []).unwrap();
        conn.execute("ALTER TABLE attempts DROP COLUMN cancelled_reason", []).unwrap();
    }
    // Re-open should migrate v3 → v4.
    let _store = ExecutionStore::open(tmp.path()).unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("executions.db")).unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name='cancelled_reason'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    let version: i64 = conn.query_row("SELECT version FROM schema_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 4);
}
```

(Note: SQLite 3.35+ supports DROP COLUMN. If the test fails on a CI runner with older sqlite, simplify the test by reading the table_info before AND after — that's already what the first test does. Drop this second test if needed; the first one is sufficient.)

Run it:

```
cargo test -p rowforge-core --lib migration_v4_upgrades_from_v3_db 2>&1 | tail -5
```

If it errors on `DROP COLUMN`, just delete this second test and keep only the fresh-install one.

- [ ] **Step 6: Run all execution_store tests**

```
cargo test -p rowforge-core --lib execution_store:: 2>&1 | tail -10
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/execution_store.rs
git commit -m "rowforge-core: Plan 14 T6 — MIGRATION_V4 adds attempts.cancelled_reason"
```

---

## Task 7: Persist cancelled_reason on hard cancel finalize

**Files:**
- Modify: `crates/rowforge-core/src/execution_store.rs` (FinishAttempt struct + persistence)
- Modify: `crates/rowforge-studio-core/src/run.rs` (call site)
- Test: integration test that hard cancel writes "hard_cancel"

- [ ] **Step 1: Add cancelled_reason field to FinishAttempt**

In `crates/rowforge-core/src/execution_store.rs`, find the `FinishAttempt` struct (around line 167; the existing fields include `aborted_reason: Option<String>`). Add:

```rust
    /// Plan 14: when set, persisted to attempts.cancelled_reason. Values:
    /// `None` (soft cancel or normal completion), `Some("hard_cancel")`
    /// (force-killed via SIGKILL), `Some("timeout")` (reserved).
    pub cancelled_reason: Option<String>,
```

- [ ] **Step 2: Persist on update_attempt_finish**

Find the `UPDATE attempts SET ... WHERE id = ?` query that uses `FinishAttempt` (around line 595-605). Add `cancelled_reason = ?N` to the SET clause and a new params slot.

Current (illustrative — match exact existing code):
```rust
"UPDATE attempts
    SET state=?1, success_count=?2, failed_count=?3,
        aborted_reason=?4, ended_at=?5
   WHERE id=?6"
```

New:
```rust
"UPDATE attempts
    SET state=?1, success_count=?2, failed_count=?3,
        aborted_reason=?4, ended_at=?5, cancelled_reason=?6
   WHERE id=?7"
```

Update the `params![...]` macro call to insert `finish.cancelled_reason` between `finish.ended_at` and the id binding.

Add `cancelled_reason: Option<String>` to the `Attempt` struct return type (around line 186) and ensure `row_to_attempt` (search for that fn or the `r.get(N)` chain in the attempt-by-id query around line 633-650) reads the new column index.

- [ ] **Step 3: Update studio-core call site**

In `crates/rowforge-studio-core/src/run.rs`, find where `update_attempt_finish` / `FinishAttempt` is called after the run completes. There's likely one site that builds a `FinishAttempt { state, success_count, failed_count, aborted_reason, ended_at }` literal. Add:

```rust
    cancelled_reason: if session.hard_cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Some("hard_cancel".to_string())
    } else {
        None
    },
```

The exact location: search `git grep "FinishAttempt {" crates/rowforge-studio-core/` to find it. Likely in the post-run finalize handler.

- [ ] **Step 4: Build clean**

```
cargo build --workspace 2>&1 | tail -5
```

Expected: clean, modulo any test-only constructors of `FinishAttempt` you missed (add `cancelled_reason: None` to those).

- [ ] **Step 5: Integration test — hard cancel sets cancelled_reason**

In `crates/rowforge-studio-core/tests/foundation.rs`, add a test that:
1. Starts a run on a sleepy handler (reuse the Plan 14 T4 sleepy-handler pattern; or write inline if no helper).
2. Triggers hard cancel.
3. After RunReport returns, queries the attempts table directly and asserts `cancelled_reason == 'hard_cancel'`.

Skeleton:

```rust
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn hard_cancel_persists_cancelled_reason() {
    use rowforge_studio_core::{OpenOpts, StudioCore, CancelMode};

    let tmp = tempfile::tempdir().unwrap();
    // Write a handler dir + stubborn handler.py (loops forever on row).
    let hdir = tmp.path().join("handlers").join("stubborn");
    std::fs::create_dir_all(&hdir).unwrap();
    std::fs::write(
        hdir.join("rowforge.yaml"),
        r#"name: stubborn
version: "1"
kind: row
primary_field: id
entry:
  cmd: ["python3", "handler.py"]
"#,
    )
    .unwrap();
    std::fs::write(
        hdir.join("handler.py"),
        r#"#!/usr/bin/env python3
import sys, json, time
for line in sys.stdin:
    msg = json.loads(line)
    t = msg.get("type")
    if t == "init":
        print(json.dumps({"type": "ready", "handler_version": "1.0"}), flush=True)
    elif t == "row":
        time.sleep(3600)
"#,
    )
    .unwrap();

    // Build the handler binary path used by exec_start — workspace handlers/<name>
    // is already at the right location.

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path())).unwrap();

    // Create input csv + exec
    let input = tmp.path().join("input.csv");
    std::fs::write(&input, b"id\n1\n2\n3\n").unwrap();
    let exec_id = core
        .start_exec(rowforge_studio_core::StartExecArgs::new(&input, "t1"))
        .unwrap();

    // Start run
    let handle = core
        .start_run(
            rowforge_studio_core::run::RunOpts::new(&exec_id, &hdir),
        )
        .await
        .unwrap()
        .handle;
    // Wait a beat so the worker dispatches its first row.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Hard cancel
    core.cancel(&handle, CancelMode::Hard).unwrap();

    // Wait for the run to finalize.
    for _ in 0..50 {
        let status = core.status(&handle);
        if matches!(status, Err(_) | Ok(rowforge_studio_core::RunStatus::Aborted) | Ok(rowforge_studio_core::RunStatus::Done)) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Query the attempts table directly to assert cancelled_reason.
    let db = tmp.path().join("executions.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let reason: Option<String> = conn
        .query_row(
            "SELECT cancelled_reason FROM attempts WHERE execution_id = ?1",
            rusqlite::params![exec_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason.as_deref(), Some("hard_cancel"));
}
```

(Method names like `RunOpts::new`, `start_run`, `start_exec`, `core.cancel`, `core.status` are best-effort — read the actual API in lib.rs / run.rs to confirm exact names. The pattern of seeding a workspace + handler + exec + run exists from prior plans; T11 of Plan 13 has a comparable handler fixture.)

- [ ] **Step 6: Run; expect PASS**

```
cargo test -p rowforge-studio-core --test foundation hard_cancel_persists_cancelled_reason 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/rowforge-core/src/execution_store.rs \
        crates/rowforge-studio-core/src/run.rs \
        crates/rowforge-studio-core/tests/foundation.rs
git commit -m "rowforge-core: Plan 14 T7 — persist cancelled_reason=hard_cancel on finalize"
```

---

## Task 8: Surface cancelled_reason on AttemptSummary + AttemptDetail DTOs

**Files:**
- Modify: `crates/rowforge-studio-core/src/attempt_detail.rs`
- Modify: `crates/rowforge-studio-core/src/exec_detail.rs`

- [ ] **Step 1: Add field to AttemptSummary**

In `crates/rowforge-studio-core/src/exec_detail.rs`, find `pub struct AttemptSummary { ... }`. Add:

```rust
    /// Plan 14: when state is `cancelled`, this carries the reason
    /// (`Some("hard_cancel")` for force-killed, `None` for soft cancel).
    pub cancelled_reason: Option<String>,
```

- [ ] **Step 2: Add field to AttemptDetail or its summary**

In `crates/rowforge-studio-core/src/attempt_detail.rs`, find `pub struct AttemptDetail { ... }` and add the same field. (If `AttemptDetail` embeds an `AttemptSummary`, this is already covered; verify.)

- [ ] **Step 3: Populate in queries**

Find where these DTOs are built from the `Attempt` row (search for `AttemptSummary {` and `AttemptDetail {`). Each construction site reads from an `Attempt` struct that now has `cancelled_reason: Option<String>` (after T7's row_to_attempt change). Add:

```rust
    cancelled_reason: row.cancelled_reason.clone(),
```

- [ ] **Step 4: Update ExecDetail-related tests**

If existing tests construct `AttemptSummary { ... }` literally, add `cancelled_reason: None,` to those constructions until `cargo build` is clean.

- [ ] **Step 5: Build + test**

```
cargo build --workspace 2>&1 | tail -5
cargo test -p rowforge-studio-core 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-studio-core/src/attempt_detail.rs \
        crates/rowforge-studio-core/src/exec_detail.rs
git commit -m "studio-core: Plan 14 T8 — cancelled_reason on AttemptSummary + AttemptDetail"
```

---

## Task 9: UI — "Force-killed" badge

**Files:**
- Modify: `apps/rowforge-studio/src/ipc/types.ts`
- Modify: `apps/rowforge-studio/src/pages/AttemptDetail.tsx`
- Modify: `apps/rowforge-studio/src/pages/ExecDetail.tsx`

- [ ] **Step 1: Add cancelled_reason to TS DTOs**

In `apps/rowforge-studio/src/ipc/types.ts`, find `interface AttemptSummary` (and `interface AttemptDetail` if present). Add:

```ts
  /** Plan 14: when state is `cancelled`, distinguishes hard-kill from soft cancel. */
  cancelled_reason: string | null;
```

- [ ] **Step 2: Failing test for the badge (vitest)**

If there's an existing test for AttemptDetail rendering (look under `apps/rowforge-studio/src/__tests__/` or `apps/rowforge-studio/src/pages/__tests__/`), add a case. Otherwise skip the vitest and rely on visual verification in HUMAN_SMOKE.

If you add a test:

```tsx
it("renders Force-killed badge when cancelled_reason is hard_cancel", () => {
  const detail: AttemptDetail = {
    // ...minimal fields with state="cancelled", cancelled_reason="hard_cancel"
  };
  render(<AttemptDetailHeader detail={detail} />);
  expect(screen.getByText(/force-killed/i)).toBeInTheDocument();
});
```

(The actual minimal AttemptDetail shape may be involved; if so, skip this vitest and rely on smoke verification.)

- [ ] **Step 3: Render the badge in AttemptDetail.tsx**

In `apps/rowforge-studio/src/pages/AttemptDetail.tsx`, find where the attempt's state badge is rendered (search for `state ==` or `Cancelled` or whatever the current label uses). Add a branch:

```tsx
{attempt.state === "cancelled" && attempt.cancelled_reason === "hard_cancel" ? (
  <span className="inline-block rounded border border-red-500/40 bg-red-500/10 px-2 py-0.5 text-xs text-red-300">
    Force-killed
  </span>
) : attempt.state === "cancelled" ? (
  <span className="inline-block rounded border border-yellow-500/40 bg-yellow-500/10 px-2 py-0.5 text-xs text-yellow-300">
    Cancelled
  </span>
) : (
  /* existing badges for other states */
)}
```

Adapt to the actual code shape — the existing state→badge mapping likely lives in a small helper. Modify that helper to take the full attempt (not just state) so it can branch on cancelled_reason.

- [ ] **Step 4: Same badge in ExecDetail.tsx AttemptsList**

In `apps/rowforge-studio/src/pages/ExecDetail.tsx`, find the AttemptsList sub-component (added in Plan 10). Apply the same badge logic in the State column.

- [ ] **Step 5: Build + tsc**

```
cd /Users/lemo/code/lemo/repo/rowforge/apps/rowforge-studio && pnpm tsc -b
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add apps/rowforge-studio/src/ipc/types.ts \
        apps/rowforge-studio/src/pages/AttemptDetail.tsx \
        apps/rowforge-studio/src/pages/ExecDetail.tsx
git commit -m "studio-ui: Plan 14 T9 — Force-killed badge"
```

---

## Task 10: CLI subcommand — `rowforge attempt hard-cancel`

**Files:**
- Create: `crates/rowforge-cli/src/attempt_hard_cancel_cmd.rs`
- Modify: `crates/rowforge-cli/src/main.rs`

- [ ] **Step 1: Look at existing CLI subcommand pattern**

Open `crates/rowforge-cli/src/main.rs` and read how Plan 10's `exec delete` subcommand is registered (it lives in `exec_delete_cmd.rs` and is wired into clap's enum). Mirror that pattern.

Also look at `crates/rowforge-cli/src/exec_delete_cmd.rs` for the structure: open StudioCore, look up the entity, call the studio-core method, print result.

- [ ] **Step 2: Write the new subcommand file**

Create `crates/rowforge-cli/src/attempt_hard_cancel_cmd.rs`:

```rust
//! `rowforge attempt hard-cancel <exec_id> <attempt_id>`.
//!
//! Triggers Plan 14 hard cancel (SIGKILL to worker process group) for a
//! running attempt. No-op + error if the attempt is not currently running.

use clap::Args;
use rowforge_studio_core::{CancelMode, OpenOpts, StudioCore};

#[derive(Debug, Args)]
pub struct AttemptHardCancelArgs {
    /// Execution id (e.g. `e_01ABC...`).
    pub exec_id: String,
    /// Attempt id (e.g. `r_01XYZ...`).
    pub attempt_id: String,
    /// Workspace path; defaults to the default workspace root.
    #[arg(long)]
    pub workspace: Option<std::path::PathBuf>,
}

pub fn run(args: AttemptHardCancelArgs) -> Result<(), anyhow::Error> {
    let core = StudioCore::open(
        OpenOpts::new()
            .with_workspace(args.workspace.unwrap_or_else(|| {
                rowforge_core::workspace::default_workspace_root()
                    .expect("no $HOME — cannot resolve default workspace")
            })),
    )?;

    let handle = core
        .active_handle_for_attempt(&args.attempt_id)
        .ok_or_else(|| anyhow::anyhow!(
            "attempt '{}' is not currently running (or finished already)",
            args.attempt_id
        ))?;

    core.cancel(&handle, CancelMode::Hard)?;
    println!("Hard cancel signalled to {} / {}", args.exec_id, args.attempt_id);
    Ok(())
}
```

(Method names: `active_handle_for_attempt` exists per the grep above. `OpenOpts::new()` + `.with_workspace(...)` is the Plan 13 pattern. The `anyhow::Error` wrapping mirrors other CLI subcommands.)

- [ ] **Step 3: Wire into main.rs**

In `crates/rowforge-cli/src/main.rs`, find the clap enum that declares `Exec(...)`, `Handler(...)` etc. Find the `Attempt` subcommand if it exists or add a new top-level group. Then:

```rust
mod attempt_hard_cancel_cmd;

// in the clap enum for attempt subcommands:
HardCancel(crate::attempt_hard_cancel_cmd::AttemptHardCancelArgs),

// in the dispatch match:
AttemptCmd::HardCancel(args) => attempt_hard_cancel_cmd::run(args)?,
```

Exact wiring depends on the existing CLI structure. If there's no `attempt` top-level subcommand yet, you may need to introduce one (look for how `exec` is structured and mirror it).

- [ ] **Step 4: Build clean**

```
cargo build -p rowforge-cli 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5: Smoke the CLI manually (not a vitest/cargo test)**

```
cargo run --bin rowforge -- attempt hard-cancel --help
```

Expected: usage message renders with both `<exec_id>` and `<attempt_id>` arguments.

(Hard-cancel against a real attempt requires a running attempt; defer end-to-end test to HUMAN_SMOKE.)

- [ ] **Step 6: Commit**

```bash
git add crates/rowforge-cli/src/attempt_hard_cancel_cmd.rs \
        crates/rowforge-cli/src/main.rs
git commit -m "rowforge-cli: Plan 14 T10 — attempt hard-cancel subcommand"
```

---

## Task 11: Spec docs + HUMAN_SMOKE Plan 14

**Files:**
- Modify: `docs/spec/studio/part-3-runtime.md` (+ zh-Hant)
- Modify: `docs/spec/studio/part-5-api.md` (+ zh-Hant)
- Modify: `docs/spec/studio/part-7-ui.md` (+ zh-Hant)
- Modify: `apps/rowforge-studio/HUMAN_SMOKE.md`

- [ ] **Step 1: part-3-runtime additions**

Find the section discussing cancel semantics or worker lifecycle. Add:

```md
### Hard cancel (Plan 14)

Workers spawn into their own POSIX process group via `setsid()` (Unix
only). `RunRequest.hard_cancel: Option<Arc<AtomicBool>>` pairs with the
existing `cancel: CancellationToken`. When the caller sets the flag
`true` THEN fires the cancel token, the worker loop calls
`Worker::hard_kill()` — which sends `SIGKILL` to the entire process
group via `killpg(pgid, SIGKILL)` — instead of the graceful
`shutdown(grace)` path. The child AND any grandchildren the handler
spawned are terminated.

Windows: hard cancel is currently equivalent to soft cancel (process
group / Job Object support deferred).

The `attempts.cancelled_reason` column (sqlite migration v4) records
`"hard_cancel"` when force-killed, `NULL` for soft cancel / clean
completion. Reserved value `"timeout"` for future timeout-based
auto-cancel.
```

- [ ] **Step 2: part-5-api additions**

Add `cancelled_reason: string | null` to the `AttemptSummary` and `AttemptDetail` JSON shapes documented in §5.2. Also document that `CancelMode::Hard` now performs a real SIGKILL on Unix (no longer a fallback to soft).

- [ ] **Step 3: part-7-ui additions**

In the AttemptDetail / ExecDetail sections, add:

```md
**Force-killed badge.** When `attempt.state === "cancelled"` AND
`attempt.cancelled_reason === "hard_cancel"`, the state badge renders
in red as "Force-killed" instead of the yellow "Cancelled". Surfaces in
both AttemptDetail header and ExecDetail AttemptsList.

The CancelDialog flow (existing) already drives this state machine: a
soft cancel that doesn't complete within 10s reveals a "Force kill"
button; confirming triggers `cancel(handle, Hard)`. Plan 14 makes that
backend call actually SIGKILL the workers.
```

- [ ] **Step 4: Mirror in zh-Hant**

For each modified English doc, update its `zh-Hant/<name>.md` counterpart.

- [ ] **Step 5: HUMAN_SMOKE Plan 14**

Append to `apps/rowforge-studio/HUMAN_SMOKE.md`:

```md
---

# Manual smoke check — Plan 14 (hard cancel)

## 1. Setup

1. Open Studio in a fresh workspace at `/tmp/plan14-smoke-ws/`.
2. Scaffold a `go_stdio` handler named `stuck` with primary_field `id`.
3. Replace the row handler body with `time.Sleep(time.Hour)` (Go) or
   `time.sleep(3600)` (Python). The handler MUST ignore stdin/shutdown.
4. Build the handler. Verify Last build success.

## 2. Soft cancel as-is

5. Create an exec on a CSV of 10 rows. Click Run.
6. Wait until status flips to Running.
7. Click Cancel. Confirm the soft cancel dialog. The status becomes
   "Cancelling…" and stays there because the handler ignores shutdown.

## 3. Force kill reveal

8. After ~10s, the "Force kill" button appears next to the Cancelling
   banner.
9. Click Force kill. The typed-confirm dialog appears.
10. Type the first 4 chars of the exec name (lowercase).
11. Click "Force kill" in the dialog.

## 4. Verify termination

12. Within 1-2s, the attempt finalizes with state `cancelled`.
13. The state badge in AttemptDetail renders in red as "Force-killed".
14. In a terminal: `ps -ef | grep <handler-binary>` — no zombie/sleeping
    children remain.

## 5. CLI path

15. Start another run with the same stuck handler.
16. In a terminal: `rowforge attempt hard-cancel <exec_id> <attempt_id>
    --workspace /tmp/plan14-smoke-ws/`
17. Studio shows the same force-killed badge after the attempt
    finalizes.

## 6. Grandchild kill

18. Modify the handler to spawn a child sleep process (e.g.
    `subprocess.Popen(["sleep", "3600"])`) before sleeping itself.
19. Run, soft cancel, force kill.
20. After force kill, `ps` shows neither the handler nor its sleep
    grandchild. (On macOS / Linux the process group kill covers both.)

## Known Plan 14 limitations

- Windows: hard cancel degrades to soft cancel. Job Object support is
  a follow-on.
- Smoke runs: hard cancel does not apply to Plan 13 smoke runs. The
  smoke runner has its own grace-shutdown but no SIGKILL path.
- Pending row outcomes: rows dispatched but not yet completed at the
  moment of force-kill are simply absent from `outcomes.jsonl` (same as
  soft cancel today). No explicit `HARD_CANCEL` row outcome is
  synthesized. The "Force-killed" badge tells users "rows are missing
  because of force-kill".
```

- [ ] **Step 6: Verify nothing is broken**

```
cd /Users/lemo/code/lemo/repo/rowforge && cargo test --workspace 2>&1 | tail -3
cd /Users/lemo/code/lemo/repo/rowforge/apps/rowforge-studio && pnpm tsc -b
```

Both clean.

- [ ] **Step 7: Commit**

```bash
git add docs/spec/studio/ apps/rowforge-studio/HUMAN_SMOKE.md
git commit -m "docs: Plan 14 — runtime + API + UI spec sync + HUMAN_SMOKE"
```

---

## Final acceptance gates

- [ ] `cargo build && cargo test --workspace` clean (target ~445+ PASS; ~+14 from 431)
- [ ] `pnpm tsc -b && pnpm test && pnpm build` clean (target ~174 PASS unchanged unless badge vitest added)
- [ ] Soft cancel UX still works for the happy path (no regression)
- [ ] Force kill button appears after 10s of "Cancelling…" (existing UI)
- [ ] Typed-confirm token (first 4 chars of exec name) works (existing UI)
- [ ] After hard cancel: worker process AND grandchildren are terminated (verified by `ps`)
- [ ] Attempt finalized with `state=cancelled`, `cancelled_reason="hard_cancel"`
- [ ] "Force-killed" red badge renders on AttemptDetail + ExecDetail AttemptsList
- [ ] CLI `rowforge attempt hard-cancel <exec> <attempt>` works
- [ ] sqlite MIGRATION_V4 applies cleanly on fresh + upgrade-from-v3
- [ ] HUMAN_SMOKE Plan 14 walkthrough added
- [ ] Spec docs (part-3 + part-5 + part-7, en + zh-Hant) updated
