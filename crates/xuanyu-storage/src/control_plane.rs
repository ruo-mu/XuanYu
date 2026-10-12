use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct ControlPlaneStore {
    db_path: PathBuf,
    lock: Mutex<()>,
}

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

impl ControlPlaneStore {
    pub fn open(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self {
            db_path,
            lock: Mutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS control_plane_tasks (
                task_id TEXT PRIMARY KEY,
                session_id TEXT,
                run_id TEXT,
                control_plane_task_id TEXT,
                agent_id TEXT,
                status TEXT NOT NULL,
                ledger_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cp_tasks_session ON control_plane_tasks(session_id);
            CREATE INDEX IF NOT EXISTS idx_cp_tasks_cp_id ON control_plane_tasks(control_plane_task_id);
            CREATE TABLE IF NOT EXISTS control_plane_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cp_events_task ON control_plane_events(task_id);
            "#,
        )?;
        Ok(())
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
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
            .map_err(|_| anyhow::anyhow!("control plane lock"))?;
        let conn = self.connect()?;
        let ledger_json = serde_json::to_string(ledger)?;
        conn.execute(
            "INSERT INTO control_plane_tasks (task_id, session_id, run_id, control_plane_task_id, agent_id, status, ledger_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(task_id) DO UPDATE SET
               session_id=excluded.session_id,
               run_id=excluded.run_id,
               control_plane_task_id=excluded.control_plane_task_id,
               agent_id=excluded.agent_id,
               status=excluded.status,
               ledger_json=excluded.ledger_json,
               updated_at=excluded.updated_at",
            params![
                task_id,
                session_id,
                run_id,
                control_plane_task_id,
                agent_id,
                status,
                ledger_json,
                created_at,
                updated_at,
            ],
        )?;
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
            .map_err(|_| anyhow::anyhow!("control plane lock"))?;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO control_plane_events (task_id, event_type, payload_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                task_id,
                event_type,
                serde_json::to_string(payload)?,
                created_at,
            ],
        )?;
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
            .map_err(|_| anyhow::anyhow!("control plane lock"))?;
        let conn = self.connect()?;
        let mut sql =
            String::from("SELECT ledger_json FROM control_plane_tasks WHERE 1=1");
        if session_id.is_some() {
            sql.push_str(" AND session_id = ?1");
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ?2");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![session_id, limit as i64], |row| {
            let text: String = row.get(0)?;
            Ok(text)
        })?;
        Ok(rows
            .filter_map(Result::ok)
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect())
    }
}
