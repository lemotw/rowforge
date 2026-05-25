//! Integration tests for pool_streaming handler-log tee behavior (Plan 9 Task 2).
//!
//! Verifies:
//!   1. `handler_log.log` is written at the correct path after a run.
//!   2. `HandlerLogCallback` is invoked for captured lines.
//!   3. `capture_raw_stdout = true` includes valid outcome JSON in the log.

use rowforge_core::handler_log::{handler_log_path, parse_line, HandlerStream};
use rowforge_core::input_stream::CsvInputStream;
use rowforge_core::manifest::{Entry, Manifest};
use rowforge_core::pool_streaming::{run_pool_streaming, HandlerLogCallback, StreamingPoolConfig};
use rowforge_core::runtime::Runtime;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Infrastructure helpers (mirrors pool_streaming tests)
// ---------------------------------------------------------------------------

fn test_handler_path() -> PathBuf {
    use std::sync::Once;
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "test-handler"])
            .status()
            .expect("invoking `cargo build -p test-handler`");
        assert!(status.success(), "cargo build -p test-handler failed");
    });
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest.parent().unwrap().parent().unwrap();
    workspace_root.join("target/debug/test-handler")
}

fn make_manifest(behavior: &str) -> Arc<Manifest> {
    Arc::new(Manifest {
        name: "test".into(),
        version: "0.0.0".into(),
        description: String::new(),
        language: String::new(),
        entry: Entry {
            cmd: vec![
                test_handler_path().to_string_lossy().into(),
                behavior.to_string(),
            ],
            build: None,
            cwd: ".".into(),
            env: Default::default(),
            startup_timeout_ms: 5000,
        },
        required_input: vec![],
        config: BTreeMap::new(),
        runtime: None,
        output: None,
    })
}

fn write_csv(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

fn make_cfg(
    dir: &TempDir,
    manifest: Arc<Manifest>,
    on_handler_log: Option<HandlerLogCallback>,
    capture_raw_stdout: bool,
) -> StreamingPoolConfig {
    StreamingPoolConfig {
        handler_dir: std::env::temp_dir(),
        manifest,
        workers: 1,
        run_id: "handler-log-test".into(),
        config: BTreeMap::new(),
        shutdown_grace: Duration::from_secs(2),
        cancel: None,
        runtime: Runtime::default(),
        jsonl_path: dir.path().join("outcomes.jsonl"),
        fsync_outcomes: false,
        stall_timeout: None,
        stall_poll_interval: None,
        on_row_done: None,
        on_handler_log,
        capture_raw_stdout,
    }
}

// ---------------------------------------------------------------------------
// Test 1: handler_log.log is written at the correct path
// ---------------------------------------------------------------------------
//
// Uses the `log-noisy` handler which emits:
//   - stderr: "log: processing row N" for each row
//   - stdout: "debug: about to process seq N" (non-JSON noise) for each row
//   - stdout: valid result JSON (normal echo outcome)
//
// After the run:
//   - handler_log.log must exist at handler_log_path(attempt_dir)
//   - file must contain the stderr line (at least one)
//   - file must contain the non-JSON stdout noise line
//   - file must NOT contain the valid outcome JSON (capture_raw_stdout=false)
#[tokio::test]
async fn pool_streaming_writes_handler_log_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = "n\n".to_string();
    for i in 0..3 {
        csv.push_str(&format!("{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);
    let manifest = make_manifest("log-noisy");
    let cfg = make_cfg(&dir, manifest, None, false);

    let attempt_dir = dir.path().to_path_buf();
    let log_path = handler_log_path(&attempt_dir);

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

    // Log file must exist.
    assert!(
        log_path.exists(),
        "handler_log.log not found at {:?}",
        log_path
    );

    let log_content = std::fs::read_to_string(&log_path).unwrap();

    // Must contain at least one stderr line from the handler.
    let has_stderr = log_content.lines().any(|l| {
        if let Some(entry) = parse_line(l) {
            entry.stream == HandlerStream::Stderr && entry.line.contains("log: processing row")
        } else {
            false
        }
    });
    assert!(
        has_stderr,
        "handler_log.log missing stderr lines; content:\n{}",
        log_content
    );

    // Must contain non-JSON stdout noise ("debug: about to process seq").
    let has_stdout_noise = log_content.lines().any(|l| {
        if let Some(entry) = parse_line(l) {
            entry.stream == HandlerStream::Stdout && entry.line.contains("debug: about to process")
        } else {
            false
        }
    });
    assert!(
        has_stdout_noise,
        "handler_log.log missing non-JSON stdout noise; content:\n{}",
        log_content
    );

    // Must NOT contain valid outcome JSON (capture_raw_stdout=false).
    let has_outcome_json = log_content.lines().any(|l| {
        if let Some(entry) = parse_line(l) {
            entry.stream == HandlerStream::Stdout && entry.line.contains("\"type\":\"result\"")
        } else {
            false
        }
    });
    assert!(
        !has_outcome_json,
        "handler_log.log should NOT contain valid outcome JSON when capture_raw_stdout=false; content:\n{}",
        log_content
    );
}

// ---------------------------------------------------------------------------
// Test 2: HandlerLogCallback is invoked for every captured line
// ---------------------------------------------------------------------------
#[tokio::test]
async fn pool_streaming_invokes_handler_log_callback() {
    use rowforge_core::handler_log::HandlerLogLine;

    let dir = tempfile::tempdir().unwrap();
    let mut csv = "n\n".to_string();
    for i in 0..3 {
        csv.push_str(&format!("{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);
    let manifest = make_manifest("log-noisy");

    let received: Arc<Mutex<Vec<HandlerLogLine>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received.clone();
    let cb: HandlerLogCallback = Arc::new(move |line: HandlerLogLine| {
        received_clone.lock().unwrap().push(line);
    });

    let cfg = make_cfg(&dir, manifest, Some(cb), false);

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

    let lines = received.lock().unwrap();
    assert!(
        !lines.is_empty(),
        "callback was never invoked; expected at least one log line"
    );

    // Must have received at least one stderr line.
    let has_stderr = lines.iter().any(|l| {
        l.stream == HandlerStream::Stderr && l.line.contains("log: processing row")
    });
    assert!(
        has_stderr,
        "callback: missing expected stderr line; received: {:?}",
        lines
    );

    // Must have received at least one stdout noise line.
    let has_stdout_noise = lines.iter().any(|l| {
        l.stream == HandlerStream::Stdout && l.line.contains("debug: about to process")
    });
    assert!(
        has_stdout_noise,
        "callback: missing expected stdout noise line; received: {:?}",
        lines
    );
}

// ---------------------------------------------------------------------------
// Test 3: capture_raw_stdout=true includes valid outcome JSON in the log
// ---------------------------------------------------------------------------
#[tokio::test]
async fn raw_stdout_flag_captures_valid_outcome_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = "n\n".to_string();
    for i in 0..2 {
        csv.push_str(&format!("{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);

    // Use plain echo handler (no noise) — its stdout is purely valid JSON outcomes.
    let manifest = make_manifest("echo");
    let cfg = make_cfg(&dir, manifest, None, true /* capture_raw_stdout */);

    let attempt_dir = dir.path().to_path_buf();
    let log_path = handler_log_path(&attempt_dir);

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

    assert!(
        log_path.exists(),
        "handler_log.log not found at {:?} (capture_raw_stdout=true run)",
        log_path
    );

    let log_content = std::fs::read_to_string(&log_path).unwrap();

    // With capture_raw_stdout=true, valid outcome JSON lines must appear in the log.
    let has_outcome_json = log_content.lines().any(|l| {
        if let Some(entry) = parse_line(l) {
            entry.stream == HandlerStream::Stdout && entry.line.contains("\"type\":\"result\"")
        } else {
            false
        }
    });
    assert!(
        has_outcome_json,
        "handler_log.log must contain valid outcome JSON when capture_raw_stdout=true; content:\n{}",
        log_content
    );
}

// ---------------------------------------------------------------------------
// Test 4: pre-ready stdout lines (boot lines) appear in handler_log.log
// ---------------------------------------------------------------------------
//
// Uses the `echo-noisy` handler, which emits several plain-text (non-protocol)
// lines to STDOUT BEFORE sending `ready`. These boot lines were previously
// discarded (only eprintln'd) because log_sink was not attached during the
// handshake. After the fix, they are buffered in `worker.pre_ready_log_lines`
// and flushed through the sink once pool_streaming attaches it.
//
// After the run, `handler_log.log` must contain at least one stdout line
// whose content matches the boot-time text ("starting up", "loaded config",
// or "this is plain text").
#[tokio::test]
async fn pool_streaming_captures_handler_boot_lines_before_handshake() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = "n\n".to_string();
    for i in 0..2 {
        csv.push_str(&format!("{}\n", i));
    }
    let csv_path = write_csv(&dir, "input.csv", &csv);

    // echo-noisy emits "starting up", "loaded config v0.0.0", and
    // "this is plain text, not JSON" to STDOUT before sending `ready`.
    let manifest = make_manifest("echo-noisy");
    let cfg = make_cfg(&dir, manifest, None, false);

    let attempt_dir = dir.path().to_path_buf();
    let log_path = handler_log_path(&attempt_dir);

    let input = Box::new(CsvInputStream::open(&csv_path, &[]).unwrap());
    let report = run_pool_streaming(
        input,
        HashSet::new(),
        None,
        BTreeMap::new(),
        false,
        cfg,
    )
    .await
    .unwrap();

    assert!(!report.aborted, "expected not aborted: {:?}", report.abort_reason);

    assert!(
        log_path.exists(),
        "handler_log.log not found at {:?}",
        log_path
    );

    let log_content = std::fs::read_to_string(&log_path).unwrap();

    // At least one of the echo-noisy boot lines must be in the log.
    let has_boot_line = log_content.lines().any(|l| {
        if let Some(entry) = parse_line(l) {
            entry.stream == HandlerStream::Stdout
                && (entry.line.contains("starting up")
                    || entry.line.contains("loaded config")
                    || entry.line.contains("this is plain text"))
        } else {
            false
        }
    });

    assert!(
        has_boot_line,
        "handler_log.log must contain pre-ready stdout boot lines; content:\n{}",
        log_content
    );
}
