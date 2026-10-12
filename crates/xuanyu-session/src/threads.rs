use chrono::Utc;
use std::collections::BTreeMap;
use tokio::sync::RwLock;
use uuid::Uuid;
use xuanyu_protocol::{SessionThread, SESSION_MAIN_THREAD_ID};

#[derive(Default)]
pub struct SessionThreadRegistry {
    by_session: RwLock<BTreeMap<String, BTreeMap<String, SessionThread>>>,
}

impl SessionThreadRegistry {
    pub async fn ensure_main(&self, session_id: &str) -> SessionThread {
        let mut sessions = self.by_session.write().await;
        let threads = sessions.entry(session_id.to_string()).or_default();
        if let Some(thread) = threads.get(SESSION_MAIN_THREAD_ID) {
            return thread.clone();
        }
        let thread = SessionThread {
            id: SESSION_MAIN_THREAD_ID.to_string(),
            title: "Main".to_string(),
            parent_thread_id: None,
            created_at: Utc::now(),
        };
        threads.insert(SESSION_MAIN_THREAD_ID.to_string(), thread.clone());
        thread
    }

    pub async fn create(
        &self,
        session_id: &str,
        title: impl Into<String>,
        parent_thread_id: Option<String>,
    ) -> SessionThread {
        let thread = SessionThread {
            id: format!("thread_{}", Uuid::new_v4().simple()),
            title: title.into(),
            parent_thread_id,
            created_at: Utc::now(),
        };
        self.by_session
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(thread.id.clone(), thread.clone());
        thread
    }

    pub async fn get(&self, session_id: &str, thread_id: &str) -> Option<SessionThread> {
        self.by_session
            .read()
            .await
            .get(session_id)?
            .get(thread_id)
            .cloned()
    }

    pub async fn list(&self, session_id: &str) -> Vec<SessionThread> {
        self.by_session
            .read()
            .await
            .get(session_id)
            .map(|threads| threads.values().cloned().collect())
            .unwrap_or_default()
    }
}
