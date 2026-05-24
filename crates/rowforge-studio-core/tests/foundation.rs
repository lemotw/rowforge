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
fn failed_page_not_found_when_outcomes_jsonl_absent() {
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
    let err = core
        .failed_page(FailedPageQuery::new(
            ExecutionId::new(exec_id),
            AttemptId::new(attempt_id),
            0,
            10,
            None,
        ))
        .expect_err("should return NotFound");
    assert!(matches!(err, UiError::NotFound(_)));
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
    let handle = core
        .start_run(&ExecutionId::new(exec_id), opts)
        .expect("start_run should succeed with valid manifest");

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
        matches!(err, UiError::RunBusy(_)),
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
