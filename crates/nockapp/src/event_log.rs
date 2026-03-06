use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use thiserror::Error;

const EVENT_LOG_SCHEMA_VERSION: i64 = 1;

const CREATE_META_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value BLOB NOT NULL
);
"#;

const SCHEMA_V1_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_num INTEGER NOT NULL UNIQUE,
  job_jam BLOB NOT NULL,
  wire_source TEXT NOT NULL,
  wire_version INTEGER NOT NULL,
  wire_tags_json TEXT NOT NULL,
  cause_hash BLOB NOT NULL,
  job_hash BLOB NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS events_event_num_idx ON events(event_num);

CREATE TABLE IF NOT EXISTS snapshots (
  snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK(kind IN ('epoch','rotating')),
  state TEXT NOT NULL CHECK(state IN ('writing','ready','failed','retired')),
  event_num INTEGER NOT NULL,
  pma_path TEXT NOT NULL,
  manifest_path TEXT NOT NULL,
  alloc_words INTEGER NOT NULL,
  kernel_root_raw INTEGER NOT NULL,
  cold_offset INTEGER NOT NULL,
  used_blake3 BLOB NOT NULL,
  structure_blake3 BLOB,
  created_at_ms INTEGER NOT NULL,
  activated_at_ms INTEGER,
  base_snapshot_id INTEGER,
  timestamp_tag TEXT NOT NULL,
  UNIQUE(kind, timestamp_tag)
);

CREATE INDEX IF NOT EXISTS snapshots_kind_ts_idx ON snapshots(kind, timestamp_tag DESC);
CREATE INDEX IF NOT EXISTS snapshots_event_idx ON snapshots(event_num DESC);
"#;

#[derive(Debug, Clone)]
pub(crate) struct EventLogConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct EventLogEntry {
    pub event_num: u64,
    pub job_jam: Vec<u8>,
    pub wire_source: String,
    pub wire_version: i64,
    pub wire_tags_json: String,
    pub cause_hash: Vec<u8>,
    pub job_hash: Vec<u8>,
    pub created_at_ms: i64,
}

#[derive(Debug, Error)]
pub(crate) enum EventLogError {
    #[error("event log io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unsupported event-log schema version {found}, expected at most {supported}")]
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    #[error("invalid event number {0}")]
    InvalidEventNum(i64),
}

pub(crate) struct EventLog {
    path: PathBuf,
    conn: Connection,
}

impl EventLog {
    pub(crate) fn open(config: EventLogConfig) -> Result<Self, EventLogError> {
        if let Some(parent) = config.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(&config.path)?;
        Self::configure(&conn)?;
        Self::migrate(&mut conn)?;
        Ok(Self {
            path: config.path,
            conn,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn append_event(&mut self, event: &EventLogEntry) -> Result<(), EventLogError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            r#"
INSERT INTO events (
  event_num,
  job_jam,
  wire_source,
  wire_version,
  wire_tags_json,
  cause_hash,
  job_hash,
  created_at_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
"#,
            params![
                i64::try_from(event.event_num).map_err(|_| {
                    EventLogError::InvalidEventNum(event.event_num.try_into().unwrap_or(i64::MAX))
                })?,
                &event.job_jam,
                &event.wire_source,
                event.wire_version,
                &event.wire_tags_json,
                &event.cause_hash,
                &event.job_hash,
                event.created_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn max_event_num(&self) -> Result<Option<u64>, EventLogError> {
        let max_event_num =
            self.conn
                .query_row("SELECT MAX(event_num) FROM events", [], |row| {
                    row.get::<_, Option<i64>>(0)
                })?;
        max_event_num
            .map(|value| u64::try_from(value).map_err(|_| EventLogError::InvalidEventNum(value)))
            .transpose()
    }

    fn configure(conn: &Connection) -> Result<(), EventLogError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "foreign_keys", 1)?;
        Ok(())
    }

    fn migrate(conn: &mut Connection) -> Result<(), EventLogError> {
        conn.execute_batch(CREATE_META_SQL)?;
        let current_version = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if current_version > EVENT_LOG_SCHEMA_VERSION {
            return Err(EventLogError::UnsupportedSchemaVersion {
                found: current_version,
                supported: EVENT_LOG_SCHEMA_VERSION,
            });
        }
        if current_version < 1 {
            let tx = conn.transaction()?;
            tx.execute_batch(SCHEMA_V1_SQL)?;
            tx.execute(
                r#"
INSERT INTO meta (key, value)
VALUES ('schema_version', ?1)
ON CONFLICT(key) DO UPDATE SET value = excluded.value
"#,
                params![EVENT_LOG_SCHEMA_VERSION],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn sample_entry(event_num: u64) -> EventLogEntry {
        EventLogEntry {
            event_num,
            job_jam: vec![0, 1, 2, event_num as u8],
            wire_source: "sys".to_string(),
            wire_version: 1,
            wire_tags_json: "[]".to_string(),
            cause_hash: vec![3; 32],
            job_hash: vec![4; 32],
            created_at_ms: 42,
        }
    }

    #[test]
    fn initializes_schema_and_appends_events() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("event-log.sqlite3");
        let mut log = EventLog::open(EventLogConfig { path }).expect("open event log");
        assert_eq!(log.max_event_num().expect("max event num"), None);

        log.append_event(&sample_entry(1)).expect("append event 1");
        log.append_event(&sample_entry(2)).expect("append event 2");

        assert_eq!(log.max_event_num().expect("max event num"), Some(2));
    }
}
