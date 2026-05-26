use crate::error::CoreError;
use crate::handler_log::{format_line, HandlerLogLine, HandlerStream};
use crate::manifest::Manifest;
use crate::pool::RowOutcome;
use crate::protocol::{BatchEntry, Inbound, Outbound, RowEnvelope};
use crate::run::ERR_BATCH_PROTOCOL_ERROR;
use chrono::Utc;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::{timeout, Duration, Instant};
use tracing::debug;

// ---------------------------------------------------------------------------
// HandlerLogSink — shared tee destination for stdout/stderr log lines
// ---------------------------------------------------------------------------

/// Callback type for live handler log broadcast (Studio uses this for the Logs tab).
pub type HandlerLogCallback = Arc<dyn Fn(HandlerLogLine) + Send + Sync>;

/// Shared tee sink — one per pool run, cloned into every worker.
///
/// Writing is serialised through the Mutex so multiple workers share a single
/// `handler_log.log` file without interleaving partial lines.
#[derive(Clone)]
pub struct HandlerLogSink {
    /// File handle for the per-attempt `handler_log.log`.
    pub file: Arc<tokio::sync::Mutex<tokio::fs::File>>,
    /// Optional live-broadcast callback (Studio's SessionRegistry subscribes here).
    pub callback: Option<HandlerLogCallback>,
    /// When `true`, valid outcome JSON lines on stdout are ALSO written to the
    /// log (for protocol debugging). Default is `false`.
    pub capture_raw_stdout: bool,
}

impl HandlerLogSink {
    /// Write `entry` to the log file and invoke the callback if present.
    ///
    /// The eprintln for CLI back-compat (stderr) is the caller's responsibility
    /// so it can control the exact prefix format (e.g. `[handler#N]`).
    pub async fn write(&self, entry: &HandlerLogLine) {
        let formatted = format_line(entry);
        {
            let mut f = self.file.lock().await;
            // Best-effort: ignore write errors so a full disk doesn't kill the run.
            let _ = tokio::io::AsyncWriteExt::write_all(&mut *f, formatted.as_bytes()).await;
        }
        if let Some(cb) = &self.callback {
            cb(entry.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// Wraps one handler subprocess speaking JSON-Lines on stdio.
pub struct Worker {
    pub id: u32,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    pub handler_version: String,
    /// Optional log sink — set by `pool_streaming` after spawn.
    pub log_sink: Option<HandlerLogSink>,
    /// Non-protocol stdout lines emitted before `ready` (boot log lines).
    /// Buffered during handshake because `log_sink` is not yet available at
    /// that point. `pool_streaming` calls `flush_pre_ready_lines` immediately
    /// after attaching the sink so these lines appear in handler_log.log.
    pub pre_ready_log_lines: Vec<String>,
    /// Unix process group id of the child. Set on Unix; `None` on Windows.
    /// Used by `Worker::hard_kill` to send SIGKILL to the entire process
    /// group (child + grandchildren). On Unix, equal to `child.id()` after
    /// `setsid()` in `pre_exec`.
    pub(crate) pgid: Option<i32>,
}

impl Worker {
    /// Spawn and perform init handshake. Returns once `ready` received or startup timeout.
    pub async fn spawn(
        id: u32,
        handler_dir: &Path,
        manifest: &Manifest,
        run_id: &str,
        config: &std::collections::BTreeMap<String, serde_json::Value>,
        columns: &[String],
    ) -> Result<Self, CoreError> {
        let cmd0 = manifest
            .entry
            .cmd
            .first()
            .ok_or_else(|| CoreError::Protocol("entry.cmd empty".into()))?;
        let mut command = Command::new(cmd0);
        command.args(&manifest.entry.cmd[1..]);
        command.current_dir(handler_dir.join(&manifest.entry.cwd));
        for (k, v) in &manifest.entry.env {
            command.env(k, v);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Plan 14: spawn the worker into its own POSIX process group so a
        // hard cancel can SIGKILL the child AND any grandchildren via killpg.
        // Set BEFORE spawn(). Tokio's Command re-exports
        // std::os::unix::process::CommandExt::pre_exec on Unix targets.
        #[cfg(unix)]
        // SAFETY: setsid(2) is async-signal-safe and has no preconditions that
        // could be violated by running it in a fork child between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().map_err(CoreError::Io)?;

        #[cfg(unix)]
        let pgid: Option<i32> = child.id().map(|p| p as i32);
        #[cfg(not(unix))]
        let pgid: Option<i32> = None;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout_raw = child.stdout.take().expect("piped stdout");
        #[allow(unused_mut)]
        let mut stdout = BufReader::new(stdout_raw);

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

        // send init
        let init = Outbound::Init {
            run_id: run_id.to_string(),
            config: config.clone(),
            meta: crate::protocol::InitMeta {
                columns: columns.to_vec(),
            },
        };
        w.stdin
            .write_all(init.to_jsonl().as_bytes())
            .await
            .map_err(CoreError::Io)?;
        w.stdin.flush().await.map_err(CoreError::Io)?;

        // Wait for `ready`. Be lenient: any line on stdout that doesn't parse as a
        // protocol message is treated as a log line and forwarded to App stderr —
        // this lets handler authors `print("starting up")` without breaking the
        // protocol. The startup timeout is the *total* time to first valid `ready`.
        let to = Duration::from_millis(manifest.entry.startup_timeout_ms);
        loop {
            let mut line = String::new();
            let read_res = timeout(to, w.stdout.read_line(&mut line)).await;
            match read_res {
                Err(_) => {
                    return Err(CoreError::StartupTimeout {
                        timeout_ms: manifest.entry.startup_timeout_ms,
                    })
                }
                Ok(Err(e)) => return Err(CoreError::Io(e)),
                Ok(Ok(0)) => {
                    return Err(CoreError::Protocol(
                        "handler closed stdout before ready".into(),
                    ))
                }
                Ok(Ok(_)) => {}
            }
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            match Inbound::from_jsonl_line(trimmed) {
                Ok(Inbound::Ready { handler_version }) => {
                    w.handler_version = handler_version;
                    break;
                }
                Ok(other) => {
                    return Err(CoreError::Protocol(format!(
                        "expected ready, got {:?}",
                        other
                    )))
                }
                Err(_) => {
                    // Non-protocol line during startup: treat as log, keep waiting for ready.
                    // log_sink is not attached yet (set by pool_streaming after spawn returns),
                    // so buffer the line for flush_pre_ready_lines. Also eprintln for CLI
                    // back-compat so these lines still appear on stderr in non-Studio runs.
                    eprintln!("[handler#{}] {}", id, trimmed);
                    w.pre_ready_log_lines.push(trimmed.to_string());
                }
            }
        }
        Ok(w)
    }

    /// Flush buffered pre-ready stdout lines through `log_sink`.
    ///
    /// Call this IMMEDIATELY after attaching `worker.log_sink` in
    /// `pool_streaming` so that handler boot lines (emitted before `ready`)
    /// appear in `handler_log.log`. No-op if `log_sink` is `None` or the
    /// buffer is empty. The buffer is drained after the flush.
    pub async fn flush_pre_ready_lines(&mut self) {
        let sink = match &self.log_sink {
            Some(s) => s.clone(),
            None => {
                self.pre_ready_log_lines.clear();
                return;
            }
        };
        let lines = std::mem::take(&mut self.pre_ready_log_lines);
        for line in lines {
            let entry = HandlerLogLine {
                timestamp: Utc::now(),
                worker_id: self.id as usize,
                stream: HandlerStream::Stdout,
                line,
            };
            sink.write(&entry).await;
        }
    }

    pub async fn send_row(&mut self, msg: &Outbound) -> Result<(), CoreError> {
        let seq = match msg {
            Outbound::Row { seq, .. } => Some(*seq),
            _ => None,
        };
        let t0 = Instant::now();
        let bytes = msg.to_jsonl();
        self.stdin
            .write_all(bytes.as_bytes())
            .await
            .map_err(CoreError::Io)?;
        self.stdin.flush().await.map_err(CoreError::Io)?;
        let elapsed_ms = t0.elapsed().as_millis();
        debug!(worker = self.id, ?seq, elapsed_ms, bytes = bytes.len(), "send_row");
        Ok(())
    }

    /// Send a `Batch` envelope built from `rows` (already-converted
    /// [`RowEnvelope`] slices). The accumulator emits `Vec<RowEnvelope>`
    /// directly, so no `RowJob → RowEnvelope` conversion is needed here.
    pub async fn send_batch_envelopes(&mut self, rows: &[RowEnvelope]) -> Result<(), CoreError> {
        let envelope = Outbound::Batch {
            rows: rows.to_vec(),
        };
        self.stdin
            .write_all(envelope.to_jsonl().as_bytes())
            .await
            .map_err(CoreError::Io)?;
        self.stdin.flush().await.map_err(CoreError::Io)?;
        Ok(())
    }


    /// Receive a `BatchResult` envelope and align it positionally with
    /// `expected_seqs`. Any length mismatch, parse failure, or wrong-variant
    /// inbound yields N synthetic `BATCH_PROTOCOL_ERROR` outcomes — one per
    /// expected row. EOF (stdout closed) is bubbled as `HandlerExit` so the
    /// caller can surface it as a crash; T7 will refine crash semantics.
    pub async fn recv_batch_result(
        &mut self,
        expected_seqs: &[u64],
    ) -> Result<Vec<RowOutcome>, CoreError> {
        let msg = match self.recv().await? {
            Some(m) => m,
            None => return Err(CoreError::HandlerExit { code: None }),
        };
        let results = match msg {
            Inbound::BatchResult { results } => results,
            Inbound::Ready { .. } => {
                return Ok(synthesize_batch_protocol_error(
                    expected_seqs,
                    "unexpected ready mid-run".to_string(),
                ));
            }
            other => {
                return Ok(synthesize_batch_protocol_error(
                    expected_seqs,
                    format!("expected batch_result, got {:?}", other),
                ));
            }
        };
        if results.len() != expected_seqs.len() {
            return Ok(synthesize_batch_protocol_error(
                expected_seqs,
                format!(
                    "batch_result length mismatch: expected {}, got {}",
                    expected_seqs.len(),
                    results.len()
                ),
            ));
        }
        let outcomes = results
            .into_iter()
            .zip(expected_seqs.iter())
            .map(|(entry, seq)| match entry {
                BatchEntry::Result { data } => RowOutcome::Success {
                    seq: *seq,
                    data,
                    dur_ms: 0,
                },
                BatchEntry::Error { code, message, data } => RowOutcome::Error {
                    seq: *seq,
                    code,
                    message,
                    dur_ms: 0,
                    data,
                },
            })
            .collect();
        Ok(outcomes)
    }

    /// Read one inbound protocol message. Returns None if stdout closed.
    ///
    /// Non-protocol lines on stdout (e.g., a `print("debug")` from the handler)
    /// are tee'd to the `HandlerLogSink` (log file + callback) and forwarded to
    /// App stderr with `[handler#N]` prefix — this way handler authors can use
    /// the most natural log mechanism in their language without breaking the wire
    /// protocol.
    ///
    /// When `log_sink.capture_raw_stdout` is `true`, valid protocol lines are
    /// ALSO written to the log (for protocol debugging). Default is `false`.
    pub async fn recv(&mut self) -> Result<Option<Inbound>, CoreError> {
        let t0 = Instant::now();
        let wid = self.id as usize;
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(CoreError::Io)?;
            if n == 0 {
                return Ok(None);
            }
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            match Inbound::from_jsonl_line(trimmed) {
                Ok(msg) => {
                    let elapsed_ms = t0.elapsed().as_millis();
                    debug!(worker = self.id, elapsed_ms, bytes = trimmed.len(), "recv");
                    // If capture_raw_stdout is enabled, also write valid protocol
                    // lines to the log (for debugging purposes).
                    if let Some(sink) = &self.log_sink {
                        if sink.capture_raw_stdout {
                            let entry = HandlerLogLine {
                                timestamp: Utc::now(),
                                worker_id: wid,
                                stream: HandlerStream::Stdout,
                                line: trimmed.to_string(),
                            };
                            sink.write(&entry).await;
                        }
                    }
                    return Ok(Some(msg));
                }
                Err(_) => {
                    // Non-protocol line on stdout: tee to log file + callback, keep reading.
                    if let Some(sink) = &self.log_sink {
                        let entry = HandlerLogLine {
                            timestamp: Utc::now(),
                            worker_id: wid,
                            stream: HandlerStream::Stdout,
                            line: trimmed.to_string(),
                        };
                        sink.write(&entry).await;
                    }
                    // CLI back-compat: non-protocol stdout lines go to stderr
                    // (same as before, avoids polluting outcomes pipeline output).
                    eprintln!("[handler#{}] {}", self.id, trimmed);
                }
            }
        }
    }

    /// Take stderr handle for the caller to drain (e.g., into Run Log).
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Send shutdown, wait grace period, kill if still alive.
    pub async fn shutdown(mut self, grace: Duration) -> Result<Option<i32>, CoreError> {
        let _ = self.send_row(&Outbound::Shutdown).await;
        let _ = self.stdin.shutdown().await;
        match timeout(grace, self.child.wait()).await {
            Ok(Ok(status)) => Ok(status.code()),
            Ok(Err(e)) => Err(CoreError::Io(e)),
            Err(_) => {
                let _ = self.child.kill().await;
                let status = self.child.wait().await.map_err(CoreError::Io)?;
                Ok(status.code())
            }
        }
    }
}

/// Build one `BATCH_PROTOCOL_ERROR` outcome per expected seq, all sharing the
/// same `reason` as their `message`. Used when a handler's `batch_result` is
/// malformed, wrong-variant, or has the wrong length — every row in the
/// dispatched batch fails attribution and must be reported.
fn synthesize_batch_protocol_error(expected_seqs: &[u64], reason: String) -> Vec<RowOutcome> {
    expected_seqs
        .iter()
        .map(|seq| RowOutcome::Error {
            seq: *seq,
            code: ERR_BATCH_PROTOCOL_ERROR.to_string(),
            message: reason.clone(),
            dur_ms: 0,
            data: None,
        })
        .collect()
}
