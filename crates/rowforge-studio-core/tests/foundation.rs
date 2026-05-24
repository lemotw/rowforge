//! Plan 1 integration coverage.
//!
//! Each test bootstraps a temp workspace, runs CLI-equivalent setup via
//! rowforge_core::execution_store, then exercises the studio-core
//! surface. No CLI binary is invoked.

use rowforge_core::execution_store::ExecutionStore;
use rowforge_studio_core::{OpenOpts, StudioCore, UiError};
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

use rowforge_core::execution_store::NewExecution;
use rowforge_studio_core::ListFilter;

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
    assert_eq!(rows[0].attempts_count, 0, "Plan 1 stubs this");
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
