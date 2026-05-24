//! Execution-centric storage layer.
//!
//! Each Execution owns a directory `<home>/executions/<exec_id>/` containing
//! the snapshotted input CSV and a `manifest.json` mirror. The global registry
//! across all executions lives in `<home>/executions.db` (SQLite).
//!
//! SQLite is the source of truth; `manifest.json` is a portable mirror written
//! after every state change so the per-execution folder is self-describing
//! even if the registry is lost.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, CoreError>;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Open,
    Iterating,
    Settled,
    Closed,
    Abandoned,
}

impl ExecutionState {
    fn as_str(self) -> &'static str {
        match self {
            ExecutionState::Open => "open",
            ExecutionState::Iterating => "iterating",
            ExecutionState::Settled => "settled",
            ExecutionState::Closed => "closed",
            ExecutionState::Abandoned => "abandoned",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "open" => ExecutionState::Open,
            "iterating" => ExecutionState::Iterating,
            "settled" => ExecutionState::Settled,
            "closed" => ExecutionState::Closed,
            "abandoned" => ExecutionState::Abandoned,
            other => return Err(CoreError::Store(format!("unknown execution state: {other}"))),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub name: Option<String>,
    pub input_csv_id: String,
    pub input_csv_hash: String,
    pub input_row_count: u64,
    pub current_handler_instance_id: Option<String>,
    pub state: ExecutionState,
    pub created_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub abandoned_at: Option<DateTime<Utc>>,
    pub abandoned_reason: Option<String>,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerInstance {
    pub id: String,
    pub handler_id: String,
    pub manifest_hash: String,
    pub source_snapshot_dir: PathBuf,
    pub binary_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Caller-supplied data for creating a new execution.
#[derive(Debug, Clone)]
pub struct NewExecution {
    pub name: Option<String>,
    /// Logical id of the registered CSV (free-form for now; the legacy library
    /// will own this).
    pub input_csv_id: String,
    /// Filesystem path to the source CSV that will be snapshotted into the
    /// execution folder.
    pub input_csv_path: PathBuf,
    pub current_handler_instance_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewHandlerInstance {
    pub handler_id: String,
    pub manifest_hash: String,
    pub source_snapshot_dir: PathBuf,
    pub binary_hash: Option<String>,
}

// -----------------------------------------------------------------------------
// Attempt layer
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Full,
    Sampled { size: u32 },
    // Resume / FromFailed: deferred per spec
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Simulation {
    Real,
    Dry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunType {
    pub source: Source,
    pub simulation: Simulation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    Running,
    Completed,
    Aborted,
}

impl AttemptState {
    fn as_str(self) -> &'static str {
        match self {
            AttemptState::Running => "running",
            AttemptState::Completed => "completed",
            AttemptState::Aborted => "aborted",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "running" => AttemptState::Running,
            "completed" => AttemptState::Completed,
            "aborted" => AttemptState::Aborted,
            other => return Err(CoreError::Store(format!("unknown attempt state: {other}"))),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub execution_id: String,
    pub handler_instance_id: String,
    pub parent_attempt_id: Option<String>,
    pub run_type: RunType,
    pub state: AttemptState,
    pub success_count: u64,
    pub failed_count: u64,
    pub aborted_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct NewAttempt {
    pub execution_id: String,
    pub handler_instance_id: String,
    pub parent_attempt_id: Option<String>,
    pub run_type: RunType,
}

#[derive(Debug, Clone)]
pub struct FinishAttempt {
    pub success_count: u64,
    pub failed_count: u64,
    pub aborted: bool,
    pub aborted_reason: Option<String>,
}

pub struct ExecutionStore {
    conn: Connection,
    home: PathBuf,
}

impl ExecutionStore {
    /// Open (or create) the store rooted at `home` (typically `~/.rowforge`).
    /// Ensures the executions/ subdir and SQLite database exist and are at
    /// the current schema version.
    /// The SQLite `schema_version` recorded after `open_with_migrations`
    /// completes. Studio uses this to enforce a hard version pin
    /// (spec part-4 §4.6).
    pub fn schema_version(&self) -> u8 {
        SCHEMA_VERSION as u8
    }

    pub fn open(home: &Path) -> Result<Self> {
        fs::create_dir_all(home).map_err(CoreError::Io)?;
        fs::create_dir_all(home.join("executions")).map_err(CoreError::Io)?;
        let db_path = home.join("executions.db");
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self {
            conn,
            home: home.to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
        )?;
        let current: Option<i64> = self
            .conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
            .optional()?;
        match current {
            None => {
                self.conn.execute_batch(MIGRATION_V1)?;
                self.conn.execute_batch(MIGRATION_V2)?;
                self.conn.execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )?;
            }
            Some(1) => {
                self.conn.execute_batch(MIGRATION_V2)?;
                self.conn
                    .execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) => {
                return Err(CoreError::Store(format!(
                    "executions.db schema version {v} not supported (expected {SCHEMA_VERSION})"
                )));
            }
        }
        Ok(())
    }

    /// Filesystem location of an execution by id.
    pub fn execution_dir(&self, id: &str) -> PathBuf {
        self.home.join("executions").join(id)
    }

    pub fn create_execution(&mut self, new: NewExecution) -> Result<Execution> {
        if !new.input_csv_path.is_file() {
            return Err(CoreError::Store(format!(
                "input csv not found: {}",
                new.input_csv_path.display()
            )));
        }
        let id = format!("e_{}", ulid::Ulid::new());
        let dir = self.execution_dir(&id);
        fs::create_dir_all(&dir).map_err(CoreError::Io)?;

        // Preserve source extension in the snapshot filename so that
        // `InputFormat::detect()` on subsequent `exec run` invocations sees
        // the correct format (.csv → Csv, .jsonl/.ndjson → Jsonl). Other
        // extensions fall back to `input.csv` and require --format on run.
        let snapshot_name = match new
            .input_csv_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("jsonl") => "input.jsonl",
            Some("ndjson") => "input.ndjson",
            _ => "input.csv",
        };
        let is_jsonl = snapshot_name.ends_with(".jsonl") || snapshot_name.ends_with(".ndjson");
        let dest = dir.join(snapshot_name);
        fs::copy(&new.input_csv_path, &dest).map_err(CoreError::Io)?;
        let (hash, row_count) = hash_and_count_rows(&dest, is_jsonl)?;
        fs::write(dir.join(format!("{snapshot_name}.sha256")), &hash).map_err(CoreError::Io)?;

        let created_at = Utc::now();
        let state = ExecutionState::Open;

        let exec = Execution {
            id: id.clone(),
            name: new.name.clone(),
            input_csv_id: new.input_csv_id.clone(),
            input_csv_hash: hash.clone(),
            input_row_count: row_count,
            current_handler_instance_id: new.current_handler_instance_id.clone(),
            state,
            created_at,
            settled_at: None,
            closed_at: None,
            abandoned_at: None,
            abandoned_reason: None,
            dir: dir.clone(),
        };

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO executions (
                id, name, input_csv_id, input_csv_hash, input_row_count,
                current_handler_instance_id, state, created_at,
                settled_at, closed_at, abandoned_at, abandoned_reason
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                exec.id,
                exec.name,
                exec.input_csv_id,
                exec.input_csv_hash,
                exec.input_row_count as i64,
                exec.current_handler_instance_id,
                exec.state.as_str(),
                exec.created_at.to_rfc3339(),
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
            ],
        )?;
        tx.commit()?;

        write_manifest(&dir, &exec)?;
        Ok(exec)
    }

    pub fn get_execution(&self, id: &str) -> Result<Option<Execution>> {
        let home = self.home.clone();
        self.conn
            .query_row(
                "SELECT id, name, input_csv_id, input_csv_hash, input_row_count,
                        current_handler_instance_id, state, created_at,
                        settled_at, closed_at, abandoned_at, abandoned_reason
                 FROM executions WHERE id = ?1",
                params![id],
                |r| row_to_execution(r, &home),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_executions(&self) -> Result<Vec<Execution>> {
        let home = self.home.clone();
        let mut stmt = self.conn.prepare(
            "SELECT id, name, input_csv_id, input_csv_hash, input_row_count,
                    current_handler_instance_id, state, created_at,
                    settled_at, closed_at, abandoned_at, abandoned_reason
             FROM executions ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| row_to_execution(r, &home))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(Ok).collect()
    }

    /// Update execution state. Caller is responsible for legal transitions;
    /// this layer only enforces that timestamps line up with the new state.
    pub fn set_execution_state(
        &mut self,
        id: &str,
        state: ExecutionState,
        abandoned_reason: Option<String>,
    ) -> Result<Execution> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let tx = self.conn.transaction()?;
        let updated = match state {
            ExecutionState::Settled => tx.execute(
                "UPDATE executions SET state=?1, settled_at=COALESCE(settled_at, ?2) WHERE id=?3",
                params![state.as_str(), now_str, id],
            )?,
            ExecutionState::Closed => tx.execute(
                "UPDATE executions SET state=?1, closed_at=COALESCE(closed_at, ?2) WHERE id=?3",
                params![state.as_str(), now_str, id],
            )?,
            ExecutionState::Abandoned => tx.execute(
                "UPDATE executions SET state=?1, abandoned_at=COALESCE(abandoned_at, ?2),
                 abandoned_reason=?3 WHERE id=?4",
                params![state.as_str(), now_str, abandoned_reason, id],
            )?,
            _ => tx.execute(
                "UPDATE executions SET state=?1 WHERE id=?2",
                params![state.as_str(), id],
            )?,
        };
        if updated == 0 {
            return Err(CoreError::Store(format!("execution not found: {id}")));
        }
        tx.commit()?;
        let exec = self
            .get_execution(id)?
            .ok_or_else(|| CoreError::Store(format!("execution vanished mid-update: {id}")))?;
        write_manifest(&exec.dir, &exec)?;
        Ok(exec)
    }

    pub fn register_handler_instance(
        &mut self,
        new: NewHandlerInstance,
    ) -> Result<HandlerInstance> {
        // Content-addressed: same manifest_hash + source_snapshot_dir returns
        // the existing record.
        if let Some(existing) = self
            .conn
            .query_row(
                "SELECT id, handler_id, manifest_hash, source_snapshot_dir, binary_hash, created_at
                 FROM handler_instances
                 WHERE handler_id=?1 AND manifest_hash=?2 AND source_snapshot_dir=?3",
                params![
                    new.handler_id,
                    new.manifest_hash,
                    new.source_snapshot_dir.to_string_lossy()
                ],
                row_to_handler_instance,
            )
            .optional()?
        {
            return Ok(existing);
        }
        let hi = HandlerInstance {
            id: format!("hi_{}", ulid::Ulid::new()),
            handler_id: new.handler_id,
            manifest_hash: new.manifest_hash,
            source_snapshot_dir: new.source_snapshot_dir,
            binary_hash: new.binary_hash,
            created_at: Utc::now(),
        };
        self.conn.execute(
            "INSERT INTO handler_instances (
                id, handler_id, manifest_hash, source_snapshot_dir, binary_hash, created_at
            ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                hi.id,
                hi.handler_id,
                hi.manifest_hash,
                hi.source_snapshot_dir.to_string_lossy(),
                hi.binary_hash,
                hi.created_at.to_rfc3339(),
            ],
        )?;
        Ok(hi)
    }

    pub fn get_handler_instance(&self, id: &str) -> Result<Option<HandlerInstance>> {
        self.conn
            .query_row(
                "SELECT id, handler_id, manifest_hash, source_snapshot_dir, binary_hash, created_at
                 FROM handler_instances WHERE id=?1",
                params![id],
                row_to_handler_instance,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn attempt_dir(&self, exec_id: &str, attempt_id: &str) -> PathBuf {
        self.execution_dir(exec_id)
            .join("attempts")
            .join(attempt_id)
    }

    /// Create a new attempt for `new.execution_id`. Validates that the
    /// execution exists and is in a state that accepts attempts. Creates
    /// the attempt's filesystem dir and inserts a `running` row.
    pub fn create_attempt(&mut self, new: NewAttempt) -> Result<Attempt> {
        let exec = self
            .get_execution(&new.execution_id)?
            .ok_or_else(|| CoreError::Store(format!("execution not found: {}", new.execution_id)))?;
        match exec.state {
            ExecutionState::Open | ExecutionState::Iterating | ExecutionState::Settled => {}
            ExecutionState::Closed | ExecutionState::Abandoned => {
                return Err(CoreError::Store(format!(
                    "execution {} is {:?}; no further attempts allowed",
                    exec.id, exec.state
                )));
            }
        }
        if self.get_handler_instance(&new.handler_instance_id)?.is_none() {
            return Err(CoreError::Store(format!(
                "handler instance not found: {}",
                new.handler_instance_id
            )));
        }

        let id = format!("r_{}", ulid::Ulid::new());
        let dir = self.attempt_dir(&exec.id, &id);
        fs::create_dir_all(&dir).map_err(CoreError::Io)?;

        let (src_kind, src_size) = match new.run_type.source {
            Source::Full => ("full", None),
            Source::Sampled { size } => ("sampled", Some(size as i64)),
        };
        let sim = match new.run_type.simulation {
            Simulation::Real => "real",
            Simulation::Dry => "dry",
        };
        let started_at = Utc::now();

        self.conn.execute(
            "INSERT INTO attempts (
                id, execution_id, handler_instance_id, parent_attempt_id,
                run_type_source, run_type_sample_size, run_type_simulation,
                state, success_count, failed_count, aborted_reason,
                started_at, ended_at
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                id,
                exec.id,
                new.handler_instance_id,
                new.parent_attempt_id,
                src_kind,
                src_size,
                sim,
                AttemptState::Running.as_str(),
                0_i64,
                0_i64,
                Option::<String>::None,
                started_at.to_rfc3339(),
                Option::<String>::None,
            ],
        )?;

        Ok(Attempt {
            id,
            execution_id: exec.id,
            handler_instance_id: new.handler_instance_id,
            parent_attempt_id: new.parent_attempt_id,
            run_type: new.run_type,
            state: AttemptState::Running,
            success_count: 0,
            failed_count: 0,
            aborted_reason: None,
            started_at,
            ended_at: None,
            dir,
        })
    }

    /// Mark an attempt terminal. `finish.aborted` decides Aborted vs Completed.
    /// Side effect: if the execution is in OPEN and this attempt completed
    /// (regardless of result), the execution is bumped to ITERATING.
    pub fn finish_attempt(&mut self, attempt_id: &str, finish: FinishAttempt) -> Result<Attempt> {
        let state = if finish.aborted {
            AttemptState::Aborted
        } else {
            AttemptState::Completed
        };
        let now = Utc::now();

        let updated = self.conn.execute(
            "UPDATE attempts SET state=?1, success_count=?2, failed_count=?3,
                                 aborted_reason=?4, ended_at=?5
             WHERE id=?6 AND state='running'",
            params![
                state.as_str(),
                finish.success_count as i64,
                finish.failed_count as i64,
                finish.aborted_reason,
                now.to_rfc3339(),
                attempt_id,
            ],
        )?;
        if updated == 0 {
            return Err(CoreError::Store(format!(
                "attempt not found or not running: {attempt_id}"
            )));
        }

        let attempt = self
            .get_attempt(attempt_id)?
            .ok_or_else(|| CoreError::Store(format!("attempt vanished mid-finish: {attempt_id}")))?;

        // Bump exec OPEN → ITERATING if we just landed any attempt.
        let exec = self
            .get_execution(&attempt.execution_id)?
            .ok_or_else(|| CoreError::Store(format!("orphan attempt {attempt_id}")))?;
        if exec.state == ExecutionState::Open {
            self.set_execution_state(&exec.id, ExecutionState::Iterating, None)?;
        }
        Ok(attempt)
    }

    pub fn get_attempt(&self, id: &str) -> Result<Option<Attempt>> {
        let home = self.home.clone();
        self.conn
            .query_row(
                "SELECT id, execution_id, handler_instance_id, parent_attempt_id,
                        run_type_source, run_type_sample_size, run_type_simulation,
                        state, success_count, failed_count, aborted_reason,
                        started_at, ended_at
                 FROM attempts WHERE id=?1",
                params![id],
                |r| row_to_attempt(r, &home),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_attempts_for_execution(&self, exec_id: &str) -> Result<Vec<Attempt>> {
        let home = self.home.clone();
        let mut stmt = self.conn.prepare(
            "SELECT id, execution_id, handler_instance_id, parent_attempt_id,
                    run_type_source, run_type_sample_size, run_type_simulation,
                    state, success_count, failed_count, aborted_reason,
                    started_at, ended_at
             FROM attempts WHERE execution_id=?1 ORDER BY started_at ASC",
        )?;
        let rows = stmt
            .query_map(params![exec_id], |r| row_to_attempt(r, &home))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(Ok).collect()
    }
}

const MIGRATION_V1: &str = r#"
CREATE TABLE executions (
    id                           TEXT PRIMARY KEY,
    name                         TEXT,
    input_csv_id                 TEXT NOT NULL,
    input_csv_hash               TEXT NOT NULL,
    input_row_count              INTEGER NOT NULL,
    current_handler_instance_id  TEXT,
    state                        TEXT NOT NULL,
    created_at                   TEXT NOT NULL,
    settled_at                   TEXT,
    closed_at                    TEXT,
    abandoned_at                 TEXT,
    abandoned_reason             TEXT
);
CREATE INDEX idx_executions_state ON executions(state);
CREATE INDEX idx_executions_created_at ON executions(created_at);

CREATE TABLE handler_instances (
    id                           TEXT PRIMARY KEY,
    handler_id                   TEXT NOT NULL,
    manifest_hash                TEXT NOT NULL,
    source_snapshot_dir          TEXT NOT NULL,
    binary_hash                  TEXT,
    created_at                   TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_handler_instances_content
    ON handler_instances(handler_id, manifest_hash, source_snapshot_dir);
"#;

const MIGRATION_V2: &str = r#"
CREATE TABLE attempts (
    id                           TEXT PRIMARY KEY,
    execution_id                 TEXT NOT NULL REFERENCES executions(id),
    handler_instance_id          TEXT NOT NULL REFERENCES handler_instances(id),
    parent_attempt_id            TEXT,
    run_type_source              TEXT NOT NULL,
    run_type_sample_size         INTEGER,
    run_type_simulation          TEXT NOT NULL,
    state                        TEXT NOT NULL,
    success_count                INTEGER NOT NULL DEFAULT 0,
    failed_count                 INTEGER NOT NULL DEFAULT 0,
    aborted_reason               TEXT,
    started_at                   TEXT NOT NULL,
    ended_at                     TEXT
);
CREATE INDEX idx_attempts_execution ON attempts(execution_id);
CREATE INDEX idx_attempts_started_at ON attempts(started_at);
"#;

fn row_to_execution(r: &rusqlite::Row<'_>, home: &Path) -> rusqlite::Result<Execution> {
    let id: String = r.get(0)?;
    let dir = home.join("executions").join(&id);
    let state_str: String = r.get(6)?;
    let state = ExecutionState::parse(&state_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
        )
    })?;
    Ok(Execution {
        id,
        name: r.get(1)?,
        input_csv_id: r.get(2)?,
        input_csv_hash: r.get(3)?,
        input_row_count: r.get::<_, i64>(4)? as u64,
        current_handler_instance_id: r.get(5)?,
        state,
        created_at: parse_rfc3339(r.get::<_, String>(7)?)?,
        settled_at: r.get::<_, Option<String>>(8)?.map(parse_rfc3339).transpose()?,
        closed_at: r.get::<_, Option<String>>(9)?.map(parse_rfc3339).transpose()?,
        abandoned_at: r.get::<_, Option<String>>(10)?.map(parse_rfc3339).transpose()?,
        abandoned_reason: r.get(11)?,
        dir,
    })
}

fn row_to_attempt(r: &rusqlite::Row<'_>, home: &Path) -> rusqlite::Result<Attempt> {
    let id: String = r.get(0)?;
    let exec_id: String = r.get(1)?;
    let src_kind: String = r.get(4)?;
    let src_size: Option<i64> = r.get(5)?;
    let sim_str: String = r.get(6)?;
    let state_str: String = r.get(7)?;

    let source = match src_kind.as_str() {
        "full" => Source::Full,
        "sampled" => Source::Sampled {
            size: src_size.unwrap_or(0) as u32,
        },
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown run_type_source: {other}"),
                )),
            ))
        }
    };
    let simulation = match sim_str.as_str() {
        "real" => Simulation::Real,
        "dry" => Simulation::Dry,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown run_type_simulation: {other}"),
                )),
            ))
        }
    };
    let state = AttemptState::parse(&state_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
        )
    })?;

    let dir = home.join("executions").join(&exec_id).join("attempts").join(&id);

    Ok(Attempt {
        id,
        execution_id: exec_id,
        handler_instance_id: r.get(2)?,
        parent_attempt_id: r.get(3)?,
        run_type: RunType { source, simulation },
        state,
        success_count: r.get::<_, i64>(8)? as u64,
        failed_count: r.get::<_, i64>(9)? as u64,
        aborted_reason: r.get(10)?,
        started_at: parse_rfc3339(r.get::<_, String>(11)?)?,
        ended_at: r.get::<_, Option<String>>(12)?.map(parse_rfc3339).transpose()?,
        dir,
    })
}

fn row_to_handler_instance(r: &rusqlite::Row<'_>) -> rusqlite::Result<HandlerInstance> {
    Ok(HandlerInstance {
        id: r.get(0)?,
        handler_id: r.get(1)?,
        manifest_hash: r.get(2)?,
        source_snapshot_dir: PathBuf::from(r.get::<_, String>(3)?),
        binary_hash: r.get(4)?,
        created_at: parse_rfc3339(r.get::<_, String>(5)?)?,
    })
}

fn parse_rfc3339(s: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
            )
        })
}

/// Stream-hash + count rows in one pass.
///
/// `is_jsonl=true` → every non-empty line is a data row (no header).
/// `is_jsonl=false` → CSV; the first line is the header, subtracted off.
fn hash_and_count_rows(path: &Path, is_jsonl: bool) -> Result<(String, u64)> {
    let mut f = fs::File::open(path).map_err(CoreError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut newlines: u64 = 0;
    let mut last_was_newline = true;
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf).map_err(CoreError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        for &b in &buf[..n] {
            if b == b'\n' {
                newlines += 1;
                last_was_newline = true;
            } else {
                last_was_newline = false;
            }
        }
        total += n as u64;
    }
    // Count the trailing (newline-less) line if any.
    let mut lines = newlines;
    if total > 0 && !last_was_newline {
        lines += 1;
    }
    // CSV: first line is header → subtract 1. JSONL: every line is a row.
    let row_count = if is_jsonl {
        lines
    } else {
        lines.saturating_sub(1)
    };
    let hash = format!("sha256:{:x}", hasher.finalize());
    Ok((hash, row_count))
}

fn write_manifest(dir: &Path, exec: &Execution) -> Result<()> {
    let tmp = dir.join("manifest.json.tmp");
    let target = dir.join("manifest.json");
    let json = serde_json::to_string_pretty(exec)
        .map_err(|e| CoreError::Store(format!("serialize manifest: {e}")))?;
    fs::write(&tmp, json).map_err(CoreError::Io)?;
    fs::rename(&tmp, &target).map_err(CoreError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_csv(dir: &Path, name: &str, rows: usize) -> PathBuf {
        let p = dir.join(name);
        let mut s = String::from("id,value\n");
        for i in 0..rows {
            s.push_str(&format!("{i},v{i}\n"));
        }
        fs::write(&p, s).unwrap();
        p
    }

    #[test]
    fn create_and_get_execution() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let csv = write_csv(src.path(), "in.csv", 5);

        let mut store = ExecutionStore::open(home.path()).unwrap();
        let created = store
            .create_execution(NewExecution {
                name: Some("first".into()),
                input_csv_id: "c_x".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();

        assert_eq!(created.input_row_count, 5);
        assert!(created.input_csv_hash.starts_with("sha256:"));
        assert_eq!(created.state, ExecutionState::Open);
        assert!(created.dir.join("input.csv").is_file());
        assert!(created.dir.join("manifest.json").is_file());

        let fetched = store.get_execution(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.input_row_count, 5);
    }

    #[test]
    fn list_executions_orders_by_created_desc() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let mut store = ExecutionStore::open(home.path()).unwrap();
        for i in 0..3 {
            let csv = write_csv(src.path(), &format!("in{i}.csv"), 2);
            store
                .create_execution(NewExecution {
                    name: Some(format!("e{i}")),
                    input_csv_id: "c_x".into(),
                    input_csv_path: csv,
                    current_handler_instance_id: None,
                })
                .unwrap();
            // ensure created_at differs in monotonic clock
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let all = store.list_executions().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name.as_deref(), Some("e2"));
        assert_eq!(all[2].name.as_deref(), Some("e0"));
    }

    #[test]
    fn state_transition_writes_manifest_and_timestamps() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let csv = write_csv(src.path(), "in.csv", 1);

        let mut store = ExecutionStore::open(home.path()).unwrap();
        let created = store
            .create_execution(NewExecution {
                name: None,
                input_csv_id: "c_x".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
        let id = created.id.clone();

        let settled = store
            .set_execution_state(&id, ExecutionState::Settled, None)
            .unwrap();
        assert_eq!(settled.state, ExecutionState::Settled);
        assert!(settled.settled_at.is_some());

        let abandoned = store
            .set_execution_state(&id, ExecutionState::Abandoned, Some("user gave up".into()))
            .unwrap();
        assert_eq!(abandoned.abandoned_reason.as_deref(), Some("user gave up"));

        // manifest.json reflects the latest state
        let manifest_bytes = fs::read(created.dir.join("manifest.json")).unwrap();
        let parsed: Execution = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(parsed.state, ExecutionState::Abandoned);
    }

    #[test]
    fn handler_instance_is_content_addressed() {
        let home = tempdir().unwrap();
        let mut store = ExecutionStore::open(home.path()).unwrap();

        let a = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_1".into(),
                manifest_hash: "sha256:aaa".into(),
                source_snapshot_dir: PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();
        let b = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_1".into(),
                manifest_hash: "sha256:aaa".into(),
                source_snapshot_dir: PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();
        assert_eq!(a.id, b.id, "same content -> same instance");

        let c = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_1".into(),
                manifest_hash: "sha256:bbb".into(),
                source_snapshot_dir: PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();
        assert_ne!(a.id, c.id);
    }

    #[test]
    fn attempt_lifecycle_bumps_exec_to_iterating() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let csv = write_csv(src.path(), "in.csv", 10);

        let mut store = ExecutionStore::open(home.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: None,
                input_csv_id: "c_x".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
        assert_eq!(exec.state, ExecutionState::Open);

        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_1".into(),
                manifest_hash: "sha256:m".into(),
                source_snapshot_dir: PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();

        let attempt = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id.clone(),
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Sampled { size: 2 },
                    simulation: Simulation::Real,
                },
            })
            .unwrap();
        assert!(attempt.dir.is_dir());
        assert_eq!(attempt.state, AttemptState::Running);

        let finished = store
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
        assert_eq!(finished.state, AttemptState::Completed);
        assert_eq!(finished.success_count, 2);

        let bumped = store.get_execution(&exec.id).unwrap().unwrap();
        assert_eq!(bumped.state, ExecutionState::Iterating);

        let list = store.list_attempts_for_execution(&exec.id).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn attempt_rejected_for_closed_execution() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let csv = write_csv(src.path(), "in.csv", 1);

        let mut store = ExecutionStore::open(home.path()).unwrap();
        let exec = store
            .create_execution(NewExecution {
                name: None,
                input_csv_id: "c_x".into(),
                input_csv_path: csv,
                current_handler_instance_id: None,
            })
            .unwrap();
        store
            .set_execution_state(&exec.id, ExecutionState::Closed, None)
            .unwrap();
        let hi = store
            .register_handler_instance(NewHandlerInstance {
                handler_id: "h_1".into(),
                manifest_hash: "sha256:m".into(),
                source_snapshot_dir: PathBuf::from("/tmp/snap"),
                binary_hash: None,
            })
            .unwrap();
        let err = store
            .create_attempt(NewAttempt {
                execution_id: exec.id.clone(),
                handler_instance_id: hi.id,
                parent_attempt_id: None,
                run_type: RunType {
                    source: Source::Full,
                    simulation: Simulation::Real,
                },
            })
            .unwrap_err();
        assert!(matches!(err, CoreError::Store(_)));
    }

    #[test]
    fn rejects_missing_input_csv() {
        let home = tempdir().unwrap();
        let mut store = ExecutionStore::open(home.path()).unwrap();
        let err = store
            .create_execution(NewExecution {
                name: None,
                input_csv_id: "c_x".into(),
                input_csv_path: PathBuf::from("/no/such/file.csv"),
                current_handler_instance_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, CoreError::Store(_)));
    }

    #[test]
    fn schema_version_is_exposed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ExecutionStore::open(tmp.path()).unwrap();
        assert!(store.schema_version() >= 1);
    }
}
