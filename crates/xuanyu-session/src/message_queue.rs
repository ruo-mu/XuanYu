use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::RwLock;
use xuanyu_protocol::ChatAttachment;

/// Mirrors `sessionMessageQueueApi.ts` `QueuedSessionMessage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedSessionMessage {
    pub content: String,
    pub queued_at: i64,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ChatAttachment>>,
    #[serde(default)]
    pub deep_thinking: bool,
    #[serde(default, rename = "fallbackOnSilent", alias = "fallbackOnBrainSilent", alias = "fallback_on_brain_silent")]
    pub fallback_on_silent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_intent: Option<String>,
}

#[derive(Default)]
pub struct SessionMessageQueue {
    queues: RwLock<BTreeMap<String, Vec<QueuedSessionMessage>>>,
    processing: RwLock<BTreeMap<String, bool>>,
}

impl SessionMessageQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_processing(&self, session_id: &str) -> bool {
        self.processing
            .read()
            .await
            .get(session_id)
            .copied()
            .unwrap_or(false)
    }

    pub async fn set_processing(&self, session_id: &str, processing: bool) {
        let mut map = self.processing.write().await;
        if processing {
            map.insert(session_id.to_string(), true);
        } else {
            map.remove(session_id);
        }
    }

    /// `queueSessionMessage` — FIFO sorted by queued_at.
    pub async fn enqueue(&self, session_id: &str, message: QueuedSessionMessage) {
        let mut queues = self.queues.write().await;
        let queue = queues.entry(session_id.to_string()).or_default();
        queue.push(message);
        queue.sort_by_key(|item| item.queued_at);
    }

    /// `drainSessionQueue` — shift one message if session is idle.
    pub async fn dequeue_if_idle(&self, session_id: &str) -> Option<QueuedSessionMessage> {
        if self.is_processing(session_id).await {
            return None;
        }
        let mut queues = self.queues.write().await;
        let queue = queues.get_mut(session_id)?;
        let next = queue.first()?.clone();
        queue.remove(0);
        if queue.is_empty() {
            queues.remove(session_id);
        }
        Some(next)
    }

    pub async fn queue_len(&self, session_id: &str) -> usize {
        self.queues
            .read()
            .await
            .get(session_id)
            .map(Vec::len)
            .unwrap_or(0)
    }

    pub fn build_queued_message(
        content: String,
        turn_id: String,
        attachments: Option<Vec<ChatAttachment>>,
        deep_thinking: bool,
        fallback_on_silent: bool,
        intent_source: Option<String>,
        force_intent: Option<String>,
    ) -> QueuedSessionMessage {
        QueuedSessionMessage {
            content,
            queued_at: Utc::now().timestamp_millis(),
            turn_id,
            attachments,
            deep_thinking,
            fallback_on_silent,
            intent_source,
            force_intent,
        }
    }
}
