//! Input format detection and streaming iterators for CSV and JSONL input files.
//!
//! # Decision notes (from plan §4)
//! - D4: `.csv → Csv`, `.jsonl | .ndjson → Jsonl`; other extensions need explicit `--format`.
//! - D5: required-input check on JSONL sniffs ONLY the first row; subsequent rows missing a key
//!   is the handler's responsibility.
//! - CSV values are always strings (no type coercion).

use crate::error::CoreError;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ---------------------------------------------------------------------------
// InputFormat
// ---------------------------------------------------------------------------

/// Supported input file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Csv,
    Jsonl,
}

impl InputFormat {
    /// Detect format from path extension; `explicit` override wins.
    ///
    /// Returns `CoreError::Store` with a helpful message when the extension is
    /// unknown and no explicit override was given (Decision D4).
    pub fn detect(path: &Path, explicit: Option<InputFormat>) -> Result<Self, CoreError> {
        if let Some(f) = explicit {
            return Ok(f);
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("csv") => Ok(InputFormat::Csv),
            Some("jsonl") | Some("ndjson") => Ok(InputFormat::Jsonl),
            _ => Err(CoreError::Store(format!(
                "cannot detect input format from extension '{}'; specify --format csv|jsonl",
                path.display()
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// RowSource
// ---------------------------------------------------------------------------

/// A single row emitted by an [`InputStream`] implementation.
pub struct RowSource {
    /// Zero-based physical sequence index (I1: monotonically increasing, no gaps).
    pub seq: u64,
    /// Row data as a JSON object map.
    pub data: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// InputStream trait
// ---------------------------------------------------------------------------

/// A streaming source of [`RowSource`] values.
///
/// Takes `self: Box<Self>` so that the reader task can call `input.yield_rows()`
/// after receiving a `Box<dyn InputStream>`.
pub trait InputStream: Send {
    fn yield_rows(
        self: Box<Self>,
    ) -> Box<dyn Iterator<Item = Result<RowSource, CoreError>> + Send>;
}

// ---------------------------------------------------------------------------
// CsvInputStream
// ---------------------------------------------------------------------------

/// CSV-backed [`InputStream`].
///
/// Opens the file and reads the header row at construction time. A required-input
/// check is performed immediately: any required column absent from the header
/// causes `open` to return `CoreError::Store` containing the text
/// `MISSING_REQUIRED_INPUT_COLUMN`.
pub struct CsvInputStream {
    headers: Vec<String>,
    reader: csv::Reader<File>,
}

impl CsvInputStream {
    /// Open a CSV file, read its header row, and check `required_input`.
    ///
    /// Error message format for missing columns:
    /// ```text
    /// MISSING_REQUIRED_INPUT_COLUMN: required column 'b' not found; present headers: a, c
    /// ```
    pub fn open(path: &Path, required_input: &[String]) -> Result<Self, CoreError> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)
            .map_err(|e| CoreError::Store(format!("csv open error: {e}")))?;

        let headers: Vec<String> = reader
            .headers()
            .map_err(|e| CoreError::Store(format!("csv header read error: {e}")))?
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Required-input check: collect ALL missing columns before reporting.
        let missing: Vec<&str> = required_input
            .iter()
            .filter(|col| !headers.iter().any(|h| h == *col))
            .map(|s| s.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(CoreError::Store(format!(
                "MISSING_REQUIRED_INPUT_COLUMN: required columns [{}] not found; \
                 present headers: {}",
                missing.join(", "),
                headers.join(", ")
            )));
        }

        Ok(CsvInputStream { headers, reader })
    }
}

impl InputStream for CsvInputStream {
    /// Yield rows as `RowSource` values.
    ///
    /// Each row is enumerated from 0 (Decision I1). CSV values are always
    /// `serde_json::Value::String`; no type coercion is performed.
    fn yield_rows(
        self: Box<Self>,
    ) -> Box<dyn Iterator<Item = Result<RowSource, CoreError>> + Send> {
        let CsvInputStream { headers, reader } = *self;
        Box::new(CsvIter {
            headers,
            records: reader.into_records(),
            seq: 0,
        })
    }
}

struct CsvIter {
    headers: Vec<String>,
    records: csv::StringRecordsIntoIter<File>,
    seq: u64,
}

impl Iterator for CsvIter {
    type Item = Result<RowSource, CoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        let record = self.records.next()?;
        let seq = self.seq;
        self.seq += 1;

        Some(match record {
            Err(e) => Err(CoreError::Store(format!("csv read error at row {seq}: {e}"))),
            Ok(rec) => {
                let mut data = serde_json::Map::new();
                for (i, header) in self.headers.iter().enumerate() {
                    let val = rec.get(i).unwrap_or("").to_string();
                    data.insert(header.clone(), serde_json::Value::String(val));
                }
                Ok(RowSource { seq, data })
            }
        })
    }
}

// ---------------------------------------------------------------------------
// JsonlInputStream
// ---------------------------------------------------------------------------

/// JSONL-backed [`InputStream`].
///
/// Opens the file and peeks the first line at construction time to validate
/// required-input keys (Decision D5). Subsequent rows missing required keys
/// are the handler's responsibility — no re-check is performed.
pub struct JsonlInputStream {
    reader: BufReader<File>,
    /// The already-parsed first row (None if the file was empty).
    first: Option<serde_json::Map<String, serde_json::Value>>,
}

impl JsonlInputStream {
    /// Open a JSONL file, peek the first line, and check `required_input`.
    ///
    /// - Empty file or all-blank file: no required check fires; `yield_rows` returns an empty iterator.
    /// - Leading blank lines are skipped when looking for the sniff target.
    /// - First non-blank line parse error → `CoreError::Store("JSONL parse error at line 1: ...")`.
    /// - First non-blank line missing required key → `CoreError::Store` containing
    ///   `MISSING_REQUIRED_INPUT_COLUMN`.
    pub fn open(path: &Path, required_input: &[String]) -> Result<Self, CoreError> {
        let file = File::open(path)
            .map_err(|e| CoreError::Store(format!("jsonl open error: {e}")))?;
        let mut reader = BufReader::new(file);

        // Read lines until we find the first non-blank line (the sniff target).
        // Blank leading lines are skipped for the purposes of required-input
        // checking (Decision D5). Each blank line consumed here is effectively
        // dropped — it carries no data, so no seq is assigned to it.
        let first_map = loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| CoreError::Store(format!("jsonl read error: {e}")))?;

            if n == 0 {
                // EOF — file is empty (or all-blank). No required check; no first row.
                return Ok(JsonlInputStream { reader, first: None });
            }

            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r').trim();
            if trimmed.is_empty() {
                // Blank line — skip and keep looking.
                continue;
            }

            let map: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(trimmed)
                    .map_err(|e| CoreError::Store(format!("JSONL parse error at line 1: {e}")))?;

            break map;
        };

        // Required-input check on the first row only (D5).
        // Collect ALL missing keys before reporting (P11 review follow-up).
        let missing: Vec<&str> = required_input
            .iter()
            .filter(|key| !first_map.contains_key(key.as_str()))
            .map(|s| s.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(CoreError::Store(format!(
                "MISSING_REQUIRED_INPUT_COLUMN: required keys [{}] not found in first JSONL row",
                missing.join(", ")
            )));
        }

        Ok(JsonlInputStream {
            reader,
            first: Some(first_map),
        })
    }
}

impl InputStream for JsonlInputStream {
    /// Yield rows from the JSONL file.
    ///
    /// - The peeked first row is yielded at `seq = 0`.
    /// - Subsequent lines are read and parsed at `seq = 1, 2, ...`.
    /// - Per-line parse errors propagate as `Err` in the iterator stream; the
    ///   iterator continues past them (lazy caller-decides strategy). The seq
    ///   counter still advances for the bad line so downstream seqs remain
    ///   physically aligned with the file line numbers.
    fn yield_rows(
        self: Box<Self>,
    ) -> Box<dyn Iterator<Item = Result<RowSource, CoreError>> + Send> {
        let JsonlInputStream { reader, first } = *self;
        Box::new(JsonlIter {
            lines: reader.lines(),
            seq: 0,
            first,
            first_emitted: false,
        })
    }
}

struct JsonlIter {
    lines: std::io::Lines<BufReader<File>>,
    seq: u64,
    first: Option<serde_json::Map<String, serde_json::Value>>,
    first_emitted: bool,
}

impl Iterator for JsonlIter {
    type Item = Result<RowSource, CoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Emit the pre-parsed first row before reading further lines.
        if !self.first_emitted {
            self.first_emitted = true;
            if let Some(first) = self.first.take() {
                let seq = self.seq;
                self.seq += 1;
                return Some(Ok(RowSource { seq, data: first }));
            }
            // first is None (empty file or all-blank) → fall through to the
            // line-reading loop below, which will return None immediately if
            // there are no more lines.
        }

        // Read subsequent lines.
        loop {
            let line_result = self.lines.next()?;
            let seq = self.seq;
            self.seq += 1;

            match line_result {
                Err(e) => {
                    return Some(Err(CoreError::Store(format!(
                        "jsonl io error at line {}: {e}",
                        seq + 1
                    ))))
                }
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // Skip blank lines but keep the seq counter advancing.
                        continue;
                    }
                    return Some(
                        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(trimmed)
                            .map(|data| RowSource { seq, data })
                            .map_err(|e| {
                                CoreError::Store(format!(
                                    "JSONL parse error at line {}: {e}",
                                    seq + 1
                                ))
                            }),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience opener
// ---------------------------------------------------------------------------

/// Dispatch to the correct [`InputStream`] implementation based on `format`.
///
/// Called by P5 reader task after format detection.
pub fn open_input(
    path: &Path,
    format: InputFormat,
    required_input: &[String],
) -> Result<Box<dyn InputStream>, CoreError> {
    match format {
        InputFormat::Csv => Ok(Box::new(CsvInputStream::open(path, required_input)?)),
        InputFormat::Jsonl => Ok(Box::new(JsonlInputStream::open(path, required_input)?)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // -----------------------------------------------------------------------
    // Helper: write a temp file with given content and extension.
    // -----------------------------------------------------------------------

    fn temp_file_with_content(content: &str, suffix: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(suffix)
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    // -----------------------------------------------------------------------
    // InputFormat::detect tests (1–6)
    // -----------------------------------------------------------------------

    #[test]
    fn format_detect_csv_extension() {
        let p = std::path::Path::new("data.csv");
        assert!(matches!(InputFormat::detect(p, None), Ok(InputFormat::Csv)));
    }

    #[test]
    fn format_detect_jsonl_extension() {
        let p = std::path::Path::new("data.jsonl");
        assert!(matches!(
            InputFormat::detect(p, None),
            Ok(InputFormat::Jsonl)
        ));
    }

    #[test]
    fn format_detect_ndjson_extension() {
        let p = std::path::Path::new("data.ndjson");
        assert!(matches!(
            InputFormat::detect(p, None),
            Ok(InputFormat::Jsonl)
        ));
    }

    #[test]
    fn format_detect_unknown_extension_errors() {
        let p = std::path::Path::new("data.foo");
        let err = InputFormat::detect(p, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("specify --format"),
            "expected 'specify --format' in: {msg}"
        );
    }

    #[test]
    fn format_detect_explicit_overrides_extension() {
        // .csv extension but explicit Jsonl override.
        let p = std::path::Path::new("data.csv");
        assert!(matches!(
            InputFormat::detect(p, Some(InputFormat::Jsonl)),
            Ok(InputFormat::Jsonl)
        ));
    }

    #[test]
    fn format_detect_explicit_with_no_extension() {
        // No extension at all, but explicit Csv provided.
        let p = std::path::Path::new("/tmp/data");
        assert!(matches!(
            InputFormat::detect(p, Some(InputFormat::Csv)),
            Ok(InputFormat::Csv)
        ));
    }

    // -----------------------------------------------------------------------
    // CsvInputStream tests (7–10)
    // -----------------------------------------------------------------------

    #[test]
    fn csv_input_stream_basic() {
        let f = temp_file_with_content("a,b,c\n1,2,3\n4,5,6\n7,8,9\n", ".csv");
        let stream = Box::new(CsvInputStream::open(f.path(), &[]).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            let src = row.as_ref().unwrap();
            assert_eq!(src.seq, i as u64);
        }
        // Check headers/values for first row.
        let r0 = rows[0].as_ref().unwrap();
        assert_eq!(
            r0.data.get("a").unwrap(),
            &serde_json::Value::String("1".into())
        );
        assert_eq!(
            r0.data.get("b").unwrap(),
            &serde_json::Value::String("2".into())
        );
        assert_eq!(
            r0.data.get("c").unwrap(),
            &serde_json::Value::String("3".into())
        );
        // Check last row.
        let r2 = rows[2].as_ref().unwrap();
        assert_eq!(r2.seq, 2);
        assert_eq!(
            r2.data.get("a").unwrap(),
            &serde_json::Value::String("7".into())
        );
    }

    #[test]
    fn csv_input_stream_empty() {
        // Header-only CSV → 0 rows yielded.
        let f = temp_file_with_content("a,b,c\n", ".csv");
        let stream = Box::new(CsvInputStream::open(f.path(), &[]).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert!(rows.is_empty());
    }

    #[test]
    fn csv_input_required_check_pass() {
        // required=[a, b]; header=[a, b, c] → open succeeds.
        let f = temp_file_with_content("a,b,c\n1,2,3\n", ".csv");
        let required = vec!["a".to_string(), "b".to_string()];
        assert!(CsvInputStream::open(f.path(), &required).is_ok());
    }

    #[test]
    fn csv_input_required_check_fail() {
        // required=[a, b]; header=[a, c] → error contains MISSING_REQUIRED_INPUT_COLUMN,
        // the missing key 'b', and the present headers 'a, c'.
        let f = temp_file_with_content("a,c\n1,2\n", ".csv");
        let required = vec!["a".to_string(), "b".to_string()];
        let err = CsvInputStream::open(f.path(), &required)
            .err()
            .expect("expected Err from CsvInputStream::open");
        let msg = err.to_string();
        assert!(
            msg.contains("MISSING_REQUIRED_INPUT_COLUMN"),
            "expected MISSING_REQUIRED_INPUT_COLUMN in: {msg}"
        );
        assert!(msg.contains('b'), "expected missing key 'b' in: {msg}");
        // Present headers should appear.
        assert!(msg.contains('a'), "expected present header 'a' in: {msg}");
        assert!(msg.contains('c'), "expected present header 'c' in: {msg}");
    }

    // -----------------------------------------------------------------------
    // JsonlInputStream tests (11–16)
    // -----------------------------------------------------------------------

    #[test]
    fn jsonl_input_stream_basic() {
        let content = r#"{"a":1,"b":"x"}
{"a":2,"b":"y"}
{"a":3,"b":"z"}
"#;
        let f = temp_file_with_content(content, ".jsonl");
        let stream = Box::new(JsonlInputStream::open(f.path(), &[]).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.as_ref().unwrap().seq, i as u64);
        }
        // Check first row data.
        let r0 = rows[0].as_ref().unwrap();
        assert_eq!(
            r0.data.get("a").unwrap(),
            &serde_json::Value::Number(1.into())
        );
        assert_eq!(
            r0.data.get("b").unwrap(),
            &serde_json::Value::String("x".into())
        );
        // Check last row.
        let r2 = rows[2].as_ref().unwrap();
        assert_eq!(r2.seq, 2);
    }

    #[test]
    fn jsonl_input_stream_empty() {
        let f = temp_file_with_content("", ".jsonl");
        let stream = Box::new(JsonlInputStream::open(f.path(), &[]).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert!(rows.is_empty());
    }

    #[test]
    fn jsonl_input_required_check_pass() {
        let f = temp_file_with_content("{\"a\":1}\n", ".jsonl");
        let required = vec!["a".to_string()];
        assert!(JsonlInputStream::open(f.path(), &required).is_ok());
    }

    #[test]
    fn jsonl_input_required_check_fail_first_row() {
        // required=[a, b]; first line {"a":1, "c":2} → error MISSING_REQUIRED_INPUT_COLUMN
        // mentioning 'b'.
        let f = temp_file_with_content("{\"a\":1,\"c\":2}\n", ".jsonl");
        let required = vec!["a".to_string(), "b".to_string()];
        let err = JsonlInputStream::open(f.path(), &required)
            .err()
            .expect("expected Err from JsonlInputStream::open");
        let msg = err.to_string();
        assert!(
            msg.contains("MISSING_REQUIRED_INPUT_COLUMN"),
            "expected MISSING_REQUIRED_INPUT_COLUMN in: {msg}"
        );
        assert!(msg.contains('b'), "expected missing key 'b' in: {msg}");
    }

    #[test]
    fn jsonl_input_first_row_sniff_only() {
        // D5: required=[a]; first line has 'a'; second line missing 'a'; third missing 'a'.
        // open() must succeed; yield_rows() must return all 3 rows as Ok.
        let content = r#"{"a":1,"b":2}
{"a":2}
{"c":3}
"#;
        let f = temp_file_with_content(content, ".jsonl");
        let required = vec!["a".to_string()];
        let stream = Box::new(JsonlInputStream::open(f.path(), &required).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert_eq!(rows.len(), 3);
        // All three rows must be Ok (missing key on row 2/3 is handler's problem).
        for (i, row) in rows.iter().enumerate() {
            assert!(
                row.is_ok(),
                "row {i} should be Ok per D5, got: {:?}",
                row.as_ref().err()
            );
        }
        assert_eq!(rows[2].as_ref().unwrap().seq, 2);
        // Third row has key 'c', not 'a' — that's fine.
        assert!(rows[2].as_ref().unwrap().data.contains_key("c"));
    }

    #[test]
    fn jsonl_input_parse_error_mid_stream() {
        // valid line 0, garbage line 1, valid line 2.
        // Expected: row 0 Ok, row 1 Err, row 2 Ok.
        // (Iterator continues past parse errors; caller decides how to handle them.)
        let content = "{\"a\":1}\nnot-valid-json\n{\"a\":3}\n";
        let f = temp_file_with_content(content, ".jsonl");
        let stream = Box::new(JsonlInputStream::open(f.path(), &[]).unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        assert_eq!(rows.len(), 3, "should yield 3 items (including the error)");
        assert!(rows[0].is_ok(), "row 0 should be Ok");
        assert!(rows[1].is_err(), "row 1 should be Err (bad JSON)");
        assert!(rows[2].is_ok(), "row 2 should be Ok");
        // Verify seq continuity.
        assert_eq!(rows[0].as_ref().unwrap().seq, 0);
        assert_eq!(rows[2].as_ref().unwrap().seq, 2);
    }

    // -----------------------------------------------------------------------
    // P4 review follow-ups
    // -----------------------------------------------------------------------

    /// P11 §16: csv_required_check_lists_all_missing
    ///
    /// required=[a, b, c], headers=[d] → error message mentions ALL THREE
    /// missing keys (a, b, c), not just the first one found.
    #[test]
    fn csv_required_check_lists_all_missing() {
        let f = temp_file_with_content("d\n1\n", ".csv");
        let required = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let err = CsvInputStream::open(f.path(), &required)
            .err()
            .expect("expected Err");
        let msg = err.to_string();
        assert!(
            msg.contains("MISSING_REQUIRED_INPUT_COLUMN"),
            "expected MISSING_REQUIRED_INPUT_COLUMN in: {msg}"
        );
        // All three missing keys must appear in the error.
        assert!(msg.contains('a'), "expected missing key 'a' in: {msg}");
        assert!(msg.contains('b'), "expected missing key 'b' in: {msg}");
        assert!(msg.contains('c'), "expected missing key 'c' in: {msg}");
    }

    /// P11 §16: jsonl_required_check_lists_all_missing
    ///
    /// required=[a, b, c], first line={"d":1} → error message mentions ALL THREE
    /// missing keys (a, b, c), not just the first one found.
    #[test]
    fn jsonl_required_check_lists_all_missing() {
        let f = temp_file_with_content("{\"d\":1}\n", ".jsonl");
        let required = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let err = JsonlInputStream::open(f.path(), &required)
            .err()
            .expect("expected Err");
        let msg = err.to_string();
        assert!(
            msg.contains("MISSING_REQUIRED_INPUT_COLUMN"),
            "expected MISSING_REQUIRED_INPUT_COLUMN in: {msg}"
        );
        assert!(msg.contains('a'), "expected missing key 'a' in: {msg}");
        assert!(msg.contains('b'), "expected missing key 'b' in: {msg}");
        assert!(msg.contains('c'), "expected missing key 'c' in: {msg}");
    }

    /// P11 §16: jsonl_blank_first_line_with_required_input
    ///
    /// First line is blank; a second valid line exists with the required key.
    /// Open must succeed (blank lines are skipped in sniff), and yield_rows must
    /// return the valid row.
    #[test]
    fn jsonl_blank_first_line_with_required_input() {
        // Blank line first, then a valid line.
        let content = "\n{\"a\":1}\n";
        let f = temp_file_with_content(content, ".jsonl");
        let required = vec!["a".to_string()];
        // open() sniffs the first non-blank line → should succeed.
        let result = JsonlInputStream::open(f.path(), &required);
        assert!(result.is_ok(), "open() should succeed: blank first line skipped; got: {:?}", result.err());
        // yield_rows should return whatever lines are present.
        let stream = Box::new(result.unwrap());
        let rows: Vec<_> = stream.yield_rows().collect();
        // The blank line may appear as a parse error or be silently skipped
        // depending on the implementation; at minimum one valid row must appear.
        let ok_rows: Vec<_> = rows.iter().filter(|r| r.is_ok()).collect();
        assert!(!ok_rows.is_empty(), "at least one valid row expected; rows: {:?}", rows.len());
    }
}
