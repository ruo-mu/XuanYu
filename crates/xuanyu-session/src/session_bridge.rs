use async_trait::async_trait;
use serde_json::Value;
use xuanyu_protocol::SessionId;

/// Cross-cutting services required by session facade methods (context + permissions).
/// Implemented by `AppCore` in `xuanyu-core`.
#[async_trait]
pub trait SessionBridge: Send + Sync {
    async fn build_context_status(&self, session_id: &SessionId) -> Value;
    async fn compact_context(
        &self,
        session_id: &SessionId,
        strategy: Option<String>,
    ) -> Value;
    async fn rollback_context(&self, session_id: &SessionId) -> Value;
    async fn permission_respond_with_scope(
        &self,
        request_id: String,
        allowed: bool,
        scope: Option<String>,
    ) -> Value;
    async fn list_pending_permissions(&self) -> Value;
}
