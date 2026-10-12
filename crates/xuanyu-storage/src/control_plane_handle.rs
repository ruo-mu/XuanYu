use crate::control_plane::{ControlPlaneStore, UpsertTaskInput};
use crate::postgres_control_plane::PostgresControlPlaneStore;
use serde_json::Value;
use std::sync::Arc;

pub enum ControlPlaneBackend {
    Sqlite(Arc<ControlPlaneStore>),
    Postgres(Arc<PostgresControlPlaneStore>),
}

impl ControlPlaneBackend {
    pub fn upsert_task(&self, input: UpsertTaskInput<'_>) -> anyhow::Result<()> {
        match self {
            Self::Sqlite(store) => store.upsert_task(input),
            Self::Postgres(store) => store.upsert_task(input),
        }
    }

    pub fn append_event(
        &self,
        task_id: &str,
        event_type: &str,
        payload: &Value,
        created_at: i64,
    ) -> anyhow::Result<()> {
        match self {
            Self::Sqlite(store) => store.append_event(task_id, event_type, payload, created_at),
            Self::Postgres(store) => store.append_event(task_id, event_type, payload, created_at),
        }
    }

    pub fn list_tasks(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        match self {
            Self::Sqlite(store) => store.list_tasks(session_id, limit),
            Self::Postgres(store) => store.list_tasks(session_id, limit),
        }
    }

    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            Self::Postgres(_) => "postgres",
        }
    }
}

pub type ControlPlaneHandle = Arc<ControlPlaneBackend>;
