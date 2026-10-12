use xuanyu_agent_bridge::{
    AgentPolicyBudget, AgentPolicyMessage, AgentPolicyRequest, SubAgentPlanRequest,
};

#[test]
fn subagent_plan_request_serializes_prompt_context_for_worker() {
    let value = serde_json::to_value(SubAgentPlanRequest {
        task_id: "task-1".to_string(),
        agent_id: "architect".to_string(),
        role: Some("architect".to_string()),
        system_prompt: Some("You are Architect".to_string()),
        task: "Design the bridge".to_string(),
        context: Some("Use Rust core boundaries.".to_string()),
        write_scope: vec!["docs/architecture-design.md".to_string()],
        tools: vec!["file_write".to_string()],
        artifact_required: true,
    })
    .expect("serialize request");

    assert_eq!(value["systemPrompt"], "You are Architect");
    assert_eq!(value["artifactRequired"], true);
    assert_eq!(value["tools"][0], "file_write");
}

#[test]
fn policy_request_serializes_main_turn_context_for_worker() {
    let value = serde_json::to_value(AgentPolicyRequest {
        session_id: Some("session-1".to_string()),
        mode: Some("person".to_string()),
        user_input: "Please refactor the storage module".to_string(),
        recent_messages: vec![AgentPolicyMessage {
            role: "user".to_string(),
            content: "Earlier request".to_string(),
        }],
        available_tools: vec!["file_read".to_string(), "search".to_string()],
        memory_hits: vec!["User prefers local models".to_string()],
        constraints: vec!["Use Rust for state".to_string()],
        budget: Some(AgentPolicyBudget {
            max_context_tokens: Some(120_000),
            remaining_tokens: Some(80_000),
            max_turns: Some(12),
        }),
    })
    .expect("serialize request");

    assert_eq!(value["sessionId"], "session-1");
    assert_eq!(value["userInput"], "Please refactor the storage module");
    assert_eq!(value["availableTools"][0], "file_read");
    assert_eq!(value["budget"]["maxTurns"], 12);
}
