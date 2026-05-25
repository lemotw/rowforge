use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum HandlerStream {
    Stdout,
    Stderr,
}

impl HandlerStream {
    pub fn as_str(&self) -> &'static str {
        match self {
            HandlerStream::Stdout => "stdout",
            HandlerStream::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HandlerLogLine {
    pub timestamp: DateTime<Utc>,
    pub worker_id: usize,
    pub stream: HandlerStream,
    pub line: String,
}

pub fn handler_log_path(attempt_dir: &Path) -> PathBuf {
    attempt_dir.join("handler_log.log")
}

/// Format a line for on-disk persistence and CLI echo.
pub fn format_line(line: &HandlerLogLine) -> String {
    format!(
        "{} [handler#{} {}] {}\n",
        line.timestamp.to_rfc3339(),
        line.worker_id,
        line.stream.as_str(),
        line.line,
    )
}

/// Parse a line back from the on-disk format. Returns None if the line
/// doesn't conform (e.g. plain-text manual edits, or non-prefix lines).
pub fn parse_line(line: &str) -> Option<HandlerLogLine> {
    // Format: "<rfc3339> [handler#<wid> <stream>] <content>"
    let (ts, rest) = line.split_once(" [handler#")?;
    let timestamp = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    let (wid_str, rest) = rest.split_once(' ')?;
    let worker_id: usize = wid_str.parse().ok()?;
    let (stream_str, content) = rest.split_once("] ")?;
    let stream = match stream_str {
        "stdout" => HandlerStream::Stdout,
        "stderr" => HandlerStream::Stderr,
        _ => return None,
    };
    Some(HandlerLogLine {
        timestamp,
        worker_id,
        stream,
        line: content.trim_end_matches('\n').to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_round_trip() {
        let line = HandlerLogLine {
            timestamp: "2026-05-25T10:00:01.234Z".parse().unwrap(),
            worker_id: 3,
            stream: HandlerStream::Stderr,
            line: "connecting to db...".into(),
        };
        let formatted = format_line(&line);
        let parsed = parse_line(&formatted).expect("parse ok");
        assert_eq!(parsed.worker_id, 3);
        assert_eq!(parsed.stream, HandlerStream::Stderr);
        assert_eq!(parsed.line, "connecting to db...");
    }

    #[test]
    fn parse_returns_none_for_non_conforming() {
        assert!(parse_line("plain text").is_none());
        assert!(parse_line("2026-05-25T10:00:00Z [bad prefix]").is_none());
    }

    #[test]
    fn handler_log_path_appends_filename() {
        let p = handler_log_path(Path::new("/tmp/exec/attempt1"));
        assert_eq!(p, Path::new("/tmp/exec/attempt1/handler_log.log"));
    }
}
