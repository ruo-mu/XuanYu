//! 本轮 timeline 有界批写：文本积压，工具边界 / 满缓冲 / finish 强制刷库。

use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;
use xuanyu_storage::ConversationSegmentRecord;

pub const TURN_WRITE_MAX_OPS: usize = 32;
pub const TURN_WRITE_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct PendingTextSegment {
    pub id: String,
    pub content: String,
    pub thinking: Option<String>,
    pub fingerprint: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TurnWriteBuffer {
    pub pending_text: Vec<PendingTextSegment>,
    pub pending_bytes: usize,
}

impl TurnWriteBuffer {
    pub fn is_empty(&self) -> bool {
        self.pending_text.is_empty()
    }

    pub fn would_exceed(&self, add_bytes: usize) -> bool {
        self.pending_text.len() >= TURN_WRITE_MAX_OPS
            || self.pending_bytes.saturating_add(add_bytes) > TURN_WRITE_MAX_BYTES
    }

    pub fn push_text(&mut self, item: PendingTextSegment) {
        self.pending_bytes = self.pending_bytes.saturating_add(item.bytes);
        self.pending_text.push(item);
    }

    pub fn take_text(&mut self) -> Vec<PendingTextSegment> {
        self.pending_bytes = 0;
        std::mem::take(&mut self.pending_text)
    }
}

#[derive(Debug, Clone, Default)]
pub struct TurnPersistState {
    pub assistant_message_id: String,
    pub segment_by_tool_call: BTreeMap<String, String>,
    /** 最近一次已落库的正文指纹，避免 finish 再插重复 text。 */
    pub last_text_fingerprint: Option<String>,
    pub write_buffer: TurnWriteBuffer,
    /** flush 时顺带更新的最新正文。 */
    pub latest_assistant_content: Option<String>,
}

pub fn text_segment_fingerprint(content: &str, thinking: Option<&str>) -> String {
    format!("{}\0{}", content.trim(), thinking.unwrap_or("").trim())
}

pub fn estimate_text_bytes(content: &str, thinking: Option<&str>) -> usize {
    content.len() + thinking.map(str::len).unwrap_or(0)
}

pub fn build_text_segment_record(
    session_id: &str,
    turn_id: &str,
    message_id: &str,
    sequence_number: u64,
    pending: &PendingTextSegment,
) -> ConversationSegmentRecord {
    let now = Utc::now();
    let mut payload = BTreeMap::new();
    payload.insert("content".to_string(), json!(pending.content));
    if let Some(thinking) = pending.thinking.as_ref() {
        payload.insert("thinking".to_string(), json!(thinking));
    }
    ConversationSegmentRecord {
        id: pending.id.clone(),
        session_id: session_id.to_string(),
        sequence_number,
        message_id: Some(message_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        segment_type: "text".to_string(),
        status: Some("done".to_string()),
        title: Some("玄语回复".to_string()),
        payload,
        created_at: now,
        updated_at: now,
    }
}

pub struct BuildToolStartSegmentInput<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub message_id: &'a str,
    pub sequence_number: u64,
    pub segment_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub args: &'a Value,
    pub segment_type: &'a str,
    pub title_content: &'a str,
}

pub fn build_tool_start_segment(
    input: BuildToolStartSegmentInput<'_>,
) -> ConversationSegmentRecord {
    let BuildToolStartSegmentInput {
        session_id,
        turn_id,
        message_id,
        sequence_number,
        segment_id,
        tool_call_id,
        tool_name,
        args,
        segment_type,
        title_content,
    } = input;
    let now = Utc::now();
    let mut payload = BTreeMap::new();
    payload.insert("name".to_string(), json!(tool_name));
    payload.insert("toolCallId".to_string(), json!(tool_call_id));
    payload.insert("args".to_string(), args.clone());
    payload.insert(
        "rawArgs".to_string(),
        json!(serde_json::to_string(args).unwrap_or_default()),
    );
    payload.insert("content".to_string(), json!(title_content));
    ConversationSegmentRecord {
        id: segment_id.to_string(),
        session_id: session_id.to_string(),
        sequence_number,
        message_id: Some(message_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        segment_type: segment_type.to_string(),
        status: Some("calling".to_string()),
        title: Some(tool_name.to_string()),
        payload,
        created_at: now,
        updated_at: now,
    }
}

pub fn new_segment_id() -> String {
    format!("seg_{}", Uuid::new_v4())
}

