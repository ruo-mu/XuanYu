use std::sync::Arc;

use xuanyu_eventbus::EventBus;
use xuanyu_protocol::{AppEvent, ChatAttachment, SessionId, SessionInfo, TurnId};
use xuanyu_storage::PostgresConversationStore;

use crate::message_queue::{QueuedSessionMessage, SessionMessageQueue};
use crate::session_bridge::SessionBridge;
use crate::SessionKernel;

/// Session facade — aggregates kernel + queue + processing (legacy `SessionManager` subset).
pub struct SessionManager {
    kernel: SessionKernel,
}

impl SessionManager {
    pub fn new(eventbus: EventBus, store: Arc<PostgresConversationStore>) -> Self {
        Self {
            kernel: SessionKernel::new(eventbus, store),
        }
    }

    pub fn kernel(&self) -> &SessionKernel {
        &self.kernel
    }

    pub fn message_queue(&self) -> &SessionMessageQueue {
        self.kernel.message_queue()
    }

    pub async fn is_processing(&self, session_id: &SessionId) -> bool {
        self.kernel.is_processing(session_id).await
    }

    pub async fn set_processing(&self, session_id: &SessionId, processing: bool) {
        self.kernel.set_processing(session_id, processing).await
    }

    pub async fn queue_message(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        content: String,
        attachments: Option<Vec<ChatAttachment>>,
        deep_thinking: bool,
    ) {
        let queued = QueuedSessionMessage {
            content,
            queued_at: chrono::Utc::now().timestamp_millis(),
            turn_id: turn_id.0.clone(),
            attachments,
            deep_thinking,
            fallback_on_silent: false,
            intent_source: None,
            force_intent: None,
        };
        self.kernel
            .message_queue()
            .enqueue(&session_id.0, queued)
            .await;
    }

    pub async fn dequeue_if_idle(&self, session_id: &SessionId) -> Option<QueuedSessionMessage> {
        self.kernel
            .message_queue()
            .dequeue_if_idle(&session_id.0)
            .await
    }

    pub async fn create_main_session(&self) -> anyhow::Result<SessionInfo> {
        self.kernel.create_main_session().await
    }

    pub async fn create_branch_session(
        &self,
        title: Option<String>,
        project_dir: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        self.kernel
            .create_branch_session(title, project_dir)
            .await
    }

    pub async fn list_participants(
        &self,
        session_id: &SessionId,
    ) -> Vec<xuanyu_protocol::SessionParticipant> {
        self.kernel.participants().list(&session_id.0).await
    }

    pub async fn join_participant(
        &self,
        session_id: &SessionId,
        role: impl Into<String>,
        display_name: Option<String>,
        external_id: Option<String>,
    ) -> xuanyu_protocol::SessionParticipant {
        self.kernel
            .participants()
            .join(&session_id.0, role, display_name, external_id)
            .await
    }

    pub async fn list_threads(
        &self,
        session_id: &SessionId,
    ) -> Vec<xuanyu_protocol::SessionThread> {
        self.kernel.threads().list(&session_id.0).await
    }

    pub async fn create_thread(
        &self,
        session_id: &SessionId,
        title: impl Into<String>,
        parent_thread_id: Option<String>,
    ) -> xuanyu_protocol::SessionThread {
        self.kernel
            .threads()
            .create(&session_id.0, title, parent_thread_id)
            .await
    }

    pub async fn isolation_scope(
        &self,
        session_id: &SessionId,
    ) -> Option<xuanyu_protocol::SessionIsolationScope> {
        self.kernel.isolation().get(&session_id.0).await
    }

    pub async fn mailbox_push(
        &self,
        session_id: &SessionId,
        from_participant_id: Option<String>,
        thread_id: Option<String>,
        payload: serde_json::Value,
    ) -> xuanyu_protocol::SessionMailboxMessage {
        self.kernel
            .mailbox()
            .push(&session_id.0, from_participant_id, thread_id, payload)
            .await
    }

    pub async fn mailbox_drain(
        &self,
        session_id: &SessionId,
    ) -> Vec<xuanyu_protocol::SessionMailboxMessage> {
        self.kernel.mailbox().drain(&session_id.0).await
    }

    /// Legacy `SessionManager.getContextStatus`.
    pub async fn get_context_status(
        &self,
        session_id: &SessionId,
        bridge: &dyn SessionBridge,
    ) -> serde_json::Value {
        bridge.build_context_status(session_id).await
    }

    /// Legacy `SessionManager.compactContext`.
    pub async fn compact_context(
        &self,
        session_id: &SessionId,
        strategy: Option<String>,
        bridge: &dyn SessionBridge,
    ) -> serde_json::Value {
        bridge.compact_context(session_id, strategy).await
    }

    /// Legacy context rollback.
    pub async fn rollback_context(
        &self,
        session_id: &SessionId,
        bridge: &dyn SessionBridge,
    ) -> serde_json::Value {
        bridge.rollback_context(session_id).await
    }

    /// Legacy `SessionManager.respondToPermission` / permission respond.
    pub async fn permission_respond(
        &self,
        request_id: String,
        allowed: bool,
        remember: bool,
        bridge: &dyn SessionBridge,
    ) -> serde_json::Value {
        bridge
            .permission_respond_with_scope(
                request_id,
                allowed,
                if remember {
                    Some("session".to_string())
                } else {
                    None
                },
            )
            .await
    }

    pub async fn permission_respond_with_scope(
        &self,
        request_id: String,
        allowed: bool,
        scope: Option<String>,
        bridge: &dyn SessionBridge,
    ) -> serde_json::Value {
        bridge
            .permission_respond_with_scope(request_id, allowed, scope)
            .await
    }

    pub async fn list_pending_permissions(&self, bridge: &dyn SessionBridge) -> serde_json::Value {
        bridge.list_pending_permissions().await
    }

    /// Channel ingress hook — legacy `SessionManager.handleChannelMessage` entry.
    pub async fn note_channel_message_queued(
        &self,
        session_id: &SessionId,
        channel: &str,
        turn_id: &TurnId,
    ) {
        self.kernel.eventbus().publish(AppEvent::LogEntry {
            level: "info".to_string(),
            message: format!(
                "Channel message queued for session {} via {} (turn {})",
                session_id.0, channel, turn_id.0
            ),
        });
    }

    /// Speech ingress hook — legacy `SessionManager.handleSpeechMessage` entry.
    pub async fn note_speech_message_queued(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) {
        self.kernel.eventbus().publish(AppEvent::LogEntry {
            level: "info".to_string(),
            message: format!(
                "Speech message queued for session {} (turn {})",
                session_id.0, turn_id.0
            ),
        });
    }
}

impl std::ops::Deref for SessionManager {
    type Target = SessionKernel;

    fn deref(&self) -> &Self::Target {
        &self.kernel
    }
}
