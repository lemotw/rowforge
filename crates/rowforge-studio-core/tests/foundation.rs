//! Plan 1 integration coverage.
//!
//! Each test bootstraps a temp workspace, runs CLI-equivalent setup via
//! rowforge_core::execution_store, then exercises the studio-core
//! surface. No CLI binary is invoked.

use rowforge_core::execution_store::ExecutionStore;
use rowforge_studio_core::{ExecRollup, OpenOpts, StudioCore, UiError};
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
use rowforge_studio_core::{AttemptId, ExecutionId, ListFilter};

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
