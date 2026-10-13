use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};

/// 控制面任务写入参数。
pub struct UpsertTaskInput<'a> {
    pub task_id: &'a str,
    pub session_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub control_plane_task_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub status: &'a str,
    pub ledger: &'a Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct PostgresControlPlaneConfig {
    pub connection_string: String,
    pub schema: String,
    pub psql_path: PathBuf,
}

/// 运行时事件日志记录。
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
    pub payload: Value,
}

pub struct PostgresControlPlaneStore {
    config: PostgresControlPlaneConfig,
    lock: Mutex<()>,
}

impl PostgresControlPlaneStore {
    pub fn open(config: PostgresControlPlaneConfig) -> anyhow::Result<Self> {
        let store = Self {
            config,
            lock: Mutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let schema = sql_identifier(&self.config.schema, "public");
        let sql = format!(
            r#"
            CREATE SCHEMA IF NOT EXISTS {schema};
            CREATE TABLE IF NOT EXISTS {schema}.control_plane_tasks (
                task_id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT,
                control_plane_task_id TEXT,
                agent_id TEXT,
                status TEXT NOT NULL,
                ledger_json JSONB NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cp_tasks_session ON {schema}.control_plane_tasks(session_id);
            CREATE INDEX IF NOT EXISTS idx_cp_tasks_cp_id ON {schema}.control_plane_tasks(control_plane_task_id);
            CREATE TABLE IF NOT EXISTS {schema}.control_plane_events (
                id BIGSERIAL PRIMARY KEY,
                task_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json JSONB NOT NULL,
                created_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cp_events_task ON {schema}.control_plane_events(task_id);
            CREATE TABLE IF NOT EXISTS {schema}.runtime_events (
                sequence BIGINT PRIMARY KEY,
                timestamp TIMESTAMPTZ NOT NULL,
                kind TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                lane TEXT,
                payload_json JSONB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_runtime_events_session ON {schema}.runtime_events(session_id);
            "#
        );
        self.exec_sql(&sql)?;
        Ok(())
    }

    pub fn upsert_task(&self, input: UpsertTaskInput<'_>) -> anyhow::Result<()> {
        let UpsertTaskInput {
            task_id,
            session_id,
            run_id,
            control_plane_task_id,
            agent_id,
            status,
            ledger,
            created_at,
            updated_at,
        } = input;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres control plane lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let ledger_json = serde_json::to_string(ledger)?;
        let sql = format!(
            "INSERT INTO {schema}.control_plane_tasks (task_id, session_id, run_id, control_plane_task_id, agent_id, status, ledger_json, created_at, updated_at)
             VALUES ({task_id}, {session_id}, {run_id}, {cp_task_id}, {agent_id}, {status}, {ledger_json}::jsonb, {created_at}, {updated_at})
             ON CONFLICT (task_id) DO UPDATE SET
               session_id = EXCLUDED.session_id,
               run_id = EXCLUDED.run_id,
               control_plane_task_id = EXCLUDED.control_plane_task_id,
               agent_id = EXCLUDED.agent_id,
               status = EXCLUDED.status,
               ledger_json = EXCLUDED.ledger_json,
               updated_at = EXCLUDED.updated_at",
            task_id = sql_literal(task_id),
            session_id = sql_optional_literal(session_id),
            run_id = sql_optional_literal(run_id),
            cp_task_id = sql_optional_literal(control_plane_task_id),
            agent_id = sql_optional_literal(agent_id),
            status = sql_literal(status),
            ledger_json = sql_literal(&ledger_json),
            created_at = created_at,
            updated_at = updated_at,
        );
        self.exec_sql(&sql)?;
        Ok(())
    }

    pub fn append_event(
        &self,
        task_id: &str,
        event_type: &str,
        payload: &Value,
        created_at: i64,
    ) -> anyhow::Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres control plane lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let payload_json = serde_json::to_string(payload)?;
        let sql = format!(
            "INSERT INTO {schema}.control_plane_events (task_id, event_type, payload_json, created_at)
             VALUES ({task_id}, {event_type}, {payload_json}::jsonb, {created_at})",
            task_id = sql_literal(task_id),
            event_type = sql_literal(event_type),
            payload_json = sql_literal(&payload_json),
            created_at = created_at,
        );
        self.exec_sql(&sql)?;
        Ok(())
    }

    pub fn list_tasks(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres control plane lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let sql = if let Some(session_id) = session_id {
            format!(
                "SELECT COALESCE(json_agg(ledger_json ORDER BY updated_at DESC), '[]'::json)
                 FROM (SELECT ledger_json FROM {schema}.control_plane_tasks
                       WHERE session_id = {session_id}
                       ORDER BY updated_at DESC LIMIT {limit}) t",
                session_id = sql_literal(session_id),
                limit = limit.max(1),
            )
        } else {
            format!(
                "SELECT COALESCE(json_agg(ledger_json ORDER BY updated_at DESC), '[]'::json)
                 FROM (SELECT ledger_json FROM {schema}.control_plane_tasks
                       ORDER BY updated_at DESC LIMIT {limit}) t",
                limit = limit.max(1),
            )
        };
        let output = self.query_scalar(&sql)?;
        let value: Value = serde_json::from_str(output.trim()).unwrap_or(Value::Array(vec![]));
        Ok(value
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect())
    }

    /// 批量写入运行时事件；序列号冲突说明该事件已落库，直接跳过。
    pub fn append_journal_events(&self, records: &[JournalEventRecord]) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres journal lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
            let payload = serde_json::to_string(&record.payload)?;
            rows.push(format!(
                "({sequence}, {timestamp}::timestamptz, {kind}, {session_id}, {turn_id}, {lane}, {payload}::jsonb)",
                sequence = record.sequence,
                timestamp = sql_literal(&record.timestamp.to_rfc3339()),
                kind = sql_literal(&record.kind),
                session_id = sql_optional_literal(record.session_id.as_deref()),
                turn_id = sql_optional_literal(record.turn_id.as_deref()),
                lane = sql_optional_literal(record.lane.as_deref()),
                payload = sql_literal(&payload),
            ));
        }
        let sql = format!(
            "INSERT INTO {schema}.runtime_events (sequence, timestamp, kind, session_id, turn_id, lane, payload_json)
             VALUES {} ON CONFLICT (sequence) DO NOTHING",
            rows.join(",")
        );
        self.exec_sql(&sql)
    }

    pub fn max_journal_sequence(&self) -> anyhow::Result<u64> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres journal lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let value = self.query_scalar(&format!(
            "SELECT COALESCE(MAX(sequence), 0) FROM {schema}.runtime_events"
        ))?;
        Ok(value.trim().parse::<i64>().unwrap_or(0).max(0) as u64)
    }

    pub fn replay_journal_since(
        &self,
        last_sequence: u64,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        lane: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<JournalEventRecord>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("postgres journal lock"))?;
        let schema = sql_identifier(&self.config.schema, "public");
        let sql = format!(
            "SELECT COALESCE(json_agg(json_build_object(
                 'sequence', sequence,
                 'timestamp', timestamp,
                 'kind', kind,
                 'sessionId', session_id,
                 'turnId', turn_id,
                 'lane', lane,
                 'payload', payload_json
             ) ORDER BY sequence ASC), '[]'::json)
             FROM (SELECT sequence, timestamp, kind, session_id, turn_id, lane, payload_json
                   FROM {schema}.runtime_events WHERE sequence > {last_sequence}
                   ORDER BY sequence ASC LIMIT {limit}) t",
            limit = limit.max(1),
        );
        let output = self.query_scalar(&sql)?;
        let mut records: Vec<JournalEventRecord> = serde_json::from_str(&output)?;
        records.retain(|record| {
            session_id
                .map(|expected| record.session_id.as_deref() == Some(expected))
                .unwrap_or(true)
                && turn_id
                    .map(|expected| record.turn_id.as_deref() == Some(expected))
                    .unwrap_or(true)
                && lane
                    .map(|expected| record.lane.as_deref() == Some(expected))
                    .unwrap_or(true)
        });
        Ok(records)
    }

    fn exec_sql(&self, sql: &str) -> anyhow::Result<()> {
        self.query_scalar(sql)?;
        Ok(())
    }

    fn query_scalar(&self, sql: &str) -> anyhow::Result<String> {
        let output = Command::new(&self.config.psql_path)
            .arg("-X")
            .arg("-q")
            .arg("-A")
            .arg("-t")
            .arg("--set")
            .arg("ON_ERROR_STOP=1")
            .arg("--pset")
            .arg("footer=off")
            .arg("--dbname")
            .arg(&self.config.connection_string)
            .arg("-c")
            .arg(sql)
            .output()
            .map_err(|err| {
                anyhow::anyhow!(
                    "Cannot execute psql '{}': {err}",
                    self.config.psql_path.display()
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("psql failed: {stderr}");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn sql_identifier(value: &str, fallback: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_optional_literal(value: Option<&str>) -> String {
    value
        .map(sql_literal)
        .unwrap_or_else(|| "NULL".to_string())
}

/// 运行时事件日志：PostgreSQL 尚未挂载时先缓冲，挂载后落库。
const JOURNAL_BATCH_MAX: usize = 64;
const JOURNAL_PENDING_MAX: usize = 1024;

pub struct PostgresEventJournal {
    store: RwLock<Option<Arc<PostgresControlPlaneStore>>>,
    pending: Mutex<Vec<JournalEventRecord>>,
}

impl Default for PostgresEventJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl PostgresEventJournal {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// 挂载存储：先把缓冲中的事件落库，再切换到直写。
    pub fn attach(&self, store: Arc<PostgresControlPlaneStore>) -> anyhow::Result<()> {
        let buffered: Vec<JournalEventRecord> = {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("journal lock"))?;
            std::mem::take(&mut *pending)
        };
        store.append_journal_events(&buffered)?;
        *self
            .store
            .write()
            .map_err(|_| anyhow::anyhow!("journal lock"))? = Some(store);
        Ok(())
    }

    pub fn append(&self, record: &JournalEventRecord) -> anyhow::Result<()> {
        let store = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("journal lock"))?
            .clone();
        let Some(store) = store else {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("journal lock"))?;
            if pending.len() >= JOURNAL_PENDING_MAX {
                pending.remove(0);
            }
            pending.push(record.clone());
            return Ok(());
        };
        let batch = {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("journal lock"))?;
            pending.push(record.clone());
            if pending.len() >= JOURNAL_BATCH_MAX {
                std::mem::take(&mut *pending)
            } else {
                Vec::new()
            }
        };
        store.append_journal_events(&batch)
    }

    pub fn max_sequence(&self) -> anyhow::Result<u64> {
        self.store()?.max_journal_sequence()
    }

    pub fn replay_since(
        &self,
        last_sequence: u64,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        lane: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<JournalEventRecord>> {
        self.store()?
            .replay_journal_since(last_sequence, session_id, turn_id, lane, limit)
    }

    fn store(&self) -> anyhow::Result<Arc<PostgresControlPlaneStore>> {
        self.store
            .read()
            .map_err(|_| anyhow::anyhow!("journal lock"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("postgres journal is not attached"))
    }
}

