use crate::pool::RowOutcome;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct RunMeta {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub input_path: String,
    pub input_row_count: u64,
    pub handler: HandlerMeta,
    pub config: BTreeMap<String, serde_json::Value>,
    pub stats: Stats,
    pub dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandlerMeta {
    pub name: String,
    pub version: String,
    pub manifest_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Stats {
    pub success: u64,
    pub failed: u64,
    pub by_error_code: BTreeMap<String, u64>,
    pub avg_dur_ms: u64,
}

pub fn manifest_hash(manifest_yaml_bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(manifest_yaml_bytes);
    let digest = h.finalize();
    format!("sha256:{:x}", digest)
}

pub fn build_stats(outcomes: &[RowOutcome]) -> Stats {
    let mut by_code: BTreeMap<String, u64> = BTreeMap::new();
    let mut succ = 0u64;
    let mut fail = 0u64;
    let mut total_dur = 0u128;
    let mut dur_count = 0u128;
    for o in outcomes {
        match o {
            RowOutcome::Success { dur_ms, .. } => {
                succ += 1;
                total_dur += *dur_ms as u128;
                dur_count += 1;
            }
            RowOutcome::Error { code, dur_ms, .. } => {
                fail += 1;
                *by_code.entry(code.clone()).or_insert(0) += 1;
                total_dur += *dur_ms as u128;
                dur_count += 1;
            }
            RowOutcome::Crash { .. } => {
                fail += 1;
                *by_code.entry(crate::run::ERR_WORKER_CRASH.into()).or_insert(0) += 1;
            }
        }
    }
    let avg = if dur_count == 0 {
        0
    } else {
        (total_dur / dur_count) as u64
    };
    Stats {
        success: succ,
        failed: fail,
        by_error_code: by_code,
        avg_dur_ms: avg,
    }
}

pub fn write_meta(path: &Path, meta: &RunMeta) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_hash_is_stable() {
        let h1 = manifest_hash(b"name: x\n");
        let h2 = manifest_hash(b"name: x\n");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn stats_counts_categories() {
        let out = vec![
            RowOutcome::Success {
                seq: 0,
                data: serde_json::Map::new(),
                dur_ms: 100,
            },
            RowOutcome::Success {
                seq: 1,
                data: serde_json::Map::new(),
                dur_ms: 200,
            },
            RowOutcome::Error {
                seq: 2,
                code: "X".into(),
                message: "m".into(),
                dur_ms: 50,
                data: None,
            },
            RowOutcome::Crash {
                seq: 3,
                worker_id: 0,
                crash_at_seq: 3,
            },
        ];
        let s = build_stats(&out);
        assert_eq!(s.success, 2);
        assert_eq!(s.failed, 2);
        assert_eq!(s.by_error_code.get("X"), Some(&1));
        assert_eq!(s.by_error_code.get("WORKER_CRASH"), Some(&1));
        // (100+200+50)/3 = 116
        assert_eq!(s.avg_dur_ms, 116);
    }
}
