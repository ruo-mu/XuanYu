use uuid::Uuid;
use xuanyu_agents::{
    build_subagent_system_prompt, subagent_requires_artifact, AgentInput, AgentRuntime,
};

#[tokio::test]
async fn agent_runtime_persists_custom_agent() {
    let root = std::env::temp_dir().join(format!("xuanyu-agents-{}", Uuid::new_v4()));
    let runtime = AgentRuntime::new(root.clone(), None);
    runtime.initialize().await.expect("init");
    let result = runtime
        .create_agent(AgentInput {
            id: Some("custom-dev".to_string()),
            name: Some("Custom Dev".to_string()),
            description: Some("Custom implementation agent".to_string()),
            prompt: Some("Implement carefully.".to_string()),
            tools: vec!["file_read".to_string()],
            permission_mode: Some("default".to_string()),
            provider: None,
            model: None,
            base_url: None,
            api_key: None,
            max_tokens: None,
            temperature: None,
            model_profile: None,
            can_write: Some(true),
            can_talk_to_user: Some(false),
            default_sandbox_mode: Some("workspace-write".to_string()),
            max_turns: Some(12),
            memory_scope: Some("session".to_string()),
            role: Some("developer".to_string()),
        })
        .await;
    assert!(result.ok);

    let restored = AgentRuntime::new(root.clone(), None);
    restored.initialize().await.expect("restored");
    let agents = restored.list_agents().await;
    assert!(agents.iter().any(|agent| agent.id == "custom-dev"));
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn builtin_agents_are_protected() {
    let runtime = AgentRuntime::new(
        std::env::temp_dir().join(format!("xuanyu-agents-{}", Uuid::new_v4())),
        None,
    );
    runtime.initialize().await.expect("init");
    let result = runtime.delete_agent("developer".to_string()).await;
    assert!(!result.ok);
}

#[tokio::test]
async fn builtin_agents_use_legacy_role_prompts_and_rules() {
    let root = std::env::temp_dir().join(format!("xuanyu-agents-{}", Uuid::new_v4()));
    let runtime = AgentRuntime::new(root.clone(), None);
    runtime.initialize().await.expect("init");
    let agents = runtime.list_agents().await;
    let developer = agents
        .iter()
        .find(|agent| agent.id == "developer")
        .expect("developer");
    assert!(developer
        .prompt
        .contains("Read existing code before editing"));
    assert!(!subagent_requires_artifact(developer));

    let architect = agents
        .iter()
        .find(|agent| agent.id == "architect")
        .expect("architect");
    assert!(architect.prompt.contains("engineering-ready"));
    assert!(subagent_requires_artifact(architect));
    let system_prompt = build_subagent_system_prompt(architect);
    assert!(system_prompt.contains("You are Architect, a XuanYu SubAgent"));
    assert!(system_prompt.contains("Create the deliverable file"));
    assert!(system_prompt.contains("Final sections: Conclusion | Evidence"));

    let _ = tokio::fs::remove_dir_all(root).await;
}
