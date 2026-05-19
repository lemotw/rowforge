//! Test handler used by integration tests. Speaks rowforge wire protocol on stdio.
//!
//! Behaviors selected via first CLI arg:
//!   echo            - echoes input row back as output
//!   echo-noisy      - same as echo but prints non-protocol debug to STDOUT
//!                     before ready and around each row, exercising the
//!                     lenient stdout-as-log path in worker.rs
//!   error-on-bad    - emits error for rows where data.bad == true
//!   error-with-data - emits error with a `data` payload echoing data.billid
//!   crash-after-3   - crashes (exit 1) after processing 3 rows
//!   crash-on-first  - crashes (exit 1) on the very first row, no response sent
//!   no-ready        - never sends `ready` (for startup timeout test)
//!   slow-ready=MS   - delays ready by MS milliseconds
//!   exit-on-shutdown - normal echo, exits cleanly on shutdown
//!   batch-echo      - reads `batch` envelope; emits `batch_result` with one
//!                     `result` entry per row, data = {"echoed": <input data>}
//!   batch-echo-slow - same as batch-echo but sleeps 50ms before responding
//!                     to each batch — used by the cancel test to give
//!                     `cancel` time to fire while a batch is in flight.
//!   batch-short     - reads `batch` envelope; emits `batch_result` with N-1
//!                     entries (one fewer than received) — exercises the
//!                     BATCH_PROTOCOL_ERROR length-mismatch path
//!   batch-crash     - reads `batch` envelope; exits non-zero WITHOUT replying.
//!                     Used by T7 tests to exercise the crash-mid-batch path
//!                     (WORKER_CRASH vs WORKER_CRASH_UNSAFE).
//!   stall-after-2   - processes 2 rows normally (echo), then sleeps forever.
//!                     Used by P8 stall monitor integration tests.
//!   hang-on-first      - reads the first row envelope but never writes a reply,
//!                        sleeping forever. Used to test cancel-during-recv synthesis.
//!   hang-on-first-batch - reads the first batch envelope but never writes a reply.
//!                        Used to test cancel-during-recv synthesis in batch mode.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let behavior = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "echo".to_string());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut processed = 0u64;

    if behavior == "no-ready" {
        // wait forever
        std::thread::sleep(Duration::from_secs(3600));
        return;
    }
    if let Some(rest) = behavior.strip_prefix("slow-ready=") {
        let ms: u64 = rest.parse().expect("ms");
        std::thread::sleep(Duration::from_millis(ms));
    }

    // first message must be init; reply ready
    let mut lines = stdin.lock().lines();
    let init = lines.next().expect("init").expect("init read");
    let parsed: Value = serde_json::from_str(&init).expect("init json");
    assert_eq!(
        parsed["type"], "init",
        "first message must be init, got {}",
        init
    );

    // echo-noisy mode: emit non-protocol log lines to STDOUT before ready.
    // This exercises rowforge's lenient parsing path. A naive author who does
    // `print("starting up")` should not break the run.
    if behavior == "echo-noisy" {
        writeln!(out, "starting up").unwrap();
        writeln!(out, "loaded config v0.0.0").unwrap();
        writeln!(out, "this is plain text, not JSON").unwrap();
        out.flush().unwrap();
    }

    writeln!(out, r#"{{"type":"ready","handler_version":"0.0.0"}}"#).unwrap();
    out.flush().unwrap();

    for line_res in lines {
        let line = line_res.expect("read");
        let v: Value = serde_json::from_str(&line).expect("json");
        match v["type"].as_str() {
            Some("row") => {
                let seq = v["seq"].as_u64().unwrap();
                let data = v["data"].clone();

                if behavior == "hang-on-first" {
                    // Read the row but never reply — simulates a handler hung
                    // mid-row. Used to test cancel-during-recv crash synthesis.
                    eprintln!("test-handler: hang-on-first at seq {}", seq);
                    std::thread::sleep(Duration::from_secs(3600));
                    return;
                }
                if behavior == "stall-after-2" && processed >= 2 {
                    // Sleep forever — simulates a handler that hangs mid-run.
                    // The stall monitor should detect no jsonl growth and cancel.
                    eprintln!("test-handler: stalling after {} rows", processed);
                    std::thread::sleep(Duration::from_secs(3600));
                    return;
                }
                if behavior == "crash-after-3" && processed >= 3 {
                    eprintln!("test-handler: intentional crash at seq {}", seq);
                    std::process::exit(1);
                }
                if behavior == "crash-on-first" {
                    // Crash immediately on the very first row, no response sent.
                    eprintln!("test-handler: crash-on-first at seq {}", seq);
                    std::process::exit(1);
                }
                if behavior == "error-on-bad" && data["bad"].as_bool() == Some(true) {
                    let resp = json!({"type":"error","seq":seq,"code":"BAD_ROW","message":"row marked bad"});
                    writeln!(out, "{}", resp).unwrap();
                } else if behavior == "error-with-data" {
                    // Always-fail handler that attaches a `data` payload to
                    // each error envelope. Used by the e2e test that verifies
                    // schema.failed_output columns appear in failed.csv.
                    let billid = data["billid"].clone();
                    let resp = json!({
                        "type": "error",
                        "seq": seq,
                        "code": "DEMO_FAIL",
                        "message": "always fails",
                        "data": {"billid": billid},
                    });
                    writeln!(out, "{}", resp).unwrap();
                } else {
                    if behavior == "echo-noisy" {
                        // Emit a plain-text log line to STDOUT before the protocol
                        // result, mid-run. rowforge should treat it as a log.
                        writeln!(out, "row {} processed", seq).unwrap();
                    }
                    // echo: wrap input data as output under key "echoed"
                    let resp = json!({"type":"result","seq":seq,"data":{"echoed":data}});
                    writeln!(out, "{}", resp).unwrap();
                }
                out.flush().unwrap();
                processed += 1;
            }
            Some("batch") => {
                // Only batch-* behaviors should receive batch envelopes.
                let rows = v["rows"].as_array().expect("batch.rows is array");
                let n = rows.len();
                if behavior == "hang-on-first-batch" {
                    // Read the batch but never reply — simulates a hung handler
                    // in batch mode. Used to test cancel-during-recv synthesis.
                    eprintln!("test-handler: hang-on-first-batch with {} rows", n);
                    std::thread::sleep(Duration::from_secs(3600));
                    return;
                } else if behavior == "batch-echo" || behavior == "batch-echo-slow" {
                    if behavior == "batch-echo-slow" {
                        // Sleep BEFORE responding so the in-flight batch holds
                        // the worker long enough for a cancel signal to arrive
                        // before the next batch is requested.
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let entries: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            let data = r["data"].clone();
                            json!({"kind": "result", "data": {"echoed": data}})
                        })
                        .collect();
                    let resp = json!({"type":"batch_result","results": entries});
                    writeln!(out, "{}", resp).unwrap();
                    out.flush().unwrap();
                    processed += n as u64;
                } else if behavior == "batch-crash" {
                    // Crash mid-batch without replying. The pool's
                    // `recv_batch_result` will see stdout close and bubble
                    // up HandlerExit, triggering crash synthesis with code
                    // determined by `runtime.idempotent`.
                    eprintln!("test-handler: intentional batch crash with {} rows", n);
                    std::process::exit(1);
                } else if behavior == "batch-short" {
                    // Return one fewer entry than received → length mismatch.
                    let short_len = n.saturating_sub(1);
                    let entries: Vec<Value> = (0..short_len)
                        .map(|i| {
                            let data = rows[i]["data"].clone();
                            json!({"kind": "result", "data": {"echoed": data}})
                        })
                        .collect();
                    let resp = json!({"type":"batch_result","results": entries});
                    writeln!(out, "{}", resp).unwrap();
                    out.flush().unwrap();
                    processed += n as u64;
                } else {
                    panic!(
                        "unexpected batch envelope in non-batch behavior: {}",
                        behavior
                    );
                }
            }
            Some("shutdown") => return,
            other => panic!("unexpected message type: {:?}", other),
        }
    }
}
