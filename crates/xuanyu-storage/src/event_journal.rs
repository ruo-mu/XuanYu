use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const JOURNAL_BATCH_MAX: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEventRecord {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    pub payload: serde_json::Value,
}

struct JournalInner {
    conn: Connection,
    pending: Vec<JournalEventRecord>,
}

pub struct EventJournalStore {
    _db_path: PathBuf,
    inner: Mutex<JournalInner>,
}

impl EventJournalStore {
    pub fn open(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS runtime_events (
                sequence INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                kind TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                lane TEXT,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_runtime_events_session ON runtime_events(session_id);
            "#,
        )?;
        Ok(Self {
            _db_path: db_path,
            inner: Mutex::new(JournalInner {
                conn,
                pending: Vec::with_capacity(JOURNAL_BATCH_MAX),
            }),
        })
    }

    pub fn max_sequence(&self) -> anyhow::Result<u64> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("journal lock"))?;
        Self::flush_pending_locked(&mut guard)?;
        let value: i64 = guard.conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM runtime_events",
            [],
            |row| row.get(0),
        )?;
        Ok(value.max(0) as u64)
    }

    pub fn append(&self, record: &JournalEventRecord) -> anyhow::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("journal lock"))?;
        guard.pending.push(record.clone());
        if guard.pending.len() >= JOURNAL_BATCH_MAX {
            Self::flush_pending_locked(&mut guard)?;
        }
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("journal lock"))?;
        Self::flush_pending_locked(&mut guard)
    }

    fn flush_pending_locked(inner: &mut JournalInner) -> anyhow::Result<()> {
        if inner.pending.is_empty() {
            return Ok(());
        }
        let tx = inner.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO runtime_events (sequence, timestamp, kind, session_id, turn_id, lane, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for record in inner.pending.drain(..) {
                let payload = serde_json::to_string(&record.payload)?;
                stmt.execute(params![
                    record.sequence as i64,
                    record.timestamp.to_rfc3339(),
                    record.kind,
                    record.session_id,
                    record.turn_id,
                    record.lane,
                    payload,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn replay_since(
        &self,
        last_sequence: u64,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        lane: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<JournalEventRecord>> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("journal lock"))?;
        Self::flush_pending_locked(&mut guard)?;
        let mut stmt = guard.conn.prepare(
            "SELECT sequence, timestamp, kind, session_id, turn_id, lane, payload
             FROM runtime_events WHERE sequence > ?1 ORDER BY sequence ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![last_sequence as i64, limit as i64], |row| {
            let sequence: i64 = row.get(0)?;
            let timestamp: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let session_id: Option<String> = row.get(3)?;
            let turn_id: Option<String> = row.get(4)?;
            let lane: Option<String> = row.get(5)?;
            let payload: String = row.get(6)?;
            Ok(JournalEventRecord {
                sequence: sequence as u64,
                timestamp: DateTime::parse_from_rfc3339(&timestamp)
                    .map(|value| value.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                kind,
                session_id,
                turn_id,
                lane,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows
            .filter_map(Result::ok)
            .filter(|record| {
                session_id
                    .map(|expected| record.session_id.as_deref() == Some(expected))
                    .unwrap_or(true)
                    && turn_id
                        .map(|expected| record.turn_id.as_deref() == Some(expected))
                        .unwrap_or(true)
                    && lane
                        .map(|expected| record.lane.as_deref() == Some(expected))
                        .unwrap_or(true)
            })
            .collect())
    }
}
