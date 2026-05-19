use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Messages App → Handler (sent on handler stdin, one per line).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    Init {
        run_id: String,
        config: BTreeMap<String, serde_json::Value>,
        meta: InitMeta,
    },
    Row {
        seq: u64,
        data: serde_json::Map<String, serde_json::Value>,
        meta: RowMeta,
    },
    /// Batch envelope (mode: batch). Carries 1..=batch_size rows with explicit
    /// `seq` per row. The handler's `batch_result` reply attributes entries
    /// positionally — see `BatchEntry`.
    Batch {
        rows: Vec<RowEnvelope>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InitMeta {
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RowMeta {
    pub dry_run: bool,
    pub row_index: u64,
}

/// One row inside a `Batch` envelope. Mirrors the `Outbound::Row` fields so
/// row mode and batch mode share the same per-row shape on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RowEnvelope {
    pub seq: u64,
    pub data: serde_json::Map<String, serde_json::Value>,
    pub meta: RowMeta,
}

/// Messages Handler → App (read from handler stdout, one per line).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    Ready {
        handler_version: String,
    },
    Result {
        seq: u64,
        data: serde_json::Map<String, serde_json::Value>,
    },
    Error {
        seq: u64,
        code: String,
        message: String,
        /// Optional handler-supplied domain payload. This is the handler's
        /// domain-specific context; `exec export` (P10) discovers which keys
        /// are present by scanning `outcomes.jsonl`. Other keys are accepted
        /// and preserved. Backward-compat: omitting `data` parses as `None`
        /// and round-trips without emitting a `data: null` field on the wire
        /// (older handlers that don't know about this field stay
        /// byte-identical).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Map<String, serde_json::Value>>,
    },
    /// Reply to a `Batch` envelope. Entries are positionally aligned with the
    /// `rows` of the corresponding `Batch` — `results[i]` is the outcome for
    /// `rows[i]`. Length must equal the batch size; entries carry no `seq`
    /// field (bijection-by-construction, spec §6.5 C1).
    BatchResult {
        results: Vec<BatchEntry>,
    },
}

/// Per-row entry inside a `batch_result` envelope. Position in the parent
/// `results` array determines attribution — entries MUST NOT carry a `seq`
/// field on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BatchEntry {
    Result {
        data: serde_json::Map<String, serde_json::Value>,
    },
    Error {
        code: String,
        message: String,
        /// Optional handler-supplied domain payload; see `Inbound::Error::data`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Map<String, serde_json::Value>>,
    },
}

impl Outbound {
    pub fn to_jsonl(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Outbound serializes");
        s.push('\n');
        s
    }
}

impl Inbound {
    pub fn from_jsonl_line(line: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(line.trim_end_matches('\n'))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn init_roundtrip() {
        let m = Outbound::Init {
            run_id: "42".into(),
            config: BTreeMap::from([("timeout_ms".into(), json!(5000))]),
            meta: InitMeta {
                columns: vec!["email".into()],
            },
        };
        let s = m.to_jsonl();
        assert!(s.ends_with('\n'));
        let parsed: Outbound = serde_json::from_str(s.trim_end()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn row_message_serializes_with_seq() {
        let m = Outbound::Row {
            seq: 7,
            data: serde_json::Map::from_iter([("email".to_string(), json!("a@x.com"))]),
            meta: RowMeta {
                dry_run: false,
                row_index: 7,
            },
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""type":"row""#));
        assert!(s.contains(r#""seq":7"#));
    }

    #[test]
    fn inbound_result_parses() {
        let line = r#"{"type":"result","seq":3,"data":{"is_valid":true}}"#;
        let m = Inbound::from_jsonl_line(line).unwrap();
        match m {
            Inbound::Result { seq, data } => {
                assert_eq!(seq, 3);
                assert_eq!(data.get("is_valid").unwrap(), &json!(true));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    #[test]
    fn inbound_error_parses_without_retriable() {
        // spec §6.3: error message has NO retriable field (retries removed)
        let line = r#"{"type":"error","seq":3,"code":"DNS_TIMEOUT","message":"no MX"}"#;
        let m = Inbound::from_jsonl_line(line).unwrap();
        match m {
            Inbound::Error { seq, code, message, data } => {
                assert_eq!(seq, 3);
                assert_eq!(code, "DNS_TIMEOUT");
                assert_eq!(message, "no MX");
                // Backward-compat: old-shape errors omit `data` → None.
                assert!(data.is_none());
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn inbound_error_with_data_roundtrips() {
        // New-shape: handler attaches domain context via `data`.
        let line = r#"{"type":"error","seq":3,"code":"INVALID","message":"x","data":{"billid":"B123"}}"#;
        let m = Inbound::from_jsonl_line(line).unwrap();
        match &m {
            Inbound::Error { data: Some(d), .. } => {
                assert_eq!(d.get("billid").unwrap(), &json!("B123"));
            }
            other => panic!("expected Error w/ data, got {:?}", other),
        }
        // Re-serialize and confirm the `data` field stays on the wire.
        let back = serde_json::to_string(&m).unwrap();
        assert!(back.contains(r#""data":{"billid":"B123"}"#), "got: {}", back);
    }

    #[test]
    fn inbound_error_without_data_does_not_emit_data_field() {
        // skip_serializing_if guarantees `data: None` does NOT emit a
        // `data: null` on the wire — old handlers stay byte-identical.
        let m = Inbound::Error {
            seq: 1,
            code: "X".into(),
            message: "y".into(),
            data: None,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(!s.contains("data"), "expected no data field, got: {}", s);
    }

    #[test]
    fn batch_entry_error_with_data_roundtrips() {
        // Same shape rules apply to batch_result entries: `data` is optional.
        let entry = BatchEntry::Error {
            code: "FOO".into(),
            message: "bar".into(),
            data: Some(serde_json::Map::from_iter([("k".to_string(), json!("v"))])),
        };
        let s = serde_json::to_string(&entry).unwrap();
        assert!(s.contains(r#""data":{"k":"v"}"#));
        let parsed: BatchEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn batch_entry_error_without_data_omits_field() {
        let entry = BatchEntry::Error {
            code: "FOO".into(),
            message: "bar".into(),
            data: None,
        };
        let s = serde_json::to_string(&entry).unwrap();
        assert!(!s.contains("data"), "expected no data field, got: {}", s);
        // And old-shape input still parses.
        let parsed: BatchEntry =
            serde_json::from_str(r#"{"kind":"error","code":"FOO","message":"bar"}"#).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn inbound_extra_fields_ignored() {
        // forward compat: handler may send extra fields, App ignores
        let line = r#"{"type":"ready","handler_version":"0.3.1","extra":"ok"}"#;
        let m = Inbound::from_jsonl_line(line).unwrap();
        assert!(matches!(m, Inbound::Ready { .. }));
    }

    #[test]
    fn batch_envelope_round_trip() {
        // App → Handler `batch` envelope carries N rows, each with its own seq.
        let batch = Outbound::Batch {
            rows: vec![
                RowEnvelope {
                    seq: 0,
                    data: serde_json::json!({"x":1}).as_object().unwrap().clone(),
                    meta: RowMeta::default(),
                },
                RowEnvelope {
                    seq: 1,
                    data: serde_json::json!({"x":2}).as_object().unwrap().clone(),
                    meta: RowMeta::default(),
                },
            ],
        };
        let json = serde_json::to_string(&batch).unwrap();
        assert!(json.contains("\"type\":\"batch\""));
        let parsed: Outbound = serde_json::from_str(&json).unwrap();
        match parsed {
            Outbound::Batch { rows } => assert_eq!(rows.len(), 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn batch_result_round_trip_no_seq() {
        // Handler → App `batch_result` entries are positional — must not carry seq.
        let result = Inbound::BatchResult {
            results: vec![
                BatchEntry::Result {
                    data: serde_json::json!({"y": 10}).as_object().unwrap().clone(),
                },
                BatchEntry::Error {
                    code: "FOO".into(),
                    message: "bar".into(),
                    data: None,
                },
            ],
        };
        let json = serde_json::to_string(&result).unwrap();
        // Critical: no seq leak in entries
        assert!(
            !json.contains("\"seq\""),
            "batch_result entries must not carry seq: {}",
            json
        );
        let parsed: Inbound = serde_json::from_str(&json).unwrap();
        match parsed {
            Inbound::BatchResult { results } => {
                assert_eq!(results.len(), 2);
                match &results[0] {
                    BatchEntry::Result { data } => assert_eq!(data["y"], 10),
                    _ => panic!("expected Result"),
                }
            }
            _ => panic!("wrong variant"),
        }
    }
}
