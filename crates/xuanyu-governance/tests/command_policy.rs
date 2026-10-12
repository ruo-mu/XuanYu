use std::collections::BTreeMap;

use xuanyu_governance::{
    GovernanceAction, GovernanceDecision, GovernanceEngine, GovernanceRequest, MemoryDecision,
    PermissionPolicy,
};

#[tokio::test]
async fn command_tool_requires_approval() {
    let engine = GovernanceEngine::default();
    let request = GovernanceRequest {
        actor: "test".to_string(),
        action: GovernanceAction::Tool {
            tool_name: "exec".to_string(),
            args: BTreeMap::new(),
            writes_files: false,
            runs_command: true,
            external_network: false,
            touches_secrets: false,
        },
        resource: None,
        reason: None,
        trace_id: None,
        budget_usage: None,
        quota_key: None,
        quota_amount: 1,
    };

    let result = engine.evaluate(request).await;
    assert_eq!(result.decision, GovernanceDecision::NeedsApproval);
    assert_eq!(engine.audit_events().await.len(), 1);
}

#[tokio::test]
async fn session_allowlist_skips_future_tool_approval() {
    let engine = GovernanceEngine::default();
    engine.allow_tool_for_session("session-1", "exec").await;
    let request = GovernanceRequest {
        actor: "test".to_string(),
        action: GovernanceAction::Tool {
            tool_name: "exec".to_string(),
            args: BTreeMap::from([("_sessionId".to_string(), serde_json::json!("session-1"))]),
            writes_files: false,
            runs_command: true,
            external_network: false,
            touches_secrets: false,
        },
        resource: None,
        reason: None,
        trace_id: None,
        budget_usage: None,
        quota_key: None,
        quota_amount: 1,
    };

    let result = engine.evaluate(request).await;
    assert_eq!(result.decision, GovernanceDecision::Allow);
    assert_eq!(result.reason, "tool_allowed_for_session");
}

#[tokio::test]
async fn app_config_permissions_drive_tool_decisions() {
    let engine = GovernanceEngine::new(PermissionPolicy::from_app_config_value(
        &serde_json::json!({
            "permissions": {
                "default": "ask",
                "mode": "all",
                "rules": [],
                "whitelist": [],
                "blacklist": []
            }
        }),
    ));
    let allowed = engine
        .evaluate(tool_request("file_write", true, false, BTreeMap::new()))
        .await;
    assert_eq!(allowed.decision, GovernanceDecision::Allow);

    engine
        .update_policy(PermissionPolicy::from_app_config_value(
            &serde_json::json!({
                "permissions": {
                    "default": "allow",
                    "mode": "auto",
                    "rules": [],
                    "whitelist": [],
                    "blacklist": ["file_write"]
                }
            }),
        ))
        .await;
    let denied = engine
        .evaluate(tool_request("file_write", true, false, BTreeMap::new()))
        .await;
    assert_eq!(denied.decision, GovernanceDecision::Deny);
    assert_eq!(denied.reason, "tool_denied_by_policy");
}

#[tokio::test]
async fn tool_access_config_filters_subagent_tools() {
    let engine = GovernanceEngine::new(PermissionPolicy::from_app_config_value(
        &serde_json::json!({
            "toolAccess": {
                "toolGroups": { "read": ["file_read", "search"] },
                "subAgents": {
                    "byId": {
                        "developer": { "mode": "allowlist", "allowedTools": ["file_read"] }
                    }
                }
            }
        }),
    ));

    let main_allowed = engine
        .evaluate(tool_request("exec", false, true, BTreeMap::new()))
        .await;
    assert_eq!(main_allowed.decision, GovernanceDecision::NeedsApproval);

    let subagent_denied = engine
        .evaluate(tool_request(
            "file_write",
            true,
            false,
            BTreeMap::from([("_agentId".to_string(), serde_json::json!("developer"))]),
        ))
        .await;
    assert_eq!(subagent_denied.decision, GovernanceDecision::Deny);
    assert_eq!(subagent_denied.reason, "tool_denied_by_tool_access");
}

#[tokio::test]
async fn secret_memory_is_denied() {
    let engine = GovernanceEngine::default();
    let request = GovernanceRequest {
        actor: "test".to_string(),
        action: GovernanceAction::MemoryWrite {
            category: "semantic".to_string(),
            content: "api_key=abc123".to_string(),
            sensitivity: None,
        },
        resource: None,
        reason: None,
        trace_id: None,
        budget_usage: None,
        quota_key: None,
        quota_amount: 1,
    };

    let result = engine.evaluate(request).await;
    assert_eq!(result.decision, GovernanceDecision::Deny);
    assert_eq!(result.memory_decision, Some(MemoryDecision::Deny));
}

fn tool_request(
    tool_name: &str,
    writes_files: bool,
    runs_command: bool,
    args: BTreeMap<String, serde_json::Value>,
) -> GovernanceRequest {
    GovernanceRequest {
        actor: "test".to_string(),
        action: GovernanceAction::Tool {
            tool_name: tool_name.to_string(),
            args,
            writes_files,
            runs_command,
            external_network: false,
            touches_secrets: false,
        },
        resource: None,
        reason: None,
        trace_id: None,
        budget_usage: None,
        quota_key: None,
        quota_amount: 1,
    }
}
