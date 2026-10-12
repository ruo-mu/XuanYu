use chrono::Utc;
use std::collections::BTreeMap;
use tokio::sync::RwLock;
use uuid::Uuid;
use xuanyu_protocol::SessionMailboxMessage;

#[derive(Default)]
pub struct SessionMailboxRegistry {
    by_session: RwLock<BTreeMap<String, Vec<SessionMailboxMessage>>>,
}

impl SessionMailboxRegistry {
    pub async fn push(
        &self,
        session_id: &str,
        from_participant_id: Option<String>,
        thread_id: Option<String>,
        payload: serde_json::Value,
    ) -> SessionMailboxMessage {
        let message = SessionMailboxMessage {
            id: format!("mail_{}", Uuid::new_v4().simple()),
            from_participant_id,
            thread_id,
            payload,
            created_at: Utc::now(),
        };
        self.by_session
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push(message.clone());
        message
    }

    pub async fn list(&self, session_id: &str) -> Vec<SessionMailboxMessage> {
        self.by_session
            .read()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn drain(&self, session_id: &str) -> Vec<SessionMailboxMessage> {
        self.by_session
            .write()
            .await
            .remove(session_id)
            .unwrap_or_default()
    }

    pub async fn clear(&self, session_id: &str) {
        self.by_session.write().await.remove(session_id);
    }
}
