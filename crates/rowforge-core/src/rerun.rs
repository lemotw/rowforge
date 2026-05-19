use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RerunError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("csv: {0}")]
    Csv(#[from] csv::Error),
}

/// Codes that are NEVER re-runnable, regardless of `--include-crash-rows`.
///
/// `WORKER_CRASH_UNSAFE` means the handler is not idempotent — re-dispatching
/// could double-apply side effects. `ROW_TOO_LARGE` rows were rejected by the
/// accumulator before any worker ever saw them; the same input would just be
/// rejected again. Hard-exclude both with no override knob.
const ALWAYS_FILTERED: &[&str] = &[
    crate::run::ERR_WORKER_CRASH_UNSAFE,
    crate::run::ERR_ROW_TOO_LARGE,
];

/// Codes that are filtered by default but can be re-included via
/// `--include-crash-rows`. `WORKER_CRASH` is the row-mode / batch-idempotent
/// crash. `CANCELLED` rows never reached a worker, so re-running them is
/// the operator's intended next step in the common case.
const FILTERED_BY_DEFAULT: &[&str] = &[
    crate::run::ERR_WORKER_CRASH,
    crate::run::ERR_CANCELLED,
];

fn should_filter(code: &str, include_crash_rows: bool) -> bool {
    if ALWAYS_FILTERED.contains(&code) {
        return true;
    }
    if FILTERED_BY_DEFAULT.contains(&code) {
        return !include_crash_rows;
    }
    false
}

/// Detect whether a CSV looks like a rowforge failed.csv.
///
/// New schema marker: an `errcode` column (no underscore prefix). The legacy
/// `_err_code` form is gone — old failed.csv files predating this format
/// change are no longer recognized.
pub fn looks_like_failed_csv(path: &Path) -> Result<bool, RerunError> {
    let mut rdr = csv::Reader::from_path(path)?;
    let headers = rdr.headers()?;
    Ok(headers.iter().any(|h| h == "errcode"))
}

/// Prepare a rerun-input CSV from a previous failed.csv.
///
/// Strategy:
///   - Strip the protocol columns (`seqid`, `errcode`, `errmessage`) and any
///     `meta_*` columns.
///   - Exclude rows whose `errcode` is in `ALWAYS_FILTERED` (no override).
///   - Exclude rows whose `errcode` is in `FILTERED_BY_DEFAULT` unless
///     `include_crash` is true.
///
/// IMPORTANT: under the current output schema, failed.csv carries NO original
/// input-CSV columns — only `seqid,errcode,errmessage` (+ optional meta_*).
/// After stripping, the resulting "rerun input" has zero data columns, so the
/// legacy `rowforge run --input failed.csv` workflow is effectively dead: the
/// downstream pool will see a CSV with no data and process nothing. This
/// function is kept for now (a) so the legacy flag doesn't panic, and (b) so
/// any future schema that re-introduces original columns can drop straight
/// back in. Real "rerun" should go through `rowforge exec run --force` instead.
///
/// Returns a tempfile that lives until dropped.
pub fn prepare_rerun_input(
    failed_csv: &Path,
    include_crash: bool,
) -> Result<tempfile::NamedTempFile, RerunError> {
    let mut rdr = csv::Reader::from_path(failed_csv)?;
    let headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();
    let err_code_idx = headers.iter().position(|h| h == "errcode");
    // Drop seqid/errcode/errmessage + anything prefixed `meta_`. Anything
    // else passes through (today there is nothing else; preserved for any
    // future evolution of the output schema).
    let keep_indices: Vec<usize> = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| {
            let h = h.as_str();
            h != "seqid"
                && h != "errcode"
                && h != "errmessage"
                && !h.starts_with("meta_")
        })
        .map(|(i, _)| i)
        .collect();

    let out = tempfile::Builder::new()
        .prefix("rerun-")
        .suffix(".csv")
        .tempfile()?;
    {
        let mut wtr = csv::Writer::from_writer(out.as_file());
        let new_headers: Vec<&str> = keep_indices.iter().map(|i| headers[*i].as_str()).collect();
        wtr.write_record(&new_headers)?;

        for rec in rdr.records() {
            let rec = rec?;
            if let Some(idx) = err_code_idx {
                if let Some(code) = rec.get(idx) {
                    if should_filter(code, include_crash) {
                        continue;
                    }
                }
            }
            let new_row: Vec<&str> = keep_indices
                .iter()
                .map(|i| rec.get(*i).unwrap_or(""))
                .collect();
            wtr.write_record(&new_row)?;
        }
        wtr.flush()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_failed_csv(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn detects_failed_csv() {
        let f = write_failed_csv("seqid,errcode,errmessage\n0,X,m\n");
        assert!(looks_like_failed_csv(f.path()).unwrap());
        // A garden-variety CSV without an `errcode` column is not failed.csv.
        let g = write_failed_csv("email,name\na@x,Alice\n");
        assert!(!looks_like_failed_csv(g.path()).unwrap());
    }

    #[test]
    fn strips_protocol_and_meta_columns() {
        // New schema has no original-CSV columns at all — the rerun input is
        // expected to be empty (header-only with zero data columns).
        let f = write_failed_csv(
            "seqid,errcode,errmessage,meta_handler_ver,meta_crash_at_seq,meta_crash_worker_id\n\
             0,INVALID,missing tld,0.0.0,,\n",
        );
        let out = prepare_rerun_input(f.path(), false).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        // Zero remaining columns: file contains only an (empty) header line +
        // one empty data row. We assert "no protocol columns and no meta
        // columns leak through" rather than enforcing exact byte layout.
        assert!(!content.contains("seqid"));
        assert!(!content.contains("errcode"));
        assert!(!content.contains("errmessage"));
        assert!(!content.contains("meta_"));
        assert!(!content.contains("INVALID"));
    }

    #[test]
    fn excludes_crash_rows_by_default() {
        // With the new schema there's nothing to "carry forward" per row, so
        // we just check that the filter shape (drop WORKER_CRASH by default)
        // affects the number of data records produced.
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,INVALID,missing\n\
             1,WORKER_CRASH,worker died\n",
        );
        let out = prepare_rerun_input(f.path(), false).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        // One data row (the INVALID one) plus the empty header line.
        let line_count = content.lines().count();
        assert_eq!(
            line_count, 2,
            "expected header + 1 data line after dropping WORKER_CRASH, got:\n{content}"
        );
    }

    #[test]
    fn includes_crash_rows_when_flag_set() {
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,WORKER_CRASH,worker died\n",
        );
        let out = prepare_rerun_input(f.path(), true).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        let line_count = content.lines().count();
        // include_crash=true keeps WORKER_CRASH; expect header + 1 data row.
        assert_eq!(line_count, 2, "expected crash row included; got:\n{content}");
    }

    #[test]
    fn worker_crash_unsafe_always_filtered() {
        // Even with include_crash=true, WORKER_CRASH_UNSAFE must never appear
        // in the rerun input — the handler said "I'm not safe to re-run".
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,WORKER_CRASH,died once\n\
             1,WORKER_CRASH_UNSAFE,non-idempotent crash\n",
        );
        let out = prepare_rerun_input(f.path(), true).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        // header + 1 data row (the safe WORKER_CRASH); the UNSAFE one must
        // be dropped even with include_crash=true.
        let line_count = content.lines().count();
        assert_eq!(
            line_count, 2,
            "expected header + 1 safe-crash row, WORKER_CRASH_UNSAFE filtered; got:\n{content}"
        );
    }

    #[test]
    fn row_too_large_always_filtered() {
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,ROW_TOO_LARGE,5MB row\n",
        );
        let out_no = prepare_rerun_input(f.path(), false).unwrap();
        let out_yes = prepare_rerun_input(f.path(), true).unwrap();
        let c_no = std::fs::read_to_string(out_no.path()).unwrap();
        let c_yes = std::fs::read_to_string(out_yes.path()).unwrap();
        // Both should be header-only (no data rows kept).
        assert_eq!(c_no.lines().count(), 1, "ROW_TOO_LARGE excluded by default");
        assert_eq!(
            c_yes.lines().count(),
            1,
            "ROW_TOO_LARGE excluded even with include flag"
        );
    }

    #[test]
    fn cancelled_filtered_by_default_included_with_flag() {
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,CANCELLED,run was cancelled\n",
        );
        let out_no = prepare_rerun_input(f.path(), false).unwrap();
        let out_yes = prepare_rerun_input(f.path(), true).unwrap();
        let c_no = std::fs::read_to_string(out_no.path()).unwrap();
        let c_yes = std::fs::read_to_string(out_yes.path()).unwrap();
        assert_eq!(c_no.lines().count(), 1, "CANCELLED excluded by default");
        assert_eq!(c_yes.lines().count(), 2, "CANCELLED included when flag set");
    }

    #[test]
    fn other_codes_pass_through() {
        let f = write_failed_csv(
            "seqid,errcode,errmessage\n\
             0,INVALID,bad email\n\
             1,NOT_FOUND,user missing\n\
             2,MATCH,already exists\n",
        );
        let out = prepare_rerun_input(f.path(), false).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        // All three pass the filter, so we expect header + 3 data rows.
        assert_eq!(
            content.lines().count(),
            4,
            "expected header + 3 data rows for non-crash codes; got:\n{content}"
        );
    }
}
