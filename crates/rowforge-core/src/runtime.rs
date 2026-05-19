//! Manifest runtime block: declares consumption mode (row | batch) and constraints.
//!
//! See spec at docs/superpowers/specs/2026-05-12-batch-mode-design.md

use serde::{Deserialize, Serialize};

const MIN_BATCH_SIZE: u32 = 1;
const MAX_BATCH_SIZE: u32 = 10_000;
const DEFAULT_MAX_BATCH_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB
pub const ROW_HARD_CAP_BYTES: u64 = 4 * 1024 * 1024;   // 4 MiB

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Row,
    Batch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Runtime {
    pub mode: Mode,

    /// Required if mode = batch. Number of rows per batch envelope.
    #[serde(default)]
    pub batch_size: Option<u32>,

    /// Optional cap on serialized JSON size of one batch envelope.
    #[serde(default = "default_max_batch_bytes")]
    pub max_batch_bytes: u64,

    /// Soft byte cap per batch. When the accumulated serialized size of the
    /// pending rows would exceed this value, the accumulator flushes the current
    /// batch early and logs a warning. Defaults to 4 MiB. Must be ≤
    /// `max_batch_bytes` to be meaningful, but the accumulator enforces
    /// `max_batch_bytes` hard cap first regardless.
    #[serde(default = "default_batch_bytes_target")]
    pub batch_bytes_target: u64,

    /// Required if mode = batch. Declares whether the handler is safe to
    /// re-run after crash. No default.
    #[serde(default)]
    pub idempotent: Option<bool>,

    /// Output depends on cross-batch state. If true, pool forces workers = 1.
    #[serde(default)]
    pub stateful: bool,
}

fn default_max_batch_bytes() -> u64 { DEFAULT_MAX_BATCH_BYTES }

const DEFAULT_BATCH_BYTES_TARGET: u64 = 4 * 1024 * 1024; // 4 MiB
fn default_batch_bytes_target() -> u64 { DEFAULT_BATCH_BYTES_TARGET }

impl Default for Runtime {
    fn default() -> Self {
        Self {
            mode: Mode::Row,
            batch_size: None,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            batch_bytes_target: DEFAULT_BATCH_BYTES_TARGET,
            idempotent: None,
            stateful: false,
        }
    }
}

impl Runtime {
    /// Validates the runtime block after deserialization. Returns Err with
    /// human-readable message on contract violation.
    pub fn validate(&self) -> Result<(), String> {
        match self.mode {
            Mode::Row => {
                // Row mode: other fields are ignored but tolerated for forward compat.
                Ok(())
            }
            Mode::Batch => {
                let bs = self.batch_size.ok_or_else(|| {
                    "runtime.batch_size required when mode=batch".to_string()
                })?;
                if !(MIN_BATCH_SIZE..=MAX_BATCH_SIZE).contains(&bs) {
                    return Err(format!(
                        "runtime.batch_size must be in {}..={}, got {}",
                        MIN_BATCH_SIZE, MAX_BATCH_SIZE, bs
                    ));
                }
                self.idempotent.ok_or_else(|| {
                    "runtime.idempotent required when mode=batch (true or false; no default)"
                        .to_string()
                })?;
                if self.max_batch_bytes == 0 {
                    return Err("runtime.max_batch_bytes must be > 0".into());
                }
                if self.max_batch_bytes < ROW_HARD_CAP_BYTES {
                    return Err(format!(
                        "runtime.max_batch_bytes ({}) must be >= per-row hard cap ({})",
                        self.max_batch_bytes, ROW_HARD_CAP_BYTES
                    ));
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_mode_default_validates() {
        let r = Runtime::default();
        assert_eq!(r.mode, Mode::Row);
        r.validate().unwrap();
    }

    #[test]
    fn batch_mode_requires_batch_size() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nidempotent: true\n",
        ).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("batch_size required"), "got: {}", err);
    }

    #[test]
    fn batch_mode_requires_idempotent() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 100\n",
        ).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("idempotent required"), "got: {}", err);
    }

    #[test]
    fn batch_mode_rejects_zero_size() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 0\nidempotent: true\n",
        ).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("must be in"), "got: {}", err);
    }

    #[test]
    fn batch_mode_rejects_oversize() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 99999\nidempotent: true\n",
        ).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("must be in"), "got: {}", err);
    }

    #[test]
    fn batch_mode_rejects_byte_cap_below_row_cap() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 10\nidempotent: true\nmax_batch_bytes: 1024\n",
        ).unwrap();
        let err = r.validate().unwrap_err();
        assert!(err.contains("max_batch_bytes"), "got: {}", err);
    }

    #[test]
    fn batch_mode_valid_full_block() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 100\nidempotent: true\nmax_batch_bytes: 33554432\nstateful: true\n",
        ).unwrap();
        r.validate().unwrap();
        assert_eq!(r.mode, Mode::Batch);
        assert_eq!(r.batch_size, Some(100));
        assert_eq!(r.idempotent, Some(true));
        assert!(r.stateful);
    }

    #[test]
    fn batch_mode_defaults_max_bytes_to_16mib() {
        let r: Runtime = serde_yaml::from_str(
            "mode: batch\nbatch_size: 100\nidempotent: true\n",
        ).unwrap();
        r.validate().unwrap();
        assert_eq!(r.max_batch_bytes, 16 * 1024 * 1024);
    }
}
