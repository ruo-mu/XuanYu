use xuanyu_execution::{
    pack_session_context, recent_dialogue_token_budget, PackSessionContextInput, ProtocolMessage,
    SessionHandoffSummary, COMPACT_RECENT_MAX_TOKENS,
};

fn msg(role: &str, content: &str) -> ProtocolMessage {
    ProtocolMessage {
        role: role.to_string(),
        content: content.to_string(),
        tool_call_ids: Vec::new(),
        tool_call_id: if role == "tool" {
            Some("t1".to_string())
        } else {
            None
        },
        tool_calls: Vec::new(),
    }
}

#[test]
fn short_history_keeps_all_verbatim() {
    let history = vec![
        msg("user", "make a page"),
        msg("assistant", "created index.html with click counter"),
    ];
    let packed = pack_session_context(PackSessionContextInput {
        messages: &history,
        handoff: None,
        current_user_input: "make it a clock",
        max_input_tokens: 128_000,
        force_compacted_layout: false,
    });
    assert!(!packed.included_handoff);
    assert_eq!(packed.messages.len(), 2);
    assert_eq!(packed.messages[0].content, "make a page");
    assert_eq!(packed.messages[1].role, "assistant");
}

#[test]
fn strips_trailing_current_user() {
    let history = vec![
        msg("user", "make a page"),
        msg("assistant", "done"),
        msg("user", "make it a clock"),
    ];
    let packed = pack_session_context(PackSessionContextInput {
        messages: &history,
        handoff: None,
        current_user_input: "make it a clock",
        max_input_tokens: 128_000,
        force_compacted_layout: false,
    });
    assert_eq!(packed.messages.len(), 2);
    assert_eq!(packed.messages.last().unwrap().role, "assistant");
}

#[test]
fn handoff_appended_after_recent() {
    let history = vec![
        msg("user", "old1"),
        msg("assistant", "a1"),
        msg("user", "recent"),
        msg("assistant", "ar"),
    ];
    let handoff = SessionHandoffSummary {
        summary: Some("Built a click counter page in index.html".to_string()),
        files_modified: vec!["index.html".to_string()],
        key_decisions: vec!["Use vanilla HTML".to_string()],
        tools_used: vec!["file_write".to_string()],
    };
    let packed = pack_session_context(PackSessionContextInput {
        messages: &history,
        handoff: Some(&handoff),
        current_user_input: "next",
        max_input_tokens: 128_000,
        force_compacted_layout: true,
    });
    assert!(packed.included_handoff);
    let last = packed.messages.last().unwrap();
    assert_eq!(last.role, "user");
    assert!(last.content.contains("handoff summary"));
    assert!(last.content.contains("index.html"));
    assert!(packed.messages.iter().any(|m| m.content == "recent"));
}

#[test]
fn compact_recent_budget_is_20k() {
    assert_eq!(recent_dialogue_token_budget(128_000), 20_000);
    assert_eq!(COMPACT_RECENT_MAX_TOKENS, 20_000);
}
