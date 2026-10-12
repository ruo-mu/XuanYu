use std::collections::BTreeMap;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct SanitizedMessageSequence {
    pub messages: Vec<ProtocolMessage>,
    pub dropped_count: usize,
    pub stripped_tool_call_count: usize,
}

#[derive(Debug, Clone)]
pub struct ProtocolToolCallSpec {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string (OpenAI tool-call shape).
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ProtocolMessage {
    pub role: String,
    pub content: String,
    pub tool_call_ids: Vec<String>,
    pub tool_call_id: Option<String>,
    /// When role=assistant and tools were requested this turn.
    pub tool_calls: Vec<ProtocolToolCallSpec>,
}

pub fn sanitize_messages_for_tool_protocol(
    messages: &[ProtocolMessage],
) -> SanitizedMessageSequence {
    let mut output = Vec::new();
    let mut dropped_count = 0usize;
    let mut stripped_tool_call_count = 0usize;
    let mut pending: Option<PendingToolExchange> = None;

    for original in messages {
        let mut message = original.clone();

        if let Some(ref mut pending_exchange) = pending {
            if message.role == "tool" {
                let id = message.tool_call_id.clone().unwrap_or_default();
                if pending_exchange.pending_ids.remove(&id) {
                    pending_exchange.tool_messages.push(message);
                    if pending_exchange.pending_ids.is_empty() {
                        flush_complete_exchange(&mut output, pending.take());
                    }
                } else {
                    dropped_count += 1;
                }
                continue;
            }
            flush_incomplete_exchange(
                &mut output,
                pending.take(),
                &mut dropped_count,
                &mut stripped_tool_call_count,
            );
        }

        if message.role == "tool" {
            dropped_count += 1;
            continue;
        }

        if message.role == "assistant" {
            if !message.tool_call_ids.is_empty() {
                pending = Some(PendingToolExchange {
                    assistant: message.clone(),
                    pending_ids: message.tool_call_ids.iter().cloned().collect(),
                    tool_messages: Vec::new(),
                });
                continue;
            }
            message.tool_call_ids.clear();
            if message.content.trim().is_empty() {
                dropped_count += 1;
                continue;
            }
        }

        output.push(message);
    }

    flush_incomplete_exchange(
        &mut output,
        pending.take(),
        &mut dropped_count,
        &mut stripped_tool_call_count,
    );

    SanitizedMessageSequence {
        messages: output,
        dropped_count,
        stripped_tool_call_count,
    }
}

pub fn protocol_message_from_record(
    role: &str,
    content: &str,
    metadata: &BTreeMap<String, Value>,
    tool_call_id: Option<&str>,
) -> ProtocolMessage {
    let tool_call_ids = metadata
        .get("toolCalls")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("id")
                        .or_else(|| item.pointer("/function/id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    ProtocolMessage {
        role: role.to_string(),
        content: content.to_string(),
        tool_call_ids,
        tool_call_id: tool_call_id.map(str::to_string),
        tool_calls: Vec::new(),
    }
}

#[derive(Debug)]
struct PendingToolExchange {
    assistant: ProtocolMessage,
    pending_ids: std::collections::BTreeSet<String>,
    tool_messages: Vec<ProtocolMessage>,
}

fn flush_incomplete_exchange(
    output: &mut Vec<ProtocolMessage>,
    pending: Option<PendingToolExchange>,
    dropped_count: &mut usize,
    stripped_tool_call_count: &mut usize,
) {
    let Some(mut pending) = pending else {
        return;
    };
    pending.assistant.tool_call_ids.clear();
    if pending.assistant.content.trim().is_empty() {
        *dropped_count += 1;
    } else {
        output.push(pending.assistant);
        *stripped_tool_call_count += 1;
    }
    *dropped_count += pending.tool_messages.len();
}

fn flush_complete_exchange(output: &mut Vec<ProtocolMessage>, pending: Option<PendingToolExchange>) {
    let Some(pending) = pending else {
        return;
    };
    output.push(pending.assistant);
    output.extend(pending.tool_messages);
}
