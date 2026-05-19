//! SharedJsonlWriter — append-only JSONL file shared across async tasks.
//!
//! Design:
//! - Direct `tokio::fs::File` write, **no BufWriter** (OS page cache is the
//!   durability layer; BufWriter would delay visibility to the Stall Monitor).
//! - `append(true)` open mode: each `write_all` is atomic at the OS level for
//!   small writes (< PIPE_BUF on Linux; on macOS O_APPEND is also atomic for
//!   local FS).  The Mutex serialises concurrent callers anyway.
//! - `bytes_written` counter is initialised from the file's on-disk size so
//!   that resuming an existing `outcomes.jsonl` gives the Stall Monitor a
//!   correct baseline.

use crate::cancel::CancellationToken;
use crate::error::CoreError;
use crate::pool::BatchOutcome;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tracing;

// ---------------------------------------------------------------------------
// Constants (§11)
// ---------------------------------------------------------------------------

/// How often the Stall Monitor polls `bytes_written`. Default 30 s.
pub const STALL_POLL_INTERVAL_SECS: u64 = 30;

/// How long the jsonl file may be stalled (no byte growth) before the monitor
/// cancels the attempt. Default 300 s (5 min).
pub const STALL_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// SharedJsonlWriter
// ---------------------------------------------------------------------------

/// A shared, append-only JSONL file handle safe to use from multiple tasks.
///
/// Each call to [`append_line`] serialises `bo` to JSON, appends a newline,
/// takes the internal `Mutex`, performs a single `write_all`, optionally calls
/// `sync_data`, then bumps the atomic byte counter.
pub struct SharedJsonlWriter {
    file: tokio::sync::Mutex<tokio::fs::File>,
    bytes_written: AtomicU64,
    fsync: bool,
    /// The filesystem path this writer is appending to.
    /// Exposed via [`path()`] so callers (e.g. stall monitor error messages)
    /// can include it in diagnostics without extra bookkeeping.
    path: PathBuf,
}

impl SharedJsonlWriter {
    /// Open (or create) the JSONL file at `path` in append mode.
    ///
    /// If the file already exists (e.g. resume scenario) the byte counter is
    /// initialised from `file.metadata().await?.len()` so the Stall Monitor
    /// sees the correct baseline without re-reading the file content.
    pub async fn open(path: &Path, fsync: bool) -> Result<Self, CoreError> {
        // Stat the path *before* opening so the size is from the filesystem,
        // not from the (possibly O_APPEND-adjusted) file descriptor.  On
        // systems where fstat on an O_APPEND fd returns a different value than
        // stat on the path we avoid a double-count bug.
        let initial_bytes = if path.exists() {
            tokio::fs::metadata(path).await?.len()
        } else {
            0
        };

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(path)
            .await?;

        Ok(Self {
            file: tokio::sync::Mutex::new(file),
            bytes_written: AtomicU64::new(initial_bytes),
            fsync,
            path: path.to_path_buf(),
        })
    }

    /// Serialise `bo` to JSON, append `\n`, take the mutex, write, optionally
    /// fsync, then update the byte counter.
    ///
    /// The byte counter is updated **after** `write_all` succeeds, so it
    /// accurately reflects bytes that have reached the OS page cache.
    pub async fn append_line(&self, bo: &BatchOutcome) -> Result<(), CoreError> {
        let mut line =
            serde_json::to_vec(bo).map_err(|e| CoreError::Store(format!("json: {e}")))?;
        line.push(b'\n');
        let n = line.len() as u64;

        let mut guard = self.file.lock().await;
        guard.write_all(&line).await?;
        // `tokio::fs::File::write_all` submits writes to a background blocking
        // task and returns before the OS has accepted the bytes.  Without an
        // explicit `flush()` the data stays in tokio's internal buffer and is
        // never written if the file is dropped before the next poll.
        guard.flush().await?;
        if self.fsync {
            guard.sync_data().await?;
        }
        // Release the mutex before updating the counter — Stall Monitor reads
        // this without holding the mutex, so it's fine.
        drop(guard);

        self.bytes_written.fetch_add(n, Ordering::Relaxed);
        Ok(())
    }

    /// Return the total bytes successfully written (including pre-existing
    /// bytes when the writer was opened over an existing file).
    ///
    /// Uses `Ordering::Relaxed` — the Stall Monitor only needs to see
    /// monotonically non-decreasing values across successive poll ticks; there
    /// is no cross-thread synchronisation requirement beyond that.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Return the filesystem path this writer is appending to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ---------------------------------------------------------------------------
// Stall Monitor Task (§7)
// ---------------------------------------------------------------------------

/// Monitor `jsonl.bytes_written()` and cancel the pipeline if the file has
/// not grown for `stall_timeout`.
///
/// Normal exit (§7.1): when `run_pool_streaming` completes successfully it
/// calls `cancel.cancel()`.  The `select!` cancel arm fires immediately and
/// returns `Ok(())` — this is **not** treated as a stall.
///
/// Stall exit (§7.2): if `bytes_written` is unchanged for `> stall_timeout`,
/// the monitor logs a structured error, calls `cancel.cancel()` (broadcasts to
/// all tasks), and returns `Err(CoreError::Store("attempt stalled: …"))`.
///
/// The `biased;` qualifier on the inner `select!` ensures the cancel arm is
/// checked first (P2 follow-up): even if both futures are ready simultaneously,
/// a normal-completion cancel wins over a concurrent poll tick, preventing a
/// spurious stall-fire race.
pub async fn stall_monitor_task(
    jsonl: Arc<SharedJsonlWriter>,
    cancel: CancellationToken,
    poll_interval: Duration,
    stall_timeout: Duration,
) -> Result<(), CoreError> {
    let path = jsonl.path().to_path_buf();
    let mut last_bytes = jsonl.bytes_written();
    let mut stall_since: Option<Instant> = None;

    loop {
        tokio::select! {
            biased;
            // Normal exit: run_pool_streaming or another task cancelled the token.
            // biased; ensures this arm wins when both are ready simultaneously,
            // preventing spurious stall-fire on normal completion (§7.1).
            _ = cancel.cancelled() => return Ok(()),
            // Poll tick.
            _ = tokio::time::sleep(poll_interval) => {}
        }

        let now_bytes = jsonl.bytes_written();
        if now_bytes > last_bytes {
            // Progress observed — reset stall timer.
            stall_since = None;
            last_bytes = now_bytes;
            continue;
        }

        // No byte growth this tick.
        match stall_since {
            None => {
                stall_since = Some(Instant::now());
            }
            Some(t) if t.elapsed() > stall_timeout => {
                let stall_secs = t.elapsed().as_secs();
                tracing::error!(
                    last_bytes,
                    stall_duration_secs = stall_secs,
                    path = %path.display(),
                    "outcomes.jsonl has not grown for {}s; cancelling attempt",
                    stall_timeout.as_secs()
                );
                cancel.cancel();
                return Err(CoreError::Store(format!(
                    "attempt stalled: outcomes.jsonl ({}) no growth for {}s",
                    path.display(),
                    stall_timeout.as_secs()
                )));
            }
            // Stall timer is running but hasn't expired yet.
            Some(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{BatchOutcome, RowOutcome};
    use std::collections::HashSet;
    use std::sync::Arc;

    fn make_outcome(seq: u64) -> BatchOutcome {
        BatchOutcome {
            first_seq: seq,
            seqs: vec![seq],
            outcomes: vec![RowOutcome::Success {
                seq,
                data: {
                    let mut m = serde_json::Map::new();
                    m.insert("seq".into(), serde_json::Value::from(seq));
                    m
                },
                dur_ms: 0,
            }],
        }
    }

    // ------------------------------------------------------------------
    // Test 1: basic append + readback
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn jsonl_writer_basic_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");

        let writer = SharedJsonlWriter::open(&path, false).await.unwrap();
        for i in 0u64..3 {
            writer.append_line(&make_outcome(i)).await.unwrap();
        }

        // Read back and parse
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines");
        for (i, line) in lines.iter().enumerate() {
            let bo: BatchOutcome = serde_json::from_str(line).unwrap();
            assert_eq!(bo.first_seq, i as u64);
        }
    }

    // ------------------------------------------------------------------
    // Test 2: concurrent appends — no corruption
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn jsonl_writer_concurrent_appends_no_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");

        let writer = Arc::new(SharedJsonlWriter::open(&path, false).await.unwrap());

        const TASKS: u64 = 8;
        const PER_TASK: u64 = 100;

        let mut handles = Vec::new();
        for task_id in 0..TASKS {
            let w = Arc::clone(&writer);
            handles.push(tokio::spawn(async move {
                for i in 0..PER_TASK {
                    // Use globally-unique seq = task_id * PER_TASK + i
                    let seq = task_id * PER_TASK + i;
                    w.append_line(&make_outcome(seq)).await.unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), (TASKS * PER_TASK) as usize, "expected 800 lines");

        // All seqs unique, all parse
        let mut seen: HashSet<u64> = HashSet::new();
        for line in &lines {
            let bo: BatchOutcome = serde_json::from_str(line).expect("corrupt line");
            assert!(seen.insert(bo.first_seq), "duplicate seq {}", bo.first_seq);
        }
        assert_eq!(seen.len(), (TASKS * PER_TASK) as usize);
    }

    // ------------------------------------------------------------------
    // Test 3: bytes_written counter matches file size
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn jsonl_writer_bytes_counter_matches_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");

        let writer = SharedJsonlWriter::open(&path, false).await.unwrap();
        for i in 0u64..5 {
            writer.append_line(&make_outcome(i)).await.unwrap();
        }

        // Read back the actual bytes; avoids macOS metadata-cache staleness.
        let file_bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(
            writer.bytes_written(),
            file_bytes.len() as u64,
            "counter should match file size"
        );
    }

    // ------------------------------------------------------------------
    // Test 4: resume — counter initialised from existing file
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn jsonl_writer_resume_initializes_counter_from_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");

        // Pre-write 2 lines as if a previous run wrote them.
        {
            let w = SharedJsonlWriter::open(&path, false).await.unwrap();
            w.append_line(&make_outcome(0)).await.unwrap();
            w.append_line(&make_outcome(1)).await.unwrap();
        }

        // Read back to get the authoritative byte count (avoids metadata cache).
        let before_bytes = tokio::fs::read(&path).await.unwrap();
        let before_size = before_bytes.len() as u64;
        assert!(before_size > 0);

        // Open fresh writer over the same path (simulates resume).
        let w2 = SharedJsonlWriter::open(&path, false).await.unwrap();
        assert_eq!(
            w2.bytes_written(),
            before_size,
            "new writer should initialise counter from existing bytes"
        );

        // Append one more line.
        w2.append_line(&make_outcome(2)).await.unwrap();
        // Read back again to get the total bytes on disk.
        let after_bytes = tokio::fs::read(&path).await.unwrap();
        let after_size = after_bytes.len() as u64;
        assert_eq!(w2.bytes_written(), after_size);
    }

    // ------------------------------------------------------------------
    // Test 5: stall_monitor_returns_ok_on_cancel
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn stall_monitor_returns_ok_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");
        let writer = Arc::new(SharedJsonlWriter::open(&path, false).await.unwrap());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let start = Instant::now();
        let handle = tokio::spawn(stall_monitor_task(
            Arc::clone(&writer),
            cancel_clone,
            Duration::from_millis(100),
            Duration::from_millis(500),
        ));

        // Cancel immediately — should return Ok well before the first tick.
        cancel.cancel();
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "expected Ok on cancel, got {:?}", result);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "should have returned before first poll tick"
        );
    }

    // ------------------------------------------------------------------
    // Test 6: stall_monitor_fires_on_no_growth
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn stall_monitor_fires_on_no_growth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");
        // Writer with no appends — bytes_written stays at 0.
        let writer = Arc::new(SharedJsonlWriter::open(&path, false).await.unwrap());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(stall_monitor_task(
            Arc::clone(&writer),
            cancel_clone,
            Duration::from_millis(50),   // poll every 50 ms
            Duration::from_millis(200),  // stall after 200 ms no growth
        ));

        let result = handle.await.unwrap();
        assert!(
            result.is_err(),
            "expected Err on stall, got {:?}",
            result
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("stalled"), "error message should mention stalled: {msg}");
        assert!(msg.contains("outcomes.jsonl"), "error message should include filename: {msg}");

        // Token must be cancelled by the monitor.
        assert!(cancel.is_cancelled(), "cancel token should be cancelled after stall");
    }

    // ------------------------------------------------------------------
    // Test 7: stall_monitor_resets_on_growth
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn stall_monitor_resets_on_growth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outcomes.jsonl");
        let writer = Arc::new(SharedJsonlWriter::open(&path, false).await.unwrap());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let cancel_stopper = cancel.clone();

        // Spawn stall monitor: 50 ms poll, 200 ms stall timeout.
        let monitor_handle = tokio::spawn(stall_monitor_task(
            Arc::clone(&writer),
            cancel_clone,
            Duration::from_millis(50),
            Duration::from_millis(200),
        ));

        // Concurrent writer appends a line every 30 ms for 500 ms total.
        let writer_clone = Arc::clone(&writer);
        let writer_handle = tokio::spawn(async move {
            let mut seq = 0u64;
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                writer_clone.append_line(&make_outcome(seq)).await.unwrap();
                seq += 1;
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        });

        // Wait for the writer to finish (~500 ms), then cancel the monitor.
        writer_handle.await.unwrap();
        cancel_stopper.cancel();

        let result = monitor_handle.await.unwrap();
        assert!(
            result.is_ok(),
            "monitor should return Ok when cancelled after growth, got {:?}",
            result
        );
    }
}
