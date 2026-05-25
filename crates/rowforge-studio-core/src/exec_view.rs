//! ExecSummary projection from the on-disk store.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.2.2.
//!
//! Plan 3: attempt fields are now backfilled by joining the attempts
//! table and reading the latest attempt's meta.json for counts.

use chrono::{DateTime, Utc};
use rowforge_core::error::CoreError;
use rowforge_core::execution_store::{Execution, ExecutionStore};
use serde::{Deserialize, Serialize};

use crate::ids::ExecutionId;

/// Filter passed to `list`. Reserved for future use; Plan 1 has no
/// filter knobs.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ListFilter;

/// Light-weight projection for the exec list page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecSummary {
    pub id: ExecutionId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub input_rows: Option<u64>,

    pub attempts_count: u32,
    pub last_attempt_state: Option<String>,
    pub last_attempt_counts: Option<AttemptCountsStub>,
    pub last_handler_dir: Option<std::path::PathBuf>,

    /// Total on-disk size of the execution directory, in bytes.
    ///
    /// Populated lazily by `exec_list`; `None` when the directory is absent
    /// (e.g. the exec was deleted externally or size was not computed yet).
    pub size_bytes: Option<u64>,
}

/// Placeholder for Plan 3's full `AttemptCounts`. Kept as its own type
/// so the public field above does not change shape later.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AttemptCountsStub {
    pub success: u64,
    pub failed: u64,
    pub crashed: u64,
}

impl ExecSummary {
    /// Build an `ExecSummary` by joining attempts from the store.
    ///
    /// Reads `<exec.dir>/attempts/<attempt_id>/meta.json` for the last
    /// attempt's counts (best-effort; None when the file is absent or
    /// malformed).
    pub fn from_execution(
        e: &Execution,
        store: &ExecutionStore,
    ) -> Result<Self, CoreError> {
        let attempts = store.list_attempts_for_execution(&e.id)?;
        let last = attempts.last();

        // AttemptState::as_str is not pub; use serde to get the snake_case string.
        let last_attempt_state = last.map(|att| {
            serde_json::to_value(&att.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| format!("{:?}", att.state).to_lowercase())
        });

        let last_attempt_counts = last.and_then(|att| {
            // attempt_dir layout: <exec.dir>/attempts/<attempt_id>
            // (mirrors ExecutionStore::attempt_dir)
            let meta_path = e.dir.join("attempts").join(&att.id).join("meta.json");
            read_meta_counts(&meta_path)
        });

        Ok(ExecSummary {
            id: ExecutionId::new(e.id.clone()),
            name: e.name.clone().unwrap_or_default(),
            created_at: e.created_at,
            input_rows: Some(e.input_row_count),
            attempts_count: attempts.len() as u32,
            last_attempt_state,
            last_attempt_counts,
            last_handler_dir: e.last_handler_dir.clone(),
            size_bytes: None,
        })
    }
}

/// Walk `dir` and sum all regular-file sizes.
///
/// Returns `None` if `dir` does not exist; `Some(0)` for an existing but
/// empty directory. Uses `saturating_add` to avoid overflow on pathological
/// inputs.
pub(crate) fn dir_size_bytes(dir: &std::path::Path) -> Option<u64> {
    if !dir.exists() {
        return None;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}

fn read_meta_counts(path: &std::path::Path) -> Option<AttemptCountsStub> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let stats = v.get("stats")?;
    Some(AttemptCountsStub {
        success: stats.get("success")?.as_u64()?,
        failed: stats.get("failed")?.as_u64()?,
        crashed: stats.get("crashed")?.as_u64()?,
    })
}
