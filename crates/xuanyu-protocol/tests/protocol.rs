//! Protocol 类型公开 API 回归测试。
//!
//! 覆盖目标：
//! - `Envelope::new` / `EventId::new` 默认值与字段
//! - `AppError` 三段式构造 + context 累积 + recoverable 标记
//! - 各 ID 类型 serde 透明序列化
//!
//! 目的：保障协议数据结构与构造函数契约不被无意识重构破坏。

use serde_json::json;
use std::collections::BTreeMap;
use xuanyu_protocol::{
    AppError, AppStatus, ChatMessage, Envelope, ErrorCode, EventId, MessageRole, SessionId, TurnId,
};

#[test]
fn event_id_is_unique_string() {
    let a = EventId::new();
    let b = EventId::new();
    assert_ne!(a.0, b.0, "EventId::new should produce unique ids");
    assert!(!a.0.is_empty());
}

#[test]
fn envelope_new_sets_kind_source_target_and_id() {
    let payload = AppStatus {
        protocol_version: xuanyu_protocol::PROTOCOL_VERSION.to_string(),
        initialized: true,
        storage_backend: "postgres".into(),
        active_sessions: 0,
        modules: Vec::new(),
        workers: Vec::new(),
    };
    let env = Envelope::new("app.status", "core", "desktop", payload.clone());
    assert_eq!(env.kind, "app.status");
    assert_eq!(env.source, "core");
    assert_eq!(env.target, "desktop");
    assert!(env.correlation_id.is_none());
    assert!(env.trace_id.is_none());
    assert_eq!(env.meta.len(), 0);
    assert_eq!(env.payload.active_sessions, 0);
}

#[test]
fn envelope_roundtrips_through_serde_json() {
    let payload = json!({ "note": "hello" });
    let env = Envelope::new("test.kind", "src", "dst", payload);
    let serialized = serde_json::to_string(&env).expect("envelope serializes");
    let restored: Envelope<serde_json::Value> =
        serde_json::from_str(&serialized).expect("envelope deserializes");
    assert_eq!(restored.kind, env.kind);
    assert_eq!(restored.source, env.source);
    assert_eq!(restored.target, env.target);
    assert_eq!(restored.payload["note"], "hello");
}

#[test]
fn session_id_and_turn_id_serialize_as_transparent_strings() {
    let sid = SessionId("abc-123".into());
    let tid = TurnId("turn-42".into());
    let sid_json = serde_json::to_value(&sid).expect("session id serializes");
    let tid_json = serde_json::to_value(&tid).expect("turn id serializes");
    assert_eq!(sid_json, json!("abc-123"));
    assert_eq!(tid_json, json!("turn-42"));

    let restored: SessionId = serde_json::from_value(json!("session-9")).expect("session id parses");
    assert_eq!(restored.0, "session-9");
}

#[test]
fn app_error_new_marks_unrecoverable_with_empty_context() {
    let err = AppError::new(ErrorCode::SessionNotFound, "session 42 missing");
    assert!(!err.recoverable);
    assert!(err.suggestion.is_none());
    assert!(err.context.is_empty());
    assert_eq!(err.code, ErrorCode::SessionNotFound);
    assert_eq!(err.message, "session 42 missing");
}

#[test]
fn app_error_recoverable_attaches_suggestion_and_flag() {
    let err = AppError::recoverable(
        ErrorCode::ToolDenied,
        "file_edit blocked",
        "request approval from the operator",
    );
    assert!(err.recoverable);
    assert_eq!(
        err.suggestion.as_deref(),
        Some("request approval from the operator")
    );
}

#[test]
fn app_error_with_context_accumulates_keys() {
    let err = AppError::new(ErrorCode::InternalInvariantBroken, "boom")
        .with_context("session_id", "s-1")
        .with_context("turn_id", "t-1");
    assert_eq!(err.context.get("session_id").map(String::as_str), Some("s-1"));
    assert_eq!(err.context.get("turn_id").map(String::as_str), Some("t-1"));
    assert_eq!(err.context.len(), 2);
}

#[test]
fn chat_message_role_enum_serializes_snake_case() {
    let user_json = serde_json::to_value(MessageRole::User).expect("role serializes");
    let assistant_json = serde_json::to_value(MessageRole::Assistant).expect("role serializes");
    assert_eq!(user_json, json!("user"));
    assert_eq!(assistant_json, json!("assistant"));
}

#[test]
fn chat_message_minimal_fields_roundtrip() {
    let msg = ChatMessage {
        id: Some("m-1".into()),
        role: MessageRole::User,
        content: "hi".into(),
        timestamp: chrono::Utc::now(),
        turn_id: Some(TurnId("t-1".into())),
        attachments: None,
        metadata: BTreeMap::new(),
        tool_call_id: None,
        name: None,
        segments: None,
    };
    let serialized = serde_json::to_string(&msg).expect("message serializes");
    let restored: ChatMessage = serde_json::from_str(&serialized).expect("message deserializes");
    assert_eq!(restored.id.as_deref(), Some("m-1"));
    assert_eq!(restored.turn_id.as_ref().unwrap().0, "t-1");
    assert_eq!(restored.content, "hi");
}
