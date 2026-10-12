//! Pack prior session turns into model-visible protocol messages.
//!
//! Layout mirrors production coding-agent compaction:
//! - Short sessions: full prior turns (within budget).
//! - After compact: recent verbatim turns (token-capped) + handoff summary last.
//! - Old tool bodies may be pruned while protecting a recent tool-result window.

use crate::estimate_tokens;
use crate::message_sequence::ProtocolMessage;

/// Compact handoff from the latest context compaction (structured sections).
#[derive(Debug, Clone, Default)]
pub struct SessionHandoffSummary {
    pub summary: Option<String>,
    pub key_decisions: Vec<String>,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
}

impl SessionHandoffSummary {
    pub fn is_empty(&self) -> bool {
        self.summary
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            && self.key_decisions.is_empty()
            && self.files_modified.is_empty()
            && self.tools_used.is_empty()
    }

    /// Handoff text for the next model turn (summary-last layout).
    pub fn to_handoff_message(&self) -> ProtocolMessage {
        let mut lines = vec![
            // Handoff prefix: prior model produced a checkpoint; continue without redoing work.
            "Another model turn produced a handoff summary of earlier work in this session. Use it together with the recent messages above. Build on completed work; do not redo it. Handoff follows:"
                .to_string(),
            String::new(),
        ];
        if let Some(summary) = self
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(summary.to_string());
            lines.push(String::new());
        }
        if !self.key_decisions.is_empty() {
            lines.push("Key decisions:".to_string());
            for item in &self.key_decisions {
                lines.push(format!("- {item}"));
            }
            lines.push(String::new());
        }
        if !self.files_modified.is_empty() {
            lines.push("Files modified:".to_string());
            for item in &self.files_modified {
                lines.push(format!("- {item}"));
            }
            lines.push(String::new());
        }
        if !self.tools_used.is_empty() {
            lines.push(format!("Tools used: {}", self.tools_used.join(", ")));
        }
        ProtocolMessage {
            role: "user".to_string(),
            content: lines.join("\n").trim().to_string(),
            tool_call_ids: Vec::new(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackSessionContextInput<'a> {
    pub messages: &'a [ProtocolMessage],
    pub handoff: Option<&'a SessionHandoffSummary>,
    /// Current turn user text; strip trailing matching user message to avoid duplication.
    pub current_user_input: &'a str,
    pub max_input_tokens: u64,
    /// When true (or handoff present), use compact layout even if raw history fits.
    pub force_compacted_layout: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PackedSessionContext {
    pub messages: Vec<ProtocolMessage>,
    pub included_handoff: bool,
    pub pruned_tool_results: usize,
    pub dropped_prefix_messages: usize,
    pub estimated_tokens: u64,
}

/// After compaction: keep about this many tokens of recent verbatim turns (production default).
pub const COMPACT_RECENT_MAX_TOKENS: u64 = 20_000;

/// Protect roughly this many recent tool-result tokens from pruning (tool-output prune window).
pub const PROTECT_RECENT_TOOL_RESULT_TOKENS: u64 = 40_000;

/// Uncompacted sessions may keep more history, still capped vs the model window.
pub fn full_history_token_budget(max_input_tokens: u64) -> u64 {
    let half = ((max_input_tokens as f64) * 0.50).floor() as u64;
    half.clamp(COMPACT_RECENT_MAX_TOKENS, 64_000)
}

pub fn recent_dialogue_token_budget(max_input_tokens: u64) -> u64 {
    // Compacted / overflow path: fixed recent window (~20k), not a % that shrinks oddly.
    let _ = max_input_tokens;
    COMPACT_RECENT_MAX_TOKENS
}

const PRUNED_TOOL_PLACEHOLDER: &str = "[tool result pruned]";

pub fn pack_session_context(input: PackSessionContextInput<'_>) -> PackedSessionContext {
    let prior = strip_current_user_turn(input.messages, input.current_user_input);
    if prior.is_empty() && input.handoff.map(SessionHandoffSummary::is_empty) != Some(false) {
        return PackedSessionContext::default();
    }

    let handoff = input
        .handoff
        .filter(|h| !h.is_empty())
        .map(SessionHandoffSummary::to_handoff_message);
    let handoff_tokens = handoff
        .as_ref()
        .map(|m| estimate_tokens(&m.content))
        .unwrap_or(0);
    let raw_tokens: u64 = prior.iter().map(message_tokens).sum();
    let full_budget = full_history_token_budget(input.max_input_tokens);
    let use_compacted = input.force_compacted_layout
        || handoff.is_some()
        || raw_tokens.saturating_add(handoff_tokens) > full_budget;

    if !use_compacted {
        let mut messages = prior;
        trim_recent_to_budget(&mut messages, full_budget);
        let estimated_tokens = messages.iter().map(message_tokens).sum();
        return PackedSessionContext {
            messages,
            included_handoff: false,
            pruned_tool_results: 0,
            dropped_prefix_messages: 0,
            estimated_tokens,
        };
    }

    // Compacted layout: keep a token-budgeted recent tail, prune old tool bodies, then
    // append handoff summary last (checkpoint after recent evidence).
    let recent_budget = recent_dialogue_token_budget(input.max_input_tokens)
        .saturating_sub(handoff_tokens.min(COMPACT_RECENT_MAX_TOKENS / 4))
        .max(2_048);
    let mut recent = prior;
    let before_len = recent.len();
    trim_recent_to_budget(&mut recent, recent_budget);
    let dropped_prefix_messages = before_len.saturating_sub(recent.len());

    let pruned_tool_results = prune_old_tool_results(&mut recent, PROTECT_RECENT_TOOL_RESULT_TOKENS);
    trim_recent_to_budget(&mut recent, recent_budget);

    let mut messages = recent;
    let included_handoff = if let Some(handoff) = handoff {
        messages.push(handoff);
        true
    } else {
        false
    };
    let estimated_tokens = messages.iter().map(message_tokens).sum();
    PackedSessionContext {
        messages,
        included_handoff,
        pruned_tool_results,
        dropped_prefix_messages,
        estimated_tokens,
    }
}

fn message_tokens(message: &ProtocolMessage) -> u64 {
    estimate_tokens(&message.content) + estimate_tokens(&message.role) + 4
}

fn strip_current_user_turn(
    messages: &[ProtocolMessage],
    current_user_input: &str,
) -> Vec<ProtocolMessage> {
    let current = current_user_input.trim();
    let mut prior = messages.to_vec();
    while let Some(last) = prior.last() {
        let drop = match last.role.as_str() {
            "user" => {
                let text = last.content.trim();
                text.is_empty()
                    || (!current.is_empty()
                        && (text == current
                            || current.starts_with(text)
                            || text.starts_with(current)))
            }
            "tool" => true,
            "assistant" => last.content.trim().is_empty(),
            _ => false,
        };
        if drop {
            prior.pop();
        } else {
            break;
        }
    }
    prior
}

fn trim_recent_to_budget(messages: &mut Vec<ProtocolMessage>, budget: u64) {
    // Drop from the front until the tail fits (preserve most recent turns).
    while !messages.is_empty() {
        let used: u64 = messages.iter().map(message_tokens).sum();
        if used <= budget {
            break;
        }
        if messages.len() == 1 {
            let msg = &mut messages[0];
            let max_chars = ((budget as usize).saturating_mul(3)).max(200);
            if msg.content.chars().count() > max_chars {
                msg.content = msg.content.chars().take(max_chars).collect::<String>()
                    + "\n...[truncated for context budget]";
            }
            break;
        }
        let drop_count = if messages.get(1).is_some_and(|m| m.role.eq_ignore_ascii_case("user"))
        {
            1
        } else {
            let mut n = 1usize;
            while n < messages.len() && !messages[n].role.eq_ignore_ascii_case("user") {
                n += 1;
            }
            n.max(1)
        };
        let keep_min = 1;
        messages.drain(0..drop_count.min(messages.len().saturating_sub(keep_min)));
    }
}

/// Prune older tool results while protecting the newest ~protect_tokens of tool content.
fn prune_old_tool_results(messages: &mut [ProtocolMessage], protect_tokens: u64) -> usize {
    let mut protected = 0u64;
    let mut prune_indexes = Vec::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if !message.role.eq_ignore_ascii_case("tool") {
            continue;
        }
        let tokens = estimate_tokens(&message.content);
        if protected < protect_tokens {
            protected = protected.saturating_add(tokens);
            continue;
        }
        if message.content.trim() != PRUNED_TOOL_PLACEHOLDER {
            prune_indexes.push(index);
        }
    }
    let count = prune_indexes.len();
    for index in prune_indexes {
        messages[index].content = PRUNED_TOOL_PLACEHOLDER.to_string();
    }
    count
}
