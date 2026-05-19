use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use crate::runtime::{Mode, Runtime};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub language: String,
    pub entry: Entry,
    /// Columns that must be present in the input CSV/JSONL. If any are
    /// missing the run fails immediately with MISSING_REQUIRED_INPUT_COLUMN.
    /// Replaces the old `schema.input.<key>.required` pattern (v3.3 P1).
    #[serde(default)]
    pub required_input: Vec<String>,
    #[serde(default)]
    pub config: BTreeMap<String, ConfigField>,
    #[serde(default)]
    pub runtime: Option<Runtime>,
    #[serde(default)]
    pub output: Option<Output>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub cmd: Vec<String>,
    #[serde(default)]
    pub build: Option<Vec<String>>,
    #[serde(default = "default_cwd")]
    pub cwd: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
}

fn default_cwd() -> String {
    ".".to_string()
}
fn default_startup_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigField {
    #[serde(default)]
    pub default: serde_json::Value,
}

/// Optional `output:` block on the manifest. Controls how rowforge writes
/// success.csv / failed.csv. Absence is treated as the default (no meta
/// columns).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Output {
    /// When true, append observability columns (meta_dur_ms /
    /// meta_handler_ver on success; meta_handler_ver / meta_crash_at_seq /
    /// meta_crash_worker_id on failed) to each output CSV.
    #[serde(default)]
    pub include_meta: bool,
}

impl Manifest {
    pub fn load_from_dir(dir: &Path) -> anyhow::Result<(Self, PathBuf)> {
        let path = dir.join("rowforge.yaml");
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
        let m: Manifest = serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;
        if let Some(rt) = &m.runtime {
            rt.validate().map_err(|e| anyhow::anyhow!("manifest.runtime: {}", e))?;
        }
        Ok((m, path))
    }

    /// Convenience: whether the manifest opts in to writing meta columns
    /// in success.csv / failed.csv. Missing `output:` block ≡ false.
    pub fn include_meta(&self) -> bool {
        self.output.as_ref().map(|o| o.include_meta).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_manifest() {
        // Legacy YAML with `schema:` block — serde ignores unknown fields,
        // so this loads cleanly (acceptance #11).
        let yaml = r#"
name: enrich-email
version: 0.1.0
entry:
  cmd: ["./bin/enrich-email"]
schema:
  input:
    email: { type: string, required: true }
  output:
    is_valid: { type: bool }
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.name, "enrich-email");
        assert_eq!(m.entry.cmd, vec!["./bin/enrich-email"]);
        assert_eq!(m.entry.startup_timeout_ms, 30_000);
        assert_eq!(m.entry.cwd, ".");
        // Missing `output:` block ≡ include_meta = false.
        assert!(!m.include_meta());
        assert!(m.output.is_none());
        // Legacy `schema:` block is silently ignored; required_input defaults empty.
        assert!(m.required_input.is_empty());
    }

    #[test]
    fn missing_required_top_level_field_errors() {
        let yaml = r#"
name: x
# missing version
entry:
  cmd: ["./x"]
"#;
        let res: Result<Manifest, _> = serde_yaml::from_str(yaml);
        assert!(res.is_err(), "expected error, got {:?}", res);
    }

    #[test]
    fn entry_defaults_apply() {
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ["./x"]
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.entry.startup_timeout_ms, 30_000);
        assert_eq!(m.entry.cwd, ".");
        assert!(m.entry.env.is_empty());
        assert!(m.entry.build.is_none());
    }

    #[test]
    fn full_manifest_with_config_parses() {
        // Legacy YAML: schema block is present but ignored (acceptance #11).
        let yaml = r#"
name: enrich-email
version: 0.3.1
description: "Validate email + lookup MX"
language: rust
entry:
  cmd: ["./target/release/enrich-email"]
  build: ["cargo", "build", "--release"]
  cwd: .
  env:
    RUST_LOG: info
  startup_timeout_ms: 60000
required_input: [email]
schema:
  input:
    email: { type: string, required: true }
  output:
    is_valid: { type: bool }
    domain:   { type: string }
  errors: [INVALID_FORMAT, DNS_TIMEOUT]
config:
  timeout_ms: { default: 5000 }
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.entry.startup_timeout_ms, 60_000);
        assert_eq!(m.entry.env.get("RUST_LOG").unwrap(), "info");
        assert_eq!(m.required_input, vec!["email".to_string()]);
        assert_eq!(
            m.config.get("timeout_ms").unwrap().default,
            serde_json::json!(5000)
        );
    }

    #[test]
    fn load_from_dir_works() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rowforge.yaml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "name: x\nversion: 0.1.0\nentry:\n  cmd: ['./x']\n").unwrap();
        let (m, p) = Manifest::load_from_dir(dir.path()).unwrap();
        assert_eq!(m.name, "x");
        assert_eq!(p, path);
    }

    #[test]
    fn manifest_loads_runtime_block() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
runtime:
  mode: batch
  batch_size: 100
  idempotent: true
"#;
        std::fs::write(dir.path().join("rowforge.yaml"), yaml).unwrap();
        let (m, _) = Manifest::load_from_dir(dir.path()).unwrap();
        let rt = m.runtime.unwrap();
        assert_eq!(rt.mode, crate::runtime::Mode::Batch);
        assert_eq!(rt.batch_size, Some(100));
    }

    #[test]
    fn manifest_rejects_invalid_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
runtime:
  mode: batch
  batch_size: 100
"#; // missing idempotent
        std::fs::write(dir.path().join("rowforge.yaml"), yaml).unwrap();
        let err = Manifest::load_from_dir(dir.path()).unwrap_err();
        assert!(format!("{}", err).contains("idempotent required"));
    }

    #[test]
    fn manifest_parses_output_include_meta_block() {
        // `output.include_meta: true` toggles the meta-column suffix.
        // Absence ≡ false; presence must round-trip through serde correctly.
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
output:
  include_meta: true
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert!(m.include_meta(), "include_meta should be true");
        let out = m.output.unwrap();
        assert!(out.include_meta);
    }

    #[test]
    fn manifest_output_block_defaults_when_empty() {
        // An empty `output:` block must default include_meta to false.
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
output: {}
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert!(!m.include_meta());
        assert!(!m.output.unwrap().include_meta);
    }

    /// Acceptance #11: legacy manifest with `schema:` block (including
    /// `schema.output`, `schema.failed_output`, etc.) loads without error.
    /// The schema data is silently dropped; `required_input` stays empty.
    #[test]
    fn manifest_legacy_schema_block_ignored() {
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
schema:
  input:
    billid: { required: true }
  output:
    billid: {}
  failed_output:
    billid: {}
    reason_code: { type: string }
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        // Core fields round-trip correctly.
        assert_eq!(m.name, "x");
        assert_eq!(m.version, "0.1.0");
        // Legacy schema data is silently discarded.
        assert!(m.required_input.is_empty());
    }

    #[test]
    fn manifest_required_input_parses() {
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
required_input: [billid, contact_id]
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            m.required_input,
            vec!["billid".to_string(), "contact_id".to_string()]
        );
    }

    #[test]
    fn manifest_required_input_default_empty() {
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert!(m.required_input.is_empty());
    }

    #[test]
    fn manifest_config_field_no_type() {
        // ConfigField no longer has a `type` field — YAML with or without
        // `type:` must parse (unknown fields ignored).
        let yaml = r#"
name: x
version: 0.1.0
entry:
  cmd: ['./x']
config:
  timeout_ms: { default: 3000 }
  mode: {}
"#;
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            m.config.get("timeout_ms").unwrap().default,
            serde_json::json!(3000)
        );
        assert_eq!(
            m.config.get("mode").unwrap().default,
            serde_json::Value::Null
        );
    }
}
