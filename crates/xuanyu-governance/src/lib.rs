use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tokio::sync::RwLock;
use uuid::Uuid;

mod command_validator;
mod git_command_policy;
mod path_validator;
pub mod turn_ledger;
mod rollback;

pub use command_validator::{CommandValidationResult, CommandValidator};
pub use git_command_policy::{
    classify_git_segment, command_has_destructive_git, extract_git_subcommand, GitCommandClass,
};
pub use path_validator::{PathValidationResult, SecurePathValidator};
pub use rollback::{RollbackActionRecord, RollbackRegistry};
pub use turn_ledger::{
    extract_write_paths, is_write_tool_name, RecordWriteInput, RejectFileMutationInput,
    RestoreConflict, RestoreWorkspaceInput, TurnFileAction, TurnFileMutation, TurnLedgerRecord,
    TurnLedgerStore, WorkspaceCheckpointRecord, WorkspaceRestoreResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceDecision {
    Allow,
    Deny,
    NeedsApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDecision {
    Direct,
    Pending,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveFindingKind {
    Config,
    Env,
    Content,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveFinding {
    pub file_path: String,
    pub kind: SensitiveFindingKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveScanResult {
    pub findings: Vec<SensitiveFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GovernanceAction {
    Tool {
        tool_name: String,
        args: BTreeMap<String, serde_json::Value>,
        writes_files: bool,
        runs_command: bool,
        external_network: bool,
        touches_secrets: bool,
    },
    MemoryWrite {
        category: String,
        content: String,
        sensitivity: Option<String>,
    },
    Output {
        content: String,
    },
    Generic {
        resource: Option<String>,
        writes_files: bool,
        runs_command: bool,
        external_network: bool,
        touches_secrets: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceRequest {
    pub actor: String,
    pub action: GovernanceAction,
    pub resource: Option<String>,
    pub reason: Option<String>,
    pub trace_id: Option<String>,
    pub budget_usage: Option<BudgetUsage>,
    pub quota_key: Option<String>,
    pub quota_amount: u64,
}

impl GovernanceRequest {
    pub fn tool(tool_name: impl Into<String>, args: BTreeMap<String, serde_json::Value>) -> Self {
        Self {
            actor: "runtime".to_string(),
            action: GovernanceAction::Tool {
                tool_name: tool_name.into(),
                args,
                writes_files: false,
                runs_command: false,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceResult {
    pub decision: GovernanceDecision,
    pub reason: String,
    pub policy: String,
    pub approval_id: Option<String>,
    pub risk: RiskAssessment,
    pub audit_id: String,
    pub budget: Option<BudgetSnapshot>,
    pub quota: Option<QuotaSnapshot>,
    pub memory_decision: Option<MemoryDecision>,
    pub redacted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    pub level: RiskLevel,
    pub score: u8,
    pub reasons: Vec<String>,
}

impl RiskAssessment {
    pub fn low(reason: impl Into<String>) -> Self {
        Self {
            level: RiskLevel::Low,
            score: 10,
            reasons: vec![reason.into()],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetUsage {
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetLimit {
    pub max_tokens: u64,
    pub max_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSnapshot {
    pub tokens: u64,
    pub cost_usd: f64,
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub limit: u64,
    pub used: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub resource: Option<String>,
    pub decision: GovernanceDecision,
    pub reason: String,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPolicy {
    pub deny_tools: BTreeSet<String>,
    pub approval_tools: BTreeSet<String>,
    pub allow_file_write: bool,
    pub allow_network: bool,
    pub protected_path_markers: Vec<String>,
    pub default_level: PermissionLevel,
    pub mode: PermissionMode,
    pub rules: Vec<PermissionRule>,
    pub whitelist: BTreeSet<String>,
    pub blacklist: BTreeSet<String>,
    pub disabled_tools: BTreeSet<String>,
    pub tool_access: ToolAccessSettings,
    pub configured_permissions: bool,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            deny_tools: BTreeSet::new(),
            approval_tools: [
                "exec",
                "start_process",
                "file_write",
                "file_edit",
                "apply_patch",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            allow_file_write: false,
            allow_network: false,
            protected_path_markers: vec![
                ".env".to_string(),
                "id_rsa".to_string(),
                "id_ed25519".to_string(),
                "credentials".to_string(),
            ],
            default_level: PermissionLevel::Ask,
            mode: PermissionMode::Default,
            rules: default_permission_rules(),
            whitelist: BTreeSet::new(),
            blacklist: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            tool_access: ToolAccessSettings::default(),
            configured_permissions: false,
        }
    }
}

impl PermissionPolicy {
    pub fn from_app_config_value(config: &serde_json::Value) -> Self {
        let mut policy = PermissionPolicy::default();
        if let Some(disabled) = string_set_at(config, "disabledTools") {
            policy.disabled_tools = disabled;
        }
        if let Some(permissions) = config
            .get("permissions")
            .and_then(serde_json::Value::as_object)
        {
            policy.configured_permissions = true;
            if let Some(level) = permissions
                .get("default")
                .and_then(serde_json::Value::as_str)
                .and_then(PermissionLevel::from_str)
            {
                policy.default_level = level;
            }
            if let Some(mode) = permissions
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(PermissionMode::from_str)
            {
                policy.mode = mode;
            }
            if let Some(rules) = permissions
                .get("rules")
                .and_then(serde_json::Value::as_array)
            {
                policy.rules = rules
                    .iter()
                    .filter_map(permission_rule_from_value)
                    .collect();
            }
            if let Some(whitelist) = string_set_from_object(permissions, "whitelist") {
                policy.whitelist = whitelist;
            }
            if let Some(blacklist) = string_set_from_object(permissions, "blacklist") {
                policy.blacklist = blacklist;
            }
        }
        if let Some(tool_access) = config.get("toolAccess") {
            policy.tool_access = ToolAccessSettings::from_value(tool_access);
        }
        policy
    }
}

fn budget_limit_from_value(value: &serde_json::Value) -> Option<BudgetLimit> {
    let object = value.as_object()?;
    let max_tokens = object
        .get("maxTokens")
        .or_else(|| object.get("max_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let max_cost_usd = object
        .get("maxCostUsd")
        .or_else(|| object.get("max_cost_usd"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    if max_tokens == 0 && max_cost_usd <= 0.0 {
        return None;
    }
    Some(BudgetLimit {
        max_tokens: if max_tokens == 0 {
            u64::MAX
        } else {
            max_tokens
        },
        max_cost_usd: if max_cost_usd <= 0.0 {
            f64::MAX
        } else {
            max_cost_usd
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    Allow,
    Ask,
    Deny,
}

impl PermissionLevel {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// 权限模式仅三种：default / plan / auto。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Default,
    Plan,
    Auto,
}

impl PermissionMode {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            // 遗留值映射到三模式，避免旧配置炸渲染/炸后端
            "default" | "ask" | "strict" => Some(Self::Default),
            "plan" => Some(Self::Plan),
            "auto" | "all" | "yolo" => Some(Self::Auto),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    pub pattern: String,
    pub level: PermissionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolAccessMode {
    Inherit,
    Allowlist,
    Denylist,
}

impl ToolAccessMode {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inherit" => Some(Self::Inherit),
            "allowlist" => Some(Self::Allowlist),
            "denylist" => Some(Self::Denylist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAccessRule {
    pub mode: Option<ToolAccessMode>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentToolAccess {
    pub defaults: Option<ToolAccessRule>,
    pub by_role: BTreeMap<String, ToolAccessRule>,
    pub by_id: BTreeMap<String, ToolAccessRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAccessSettings {
    pub tool_groups: BTreeMap<String, Vec<String>>,
    pub sub_agents: Option<SubAgentToolAccess>,
}

impl ToolAccessSettings {
    fn from_value(value: &serde_json::Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        let tool_groups = object
            .get("toolGroups")
            .and_then(serde_json::Value::as_object)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|(key, value)| {
                        string_vec_value(value).map(|items| (key.clone(), items))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let sub_agents = object
            .get("subAgents")
            .and_then(serde_json::Value::as_object)
            .map(|sub_agents| SubAgentToolAccess {
                defaults: sub_agents
                    .get("defaults")
                    .and_then(ToolAccessRule::from_value),
                by_role: tool_access_rule_map(sub_agents.get("byRole")),
                by_id: tool_access_rule_map(sub_agents.get("byId")),
            });
        Self {
            tool_groups,
            sub_agents,
        }
    }

    fn expand_patterns(&self, patterns: &[String]) -> Vec<String> {
        let mut expanded = Vec::new();
        for pattern in patterns {
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(group_name) = trimmed.strip_prefix('@') {
                if let Some(group) = self.tool_groups.get(group_name) {
                    expanded.extend(group.iter().cloned());
                }
            } else {
                expanded.push(trimmed.to_string());
            }
        }
        unique_strings(expanded)
    }
}

impl ToolAccessRule {
    fn from_value(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        Some(Self {
            mode: object
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(ToolAccessMode::from_str),
            allowed_tools: object
                .get("allowedTools")
                .and_then(string_vec_value)
                .unwrap_or_default(),
            denied_tools: object
                .get("deniedTools")
                .and_then(string_vec_value)
                .unwrap_or_default(),
        })
    }

    fn mode_or_inherit(&self) -> ToolAccessMode {
        self.mode.unwrap_or(ToolAccessMode::Inherit)
    }
}

#[derive(Debug, Clone)]
struct QuotaBucket {
    limit: u64,
    used: u64,
}

impl QuotaBucket {
    fn snapshot(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            limit: self.limit,
            used: self.used,
            remaining: self.limit.saturating_sub(self.used),
        }
    }
}

pub struct GovernanceEngine {
    policy: RwLock<PermissionPolicy>,
    budget_limit: RwLock<Option<BudgetLimit>>,
    budget_usage: RwLock<BudgetUsage>,
    quotas: RwLock<BTreeMap<String, QuotaBucket>>,
    session_tool_allowlist: RwLock<BTreeSet<String>>,
    session_permission_modes: RwLock<BTreeMap<String, PermissionMode>>,
    audit_log: RwLock<Vec<AuditEvent>>,
    audit_persist_path: RwLock<Option<PathBuf>>,
    rollback_registry: RollbackRegistry,
}

impl GovernanceEngine {
    pub fn new(policy: PermissionPolicy) -> Self {
        Self {
            policy: RwLock::new(policy),
            budget_limit: RwLock::new(None),
            budget_usage: RwLock::new(BudgetUsage::default()),
            quotas: RwLock::new(BTreeMap::new()),
            session_tool_allowlist: RwLock::new(BTreeSet::new()),
            session_permission_modes: RwLock::new(BTreeMap::new()),
            audit_log: RwLock::new(Vec::new()),
            audit_persist_path: RwLock::new(None),
            rollback_registry: RollbackRegistry::default(),
        }
    }

    pub fn with_budget(policy: PermissionPolicy, budget_limit: BudgetLimit) -> Self {
        Self {
            policy: RwLock::new(policy),
            budget_limit: RwLock::new(Some(budget_limit)),
            budget_usage: RwLock::new(BudgetUsage::default()),
            quotas: RwLock::new(BTreeMap::new()),
            session_tool_allowlist: RwLock::new(BTreeSet::new()),
            session_permission_modes: RwLock::new(BTreeMap::new()),
            audit_log: RwLock::new(Vec::new()),
            audit_persist_path: RwLock::new(None),
            rollback_registry: RollbackRegistry::default(),
        }
    }

    /// 可选落盘路径；设置后立刻从磁盘加载历史审计。
    pub async fn set_audit_persist_path(&self, path: impl Into<PathBuf>) {
        let path = path.into();
        *self.audit_persist_path.write().await = Some(path.clone());
        if let Some(events) = load_audit_events_from_disk(&path) {
            let mut log = self.audit_log.write().await;
            if log.is_empty() {
                *log = events;
            }
        }
    }

    pub fn rollback_registry(&self) -> &RollbackRegistry {
        &self.rollback_registry
    }

    pub async fn rollback_actions(&self) -> Vec<RollbackActionRecord> {
        self.rollback_registry.list().await
    }

    pub async fn update_policy(&self, policy: PermissionPolicy) {
        *self.policy.write().await = policy;
    }

    /// Apply `AppConfig.governance` budget/quotas (Node gateway parity).
    pub async fn apply_app_config_limits(&self, config: &serde_json::Value) {
        let governance = config.get("governance");
        let budget_limit = governance
            .and_then(|value| value.get("budget"))
            .and_then(budget_limit_from_value);
        *self.budget_limit.write().await = budget_limit;

        let mut quotas = self.quotas.write().await;
        quotas.remove("toolCalls");
        quotas.remove("tool_calls");
        if let Some(limit) = governance
            .and_then(|value| value.get("quotas"))
            .and_then(|quotas| quotas.get("toolCalls").or_else(|| quotas.get("tool_calls")))
            .and_then(serde_json::Value::as_u64)
            .filter(|limit| *limit > 0)
        {
            quotas.insert(
                "toolCalls".to_string(),
                QuotaBucket { limit, used: 0 },
            );
        }
    }

    pub async fn register_quota(&self, key: impl Into<String>, limit: u64) {
        self.quotas
            .write()
            .await
            .insert(key.into(), QuotaBucket { limit, used: 0 });
    }

    pub async fn allow_tool_for_session(&self, session_id: &str, tool_name: &str) {
        if session_id.trim().is_empty() || tool_name.trim().is_empty() {
            return;
        }
        self.session_tool_allowlist
            .write()
            .await
            .insert(session_tool_key(session_id, tool_name));
    }

    pub async fn set_session_permission_mode(&self, session_id: &str, mode: PermissionMode) {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return;
        }
        self.session_permission_modes
            .write()
            .await
            .insert(session_id.to_string(), mode);
    }

    pub async fn clear_session_permission_mode(&self, session_id: &str) {
        self.session_permission_modes
            .write()
            .await
            .remove(session_id.trim());
    }

    pub async fn session_permission_mode(&self, session_id: &str) -> Option<PermissionMode> {
        self.session_permission_modes
            .read()
            .await
            .get(session_id.trim())
            .copied()
    }

    pub async fn evaluate(&self, request: GovernanceRequest) -> GovernanceResult {
        if let Some(budget) = self.preview_budget(request.budget_usage).await {
            if !budget.allowed {
                return self
                    .finish(
                        &request,
                        GovernanceDecision::Deny,
                        "budget_exceeded",
                        "budget",
                        assess_action_risk(&request.action),
                        FinishExtras {
                            budget: Some(budget),
                            ..FinishExtras::default()
                        },
                    )
                    .await;
            }
        }

        let quota = self.preview_quota(&request).await;
        if let Some(snapshot) = quota.clone() {
            if snapshot.remaining < request.quota_amount.max(1) {
                return self
                    .finish(
                        &request,
                        GovernanceDecision::Deny,
                        "quota_exceeded",
                        "quota",
                        assess_action_risk(&request.action),
                        FinishExtras {
                            quota: Some(snapshot),
                            ..FinishExtras::default()
                        },
                    )
                    .await;
            }
        }

        let (decision, reason, policy, risk, extras) = self.evaluate_action(&request).await;

        if decision == GovernanceDecision::Allow {
            if let Some(usage) = request.budget_usage {
                let mut current = self.budget_usage.write().await;
                current.tokens = current.tokens.saturating_add(usage.tokens);
                current.cost_usd += usage.cost_usd;
            }
            if let Some(key) = &request.quota_key {
                if let Some(bucket) = self.quotas.write().await.get_mut(key) {
                    bucket.used = bucket.used.saturating_add(request.quota_amount.max(1));
                }
            }
        }

        self.finish(&request, decision, reason, policy, risk, extras)
            .await
    }

    pub async fn audit_events(&self) -> Vec<AuditEvent> {
        self.audit_log.read().await.clone()
    }

    pub async fn record_audit_event(
        &self,
        actor: impl Into<String>,
        action: impl Into<String>,
        resource: Option<String>,
        decision: GovernanceDecision,
        reason: impl Into<String>,
        trace_id: Option<String>,
    ) -> AuditEvent {
        let event = AuditEvent {
            id: new_id("audit"),
            actor: actor.into(),
            action: action.into(),
            resource,
            decision,
            reason: reason.into(),
            trace_id,
            created_at: Utc::now(),
        };
        self.push_audit_event(event.clone()).await;
        event
    }

    async fn push_audit_event(&self, event: AuditEvent) {
        {
            let mut log = self.audit_log.write().await;
            log.push(event);
            const MAX_EVENTS: usize = 10_000;
            if log.len() > MAX_EVENTS {
                let drain = log.len() - MAX_EVENTS;
                log.drain(0..drain);
            }
        }
        self.flush_audit_to_disk().await;
    }

    async fn flush_audit_to_disk(&self) {
        let path = self.audit_persist_path.read().await.clone();
        let Some(path) = path else {
            return;
        };
        let events = self.audit_log.read().await.clone();
        if let Err(_err) = persist_audit_events_to_disk(&path, &events) {
            // 写入失败不阻塞主路径（与参考一致）。
        }
    }

    pub fn redact_output(&self, value: impl AsRef<str>) -> String {
        redact_string(value.as_ref())
    }

    async fn evaluate_action(
        &self,
        request: &GovernanceRequest,
    ) -> (
        GovernanceDecision,
        &'static str,
        &'static str,
        RiskAssessment,
        FinishExtras,
    ) {
        match &request.action {
            GovernanceAction::Tool {
                tool_name,
                args,
                writes_files,
                external_network,
                ..
            } => {
                let mut policy = self.policy.read().await.clone();
                if let Some(session_id) = session_id_from_args(args) {
                    if let Some(mode) = self.session_permission_modes.read().await.get(&session_id) {
                        policy.mode = *mode;
                        // Force mode-aware evaluation even when config omitted permissions block.
                        policy.configured_permissions = true;
                    }
                }
                let risk = assess_action_risk(&request.action);
                if let Some(reason) = hard_safety_denial(tool_name, args) {
                    return (
                        GovernanceDecision::Deny,
                        reason,
                        "safety",
                        risk,
                        FinishExtras::default(),
                    );
                }
                if matches_any(tool_name, policy.disabled_tools.iter().map(String::as_str)) {
                    return (
                        GovernanceDecision::Deny,
                        "tool_disabled_by_config",
                        "permission",
                        risk,
                        FinishExtras::default(),
                    );
                }
                if !tool_access_allows(&policy, tool_name, args) {
                    return (
                        GovernanceDecision::Deny,
                        "tool_denied_by_tool_access",
                        "tool_access",
                        risk,
                        FinishExtras::default(),
                    );
                }
                if policy.deny_tools.contains(tool_name)
                    || matches_any(tool_name, policy.blacklist.iter().map(String::as_str))
                {
                    return (
                        GovernanceDecision::Deny,
                        "tool_denied_by_policy",
                        "permission",
                        risk,
                        FinishExtras::default(),
                    );
                }
                if let Some(session_id) = session_id_from_args(args) {
                    if self
                        .session_tool_allowlist
                        .read()
                        .await
                        .contains(&session_tool_key(&session_id, tool_name))
                    {
                        return (
                            GovernanceDecision::Allow,
                            "tool_allowed_for_session",
                            "permission",
                            risk,
                            FinishExtras::default(),
                        );
                    }
                }
                if policy.configured_permissions {
                    let permission_level = configured_permission_level(&policy, tool_name);
                    match permission_level {
                        PermissionLevel::Deny => {
                            return (
                                GovernanceDecision::Deny,
                                "tool_denied_by_policy",
                                "permission",
                                risk,
                                FinishExtras::default(),
                            );
                        }
                        PermissionLevel::Allow => {
                            return (
                                GovernanceDecision::Allow,
                                "tool_allowed",
                                "permission",
                                risk,
                                FinishExtras::default(),
                            );
                        }
                        PermissionLevel::Ask => {
                            return (
                                GovernanceDecision::NeedsApproval,
                                "tool_requires_approval",
                                "permission",
                                risk,
                                FinishExtras::approval(),
                            );
                        }
                    }
                }
                if *external_network && !policy.allow_network {
                    return (
                        GovernanceDecision::NeedsApproval,
                        "network_tool_requires_approval",
                        "permission",
                        risk,
                        FinishExtras::approval(),
                    );
                }
                if *writes_files && !policy.allow_file_write {
                    return (
                        GovernanceDecision::NeedsApproval,
                        "write_tool_requires_approval",
                        "permission",
                        risk,
                        FinishExtras::approval(),
                    );
                }
                if policy.approval_tools.contains(tool_name) || contains_protected_marker(args) {
                    return (
                        GovernanceDecision::NeedsApproval,
                        "tool_requires_approval",
                        "permission",
                        risk,
                        FinishExtras::approval(),
                    );
                }
                if risk.level == RiskLevel::High {
                    return (
                        GovernanceDecision::NeedsApproval,
                        "high_risk_tool_action",
                        "risk",
                        risk,
                        FinishExtras::approval(),
                    );
                }
                (
                    GovernanceDecision::Allow,
                    "tool_allowed",
                    "permission",
                    risk,
                    FinishExtras::default(),
                )
            }
            GovernanceAction::MemoryWrite {
                category,
                content,
                sensitivity,
            } => {
                let decision = classify_memory_write(category, content, sensitivity.as_deref());
                let risk = assess_action_risk(&request.action);
                match decision {
                    MemoryDecision::Deny => (
                        GovernanceDecision::Deny,
                        "memory_policy_denied",
                        "memory_policy",
                        risk,
                        FinishExtras {
                            memory_decision: Some(decision),
                            ..FinishExtras::default()
                        },
                    ),
                    MemoryDecision::Pending => (
                        GovernanceDecision::NeedsApproval,
                        "memory_policy_pending",
                        "memory_policy",
                        risk,
                        FinishExtras {
                            approval_id: Some(new_id("approval")),
                            memory_decision: Some(decision),
                            ..FinishExtras::default()
                        },
                    ),
                    MemoryDecision::Direct => (
                        GovernanceDecision::Allow,
                        "memory_policy_direct",
                        "memory_policy",
                        risk,
                        FinishExtras {
                            memory_decision: Some(decision),
                            ..FinishExtras::default()
                        },
                    ),
                }
            }
            GovernanceAction::Output { content } => {
                let redacted = redact_string(content);
                (
                    GovernanceDecision::Allow,
                    if redacted == *content {
                        "output_allowed"
                    } else {
                        "output_redacted"
                    },
                    "output",
                    RiskAssessment::low("output"),
                    FinishExtras {
                        redacted_content: Some(redacted),
                        ..FinishExtras::default()
                    },
                )
            }
            GovernanceAction::Generic { .. } => {
                let risk = assess_action_risk(&request.action);
                if risk.level == RiskLevel::High {
                    (
                        GovernanceDecision::NeedsApproval,
                        "high_risk_action",
                        "risk",
                        risk,
                        FinishExtras::approval(),
                    )
                } else {
                    (
                        GovernanceDecision::Allow,
                        "generic_action_allowed",
                        "generic",
                        risk,
                        FinishExtras::default(),
                    )
                }
            }
        }
    }

    async fn preview_budget(&self, usage: Option<BudgetUsage>) -> Option<BudgetSnapshot> {
        let limit = (*self.budget_limit.read().await)?;
        let usage = usage?;
        let current = *self.budget_usage.read().await;
        let next_tokens = current.tokens.saturating_add(usage.tokens);
        let next_cost = current.cost_usd + usage.cost_usd;
        let allowed = next_tokens <= limit.max_tokens && next_cost <= limit.max_cost_usd;
        Some(BudgetSnapshot {
            tokens: next_tokens,
            cost_usd: next_cost,
            allowed,
            reason: (!allowed).then(|| "budget_exceeded".to_string()),
        })
    }

    async fn preview_quota(&self, request: &GovernanceRequest) -> Option<QuotaSnapshot> {
        let key = request.quota_key.as_ref()?;
        self.quotas.read().await.get(key).map(QuotaBucket::snapshot)
    }

    async fn finish(
        &self,
        request: &GovernanceRequest,
        decision: GovernanceDecision,
        reason: impl Into<String>,
        policy: impl Into<String>,
        risk: RiskAssessment,
        extras: FinishExtras,
    ) -> GovernanceResult {
        let reason = reason.into();
        let audit_id = new_id("audit");
        let audit = AuditEvent {
            id: audit_id.clone(),
            actor: request.actor.clone(),
            action: action_name(&request.action),
            resource: request
                .resource
                .clone()
                .or_else(|| action_resource(&request.action)),
            decision,
            reason: reason.clone(),
            trace_id: request.trace_id.clone(),
            created_at: Utc::now(),
        };
        self.push_audit_event(audit).await;
        GovernanceResult {
            decision,
            reason,
            policy: policy.into(),
            approval_id: extras.approval_id,
            risk,
            audit_id,
            budget: extras.budget,
            quota: extras.quota,
            memory_decision: extras.memory_decision,
            redacted_content: extras.redacted_content,
        }
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        Self::new(PermissionPolicy::default())
    }
}

#[derive(Default)]
struct FinishExtras {
    approval_id: Option<String>,
    budget: Option<BudgetSnapshot>,
    quota: Option<QuotaSnapshot>,
    memory_decision: Option<MemoryDecision>,
    redacted_content: Option<String>,
}

impl FinishExtras {
    fn approval() -> Self {
        Self {
            approval_id: Some(new_id("approval")),
            ..Self::default()
        }
    }
}

pub fn assess_action_risk(action: &GovernanceAction) -> RiskAssessment {
    let mut score = 0u8;
    let mut reasons = Vec::new();
    let mut add = |points: u8, reason: &str| {
        score = score.saturating_add(points);
        reasons.push(reason.to_string());
    };

    match action {
        GovernanceAction::Tool {
            writes_files,
            runs_command,
            external_network,
            touches_secrets,
            args,
            ..
        } => {
            if *writes_files {
                add(35, "writes_files");
            }
            if *runs_command {
                add(45, "runs_command");
            }
            if *external_network {
                add(25, "external_network");
            }
            if *touches_secrets || contains_protected_marker(args) {
                add(40, "touches_secrets");
            }
        }
        GovernanceAction::MemoryWrite {
            content,
            sensitivity,
            ..
        } => {
            if is_secret_like(content) || sensitivity.as_deref() == Some("secret") {
                add(45, "sensitive_memory");
            } else {
                add(10, "memory_write");
            }
        }
        GovernanceAction::Output { content } => {
            if is_secret_like(content) {
                add(30, "secret_like_output");
            }
        }
        GovernanceAction::Generic {
            writes_files,
            runs_command,
            external_network,
            touches_secrets,
            ..
        } => {
            if *writes_files {
                add(35, "writes_files");
            }
            if *runs_command {
                add(45, "runs_command");
            }
            if *external_network {
                add(25, "external_network");
            }
            if *touches_secrets {
                add(40, "touches_secrets");
            }
        }
    }

    if reasons.is_empty() {
        reasons.push("low_risk".to_string());
    }

    let level = if score >= 60 {
        RiskLevel::High
    } else if score >= 25 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    RiskAssessment {
        level,
        score,
        reasons,
    }
}

fn configured_permission_level(policy: &PermissionPolicy, tool_name: &str) -> PermissionLevel {
    if matches_any(tool_name, policy.whitelist.iter().map(String::as_str)) {
        return PermissionLevel::Allow;
    }
    let matched_rule = policy
        .rules
        .iter()
        .find(|rule| matches_tool_pattern(tool_name, &rule.pattern));
    let level = matched_rule
        .map(|rule| rule.level)
        .unwrap_or(policy.default_level);
    match policy.mode {
        PermissionMode::Default => default_mode_level(tool_name, level, matched_rule),
        PermissionMode::Auto => auto_mode_level(level, matched_rule),
        PermissionMode::Plan => {
            if is_execute_tool(tool_name) || level == PermissionLevel::Deny {
                PermissionLevel::Deny
            } else {
                PermissionLevel::Allow
            }
        }
    }
}

/// default：低风险放行，高风险确认；显式规则优先。
fn default_mode_level(
    tool_name: &str,
    configured: PermissionLevel,
    matched_rule: Option<&PermissionRule>,
) -> PermissionLevel {
    if configured == PermissionLevel::Deny {
        return PermissionLevel::Deny;
    }
    if let Some(rule) = matched_rule {
        if rule.level == PermissionLevel::Allow || rule.level == PermissionLevel::Ask {
            return rule.level;
        }
    }
    if is_always_ask_tool(tool_name) {
        return PermissionLevel::Ask;
    }
    if is_basic_tool(tool_name) {
        return PermissionLevel::Allow;
    }
    configured
}

/// auto：除显式 deny 外尽量不打断。
fn auto_mode_level(
    configured: PermissionLevel,
    matched_rule: Option<&PermissionRule>,
) -> PermissionLevel {
    if configured == PermissionLevel::Deny
        || matched_rule.map(|rule| rule.level == PermissionLevel::Deny).unwrap_or(false)
    {
        return PermissionLevel::Deny;
    }
    PermissionLevel::Allow
}

fn tool_access_allows(
    policy: &PermissionPolicy,
    tool_name: &str,
    args: &BTreeMap<String, serde_json::Value>,
) -> bool {
    let agent_id = string_arg_any(args, &["_agentId", "_agent_id"]);
    let agent_role = string_arg_any(args, &["_agentRole", "_agent_role"]);
    if agent_id.is_some() || agent_role.is_some() {
        let Some(rule) = resolve_subagent_tool_access(policy, agent_id, agent_role) else {
            return true;
        };
        return tool_access_rule_allows(&policy.tool_access, &rule, tool_name);
    }

    let _ = policy;
    let _ = tool_name;
    true
}

fn resolve_subagent_tool_access(
    policy: &PermissionPolicy,
    agent_id: Option<&str>,
    agent_role: Option<&str>,
) -> Option<ToolAccessRule> {
    let sub_agents = policy.tool_access.sub_agents.as_ref()?;
    if let Some(id) = agent_id {
        if let Some(rule) = sub_agents.by_id.get(id).cloned() {
            if rule.mode_or_inherit() != ToolAccessMode::Inherit {
                return Some(rule);
            }
        }
    }
    if let Some(role) = agent_role {
        if let Some(rule) = sub_agents.by_role.get(role).cloned() {
            if rule.mode_or_inherit() != ToolAccessMode::Inherit {
                return Some(rule);
            }
        }
    }
    sub_agents.defaults.clone()
}

fn tool_access_rule_allows(
    settings: &ToolAccessSettings,
    rule: &ToolAccessRule,
    tool_name: &str,
) -> bool {
    match rule.mode_or_inherit() {
        ToolAccessMode::Inherit => true,
        ToolAccessMode::Allowlist => matches_any(
            tool_name,
            settings
                .expand_patterns(&rule.allowed_tools)
                .iter()
                .map(String::as_str),
        ),
        ToolAccessMode::Denylist => !matches_any(
            tool_name,
            settings
                .expand_patterns(&rule.denied_tools)
                .iter()
                .map(String::as_str),
        ),
    }
}

fn hard_safety_denial(
    tool_name: &str,
    args: &BTreeMap<String, serde_json::Value>,
) -> Option<&'static str> {
    // 硬拦截只做敏感路径 / 危险 git / 危险命令。
    // 工作区边界由工具层按会话 cwd 校验，禁止用引擎进程 current_dir 当项目根。
    let file_related = tool_name.starts_with("file_") || tool_name.starts_with("mcp__");
    let exec_related = tool_name == "exec"
        || tool_name == "start_process"
        || tool_name == "write_process_stdin"
        || tool_name == "stop_process"
        || tool_name.starts_with("mcp__");
    let git_related = tool_name.starts_with("git_") || tool_name.starts_with("mcp__");
    if file_related || exec_related || git_related {
        for key in ["path", "file_path", "cwd", "target", "directory"] {
            if let Some(path_str) = args.get(key).and_then(serde_json::Value::as_str) {
                if path_str.trim().is_empty() {
                    continue;
                }
                if is_sensitive_path(path_str) {
                    return Some("sensitive_path_denied");
                }
            }
        }
    }
    if (tool_name == "git_branch" || tool_name.starts_with("mcp__"))
        && args.get("action").is_some()
        && !matches!(
            args.get("action").and_then(serde_json::Value::as_str),
            Some("list" | "create")
        )
    {
        return Some("dangerous_git_action_denied");
    }
    if (tool_name.starts_with("git_") || tool_name.starts_with("mcp__"))
        && [
            "reset",
            "clean",
            "force",
            "overwrite",
            "checkout",
            "switch",
            "rebase",
            "push",
        ]
        .iter()
        .any(|marker| tool_name.contains(marker))
    {
        return Some("dangerous_git_tool_denied");
    }
    if matches!(tool_name, "exec" | "start_process")
        || tool_name.starts_with("mcp__")
    {
        if let Some(command) = args.get("command").and_then(serde_json::Value::as_str) {
            if command_has_destructive_git(command) {
                return Some("destructive_git_denied");
            }
            let validation = CommandValidator::validate(command);
            if !validation.safe {
                return Some("dangerous_command_denied");
            }
            if is_dangerous_command(command) {
                return Some("dangerous_command_denied");
            }
        }
    }
    None
}

fn is_sensitive_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let segments = normalized.split('/').collect::<Vec<_>>();
    for marker in [
        ".git",
        ".claude",
        ".ssh",
        ".gnupg",
        ".npmrc",
        ".env",
        "id_rsa",
        "id_ed25519",
        ".p12",
        ".pfx",
        ".key",
        "/etc/shadow",
        "/etc/passwd",
        "/etc/hosts",
        "/etc/hostname",
        "/etc/ssl",
        "/etc/ssh",
        "/root",
        "/var/log",
        "c:/windows",
        "c:/windows/system32",
    ] {
        if marker.contains('/') && normalized.contains(marker) {
            return true;
        }
        if marker.contains('.') {
            if normalized.ends_with(marker)
                || segments
                    .iter()
                    .any(|segment| *segment == marker || segment.starts_with(&format!("{marker}.")))
            {
                return true;
            }
        } else if segments.contains(&marker) {
            return true;
        }
    }
    false
}

fn is_dangerous_command(command: &str) -> bool {
    dangerous_command_regex().is_match(command)
}

fn dangerous_command_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)(\brm\s+-rf\b|\bremove-item\b.*\b-recurse\b|\bgit\s+reset\s+--hard\b|\bgit\s+clean\b|\bformat\b|\bdel\s+/s\b)"#,
        )
        .expect("valid dangerous command regex")
    })
}

fn is_basic_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_read"
            | "read_file"
            | "search"
            | "search_files"
            | "glob"
            | "list_files"
            | "search_tools"
            | "load_tools"
            | "ask_user"
            | "search_memory"
            | "load_memories"
            | "codebase_index"
            | "read_process_output"
            | "web_search"
            | "git_status"
            | "git_diff"
            | "git_log"
            | "git_blame"
            | "git_platform_status"
            | "git_issue_task_draft"
            | "git_patch_create"
            | "validation_discover"
            | "validation_diagnose"
            | "validation_replay"
            | "acceptance_check"
            | "self_review"
            | "sandbox_modes"
            | "sandbox_list"
            | "sandbox_diff"
            | "changeset_list"
            | "changeset_preview"
            | "workspace_checkpoint_list"
            | "submit_task"
            | "get_task_status"
            | "list_tasks"
            | "cancel_task"
    ) || tool_name.starts_with("validation_")
        || tool_name.starts_with("delivery_")
}

fn is_always_ask_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "exec"
            | "execute_command"
            | "start_process"
            | "stop_process"
            | "file_write"
            | "write_to_file"
            | "file_edit"
            | "apply_patch"
            | "apply_diff"
            | "git_branch"
            | "git_commit"
            | "git_patch_apply"
            | "write_memory"
            | "workspace_checkpoint_create"
            | "workspace_checkpoint_restore"
    ) || tool_name.starts_with("sandbox_")
        || tool_name.starts_with("mcp__")
}

fn is_execute_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_write"
            | "file_edit"
            | "apply_patch"
            | "exec"
            | "start_process"
            | "stop_process"
            | "write_process_stdin"
            | "write_memory"
            | "workspace_checkpoint_create"
            | "workspace_checkpoint_restore"
    ) || tool_name.starts_with("sandbox_")
        || tool_name.starts_with("web_")
        || tool_name.starts_with("mcp__")
}

fn default_permission_rules() -> Vec<PermissionRule> {
    [
        ("file_read", PermissionLevel::Allow),
        ("search", PermissionLevel::Allow),
        ("glob", PermissionLevel::Allow),
        ("list_files", PermissionLevel::Allow),
        ("search_tools", PermissionLevel::Allow),
        ("load_tools", PermissionLevel::Allow),
        ("ask_user", PermissionLevel::Allow),
        ("search_memory", PermissionLevel::Allow),
        ("load_memories", PermissionLevel::Allow),
        ("codebase_index", PermissionLevel::Allow),
        ("list_processes", PermissionLevel::Allow),
        ("read_process_output", PermissionLevel::Allow),
        ("web_search", PermissionLevel::Allow),
        ("web_fetch", PermissionLevel::Ask),
        ("mcp__*", PermissionLevel::Ask),
        ("file_write", PermissionLevel::Ask),
        ("file_edit", PermissionLevel::Ask),
        ("apply_patch", PermissionLevel::Ask),
        ("exec", PermissionLevel::Ask),
        ("start_process", PermissionLevel::Ask),
        ("write_process_stdin", PermissionLevel::Ask),
        ("stop_process", PermissionLevel::Ask),
        ("sandbox_*", PermissionLevel::Ask),
        ("validation_replay", PermissionLevel::Ask),
        ("validation_*", PermissionLevel::Allow),
        ("acceptance_check", PermissionLevel::Allow),
        ("self_review", PermissionLevel::Allow),
        ("git_status", PermissionLevel::Allow),
        ("git_diff", PermissionLevel::Allow),
        ("git_log", PermissionLevel::Allow),
        ("git_blame", PermissionLevel::Allow),
        ("git_platform_status", PermissionLevel::Allow),
        ("git_issue_task_draft", PermissionLevel::Allow),
        ("git_branch", PermissionLevel::Ask),
        ("git_commit", PermissionLevel::Ask),
        ("git_patch_create", PermissionLevel::Allow),
        ("git_patch_apply", PermissionLevel::Ask),
        ("delivery_*", PermissionLevel::Allow),
        ("write_memory", PermissionLevel::Ask),
        ("changeset_list", PermissionLevel::Allow),
        ("changeset_preview", PermissionLevel::Allow),
        ("workspace_checkpoint_list", PermissionLevel::Allow),
        ("workspace_checkpoint_create", PermissionLevel::Ask),
        ("workspace_checkpoint_restore", PermissionLevel::Ask),
        ("submit_task", PermissionLevel::Allow),
        ("agent", PermissionLevel::Allow),
        ("get_task_status", PermissionLevel::Allow),
        ("list_tasks", PermissionLevel::Allow),
        ("cancel_task", PermissionLevel::Allow),
        ("enter_plan_mode", PermissionLevel::Allow),
        ("exit_plan_mode", PermissionLevel::Allow),
        ("todo_write", PermissionLevel::Allow),
        ("update_goal", PermissionLevel::Allow),
        ("activate_skill", PermissionLevel::Allow),
    ]
    .into_iter()
    .map(|(pattern, level)| PermissionRule {
        pattern: pattern.to_string(),
        level,
    })
    .collect()
}

fn permission_rule_from_value(value: &serde_json::Value) -> Option<PermissionRule> {
    let object = value.as_object()?;
    let pattern = object
        .get("pattern")
        .and_then(serde_json::Value::as_str)?
        .trim();
    let level = object
        .get("level")
        .and_then(serde_json::Value::as_str)
        .and_then(PermissionLevel::from_str)?;
    if pattern.is_empty() {
        return None;
    }
    Some(PermissionRule {
        pattern: pattern.to_string(),
        level,
    })
}

fn tool_access_rule_map(value: Option<&serde_json::Value>) -> BTreeMap<String, ToolAccessRule> {
    value
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    ToolAccessRule::from_value(value).map(|rule| (key.clone(), rule))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_set_at(value: &serde_json::Value, key: &str) -> Option<BTreeSet<String>> {
    value.get(key).and_then(string_vec_value).map(|items| {
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect()
    })
}

fn string_set_from_object(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<BTreeSet<String>> {
    object.get(key).and_then(string_vec_value).map(|items| {
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect()
    })
}

fn string_vec_value(value: &serde_json::Value) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    })
}

fn string_arg_any<'a>(
    args: &'a BTreeMap<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn matches_any<'a, I>(tool_name: &str, patterns: I) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    patterns
        .into_iter()
        .any(|pattern| matches_tool_pattern(tool_name, pattern))
}

fn matches_tool_pattern(tool_name: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" || pattern == tool_name {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    // Align with UI foundation: `*` = segment (`[^/]*`), `**` = cross-segment (`.*`).
    let mut regex = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                regex.push_str(".*");
                i += 2;
            }
            '*' => {
                regex.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                regex.push('.');
                i += 1;
            }
            other => {
                regex.push_str(&regex::escape(&other.to_string()));
                i += 1;
            }
        }
    }
    regex.push('$');
    Regex::new(&regex)
        .map(|compiled| compiled.is_match(tool_name))
        .unwrap_or(false)
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if !value.is_empty() && seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

pub fn classify_memory_write(
    category: &str,
    content: &str,
    sensitivity: Option<&str>,
) -> MemoryDecision {
    if sensitivity == Some("secret") || is_secret_like(content) {
        return MemoryDecision::Deny;
    }
    if matches!(
        category,
        "relationship" | "self_archive" | "user_profile" | "voice_profile" | "safety_note"
    ) || sensitivity == Some("personal")
    {
        return MemoryDecision::Pending;
    }
    MemoryDecision::Direct
}

pub fn redact_string(value: &str) -> String {
    let mut output = value.to_string();
    for pattern in secret_value_patterns() {
        output = pattern.replace_all(&output, "[REDACTED_SECRET]").into_owned();
    }
    output = bearer_pattern()
        .replace_all(&output, "Bearer [REDACTED]")
        .into_owned();
    output = aws_key_pattern()
        .replace_all(&output, "[REDACTED_AWS_KEY]")
        .into_owned();
    output = jwt_pattern()
        .replace_all(&output, "[REDACTED_JWT]")
        .into_owned();
    output = connection_string_pattern()
        .replace_all(&output, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let scheme_end = full.find("://").map(|i| i + 3).unwrap_or(0);
            if let Some(at) = full.find('@') {
                format!("{}[REDACTED]{}", &full[..scheme_end], &full[at..])
            } else {
                "[REDACTED_CONNECTION]".to_string()
            }
        })
        .into_owned();
    output = private_key_pattern()
        .replace_all(&output, "[REDACTED_PRIVATE_KEY]")
        .into_owned();
    output = secret_assignment_pattern()
        .replace_all(&output, |caps: &regex::Captures| {
            format!("{}[REDACTED]", caps.get(1).map(|m| m.as_str()).unwrap_or(""))
        })
        .into_owned();
    output = email_pattern()
        .replace_all(&output, "[REDACTED_EMAIL]")
        .into_owned();
    if output.len() > 32_000 {
        output.truncate(32_000);
        output.push_str("…[TRUNCATED]");
    }
    output
}

fn secret_value_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"\bsk-[A-Za-z0-9_-]{16,}\b",
            r"\b(?:xai|gsk|ghp|github_pat|anthropic|AIza)[A-Za-z0-9_-]{16,}\b",
            r"\b[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{24,}\b",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("secret value pattern"))
        .collect()
    })
}

fn bearer_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{16,}\b").expect("bearer pattern")
    })
}

fn aws_key_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?:AKIA|ASIA|AIDA|AROA)[0-9A-Z]{16}").expect("aws key pattern")
    })
}

fn jwt_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}")
            .expect("jwt pattern")
    })
}

fn connection_string_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)(?:mongodb|postgres|mysql|redis)://[^:\s]+:[^@\s]+@")
            .expect("connection string pattern")
    })
}

fn private_key_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]{0,3000}?-----END [A-Z ]*PRIVATE KEY-----")
            .expect("private key pattern")
    })
}

fn secret_assignment_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)(\b(?:api[_-]?key|token|secret|password|authorization|cookie)\b\s*[:=]\s*)['"]?([^'"\s,}]+)"#,
        )
        .expect("secret assignment pattern")
    })
}

fn email_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").expect("email pattern")
    })
}

fn contains_protected_marker(args: &BTreeMap<String, serde_json::Value>) -> bool {
    args.values().any(|value| match value {
        serde_json::Value::String(text) => is_secret_like(text),
        other => is_secret_like(&other.to_string()),
    })
}

fn session_id_from_args(args: &BTreeMap<String, serde_json::Value>) -> Option<String> {
    ["sessionId", "session_id", "_sessionId", "_session_id"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_tool_key(session_id: &str, tool_name: &str) -> String {
    format!("{session_id}\u{1f}{tool_name}")
}

fn is_secret_like(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        ".env",
        "id_rsa",
        "id_ed25519",
        "api_key",
        "apikey",
        "secret",
        "token",
        "password",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn action_name(action: &GovernanceAction) -> String {
    match action {
        GovernanceAction::Tool { tool_name, .. } => format!("tool:{tool_name}"),
        GovernanceAction::MemoryWrite { category, .. } => format!("memory:{category}"),
        GovernanceAction::Output { .. } => "output".to_string(),
        GovernanceAction::Generic { .. } => "generic".to_string(),
    }
}

fn action_resource(action: &GovernanceAction) -> Option<String> {
    match action {
        GovernanceAction::Tool { tool_name, .. } => Some(tool_name.clone()),
        GovernanceAction::MemoryWrite { category, .. } => Some(format!("memory:{category}")),
        GovernanceAction::Output { .. } => Some("output".to_string()),
        GovernanceAction::Generic { resource, .. } => resource.clone(),
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}

const MAX_AUDIT_EVENTS: usize = 10_000;

fn load_audit_events_from_disk(path: &Path) -> Option<Vec<AuditEvent>> {
    let raw = fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let items = parsed.as_array()?;
    let mut events = Vec::new();
    for item in items.iter().rev().take(MAX_AUDIT_EVENTS).rev() {
        if let Ok(event) = serde_json::from_value::<AuditEvent>(item.clone()) {
            events.push(event);
        }
    }
    Some(events)
}

fn persist_audit_events_to_disk(path: &Path, events: &[AuditEvent]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("mkdir audit dir: {err}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_vec_pretty(events).map_err(|err| err.to_string())?;
    fs::write(&tmp, payload).map_err(|err| format!("write audit tmp: {err}"))?;
    fs::rename(&tmp, path).map_err(|err| format!("rename audit: {err}"))?;
    Ok(())
}

const MAX_SENSITIVE_SCAN_BYTES: u64 = 512 * 1024;
const PROJECT_CONFIG_FILE_NAME: &str = "xuanyu.json";

pub fn scan_sensitive_files(project_dir: impl AsRef<Path>) -> SensitiveScanResult {
    let project_dir = project_dir.as_ref();
    let mut findings = Vec::new();
    for file_path in sensitive_candidates(project_dir) {
        let Some(content) = read_small_text_file(&file_path) else {
            continue;
        };
        let base = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if base == PROJECT_CONFIG_FILE_NAME {
            scan_json_config(&file_path, &content, &mut findings);
        } else if base.starts_with(".env") {
            scan_env_file(&file_path, &content, &mut findings);
        } else {
            scan_content(&file_path, &content, &mut findings);
        }
    }
    SensitiveScanResult { findings }
}

pub fn format_sensitive_scan_warnings(result: &SensitiveScanResult) -> Vec<String> {
    if result.findings.is_empty() {
        return Vec::new();
    }
    let mut warnings = vec!["Sensitive configuration warning:".to_string()];
    warnings.extend(
        result
            .findings
            .iter()
            .map(|finding| format!("  - {}: {}", finding.file_path, finding.message)),
    );
    warnings.push(
        "These files are ignored by git by default; keep real keys out of logs, traces, and shared output."
            .to_string(),
    );
    warnings
}

fn sensitive_candidates(project_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files.push(xuanyu_storage::resolve_global_config_file());
    files.push(project_dir.join(".env"));
    files.push(project_dir.join(".env.local"));
    if let Ok(entries) = fs::read_dir(project_dir) {
        for entry in entries.flatten() {
            let file_path = entry.path();
            let Some(name) = file_path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with(".env.") && !files.iter().any(|item| item == &file_path) {
                files.push(file_path);
            }
        }
    }
    files
}

fn scan_json_config(file_path: &Path, content: &str, findings: &mut Vec<SensitiveFinding>) {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(parsed) => {
            let api_key = parsed
                .get("provider")
                .and_then(serde_json::Value::as_object)
                .and_then(|provider| provider.get("apiKey"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !api_key.is_empty() && !is_placeholder(api_key) {
                findings.push(SensitiveFinding {
                    file_path: path_to_string(file_path),
                    kind: SensitiveFindingKind::Config,
                    message: "global config contains a non-placeholder provider apiKey".to_string(),
                });
            }
        }
        Err(_) => scan_content(file_path, content, findings),
    }
}

fn scan_env_file(file_path: &Path, content: &str, findings: &mut Vec<SensitiveFinding>) {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(captures) = env_assignment_regex().captures(trimmed) else {
            continue;
        };
        let key = captures
            .get(1)
            .map(|item| item.as_str())
            .unwrap_or_default();
        let value = captures
            .get(2)
            .map(|item| item.as_str())
            .unwrap_or_default()
            .trim_matches(['"', '\''])
            .trim();
        if !value.is_empty() && !is_placeholder(value) {
            findings.push(SensitiveFinding {
                file_path: path_to_string(file_path),
                kind: SensitiveFindingKind::Env,
                message: format!("{key} is set in a local env file"),
            });
        }
    }
}

fn scan_content(file_path: &Path, content: &str, findings: &mut Vec<SensitiveFinding>) {
    if secret_assignment_regex().is_match(content)
        || secret_value_regexes()
            .iter()
            .any(|pattern| pattern.is_match(content))
    {
        findings.push(SensitiveFinding {
            file_path: path_to_string(file_path),
            kind: SensitiveFindingKind::Content,
            message: "file appears to contain secret-like content".to_string(),
        });
    }
}

fn read_small_text_file(file_path: &Path) -> Option<String> {
    let metadata = fs::metadata(file_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SENSITIVE_SCAN_BYTES {
        return None;
    }
    fs::read_to_string(file_path).ok()
}

fn is_placeholder(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "your-api-key"
            | "your_api_key"
            | "replace-me"
            | "changeme"
            | "placeholder"
            | "<your-api-key>"
            | "${xy_api_key}"
    )
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn env_assignment_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)^([A-Za-z_][A-Za-z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|AUTH)[A-Za-z0-9_]*)\s*=\s*(.+)$"#,
        )
        .expect("valid env sensitive assignment regex")
    })
}

fn secret_assignment_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)(\b(?:api[_-]?key|token|secret|password|authorization|cookie)\b\s*[:=]\s*)['"]?([^'"\s,}]+)"#,
        )
        .expect("valid secret assignment regex")
    })
}

fn secret_value_regexes() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r#"\bsk-[A-Za-z0-9_-]{16,}\b"#).expect("valid sk regex"),
            Regex::new(r#"\b(xai|gsk|ghp|github_pat|anthropic|AIza)[A-Za-z0-9_-]{16,}\b"#)
                .expect("valid provider key regex"),
            Regex::new(r#"\b[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{24,}\b"#)
                .expect("valid jwt-like regex"),
        ]
    })
}
