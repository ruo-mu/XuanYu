use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub String);

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T> {
    pub id: EventId,
    pub correlation_id: Option<String>,
    pub trace_id: Option<String>,
    pub source: String,
    pub target: String,
    pub kind: String,
    pub timestamp: DateTime<Utc>,
    pub payload: T,
    pub meta: BTreeMap<String, String>,
}

impl<T> Envelope<T> {
    pub fn new(
        kind: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
        payload: T,
    ) -> Self {
        Self {
            id: EventId::new(),
            correlation_id: None,
            trace_id: None,
            source: source.into(),
            target: target.into(),
            kind: kind.into(),
            timestamp: Utc::now(),
            payload,
            meta: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub protocol_version: String,
    pub initialized: bool,
    pub storage_backend: String,
    pub active_sessions: usize,
    pub modules: Vec<RuntimeModuleStatus>,
    pub workers: Vec<WorkerHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModuleStatus {
    pub name: String,
    pub status: WorkerStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerHealth {
    pub name: String,
    pub status: WorkerStatus,
    pub last_seen_ms: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Starting,
    Healthy,
    Degraded,
    Stopped,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    pub is_processing: bool,
    pub storage_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_generated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_generation_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_model: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Workspace absolute path when this session is project-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
}

pub const SESSION_MAIN_THREAD_ID: &str = "main";
pub const SESSION_DEFAULT_PARTICIPANT_ID: &str = "local-user";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionParticipant {
    pub id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionThread {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIsolationScope {
    pub workspace_root: String,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMailboxMessage {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ChatAttachment>>,
    /** 持久化消息 id；历史回放 / segment 挂载用。 */
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /** 已挂到本条消息的时间线 segments（历史回放）。 */
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub data_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    AppReady {
        status: AppStatus,
    },
    SessionCreated {
        session: SessionInfo,
    },
    SessionUpdated {
        session: SessionInfo,
    },
    SessionTitleStatus {
        session_id: SessionId,
        status: String,
        generated_by: Option<String>,
        error: Option<String>,
        provider: Option<String>,
        model: Option<String>,
    },
    SessionHistory {
        session_id: SessionId,
        messages: Vec<ChatMessage>,
    },
    TurnStarted {
        session_id: SessionId,
        turn_id: TurnId,
    },
    TurnDelta {
        session_id: SessionId,
        turn_id: TurnId,
        content: String,
    },
    TurnFinished {
        session_id: SessionId,
        turn_id: TurnId,
    },
    TurnFailed {
        session_id: SessionId,
        turn_id: TurnId,
        error: AppError,
    },
    TurnUsage {
        session_id: SessionId,
        turn_id: TurnId,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_tokens: Option<u64>,
    },
    TurnReasoning {
        session_id: SessionId,
        turn_id: TurnId,
        content: String,
    },
    AgentToolRequested {
        session_id: SessionId,
        turn_id: TurnId,
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    AgentToolProgress {
        session_id: SessionId,
        turn_id: TurnId,
        tool_call_id: String,
        tool_name: String,
        data: Value,
    },
    AgentToolResult {
        session_id: SessionId,
        turn_id: TurnId,
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
    RuntimeModuleUpdated {
        module: RuntimeModuleStatus,
    },
    ToolCatalogUpdated {
        tools: Vec<ToolCatalogItem>,
    },
    MemoryUpdated {
        total_entries: usize,
    },
    AudioStateUpdated {
        session_id: SessionId,
        state: String,
    },
    AsrPartial {
        session_id: SessionId,
        text: String,
        is_final: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        confidence: Option<f32>,
    },
    LogEntry {
        level: String,
        message: String,
    },
    /// 嵌入式 PostgreSQL 自动安装/启动进度（event 为与 init 向导一致的
    /// {type: progress|log|done, phase, percent, message, ...} JSON）。
    StorageProgress {
        event: Value,
    },
    ModelRouteSelected {
        session_id: SessionId,
        task: String,
        provider: String,
        model: String,
    },
    WorkerHealth {
        worker: WorkerHealth,
    },
    TeamAgentStarted {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        team: Value,
        ledger: Value,
    },
    TeamAgentProgress {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        team: Value,
        ledger: Value,
    },
    TeamAgentCompleted {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        team: Value,
        ledger: Value,
    },
    TeamAgentFailed {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        team: Value,
        ledger: Value,
    },
    TaskSnapshot {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        task: Value,
    },
    TaskEvent {
        session_id: String,
        task_id: String,
        event: Value,
    },
    TaskResult {
        session_id: String,
        task_id: String,
        result: Value,
    },
    RunPlan {
        session_id: SessionId,
        run_id: String,
        plan: Option<Value>,
    },
    QuestionRequested {
        session_id: SessionId,
        request_id: String,
        questions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
    },
    QuestionAnswered {
        session_id: SessionId,
        request_id: String,
        answers: Value,
    },
    PermissionRequested {
        session_id: SessionId,
        request_id: String,
        tool_name: String,
        args: Value,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        risk: Option<Value>,
    },
    PermissionAnswered {
        session_id: SessionId,
        request_id: String,
        allowed: bool,
    },
    ChannelQr {
        draft_id: String,
        qr_url: String,
    },
    ChannelQrResult {
        draft_id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogItem {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub source: String,
    pub is_write: bool,
    pub is_execute: bool,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub recoverable: bool,
    pub suggestion: Option<String>,
    pub context: BTreeMap<String, String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            recoverable: false,
            suggestion: None,
            context: BTreeMap::new(),
        }
    }

    pub fn recoverable(
        code: ErrorCode,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            recoverable: true,
            suggestion: Some(suggestion.into()),
            context: BTreeMap::new(),
        }
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    ConfigInvalid,
    SessionNotFound,
    TurnAborted,
    ModelUnavailable,
    ToolDenied,
    ToolTimeout,
    WorkerUnhealthy,
    InferenceModelNotLoaded,
    AudioDeviceUnavailable,
    MemoryStoreUnavailable,
    PermissionRequired,
    InternalInvariantBroken,
}

pub type Result<T> = std::result::Result<T, AppError>;
