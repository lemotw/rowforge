//! Plan 1-4 integration coverage.
//!
//! Each test bootstraps a temp workspace, runs CLI-equivalent setup via
//! rowforge_core::execution_store, then exercises the studio-core
//! surface. No CLI binary is invoked.

use rowforge_core::execution_store::ExecutionStore;
use rowforge_studio_core::{ExecRollup, OpenOpts, ProgressEvent, RunHandle, RunOpts, StudioCore, UiError};
use std::path::PathBuf;

/// Helper: produces a temp workspace dir with an initialized SQLite
/// store and zero executions.
fn empty_workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    // Trigger schema bootstrap by opening once.
    let _store = ExecutionStore::open(tmp.path()).unwrap();
    tmp
}

#[test]
fn open_records_workspace_root_and_schema_version() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .expect("open");
    assert_eq!(core.workspace().root, PathBuf::from(tmp.path()));
    assert!(core.workspace().schema_version >= 1);
}

#[test]
fn open_with_nonexistent_workspace_path_creates_it() {
    // ExecutionStore::open is permissive and creates the dir if needed.
    // Studio inherits this behaviour in Plan 1; Plan 3 will tighten
    // (read-only mode + explicit "this is a new workspace" UX).
    let tmp = tempfile::tempdir().unwrap();
    let fresh = tmp.path().join("brand-new");
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(fresh.clone()),
    )
    .expect("open creates dir");
    assert_eq!(core.workspace().root, fresh);
    assert!(fresh.join("executions.db").exists());
}

use rowforge_core::execution_store::{
    FinishAttempt, NewAttempt, NewExecution, NewHandlerInstance, RunType, Simulation, Source,
};
use rowforge_studio_core::{AttemptId, ExecutionId, FailedPageQuery, ListFilter, RowHistory, RowOutcomeKind};

#[test]
fn rollup_returns_zero_counts_for_exec_with_no_attempts() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();
    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store
            .create_execution(NewExecution {
                name: Some("rollup-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap()
            .id
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let r = core.rollup(&ExecutionId::new(exec_id)).unwrap();
    assert_eq!(r.resolved, 0);
    // never_attempted should equal input_row_count (2) since no attempt has dispatched.
    assert_eq!(r.never_attempted, 2);
    assert!(r.by_error_code.is_empty());
}

#[test]
fn rollup_returns_not_found_for_unknown_exec() {
    let tmp = empty_workspace();
    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core
        .rollup(&ExecutionId::new("missing"))
        .expect_err("should return NotFound");
    matches!(err, UiError::NotFound(_));
}

#[test]
fn list_empty_workspace_returns_empty_vec() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let rows = core.list(ListFilter::default()).expect("list");
    assert!(rows.is_empty(), "got {:?}", rows);
}

#[test]
fn list_reflects_executions_created_via_core() {
    let tmp = empty_workspace();
    // Write a tiny CSV the core store can snapshot. The store computes
    // input_row_count and input_csv_hash itself from the file.
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();

    // Create an execution row directly through the core store, bypassing
    // the CLI command machinery. Scope it so the connection drops before
    // we open a second one via StudioCore.
    {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store
            .create_execution(NewExecution {
                name: Some("smoke".into()),
                input_csv_id: "smoke-csv".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
    }

    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let rows = core.list(ListFilter::default()).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "smoke");
    assert_eq!(rows[0].input_rows, Some(2));
    assert_eq!(rows[0].attempts_count, 0, "no attempts created in this test");
}

#[test]
fn list_serves_from_cache_on_repeated_call() {
    let tmp = empty_workspace();
    // First call populates the cache.
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let first = core.list(ListFilter::default()).unwrap();
    // Second call: cache should be hit (mtime unchanged, TTL not expired).
    // We can't directly assert "no DB hit" without a counter, but we can
    // assert the returned data is identical.
    let second = core.list(ListFilter::default()).unwrap();
    assert_eq!(first.len(), second.len());
    // Both empty for an empty workspace — exercise still proves no error.
}

#[test]
fn open_refuses_newer_schema_version() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let _store = ExecutionStore::open(tmp.path()).unwrap();
    }

    // Bump the schema_version table to simulate a future schema written
    // by a newer rowforge binary.
    let conn = rusqlite::Connection::open(tmp.path().join("executions.db")).unwrap();
    conn.execute("UPDATE schema_version SET version = 99", []).unwrap();
    drop(conn);

    let result = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    );
    assert!(result.is_err(), "should refuse newer schema");
    let err = result.err().unwrap();
    match err {
        UiError::WorkspaceLocked(msg) => {
            assert!(
                msg.contains("schema") || msg.contains("version"),
                "expected schema/version in message, got: {msg}"
            );
        }
        other => panic!("expected WorkspaceLocked, got: {other:?}"),
    }
}

#[test]
fn list_reflects_attempts_count_and_last_state() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();

    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("backfill-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_test".into(),
                manifest_hash: "sha256:test".into(),
                source_snapshot_dir: std::path::PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();

        let attempt = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id,
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Full,
                    simulation: Simulation::Real,
                },
            })
            .unwrap();

        store
            .finish_attempt(
                &attempt.id,
                FinishAttempt {
                    success_count: 2,
                    failed_count: 0,
                    aborted: false,
                    aborted_reason: None,
                },
            )
            .unwrap();

        exec.id
    };

    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let rows = core.list(ListFilter::default()).unwrap();
    let row = rows.iter().find(|r| r.id.as_str() == exec_id).unwrap();
    assert_eq!(row.attempts_count, 1, "attempts_count should reflect created attempt");
    assert!(
        row.last_attempt_state.is_some(),
        "last_attempt_state should be populated"
    );
    assert_eq!(
        row.last_attempt_state.as_deref(),
        Some("completed"),
        "last_attempt_state should be 'completed'"
    );
    // last_attempt_counts is None because no meta.json was written — acceptable.
}

#[test]
fn show_returns_exec_detail_for_existing_exec() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();
    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store.create_execution(NewExecution {
            name: Some("show-test".into()),
            input_csv_id: "csv1".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        }).unwrap().id
    };

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let detail = core.show(&ExecutionId::new(exec_id.clone())).unwrap();
    assert_eq!(detail.summary.id.as_str(), exec_id);
    assert_eq!(detail.summary.name, "show-test");
    assert_eq!(detail.attempts.len(), 0);
}

#[test]
fn show_returns_not_found_for_unknown_exec() {
    let tmp = empty_workspace();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core.show(&ExecutionId::new("missing")).expect_err("should not exist");
    matches!(err, UiError::NotFound(_));
}

#[test]
fn attempt_returns_detail_for_existing_attempt() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("attempt-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_test".into(),
                manifest_hash: "sha256:test".into(),
                source_snapshot_dir: std::path::PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();

        let attempt = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id,
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Full,
                    simulation: Simulation::Real,
                },
            })
            .unwrap();

        store
            .finish_attempt(
                &attempt.id,
                FinishAttempt {
                    success_count: 1,
                    failed_count: 0,
                    aborted: false,
                    aborted_reason: None,
                },
            )
            .unwrap();

        (exec.id, attempt.id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let det = core
        .attempt(
            &ExecutionId::new(exec_id),
            &AttemptId::new(attempt_id),
        )
        .unwrap();
    assert!(det.is_terminal, "finished attempt should be terminal");
    assert!(det.finished_at.is_some());
}

#[test]
fn attempt_returns_not_found_for_unknown_attempt() {
    let tmp = empty_workspace();
    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core
        .attempt(&ExecutionId::new("missing"), &AttemptId::new("a1"))
        .expect_err("should not exist");
    matches!(err, UiError::NotFound(_));
}

// ---------------------------------------------------------------------------
// T11: failed_page tests
// ---------------------------------------------------------------------------

/// Helper: create one attempt in `store` for `exec_id`, return the attempt id.
/// The attempt directory is created by `create_attempt`; no outcomes.jsonl is
/// written by this helper.
fn create_bare_attempt(store: &mut ExecutionStore, exec_id: &str) -> String {
    let hi = store
        .register_handler_instance(NewHandlerInstance {
            handler_id: "h_test".into(),
            manifest_hash: "sha256:test".into(),
            source_snapshot_dir: std::path::PathBuf::from("/tmp/snap"),
            binary_hash: None,
        })
        .unwrap();

    let attempt = store
        .create_attempt(NewAttempt {
            execution_id: exec_id.to_owned(),
            handler_instance_id: hi.id,
            parent_attempt_id: None,
            run_type: RunType {
                source: Source::Full,
                simulation: Simulation::Real,
            },
        })
        .unwrap();

    attempt.id
}

/// Write a synthetic outcomes.jsonl to `path`.
///
/// Line 0: BatchOutcome with one Success row (seq 0).
/// Line 1: BatchOutcome with one Error row  (seq 1, code "X").
/// Line 2: BatchOutcome with one Error row  (seq 2, code "Y").
///
/// Field names match the real RowOutcome serde shape exactly:
///   `{"first_seq":N,"seqs":[N],"outcomes":[{"type":"...","seq":N,...}]}`
fn write_fixture_outcomes(path: &std::path::Path) {
    let lines = concat!(
        "{\"first_seq\":0,\"seqs\":[0],\"outcomes\":[{\"type\":\"success\",\"seq\":0,\"data\":{},\"dur_ms\":12}]}\n",
        "{\"first_seq\":1,\"seqs\":[1],\"outcomes\":[{\"type\":\"error\",\"seq\":1,\"code\":\"X\",\"message\":\"err X\",\"dur_ms\":15,\"data\":{\"detail\":\"foo\"}}]}\n",
        "{\"first_seq\":2,\"seqs\":[2],\"outcomes\":[{\"type\":\"error\",\"seq\":2,\"code\":\"Y\",\"message\":\"err Y\",\"dur_ms\":17}]}\n",
    );
    std::fs::write(path, lines).unwrap();
}

#[test]
fn failed_page_returns_only_failed_rows() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\nb03\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("failed-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let attempt_id = create_bare_attempt(&mut store, &exec.id);

        // Write outcomes.jsonl into the attempt directory that create_attempt created.
        let attempt_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt_id);
        write_fixture_outcomes(&attempt_dir.join("outcomes.jsonl"));

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let page = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            0,
            100,
            None,
        ))
        .unwrap();

    assert_eq!(page.rows.len(), 2, "should return only the 2 error rows, not the success");
    assert!(
        matches!(page.rows[0].kind, RowOutcomeKind::Error),
        "first row kind should be Error"
    );
    assert!(
        matches!(page.rows[1].kind, RowOutcomeKind::Error),
        "second row kind should be Error"
    );
    assert_eq!(page.rows[0].seq, 1, "first failed row has seq 1");
    assert_eq!(page.rows[1].seq, 2, "second failed row has seq 2");
    assert_eq!(page.rows[0].error_code.as_deref(), Some("X"));
    assert_eq!(page.rows[1].error_code.as_deref(), Some("Y"));
    assert!(page.total_known.is_none(), "no sidecar index in v1");
    assert!(page.next_offset.is_none(), "file exhausted; no next page");
}

#[test]
fn failed_page_error_code_filter() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\nb03\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("filter-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let attempt_id = create_bare_attempt(&mut store, &exec.id);
        let attempt_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt_id);
        write_fixture_outcomes(&attempt_dir.join("outcomes.jsonl"));

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    let page = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            0,
            100,
            Some("X".into()),
        ))
        .unwrap();

    assert_eq!(page.rows.len(), 1, "filter should keep only code X");
    assert_eq!(page.rows[0].error_code.as_deref(), Some("X"));
}

#[test]
fn failed_page_offset_pagination() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\nb03\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("page-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let attempt_id = create_bare_attempt(&mut store, &exec.id);
        let attempt_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt_id);
        write_fixture_outcomes(&attempt_dir.join("outcomes.jsonl"));

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // Skip 1 failed row; should get only the second.
    let page = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            1,
            100,
            None,
        ))
        .unwrap();

    assert_eq!(page.rows.len(), 1, "offset 1 should skip first error");
    assert_eq!(page.rows[0].error_code.as_deref(), Some("Y"));
}

#[test]
fn failed_page_not_found_for_missing_execution() {
    let tmp = empty_workspace();
    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new("missing"),
            AttemptId::new("a1"),
            0,
            10,
            None,
        ))
        .expect_err("should return NotFound");
    assert!(matches!(err, UiError::NotFound(_)));
}

#[test]
fn failed_page_returns_empty_when_outcomes_jsonl_absent() {
    // Attempt-created-but-never-ran (no outcomes.jsonl yet) is a legitimate
    // state — handshake failures, replay-just-started, just-created attempts.
    // UI treats it as "no failed rows", so the call returns an empty page,
    // not NotFound.
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("no-outcomes".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
        let attempt_id = create_bare_attempt(&mut store, &exec.id);
        // Do NOT write outcomes.jsonl.
        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let page = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            0,
            10,
            None,
        ))
        .expect("missing outcomes.jsonl should yield empty page, not error");
    assert!(page.rows.is_empty());
    assert_eq!(page.next_offset, None);
    assert_eq!(page.total_known, None);
}

// ---------------------------------------------------------------------------
// T12: row_history tests
// ---------------------------------------------------------------------------

#[test]
fn row_history_returns_failures_then_success() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\n").unwrap();

    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("history-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        // Attempt 1: error for seq=0.
        let attempt1_id = create_bare_attempt(&mut store, &exec.id);
        let attempt1_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt1_id);
        std::fs::write(
            attempt1_dir.join("outcomes.jsonl"),
            "{\"first_seq\":0,\"seqs\":[0],\"outcomes\":[{\"type\":\"error\",\"seq\":0,\"code\":\"X\",\"message\":\"m\",\"dur_ms\":1}]}\n",
        )
        .unwrap();
        store
            .finish_attempt(
                &attempt1_id,
                FinishAttempt {
                    success_count: 0,
                    failed_count: 1,
                    aborted: false,
                    aborted_reason: None,
                },
            )
            .unwrap();

        // Attempt 2: success for seq=0.
        let attempt2_id = create_bare_attempt(&mut store, &exec.id);
        let attempt2_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt2_id);
        std::fs::write(
            attempt2_dir.join("outcomes.jsonl"),
            "{\"first_seq\":0,\"seqs\":[0],\"outcomes\":[{\"type\":\"success\",\"seq\":0,\"dur_ms\":2}]}\n",
        )
        .unwrap();
        store
            .finish_attempt(
                &attempt2_id,
                FinishAttempt {
                    success_count: 1,
                    failed_count: 0,
                    aborted: false,
                    aborted_reason: None,
                },
            )
            .unwrap();

        exec.id
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let hist = core
        .row_history(&ExecutionId::new(exec_id), 0)
        .unwrap();
    assert_eq!(hist.seq, 0);
    assert!(hist.resolved_at.is_some(), "should record the success");
    assert_eq!(hist.rows.len(), 1, "1 failure before the success");
    assert!(matches!(hist.rows[0].1, RowOutcomeKind::Error));
}

#[test]
fn row_history_returns_not_found_for_unknown_exec() {
    let tmp = empty_workspace();
    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let err = core
        .row_history(&ExecutionId::new("missing"), 0)
        .expect_err("should return NotFound");
    assert!(matches!(err, UiError::NotFound(_)));
}

// ---------------------------------------------------------------------------
// T20: round-out edge-case tests
// ---------------------------------------------------------------------------

/// A running attempt (never finished) must have `is_terminal = false`
/// and `finished_at = None`.
#[test]
fn attempt_for_running_attempt_marks_is_terminal_false() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("running-attempt".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        // create_attempt inserts a "running" row; we intentionally do NOT
        // call finish_attempt so the state remains non-terminal.
        let attempt_id = create_bare_attempt(&mut store, &exec.id);

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let det = core
        .attempt(
            &ExecutionId::new(exec_id),
            &AttemptId::new(attempt_id),
        )
        .unwrap();
    assert!(!det.is_terminal, "running attempt should NOT be terminal");
    assert!(det.finished_at.is_none(), "no finished_at for a running attempt");
}

/// Write a synthetic outcomes.jsonl with 5 error rows (seqs 0–4).
fn write_five_error_outcomes(path: &std::path::Path) {
    let mut lines = String::new();
    for seq in 0u64..5 {
        lines.push_str(&format!(
            "{{\"first_seq\":{seq},\"seqs\":[{seq}],\"outcomes\":[{{\"type\":\"error\",\"seq\":{seq},\"code\":\"E\",\"message\":\"err {seq}\",\"dur_ms\":10}}]}}\n"
        ));
    }
    std::fs::write(path, lines).unwrap();
}

/// Page 1 (offset=0, limit=2) returns 2 rows and `next_offset = Some(2)`.
/// Page 2 (offset=2, limit=2) returns the next 2 rows and `next_offset = Some(4)`.
/// The two pages contain non-overlapping seqs.
#[test]
fn failed_page_pagination_advances_offset() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\nb03\nb04\nb05\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("pagination-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let attempt_id = create_bare_attempt(&mut store, &exec.id);
        let attempt_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt_id);
        write_five_error_outcomes(&attempt_dir.join("outcomes.jsonl"));

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // Page 1: offset 0, limit 2 → 2 rows; more remain so next_offset = Some(2).
    let page1 = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id.clone()),
            AttemptId::new(attempt_id.clone()),
            0,
            2,
            None,
        ))
        .unwrap();
    assert_eq!(page1.rows.len(), 2, "page 1 should return 2 rows");
    assert_eq!(page1.next_offset, Some(2), "next_offset should advance to 2");

    // Page 2: offset 2, limit 2 → 2 more rows; more remain so next_offset = Some(4).
    let page2 = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            2,
            2,
            None,
        ))
        .unwrap();
    assert_eq!(page2.rows.len(), 2, "page 2 should return 2 rows");
    assert_eq!(page2.next_offset, Some(4), "next_offset should advance to 4");

    // No seq overlap between pages.
    assert_ne!(
        page1.rows[0].seq,
        page2.rows[0].seq,
        "pages must not overlap"
    );
}

/// Write exactly 2 error rows (seqs 0 and 1) to `path`.
fn write_two_error_outcomes(path: &std::path::Path) {
    let lines = concat!(
        "{\"first_seq\":0,\"seqs\":[0],\"outcomes\":[{\"type\":\"error\",\"seq\":0,\"code\":\"E\",\"message\":\"err 0\",\"dur_ms\":10}]}\n",
        "{\"first_seq\":1,\"seqs\":[1],\"outcomes\":[{\"type\":\"error\",\"seq\":1,\"code\":\"E\",\"message\":\"err 1\",\"dur_ms\":11}]}\n",
    );
    std::fs::write(path, lines).unwrap();
}

/// Regression test for "phantom next_offset at EOF":
/// When the page fills exactly at the limit AND we've reached EOF (no further
/// matching rows), `next_offset` must be `None`, not `Some(...)`.
///
/// With the buggy code this test FAILS because the EOF fallback path at the
/// bottom of `read_failed_page` sets `next_offset = Some(failed_seen)` whenever
/// `rows.len() >= limit`, even when the file is exhausted.
#[test]
fn failed_page_exactly_at_limit_with_eof_returns_no_next_offset() {
    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "billid\nb01\nb02\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("eof-limit-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let attempt_id = create_bare_attempt(&mut store, &exec.id);
        let attempt_dir = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt_id);
        write_two_error_outcomes(&attempt_dir.join("outcomes.jsonl"));

        (exec.id, attempt_id)
    };

    let core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // Request exactly 2 rows. File has exactly 2 error rows → page fills at
    // EOF. next_offset must be None (no phantom "Load more").
    let page = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            0,
            2,
            None,
        ))
        .unwrap();

    assert_eq!(page.rows.len(), 2);
    assert_eq!(
        page.next_offset,
        None,
        "exact limit + EOF should NOT advertise more pages: got {:?}",
        page.next_offset
    );
}

// ---------------------------------------------------------------------------
// Plan 4 — start_run / subscribe / cancel integration tests
// ---------------------------------------------------------------------------

/// Minimal valid `rowforge.yaml` whose `cmd` points to a nonexistent binary.
/// The manifest loads cleanly; workers fail to start → run eventually aborts.
fn minimal_handler_dir(tmp: &tempfile::TempDir) -> PathBuf {
    let handler = tmp.path().join("handler");
    std::fs::create_dir_all(&handler).unwrap();
    std::fs::write(
        handler.join("rowforge.yaml"),
        "name: test-handler\nversion: 0.1.0\nentry:\n  cmd: [\"/nonexistent-binary\"]\n",
    )
    .unwrap();
    handler
}

/// Create an execution with a small CSV input inside `tmp`. Returns the
/// execution id string.
fn create_execution_with_csv(tmp: &tempfile::TempDir) -> String {
    use rowforge_core::execution_store::NewExecution;
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "id\nr1\nr2\n").unwrap();
    let mut store = ExecutionStore::open(tmp.path()).unwrap();
    store
        .create_execution(NewExecution {
            name: Some("plan4-test".into()),
            input_csv_id: "csv1".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        })
        .unwrap()
        .id
}

// ---------------------------------------------------------------------------
// T7: orphan recovery tests (spec §3.7)
// ---------------------------------------------------------------------------

/// An attempt stuck in `running` with outcomes.jsonl mtime > 5 min should be
/// marked `aborted` when the workspace is opened.
#[test]
fn open_marks_orphan_attempts_as_aborted() {
    use filetime::{set_file_mtime, FileTime};
    use rowforge_core::execution_store::{NewAttempt, NewExecution, NewHandlerInstance, RunType, Simulation, Source};

    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "x\n1\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("orphan-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "test".into(),
                manifest_hash: "deadbeef".into(),
                source_snapshot_dir: tmp.path().to_path_buf(),
                binary_hash: None,
            })
            .unwrap();

        let attempt = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id,
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Full,
                    simulation: Simulation::Real,
                },
            })
            .unwrap();

        // Write a dummy outcomes.jsonl and back-date its mtime to 10 min ago.
        let outcomes = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt.id)
            .join("outcomes.jsonl");
        std::fs::create_dir_all(outcomes.parent().unwrap()).unwrap();
        std::fs::write(&outcomes, "").unwrap();
        let ten_min_ago =
            std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 60);
        set_file_mtime(&outcomes, FileTime::from_system_time(ten_min_ago)).unwrap();

        (exec.id, attempt.id)
    };

    // Opening should trigger orphan recovery and mark the attempt aborted.
    let _core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // Verify the attempt is now aborted.
    let store = ExecutionStore::open(tmp.path()).unwrap();
    let attempts = store.list_attempts_for_execution(&exec_id).unwrap();
    assert_eq!(attempts.len(), 1);
    let state_str = serde_json::to_value(&attempts[0].state)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    assert_eq!(
        state_str, "aborted",
        "orphan should be marked aborted; got attempt_id={attempt_id} state={state_str}"
    );
    assert_eq!(
        attempts[0].aborted_reason.as_deref(),
        Some("orphaned_on_restart"),
        "aborted_reason should be orphaned_on_restart"
    );
}

// ---------------------------------------------------------------------------
// Plan 5 T6: start_exec tests
// ---------------------------------------------------------------------------

#[test]
fn start_exec_creates_and_returns_id() {
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore};
    let tmp = tempfile::tempdir().unwrap();
    let csv_path = tmp.path().join("in.csv");
    std::fs::write(&csv_path, "row_id\nr1\nr2\nr3\n").unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core
        .start_exec(StartExecArgs::new(csv_path, "plan5_test_exec"))
        .unwrap();

    assert!(id.as_str().starts_with("e_"), "id should be e_<ulid>, got {}", id.as_str());
    let summaries = core.list(Default::default()).unwrap();
    assert!(summaries.iter().any(|s| s.id.as_str() == id.as_str()));
}

#[test]
fn start_exec_rejects_missing_input() {
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore, UiError};
    let tmp = tempfile::tempdir().unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let err = core
        .start_exec(StartExecArgs::new(tmp.path().join("nope.csv"), "x"))
        .unwrap_err();
    assert!(matches!(err, UiError::InvalidInput { .. }), "got {:?}", err);
}

#[test]
fn start_exec_rejects_duplicate_name() {
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore, UiError};
    let tmp = tempfile::tempdir().unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();

    let args = StartExecArgs::new(csv, "dup_name");
    core.start_exec(args.clone()).unwrap();
    let err = core.start_exec(args).unwrap_err();
    assert!(matches!(err, UiError::DuplicateExecName { .. }), "got {:?}", err);
}

/// A running attempt whose outcomes.jsonl was written < 5 min ago must NOT be
/// touched by orphan recovery — a live CLI run may still be active externally.
#[test]
fn open_leaves_recent_running_attempts_alone() {
    use rowforge_core::execution_store::{NewAttempt, NewExecution, NewHandlerInstance, RunType, Simulation, Source};

    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "x\n1\n").unwrap();

    let (exec_id, attempt_id) = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: Some("recent-test".into()),
                input_csv_id: "csv1".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "test".into(),
                manifest_hash: "deadbeef".into(),
                source_snapshot_dir: tmp.path().to_path_buf(),
                binary_hash: None,
            })
            .unwrap();

        let attempt = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id,
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Full,
                    simulation: Simulation::Real,
                },
            })
            .unwrap();

        // Write outcomes.jsonl with a current mtime (just now — well within 5 min).
        let outcomes = tmp
            .path()
            .join("executions")
            .join(&exec.id)
            .join("attempts")
            .join(&attempt.id)
            .join("outcomes.jsonl");
        std::fs::create_dir_all(outcomes.parent().unwrap()).unwrap();
        std::fs::write(&outcomes, "").unwrap();
        // mtime left at filesystem default (now) — no back-dating.

        (exec.id, attempt.id)
    };

    // Opening should NOT mark the recent attempt as orphaned.
    let _core =
        StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // The attempt must still be running.
    let store = ExecutionStore::open(tmp.path()).unwrap();
    let attempts = store.list_attempts_for_execution(&exec_id).unwrap();
    assert_eq!(attempts.len(), 1);
    let state_str = serde_json::to_value(&attempts[0].state)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    assert_eq!(
        state_str, "running",
        "recent attempt should remain running; got attempt_id={attempt_id} state={state_str}"
    );
}

/// Test 1: start_run plumbing — a valid manifest with a nonexistent binary
/// should spawn, register the session, and eventually emit an Aborted event
/// once all workers fail to start.
///
/// Marked #[ignore] because the pipeline startup timeout (30 s default) makes
/// this test too slow for the regular CI matrix. Run with
/// `cargo test -- --ignored start_run_returns_handle_and_subscriber_gets_event`
/// to exercise the full async plumbing.
#[ignore = "startup-timeout makes this 30 s; run manually to verify plumbing"]
#[tokio::test]
async fn start_run_returns_handle_and_subscriber_gets_event() {
    use tokio::time::{timeout, Duration};

    let tmp = tempfile::tempdir().unwrap();
    let exec_id = create_execution_with_csv(&tmp);
    let handler = minimal_handler_dir(&tmp);

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let opts = RunOpts::new(handler);
    let started = core
        .start_run(&ExecutionId::new(exec_id), opts)
        .expect("start_run should succeed with valid manifest");
    let handle = started.handle;

    let mut stream = core.subscribe(&handle).expect("subscribe should succeed");

    // Wait up to 35 s for any terminal event.
    let received = timeout(Duration::from_secs(35), async move {
        loop {
            match stream.rx.recv().await {
                Ok(ProgressEvent::Aborted { .. }) | Ok(ProgressEvent::Done(_)) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await;

    assert!(
        matches!(received, Ok(true)),
        "expected Aborted or Done event within 35 s; got: {:?}",
        received
    );
}

#[test]
fn export_writes_files_for_csv_format() {
    use rowforge_core::export::{ExportFormat, ExportOpts};
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore};

    let tmp = tempfile::tempdir().unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\nr2\n").unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(
        StartExecArgs::new(csv, "export_test")
    ).unwrap();

    let opts = ExportOpts::new(ExportFormat::Csv)
        .with_output_dir(tmp.path().join("out"));
    let report = core.export(&id, opts).unwrap();
    assert!(report.output_dir.exists());
    let names: Vec<&str> = report.written_files.iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .collect();
    assert!(names.contains(&"success.csv"), "got files: {:?}", names);
    assert!(names.contains(&"failed.csv"), "got files: {:?}", names);
}

#[test]
fn export_require_complete_refuses_when_unresolved() {
    use rowforge_core::export::{ExportFormat, ExportOpts};
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore, UiError};

    let tmp = tempfile::tempdir().unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(
        StartExecArgs::new(csv, "strict_test")
    ).unwrap();

    let opts = ExportOpts::new(ExportFormat::Csv)
        .with_output_dir(tmp.path().join("out"))
        .with_require_complete(true);
    let err = core.export(&id, opts).unwrap_err();
    assert!(
        matches!(err, UiError::ExportIncomplete { missing_count } if missing_count > 0),
        "got {:?}",
        err
    );
}

/// Test 2: start_run enforces the per-execution concurrency limit of 1.
/// A second start_run for the same exec_id must return UiError::RunBusy.
///
/// Must run inside a tokio runtime because start_run spawns async tasks.
#[tokio::test]
async fn start_run_enforces_per_exec_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let exec_id = create_execution_with_csv(&tmp);
    let handler = minimal_handler_dir(&tmp);

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();
    let opts = RunOpts::new(handler.clone());

    // First start_run should succeed.
    let _handle = core
        .start_run(&ExecutionId::new(exec_id.clone()), opts)
        .expect("first start_run should succeed");

    // Second start_run for the same exec should be rejected.
    let opts2 = RunOpts::new(handler);
    let err = core
        .start_run(&ExecutionId::new(exec_id), opts2)
        .expect_err("second start_run must return RunBusy");

    assert!(
        matches!(err, UiError::RunBusy { .. }),
        "expected RunBusy, got: {:?}",
        err
    );
}

/// Test 3: cancel called with an unknown / expired RunHandle returns
/// UiError::UnknownHandle.
///
/// Must run inside a tokio runtime because cancel looks up the SessionRegistry
/// which may interact with async state.
#[tokio::test]
async fn cancel_unknown_handle_returns_unknown_handle_error() {
    use rowforge_studio_core::CancelMode;

    let tmp = empty_workspace();
    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    let bogus = RunHandle::from("run-BOGUS0000000000000000000".to_string());
    let err = core
        .cancel(&bogus, CancelMode::Soft)
        .expect_err("cancel on unknown handle must error");

    assert!(
        matches!(err, UiError::UnknownHandle(_)),
        "expected UnknownHandle, got: {:?}",
        err
    );
}

#[tokio::test]
async fn active_runs_stream_emits_at_1hz() {
    use futures::StreamExt;

    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();

    let mut stream = Box::pin(core.active_runs_stream());

    // Collect 2 ticks within 2.5 seconds.
    let mut ticks = Vec::new();
    for _ in 0..2 {
        let tick = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            stream.next(),
        ).await.expect("timed out").expect("stream ended");
        ticks.push(tick);
    }

    assert_eq!(ticks.len(), 2);
    assert_eq!(ticks[0].active_runs, 0, "empty workspace = 0 active runs");
}

#[tokio::test]
async fn active_runs_stream_reflects_started_runs() {
    use futures::StreamExt;

    let tmp = empty_workspace();
    let csv = tmp.path().join("input.csv");
    std::fs::write(&csv, "x\n1\n").unwrap();
    let exec_id = {
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        store.create_execution(NewExecution {
            name: Some("rollup-test".into()),
            input_csv_id: "csv1".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        }).unwrap().id
    };

    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();

    // Start a run (will fail quickly with bad handler — but the session
    // will exist briefly before being removed).
    let _started = core.start_run(
        &rowforge_studio_core::ExecutionId::new(exec_id),
        rowforge_studio_core::RunOpts::new(
            std::path::PathBuf::from("/non-existent"),
        ),
    );

    let mut stream = Box::pin(core.active_runs_stream());
    let tick = tokio::time::timeout(
        std::time::Duration::from_millis(1500),
        stream.next(),
    ).await.expect("timed out").expect("stream ended");

    // active_runs is racy — could be 0 (if session was removed already)
    // or 1 (if still running). Both are acceptable; the test verifies
    // the stream yields a tick.
    assert!(tick.active_runs <= 1);
}

// ---------------------------------------------------------------------------
// Plan 6 T3: ExecSummary.last_handler_dir projection
// ---------------------------------------------------------------------------

#[test]
fn exec_summary_carries_last_handler_dir() {
    use rowforge_studio_core::{OpenOpts, StartExecArgs, StudioCore};

    let tmp = tempfile::tempdir().unwrap();
    let csv = tmp.path().join("in.csv");
    std::fs::write(&csv, "row_id\nr1\n").unwrap();

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().into())).unwrap();
    let id = core.start_exec(StartExecArgs::new(csv, "t3_test")).unwrap();

    // Fresh exec: last_handler_dir is None.
    let summaries = core.list(Default::default()).unwrap();
    let s = summaries.iter().find(|s| s.id == id).unwrap();
    assert_eq!(s.last_handler_dir, None);
}

// ---------------------------------------------------------------------------
// Plan 6 T4: start_run persists exec.last_handler_dir
// ---------------------------------------------------------------------------

/// Verifies that `start_run` writes the canonicalized handler dir to sqlite
/// (via `store.set_last_handler_dir`) under the same lock window that creates
/// the attempt. The persistence must happen synchronously before the function
/// returns, so `list()` reflects it immediately after `start_run` succeeds.
///
/// The pipeline task will fail asynchronously (./nope doesn't exist), but
/// that's irrelevant — we only care about the synchronous sqlite write.
#[tokio::test]
async fn start_run_persists_last_handler_dir() {
    use rowforge_studio_core::{ExecutionId, OpenOpts, StudioCore};

    let tmp = tempfile::tempdir().unwrap();
    let exec_id = create_execution_with_csv(&tmp);
    let handler = minimal_handler_dir(&tmp);

    let core = StudioCore::open(OpenOpts::new().with_workspace(tmp.path().to_path_buf())).unwrap();

    // start_run will succeed at the sqlite write level (creates the attempt,
    // persists last_handler_dir, returns RunStartedHandle). The background
    // pipeline task will then fail because /nonexistent-binary doesn't exist,
    // but that's fine — we're testing the synchronous write path.
    let _started = core
        .start_run(&ExecutionId::new(exec_id.clone()), RunOpts::new(handler.clone()))
        .expect("start_run should succeed; pipeline may fail asynchronously");

    let summaries = core.list(Default::default()).unwrap();
    let s = summaries
        .iter()
        .find(|s| s.id == ExecutionId::new(exec_id.clone()))
        .unwrap();
    let canon = handler.canonicalize().unwrap();
    assert_eq!(
        s.last_handler_dir,
        Some(canon),
        "last_handler_dir should be the canonicalized handler dir",
    );
}

// ---------------------------------------------------------------------------
// Plan 6 T9 — SessionRegistry workspace_limit sourced from OpenOpts
// ---------------------------------------------------------------------------

#[test]
fn workspace_limit_honors_max_concurrent_runs() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new()
            .with_workspace(tmp.path().to_path_buf())
            .with_max_concurrent_runs(Some(7)),
    )
    .unwrap();
    assert_eq!(core.sessions().workspace_limit(), 7);
}

#[test]
fn workspace_limit_defaults_to_three_when_unset() {
    let tmp = empty_workspace();
    let core = StudioCore::open(
        OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(core.sessions().workspace_limit(), 3);
}

// ---------------------------------------------------------------------------
// Plan 7 T3 — handler_list + handler_show
// ---------------------------------------------------------------------------

#[test]
fn handler_list_finds_dirs_under_workspace_handlers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let handlers_dir = tmp.path().join("handlers");
    std::fs::create_dir_all(&handlers_dir).unwrap();

    // Three test handlers: valid manifest / no manifest / invalid manifest.
    let valid = handlers_dir.join("alpha");
    std::fs::create_dir_all(&valid).unwrap();
    std::fs::write(
        valid.join("rowforge.yaml"),
        "name: alpha\nversion: 0.1.0\nlanguage: go\nentry:\n  cmd: [\"./alpha\"]\n",
    ).unwrap();

    let no_manifest = handlers_dir.join("bravo");
    std::fs::create_dir_all(&no_manifest).unwrap();

    let invalid = handlers_dir.join("charlie");
    std::fs::create_dir_all(&invalid).unwrap();
    std::fs::write(invalid.join("rowforge.yaml"), "this: is: bad: yaml: :::").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();

    let mut list = core.handler_list().unwrap();
    list.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "alpha");
    assert_eq!(list[0].manifest_status, rowforge_studio_core::ManifestStatus::Valid);
    assert_eq!(list[0].version.as_deref(), Some("0.1.0"));
    assert_eq!(list[0].language.as_deref(), Some("go"));

    assert_eq!(list[1].name, "bravo");
    assert_eq!(list[1].manifest_status, rowforge_studio_core::ManifestStatus::Missing);
    assert_eq!(list[1].version, None);

    assert_eq!(list[2].name, "charlie");
    assert_eq!(list[2].manifest_status, rowforge_studio_core::ManifestStatus::Invalid);
}

#[test]
fn handler_list_returns_empty_when_handlers_dir_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    // No handlers/ subdir at all.
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    assert_eq!(core.handler_list().unwrap().len(), 0);
}

// Regression: last_modified must be the max of dir mtime AND every top-level
// entry's mtime, because writing a file does not always update the parent
// directory's own mtime (platform-dependent).
//
// Strategy: snapshot last_modified before a file write, sleep 1.1s (enough
// to advance the filesystem clock), write a new file, snapshot again.
// The second snapshot must be strictly later than the first. This avoids
// adding a dep on filetime's set_file_mtime, which is unreliable on APFS.
#[test]
fn list_uses_max_mtime_over_dir_contents() {
    let tmp = tempfile::TempDir::new().unwrap();
    let h = tmp.path().join("handlers").join("zeta");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: zeta\nversion: 0.1.0\nentry:\n  cmd: [\"./zeta\"]\n",
    ).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();

    // Baseline: last_modified before any file is written inside the handler dir
    // (only rowforge.yaml exists at this point).
    let list_before = core.handler_list().unwrap();
    assert_eq!(list_before.len(), 1);
    let before = list_before[0].last_modified;

    // Sleep past filesystem clock resolution (1.1 s covers HFS+ 1-s and
    // APFS sub-second granularity).
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Write a new file inside the handler dir. The file's mtime will be later
    // than the dir's own mtime (which the OS may not update on file creation).
    std::fs::write(h.join("handler.go"), "package main").unwrap();

    let list_after = core.handler_list().unwrap();
    assert_eq!(list_after.len(), 1);
    let after = list_after[0].last_modified;

    assert!(
        after > before,
        "last_modified should advance when a file inside the handler dir is \
         written; before={before:?}, after={after:?}. The fold over dir entries \
         is likely broken."
    );
}

#[test]
fn handler_show_returns_manifest_and_source_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let h = tmp.path().join("handlers").join("alpha");
    std::fs::create_dir_all(&h).unwrap();
    std::fs::write(
        h.join("rowforge.yaml"),
        "name: alpha\nversion: 0.1.0\nentry:\n  cmd: [\"./alpha\"]\n",
    ).unwrap();
    std::fs::write(h.join("handler.go"), "package main").unwrap();
    std::fs::write(h.join("go.mod"), "module alpha\ngo 1.22").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let detail = core.handler_show("alpha").unwrap();

    assert_eq!(detail.summary.name, "alpha");
    assert_eq!(detail.summary.manifest_status, rowforge_studio_core::ManifestStatus::Valid);
    assert!(detail.manifest.is_some());
    assert!(detail.manifest_errors.is_empty());

    let names: Vec<&str> = detail.source_files.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"handler.go"), "got files: {:?}", names);
    assert!(names.contains(&"go.mod"), "got files: {:?}", names);
    // rowforge.yaml is the manifest, not "source" — must be excluded.
    assert!(!names.contains(&"rowforge.yaml"));
}

#[test]
fn handler_show_errors_on_unknown_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let err = core.handler_show("ghost").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerNotFound { .. }),
        "got: {:?}", err);
}

#[test]
fn handler_show_rejects_invalid_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let err = core.handler_show("Bad Name").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "got: {:?}", err);
}

// ---------------------------------------------------------------------------
// Plan 7 T4 — handler_reveal_path + handler_open_editor integration tests
// ---------------------------------------------------------------------------

#[test]
fn handler_reveal_path_returns_dir_for_existing_handler() {
    let tmp = tempfile::TempDir::new().unwrap();
    let h = tmp.path().join("handlers").join("alpha");
    std::fs::create_dir_all(&h).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();

    let path = core.handler_reveal_path("alpha").unwrap();
    assert_eq!(path, h);
}

#[test]
fn handler_reveal_path_errors_on_unknown_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let err = core.handler_reveal_path("ghost").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerNotFound { .. }));
}

#[test]
fn handler_open_editor_rejects_invalid_name_before_resolver() {
    let tmp = tempfile::TempDir::new().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    ).unwrap();
    let err = core.handler_open_editor("../etc").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }));
}

// ---------------------------------------------------------------------------
// Plan 7 T6 — handler_scaffold
// ---------------------------------------------------------------------------

#[test]
fn scaffold_writes_go_stdio_template() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    let name = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "my-handler",
            rowforge_studio_core::ScaffoldTemplate::GoStdio,
            "email",
        ))
        .unwrap();
    assert_eq!(name, "my-handler");

    let dir = tmp.path().join("handlers").join("my-handler");
    assert!(dir.is_dir(), "handler dir should be created");

    // 3 files, with variable substitution.
    let yaml = std::fs::read_to_string(dir.join("rowforge.yaml")).unwrap();
    assert!(yaml.contains("name: my-handler"), "rowforge.yaml didn't get {{name}} replaced; got:\n{}", yaml);
    assert!(yaml.contains("email"), "rowforge.yaml didn't get {{primary_field}} replaced; got:\n{}", yaml);
    assert!(!yaml.contains("{{name}}"), "rowforge.yaml still has unrendered {{name}}");
    assert!(!yaml.contains("{{primary_field}}"), "rowforge.yaml still has unrendered {{primary_field}}");

    assert!(dir.join("handler.go").is_file());
    assert!(dir.join("go.mod").is_file());

    // go.mod should reference the handler name.
    let gomod = std::fs::read_to_string(dir.join("go.mod")).unwrap();
    assert!(gomod.contains("my-handler"), "go.mod should reference name; got:\n{}", gomod);
}

#[test]
fn scaffold_writes_go_batch_template_with_batch_mode() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-batch").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    core.handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
        "batch-handler",
        rowforge_studio_core::ScaffoldTemplate::GoBatch,
        "order_id",
    )).unwrap();

    let dir = tmp.path().join("handlers").join("batch-handler");
    let yaml = std::fs::read_to_string(dir.join("rowforge.yaml")).unwrap();
    assert!(yaml.contains("batch"), "go_batch template should mention batch mode; got:\n{}", yaml);
}

#[test]
fn scaffold_writes_empty_template_two_files() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-empty").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    core.handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
        "skel",
        rowforge_studio_core::ScaffoldTemplate::Empty,
        "id",
    )).unwrap();

    let dir = tmp.path().join("handlers").join("skel");
    assert!(dir.join("rowforge.yaml").is_file());
    assert!(dir.join("handler.go").is_file());
    // Empty template explicitly does NOT include go.mod.
    assert!(!dir.join("go.mod").exists(),
        "Empty template should not write go.mod (user builds however they want)");
}

#[test]
fn scaffold_rejects_invalid_name() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-bn").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "Has Space",
            rowforge_studio_core::ScaffoldTemplate::Empty,
            "x",
        ))
        .unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "got: {:?}", err);

    // Verify no partial directory was created.
    assert!(!tmp.path().join("handlers").join("Has Space").exists());
}

#[test]
fn scaffold_errors_when_name_already_exists() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-ex").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("taken")).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "taken",
            rowforge_studio_core::ScaffoldTemplate::Empty,
            "x",
        ))
        .unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerExists { .. }),
        "got: {:?}", err);
}

#[test]
fn scaffold_rejects_leading_hyphen_name() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-lh").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    // Leading hyphen — must be rejected by the tightened validate_name regex.
    let err = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "-foo",
            rowforge_studio_core::ScaffoldTemplate::Empty,
            "id",
        ))
        .unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "leading-hyphen name should be rejected; got: {:?}", err);

    let err2 = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "-",
            rowforge_studio_core::ScaffoldTemplate::Empty,
            "id",
        ))
        .unwrap_err();
    assert!(matches!(err2, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "bare hyphen name should be rejected; got: {:?}", err2);
}

#[test]
fn scaffold_rejects_unsafe_primary_field() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-pf").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    // primary_field with a quote (YAML / Go injection risk).
    let err = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "my-handler",
            rowforge_studio_core::ScaffoldTemplate::GoStdio,
            "id\"",
        ))
        .unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidArg(_)),
        "primary_field with quote should return InvalidArg; got: {:?}", err);

    // primary_field with embedded newline.
    let err2 = core
        .handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
            "my-handler2",
            rowforge_studio_core::ScaffoldTemplate::GoStdio,
            "id\nentry:\n  cmd: [\"rm\", \"-rf\"]",
        ))
        .unwrap_err();
    assert!(matches!(err2, rowforge_studio_core::UiError::InvalidArg(_)),
        "primary_field with newline should return InvalidArg; got: {:?}", err2);
}

#[test]
fn scaffold_accepts_valid_primary_field() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-sc-pfok").tempdir().unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    core.handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
        "good-handler",
        rowforge_studio_core::ScaffoldTemplate::GoStdio,
        "order_id",
    )).expect("valid identifier primary_field should be accepted");
}

#[test]
fn delete_removes_handler_dir() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-del").tempdir().unwrap();
    let dir = tmp.path().join("handlers").join("doomed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rowforge.yaml"), "name: doomed\nversion: 0.1.0\nentry:\n  cmd: [\"./x\"]\n").unwrap();
    std::fs::write(dir.join("handler.go"), "package main").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    core.handler_delete("doomed").unwrap();

    assert!(!dir.exists(), "handler dir should be gone");
    // Sibling handlers/ dir still exists (we only removed the one named dir).
    assert!(tmp.path().join("handlers").is_dir());
}

#[test]
fn delete_errors_on_unknown_name() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-del-nf").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_delete("ghost").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerNotFound { .. }));
}

#[test]
fn delete_rejects_invalid_name_before_any_fs_op() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-del-bn").tempdir().unwrap();
    // Regression: even if a path like ../etc were ALLOWED through validation,
    // we'd be opening fs::remove_dir_all on a user-controllable absolute
    // path. The regex fence guarantees we never get there.
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_delete("../etc").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "got: {:?}", err);
}

#[cfg(unix)]
#[test]
fn delete_rejects_symlinked_dir_pointing_outside_workspace() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::Builder::new().prefix("rfs-plan7-del-sym").tempdir().unwrap();
    let outside = tempfile::Builder::new().prefix("rfs-plan7-del-victim").tempdir().unwrap();

    // Build a victim dir OUTSIDE the workspace that we should not be able
    // to delete by symlinking into handlers/.
    let victim = outside.path().join("precious");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("important.txt"), "do not delete me").unwrap();

    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    symlink(&victim, tmp.path().join("handlers").join("evil"))
        .expect("symlink creation should succeed on unix");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let result = core.handler_delete("evil");

    // The canonicalize() check MUST reject this: the symlink resolves
    // outside the workspace's handlers/ dir, so layer 3 fires.
    assert!(
        matches!(result, Err(_)),
        "delete of a symlink pointing outside the workspace must return Err; got: {:?}",
        result
    );

    // Verify the symlink entry itself is still intact (not removed as a
    // side-effect of the rejection). Use symlink_metadata so we detect the
    // link itself rather than following it to the (still-existing) target.
    assert!(
        std::fs::symlink_metadata(tmp.path().join("handlers").join("evil")).is_ok(),
        "the symlink entry should still exist after rejection"
    );

    // The victim dir and its contents must be untouched.
    assert!(victim.is_dir(), "victim dir should NOT have been deleted");
    assert!(victim.join("important.txt").is_file(),
        "victim's contents should NOT have been removed");
}

#[test]
fn rename_moves_handler_dir() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn").tempdir().unwrap();
    let src = tmp.path().join("handlers").join("old-name");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("rowforge.yaml"),
        "name: old-name\nversion: 0.1.0\nentry:\n  cmd: [\"./old-name\"]\n",
    ).unwrap();
    std::fs::write(src.join("handler.go"), "package main").unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    core.handler_rename("old-name", "new-name").unwrap();

    let dst = tmp.path().join("handlers").join("new-name");
    assert!(!src.is_dir(), "old dir should be gone");
    assert!(dst.is_dir(), "new dir should exist");
    // Contents preserved (rename, not copy).
    assert!(dst.join("rowforge.yaml").is_file());
    assert!(dst.join("handler.go").is_file());
    // Note: rowforge.yaml's `name:` field is NOT auto-rewritten —
    // that's user's responsibility after rename. The dir was renamed,
    // the file contents are byte-identical.
}

#[test]
fn rename_errors_on_unknown_source() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn-nf").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("ghost", "new-name").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerNotFound { .. }),
        "got: {:?}", err);
}

#[test]
fn rename_errors_when_destination_exists() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn-ex").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("a")).unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("b")).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("a", "b").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::HandlerExists { .. }),
        "got: {:?}", err);

    // Both dirs still there — rename was a no-op.
    assert!(tmp.path().join("handlers").join("a").is_dir());
    assert!(tmp.path().join("handlers").join("b").is_dir());
}

#[test]
fn rename_rejects_invalid_old_name() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn-bo").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("../etc", "new-name").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }));
}

#[test]
fn rename_rejects_invalid_new_name() {
    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn-bn").tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers").join("ok")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    let err = core.handler_rename("ok", "Bad Name").unwrap_err();
    assert!(matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "got: {:?}", err);
    // Source still intact — pre-flight regex blocked anything from happening.
    assert!(tmp.path().join("handlers").join("ok").is_dir());
}

/// Renaming a handler dir is a pure `fs::rename` — sqlite is NOT updated.
/// Existing executions whose `last_handler_dir` pointed at the old path
/// must still see the old path after the rename (lazy / content-addressed
/// semantics from spec part-2 footnote).
#[test]
fn rename_preserves_executions_last_handler_dir() {
    use rowforge_core::execution_store::{ExecutionStore, NewExecution};

    let tmp = tempfile::Builder::new().prefix("rfs-plan7-rn-lazy").tempdir().unwrap();

    // 1. Create the handler dir via scaffold.
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();
    core.handler_scaffold(rowforge_studio_core::ScaffoldArgs::new(
        "old-name",
        rowforge_studio_core::ScaffoldTemplate::Empty,
        "id",
    )).unwrap();
    let handler_abs = tmp.path().join("handlers").join("old-name");

    // 2. Insert an execution row with last_handler_dir pointing at the handler dir.
    let exec_id = {
        let csv = tmp.path().join("input.csv");
        std::fs::write(&csv, "id\nr1\n").unwrap();
        let mut store = ExecutionStore::open(tmp.path()).unwrap();
        let exec = store.create_execution(NewExecution {
            name: Some("lazy-rename-test".into()),
            input_csv_id: "csv-lazy".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        }).unwrap();
        store.set_last_handler_dir(&exec.id, &handler_abs).unwrap();
        exec.id
    };

    // 3. Rename the handler.
    core.handler_rename("old-name", "new-name").unwrap();

    // 4. Assert new dir exists, old dir is gone.
    assert!(!tmp.path().join("handlers").join("old-name").is_dir(),
        "old handler dir should be gone after rename");
    assert!(tmp.path().join("handlers").join("new-name").is_dir(),
        "new handler dir should exist after rename");

    // 5. Assert last_handler_dir in sqlite is UNCHANGED (still the old path).
    let store = ExecutionStore::open(tmp.path()).unwrap();
    let exec = store.get_execution(&exec_id).unwrap()
        .expect("execution should still exist");
    assert_eq!(
        exec.last_handler_dir.as_deref(),
        Some(handler_abs.as_path()),
        "rename must NOT update last_handler_dir in sqlite; got: {:?}",
        exec.last_handler_dir,
    );
}

// ============================================================
// Plan 7 T15 — Settings.preferred_editor roundtrip + plumbing
// ============================================================

/// Settings with preferred_editor = Some(...) survives a JSON roundtrip.
#[test]
fn settings_preferred_editor_roundtrip() {
    let mut s = rowforge_studio_core::Settings::default();
    s.preferred_editor = Some("code --wait".into());
    let mut buf = Vec::new();
    s.save_to(&mut buf).unwrap();
    let loaded = rowforge_studio_core::Settings::load_from(buf.as_slice()).unwrap();
    assert_eq!(loaded.preferred_editor, Some("code --wait".into()));
}

/// Settings parsed from JSON without the field defaults to None (backward compat).
#[test]
fn settings_preferred_editor_absent_defaults_to_none() {
    let json = br#"{"schema_version": 1}"#;
    let parsed = rowforge_studio_core::Settings::load_from(json.as_slice()).unwrap();
    assert_eq!(parsed.preferred_editor, None);
}

/// OpenOpts.with_preferred_editor propagates into StudioCore.
/// Verified via handler_open_editor — when no handler dir exists it returns
/// HandlerNotFound (not EditorNotFound), confirming the editor value reached
/// the resolver but the handler guard fired first.
#[test]
fn studio_core_preferred_editor_set_from_open_opts() {
    let tmp = empty_workspace();
    std::fs::create_dir_all(tmp.path().join("handlers").join("my-handler")).unwrap();
    // Write a minimal manifest so show() works.
    std::fs::write(
        tmp.path().join("handlers").join("my-handler").join("rowforge.yaml"),
        "name: my-handler\nversion: 0.1.0\nentry:\n  cmd: [\"./my-handler\"]\n",
    ).unwrap();

    // Pass a deliberately bogus command so resolve_editor returns an error.
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new()
            .with_workspace(tmp.path().into())
            .with_preferred_editor(Some("__rowforge_t15_bogus_editor__".into())),
    ).unwrap();

    let err = core.handler_open_editor("my-handler").unwrap_err();
    // The bogus preferred editor is tried first — the error should be
    // EditorNotFound or Io (failed to spawn), NOT HandlerNotFound.
    let is_editor_error = matches!(
        err,
        rowforge_studio_core::UiError::EditorNotFound { .. }
            | rowforge_studio_core::UiError::Io(_)
    );
    assert!(is_editor_error, "expected editor error, got: {:?}", err);
}

/// set_preferred_editor updates the live field without workspace re-open.
#[test]
fn studio_core_set_preferred_editor_updates_live_field() {
    let tmp = empty_workspace();
    std::fs::create_dir_all(tmp.path().join("handlers").join("h1")).unwrap();
    std::fs::write(
        tmp.path().join("handlers").join("h1").join("rowforge.yaml"),
        "name: h1\nversion: 0.1.0\nentry:\n  cmd: [\"./h1\"]\n",
    ).unwrap();

    let mut core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    ).unwrap();

    // Initially None — resolver falls through to environment/probes.
    // Now set a bogus value and verify it's picked up.
    core.set_preferred_editor(Some("__rowforge_t15_updated_bogus__".into()));

    let err = core.handler_open_editor("h1").unwrap_err();
    let is_editor_error = matches!(
        err,
        rowforge_studio_core::UiError::EditorNotFound { .. }
            | rowforge_studio_core::UiError::Io(_)
    );
    assert!(is_editor_error, "expected editor error after set, got: {:?}", err);
}

// ============================================================
// Plan 8 T6 — handler_build + build cache + HandlerDetail.last_build
// ============================================================

/// Helper: write a minimal handler under `<workspace>/handlers/<name>/`.
fn write_handler(workspace: &tempfile::TempDir, name: &str, manifest_yaml: &str) {
    let dir = workspace.path().join("handlers").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rowforge.yaml"), manifest_yaml).unwrap();
}

/// A handler whose manifest has no entry.build returns NoBuildCommand.
#[test]
fn handler_build_no_command_returns_no_build_command_error() {
    let tmp = empty_workspace();
    write_handler(
        &tmp,
        "no-build",
        "name: no-build\nversion: 0.1.0\nentry:\n  cmd: [\"./no-build\"]\n",
    );

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    )
    .unwrap();

    let err = core.handler_build("no-build").unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::NoBuildCommand { .. }),
        "expected NoBuildCommand, got: {:?}",
        err
    );
}

/// Successful build is cached; subsequent handler_show returns last_build with exit_code 0.
#[test]
fn handler_build_success_populates_last_build_in_show() {
    let tmp = empty_workspace();
    write_handler(
        &tmp,
        "build-ok",
        "name: build-ok\nversion: 0.1.0\nentry:\n  cmd: [\"./build-ok\"]\n  build: [\"sh\", \"-c\", \"echo hi\"]\n",
    );

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    )
    .unwrap();

    let outcome = core.handler_build("build-ok").expect("build should succeed");
    assert_eq!(outcome.exit_code, 0);

    let detail = core.handler_show("build-ok").expect("show should succeed");
    let last = detail.last_build.expect("last_build should be Some after successful build");
    assert_eq!(last.exit_code, 0, "cached outcome should have exit_code 0");
}

/// Failed build is still cached; subsequent handler_show returns last_build with the non-zero exit code and captured stderr.
#[test]
fn handler_build_failure_caches_outcome_for_inspection() {
    let tmp = empty_workspace();
    write_handler(
        &tmp,
        "build-fail",
        "name: build-fail\nversion: 0.1.0\nentry:\n  cmd: [\"./build-fail\"]\n  build: [\"sh\", \"-c\", \"echo oops >&2; exit 5\"]\n",
    );

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    )
    .unwrap();

    let err = core.handler_build("build-fail").unwrap_err();
    assert!(
        matches!(
            err,
            rowforge_studio_core::UiError::BuildFailed { exit_code: 5, .. }
        ),
        "expected BuildFailed(5), got: {:?}",
        err
    );

    // Even on failure, the outcome must be cached for UI log display.
    let detail = core.handler_show("build-fail").expect("show should succeed");
    let last = detail
        .last_build
        .expect("last_build should be Some even after a failed build");
    assert_eq!(last.exit_code, 5, "cached failed outcome should preserve exit code");
    assert!(
        last.stderr.contains("oops"),
        "cached stderr should contain 'oops', got: {:?}",
        last.stderr
    );
}

/// BLOCKER regression: handler_build must reject path-traversal names before
/// any filesystem access (manifest read must never touch out-of-workspace paths).
#[test]
fn handler_build_rejects_traversal_name() {
    let tmp = tempfile::Builder::new()
        .prefix("rfs-plan8-build-trav")
        .tempdir()
        .unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core.handler_build("../etc/passwd").unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { ref name } if name.contains("..")),
        "expected InvalidHandlerName for traversal, got: {:?}",
        err
    );
}

/// BLOCKER regression: handler_build must reject absolute paths as names.
#[test]
fn handler_build_rejects_absolute_name() {
    let tmp = tempfile::Builder::new()
        .prefix("rfs-plan8-build-abs")
        .tempdir()
        .unwrap();
    std::fs::create_dir_all(tmp.path().join("handlers")).unwrap();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core.handler_build("/etc/passwd").unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::InvalidHandlerName { .. }),
        "expected InvalidHandlerName for absolute path name, got: {:?}",
        err
    );
}

/// ToolchainMissing does NOT write to the cache; handler_show returns last_build == None.
#[test]
fn handler_build_toolchain_missing_returns_error_without_cache_write() {
    let tmp = empty_workspace();
    write_handler(
        &tmp,
        "no-tool",
        "name: no-tool\nversion: 0.1.0\nentry:\n  cmd: [\"./no-tool\"]\n  build: [\"__rowforge_nonexistent_tool_xyz__\"]\n",
    );

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().into()),
    )
    .unwrap();

    let err = core.handler_build("no-tool").unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::ToolchainMissing { .. }),
        "expected ToolchainMissing, got: {:?}",
        err
    );

    // Cache must remain empty — no outcome to display.
    let detail = core.handler_show("no-tool").expect("show should succeed");
    assert!(
        detail.last_build.is_none(),
        "last_build must be None when ToolchainMissing (no outcome cached)"
    );
}

// ============================================================
// Plan 9 T4 — handler_log_tail + handler_log_subscribe
// ============================================================

#[test]
fn handler_log_tail_returns_empty_when_no_file() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let result = core
        .handler_log_tail("e_nonexistent", "att_x", 100)
        .unwrap();
    assert!(result.is_empty(), "expected empty Vec, got {:?}", result);
}

#[test]
fn handler_log_tail_parses_lines_from_disk() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let attempt_dir = tmp.path().join("executions/e_test/attempts/att_test");
    std::fs::create_dir_all(&attempt_dir).unwrap();
    let log = attempt_dir.join("handler_log.log");
    std::fs::write(
        &log,
        "2026-05-25T10:00:00+00:00 [handler#0 stderr] hello\n\
         2026-05-25T10:00:01+00:00 [handler#1 stdout] garbage\n",
    )
    .unwrap();
    let lines = core
        .handler_log_tail("e_test", "att_test", 100)
        .unwrap();
    assert_eq!(lines.len(), 2, "expected 2 lines, got {}", lines.len());
    assert_eq!(lines[0].line, "hello");
    assert_eq!(lines[1].line, "garbage");
}

#[test]
fn handler_log_tail_caps_to_max_lines() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let attempt_dir = tmp.path().join("executions/e_test/attempts/att_cap");
    std::fs::create_dir_all(&attempt_dir).unwrap();
    let log = attempt_dir.join("handler_log.log");
    let mut content = String::new();
    for i in 0..20u32 {
        content.push_str(&format!(
            "2026-05-25T10:00:{:02}+00:00 [handler#0 stderr] line {}\n",
            i, i,
        ));
    }
    std::fs::write(&log, content).unwrap();
    let lines = core.handler_log_tail("e_test", "att_cap", 5).unwrap();
    assert_eq!(lines.len(), 5, "expected 5 lines, got {}", lines.len());
    // Should be the LAST 5 lines, chronologically.
    assert_eq!(lines[0].line, "line 15");
    assert_eq!(lines[4].line, "line 19");
}

#[test]
fn handler_log_subscribe_fails_for_inactive_attempt() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let result = core.handler_log_subscribe("att_not_running");
    assert!(
        result.is_err(),
        "expected Err for inactive attempt, got Ok"
    );
}

// ---------------------------------------------------------------------------
// Plan 9 review round-1 — ID validation regression tests (BLOCKER fix)
// ---------------------------------------------------------------------------

/// Regression: `../etc` in exec_id must be rejected before any filesystem probe.
#[test]
fn handler_log_tail_rejects_traversal_exec_id() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core.handler_log_tail("../etc", "att_x", 100).unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::Io(_)),
        "expected UiError::Io, got: {:?}", err
    );
}

/// Regression: `../../etc/passwd` in attempt_id must be rejected.
#[test]
fn handler_log_tail_rejects_traversal_attempt_id() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core
        .handler_log_tail("e_test", "../../etc/passwd", 100)
        .unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::Io(_)),
        "expected UiError::Io, got: {:?}", err
    );
}

/// Regression: an absolute path in exec_id (`/etc/passwd`) must be rejected.
#[test]
fn handler_log_tail_rejects_absolute_exec_id() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core
        .handler_log_tail("/etc/passwd", "att_x", 100)
        .unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::Io(_)),
        "expected UiError::Io, got: {:?}", err
    );
}

/// Regression: an empty exec_id must be rejected.
#[test]
fn handler_log_tail_rejects_empty_id() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    let err = core.handler_log_tail("", "att_x", 100).unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::Io(_)),
        "expected UiError::Io, got: {:?}", err
    );
}

// ---------------------------------------------------------------------------
// Plan 9 review round-1 — capture_raw_stdout setter regression test
// ---------------------------------------------------------------------------

/// Regression: set_handler_log_capture_raw_stdout must update the in-memory flag
/// so the next start_run call picks up the new value without a workspace re-open.
#[test]
fn studio_core_capture_raw_stdout_reflects_set_value() {
    let tmp = empty_workspace();
    let mut core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();
    // Default is false (Settings::default).
    assert!(
        !core.capture_raw_stdout(),
        "default capture_raw_stdout should be false"
    );
    core.set_handler_log_capture_raw_stdout(true);
    assert!(
        core.capture_raw_stdout(),
        "capture_raw_stdout should be true after set"
    );
    core.set_handler_log_capture_raw_stdout(false);
    assert!(
        !core.capture_raw_stdout(),
        "capture_raw_stdout should be false after reset"
    );
}

// ---------------------------------------------------------------------------
// Plan 10 T1 — execution_delete
// ---------------------------------------------------------------------------

/// Seed a single execution (with a CSV file) into `tmp`'s workspace and
/// return the exec id string. Mirrors `create_execution_with_csv` but with a
/// unique name so multiple calls within one test don't collide.
fn seed_exec(tmp: &tempfile::TempDir, name: &str) -> String {
    use rowforge_core::execution_store::NewExecution;
    let csv = tmp.path().join(format!("{name}.csv"));
    std::fs::write(&csv, "id\nr1\nr2\n").unwrap();
    let mut store = rowforge_core::execution_store::ExecutionStore::open(tmp.path()).unwrap();
    store
        .create_execution(NewExecution {
            name: Some(name.into()),
            input_csv_id: "csv1".into(),
            input_csv_path: csv,
            current_handler_instance_id: None,
        })
        .unwrap()
        .id
}

/// Happy-path: delete removes the sqlite row, child attempts, and the
/// on-disk execution directory.
#[test]
fn execution_delete_removes_row_attempts_and_dir() {
    let tmp = empty_workspace();
    let exec_id = seed_exec(&tmp, "exec-delete-happy");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    // The exec dir is created by ExecutionStore::create_execution.
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    assert!(exec_dir.exists(), "exec dir must exist before delete");

    core.execution_delete(&exec_id).expect("delete should succeed");

    // Sqlite row should be gone — show() returns NotFound.
    let id = rowforge_studio_core::ExecutionId::new(exec_id.clone());
    assert!(
        matches!(core.show(&id), Err(rowforge_studio_core::UiError::NotFound(_))),
        "expected NotFound after delete"
    );
    // On-disk dir should be gone.
    assert!(!exec_dir.exists(), "exec dir should have been removed");
}

/// Active-run gate: execution_delete must refuse when a session is registered
/// for that exec_id, returning UiError::ExecutionInUse.
#[test]
fn execution_delete_refuses_when_active_run() {
    let tmp = empty_workspace();
    let exec_id = seed_exec(&tmp, "exec-delete-active");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Inject a fake session via the test helper on SessionRegistry.
    // Session is #[non_exhaustive] so we cannot construct it cross-crate.
    core.sessions().register_fake_session_for_test(&exec_id);

    let err = core.execution_delete(&exec_id).unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::ExecutionInUse { .. }),
        "expected ExecutionInUse, got: {:?}", err
    );
}

/// Idempotent NotFound: deleting an execution that doesn't exist returns
/// UiError::NotFound (not a panic or internal error).
#[test]
fn execution_delete_idempotent_returns_not_found() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    let err = core.execution_delete("e_nonexistent").unwrap_err();
    assert!(
        matches!(err, rowforge_studio_core::UiError::NotFound(_)),
        "expected NotFound, got: {:?}", err
    );
}

/// Traversal rejection: IDs containing `..` or `/` must be rejected with
/// UiError::Io before any sqlite or fs access.
#[test]
fn execution_delete_rejects_traversal_id() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    for bad_id in &["../etc", "../../etc/passwd", "/etc/passwd", ""] {
        let err = core.execution_delete(bad_id).unwrap_err();
        assert!(
            matches!(err, rowforge_studio_core::UiError::Io(_)),
            "expected UiError::Io for id {:?}, got: {:?}", bad_id, err
        );
    }
}

/// Dir-already-missing: if the on-disk dir is gone before deletion (e.g.
/// externally rm'd), execution_delete should still succeed — sqlite is
/// authoritative.
#[test]
fn execution_delete_succeeds_when_dir_already_missing() {
    let tmp = empty_workspace();
    let exec_id = seed_exec(&tmp, "exec-delete-dir-missing");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Externally remove the directory before calling delete.
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    std::fs::remove_dir_all(&exec_dir).unwrap();
    assert!(!exec_dir.exists());

    // Delete should succeed (sqlite cascade works; missing dir is tolerated).
    core.execution_delete(&exec_id).expect("delete should succeed even if dir missing");

    // Sqlite row must be gone.
    let id = rowforge_studio_core::ExecutionId::new(exec_id.clone());
    assert!(
        matches!(core.show(&id), Err(rowforge_studio_core::UiError::NotFound(_))),
        "expected NotFound after delete"
    );
}

// ---------------------------------------------------------------------------
// Plan 10 T2 — execution_delete_bulk
// ---------------------------------------------------------------------------

/// All-succeed: bulk-delete two executions with no active runs.
/// Both ids appear in `deleted`; `failed` is empty.
#[test]
fn execution_delete_bulk_all_succeed() {
    let tmp = empty_workspace();
    let id_a = seed_exec(&tmp, "bulk-all-succeed-a");
    let id_b = seed_exec(&tmp, "bulk-all-succeed-b");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    let result = core.execution_delete_bulk(&[id_a.clone(), id_b.clone()]);

    assert_eq!(result.deleted.len(), 2, "expected 2 deleted, got: {:?}", result.deleted);
    assert!(result.failed.is_empty(), "expected no failures, got: {:?}", result.failed);
}

/// Partial failure: one exec has an active run and must end up in `failed[]`
/// with a reason containing "active run"; the other succeeds in `deleted[]`.
#[test]
fn execution_delete_bulk_partial_failure() {
    let tmp = empty_workspace();
    let id_a = seed_exec(&tmp, "bulk-partial-a");
    let id_b = seed_exec(&tmp, "bulk-partial-b");

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Mark id_a as having an active run so execution_delete refuses it.
    core.sessions().register_fake_session_for_test(&id_a);

    let result = core.execution_delete_bulk(&[id_a.clone(), id_b.clone()]);

    // id_b is the one that was actually deleted (id_a had an active run).
    assert_eq!(result.deleted, vec![id_b.clone()],
        "expected only id_b in deleted, got: {:?}", result.deleted);
    assert_eq!(result.failed.len(), 1,
        "expected 1 failure, got: {:?}", result.failed);
    assert_eq!(result.failed[0].exec_id, id_a,
        "failure exec_id mismatch");
    assert!(
        result.failed[0].reason.contains("active run"),
        "expected 'active run' in reason, got: {:?}", result.failed[0].reason
    );
}

/// Empty input: an empty slice produces an empty result (no panic, no error).
#[test]
fn execution_delete_bulk_empty_input_returns_empty_result() {
    let tmp = empty_workspace();
    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    let result = core.execution_delete_bulk(&[]);

    assert!(result.deleted.is_empty(), "expected no deleted, got: {:?}", result.deleted);
    assert!(result.failed.is_empty(), "expected no failed, got: {:?}", result.failed);
}

// ---------------------------------------------------------------------------
// T3 — ExecSummary.size_bytes
// ---------------------------------------------------------------------------

/// `exec_list` populates `size_bytes >= 1024` for an execution whose
/// on-disk directory contains a 1 KB file.
#[test]
fn exec_list_includes_size_bytes() {
    let tmp = empty_workspace();
    let exec_id = seed_exec(&tmp, "size-bytes-happy");

    // The exec dir is created by seed_exec → create_execution.
    // Write a 1 KB file into it so we have a measurable size.
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    std::fs::create_dir_all(&exec_dir).unwrap();
    std::fs::write(exec_dir.join("data.bin"), vec![0u8; 1024]).unwrap();

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    let summaries = core.list(rowforge_studio_core::ListFilter::default()).unwrap();
    let summary = summaries
        .iter()
        .find(|s| s.id.as_str() == exec_id)
        .expect("exec not found in list");

    assert!(
        summary.size_bytes.is_some(),
        "size_bytes should be Some when dir exists"
    );
    assert!(
        summary.size_bytes.unwrap() >= 1024,
        "size_bytes should be >= 1024, got {:?}",
        summary.size_bytes
    );
}

/// `exec_list` returns `size_bytes: None` for an execution whose directory
/// has been removed externally.
#[test]
fn exec_list_size_bytes_none_when_dir_missing() {
    let tmp = empty_workspace();
    let exec_id = seed_exec(&tmp, "size-bytes-dir-missing");

    // Remove the execution directory to simulate external deletion.
    let exec_dir = tmp.path().join("executions").join(&exec_id);
    if exec_dir.exists() {
        std::fs::remove_dir_all(&exec_dir).unwrap();
    }

    let core = rowforge_studio_core::StudioCore::open(
        rowforge_studio_core::OpenOpts::new().with_workspace(tmp.path().to_path_buf()),
    )
    .unwrap();

    let summaries = core.list(rowforge_studio_core::ListFilter::default()).unwrap();
    let summary = summaries
        .iter()
        .find(|s| s.id.as_str() == exec_id)
        .expect("exec not found in list");

    assert!(
        summary.size_bytes.is_none(),
        "size_bytes should be None when dir is missing, got {:?}",
        summary.size_bytes
    );
}
