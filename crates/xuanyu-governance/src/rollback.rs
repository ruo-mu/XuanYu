use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackActionRecord {
    pub id: String,
    pub description: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct RollbackRegistry {
    actions: RwLock<Vec<RollbackActionRecord>>,
}

impl RollbackRegistry {
    pub async fn register(
        &self,
        description: impl Into<String>,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> RollbackActionRecord {
        let action = RollbackActionRecord {
            id: format!("rollback_{}", Uuid::new_v4()),
            description: description.into(),
            kind: kind.into(),
            payload,
            created_at: Utc::now(),
        };
        self.actions.write().await.push(action.clone());
        action
    }

    pub async fn list(&self) -> Vec<RollbackActionRecord> {
        self.actions.read().await.clone()
    }

    pub async fn find(&self, id: &str) -> Option<RollbackActionRecord> {
        self.actions
            .read()
            .await
            .iter()
            .find(|action| action.id == id)
            .cloned()
    }

    pub async fn remove(&self, id: &str) -> Option<RollbackActionRecord> {
        let mut actions = self.actions.write().await;
        let index = actions.iter().position(|action| action.id == id)?;
        Some(actions.remove(index))
    }
}
