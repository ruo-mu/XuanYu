use crate::control_plane::UpsertTaskInput;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PostgresControlPlaneConfig {
    pub connection_string: String,
    pub schema: String,
    pub psql_path: PathBuf,
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

pub fn postgres_control_plane_from_config(
    config: &serde_json::Value,
) -> Option<PostgresControlPlaneConfig> {
    let cp_backend = config
        .get("controlPlane")
        .and_then(|value| value.get("backend"))
        .and_then(|value| value.as_str());
    let memory_backend = config
        .get("memory")
        .and_then(|value| value.get("backend"))
        .and_then(|value| value.as_str());
    let use_postgres = cp_backend == Some("postgres")
        || (cp_backend.is_none() && memory_backend == Some("postgres"));
    if !use_postgres {
        return None;
    }
    let connection_string = config
        .get("controlPlane")
        .and_then(|value| value.get("connectionString"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            config
                .get("memory")
                .and_then(|value| value.get("postgres"))
                .and_then(|value| value.get("connectionString"))
                .and_then(|value| value.as_str())
        })?
        .trim()
        .to_string();
    if connection_string.is_empty()
        || !(connection_string.starts_with("postgres://")
            || connection_string.starts_with("postgresql://"))
    {
        return None;
    }
    let schema = config
        .get("controlPlane")
        .and_then(|value| value.get("schema"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            config
                .get("memory")
                .and_then(|value| value.get("postgres"))
                .and_then(|value| value.get("schema"))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("public")
        .to_string();
    let psql_path = config
        .get("controlPlane")
        .and_then(|value| value.get("psqlPath"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            config
                .get("memory")
                .and_then(|value| value.get("postgres"))
                .and_then(|value| value.get("psqlPath"))
                .and_then(|value| value.as_str())
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from("psql.exe")
            } else {
                PathBuf::from("psql")
            }
        });
    Some(PostgresControlPlaneConfig {
        connection_string,
        schema: sql_identifier(&schema, "public"),
        psql_path,
    })
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
