use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;
use xuanyu_storage::{ControlPlaneHandle, UpsertTaskInput};

mod assess_merge;
mod watchdog;

pub use assess_merge::{
    assess_merge, SubAgentConflict, SubAgentMergeAssessment, MergeConflictSeverity,
    MergeConflictType,
};
pub use watchdog::{
    build_recovery_hint, SubAgentTimelineBuffer, SubAgentWatchdog, WatchdogIssue,
    WatchdogRecoveryAction, WatchdogRecoveryPlan, WatchdogSettings, WatchdogToolCall,
    WatchdogToolOutput,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub permission_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    pub can_write: bool,
    pub can_talk_to_user: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_sandbox_mode: Option<String>,
    pub max_turns: u32,
    pub memory_scope: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInput {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    pub permission_mode: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub model_profile: Option<String>,
    pub can_write: Option<bool>,
    pub can_talk_to_user: Option<bool>,
    pub default_sandbox_mode: Option<String>,
    pub max_turns: Option<u32>,
    pub memory_scope: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentProgressUpdate {
    pub task_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub step_title: String,
    pub summary: String,
    pub evidence: Vec<String>,
    pub risks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTimelineEvent {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub payload: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskPlanStep {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskPlan {
    pub title: String,
    pub steps: Vec<SubAgentTaskPlanStep>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskInput {
    pub agent_id: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskResult {
    pub task_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub status: String,
    pub conclusion: String,
    pub evidence: Vec<String>,
    pub modified_files: Vec<String>,
    pub risks: Vec<String>,
    pub blockers: Vec<String>,
    pub next_steps: Vec<String>,
    pub output: String,
    pub started_at: i64,
    pub completed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentWatchdogState {
    pub last_event_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<i64>,
    pub repeated_tool_count: u32,
    pub consecutive_errors: u32,
    pub retries: u32,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTaskLedger {
    pub task_id: String,
    pub agent_id: String,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub task: SubAgentTaskInput,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_plane_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_plane_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_plane_session_id: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<SubAgentTaskPlan>,
    pub progress: Vec<SubAgentProgressUpdate>,
    pub timeline: Vec<SubAgentTimelineEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<SubAgentTaskResult>,
    pub watchdog: SubAgentWatchdogState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTeamMember {
    pub agent_id: String,
    pub name: String,
    pub role: String,
    pub description: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,
    pub model: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    pub tools: Vec<String>,
    pub can_write: bool,
    pub can_talk_to_user: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_sandbox_mode: Option<String>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_result: Option<SubAgentTaskResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_progress: Option<SubAgentProgressUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_task_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTeamSnapshot {
    pub generated_at: i64,
    pub members: Vec<SubAgentTeamMember>,
    pub running_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub active_tasks: Vec<SubAgentTaskLedger>,
    pub recent_tasks: Vec<SubAgentTaskLedger>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMutationResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct AgentRuntime {
    agents_dir: PathBuf,
    tasks_path: PathBuf,
    agents: RwLock<BTreeMap<String, AgentDefinition>>,
    tasks: RwLock<BTreeMap<String, SubAgentTaskLedger>>,
    control_plane: Option<ControlPlaneHandle>,
    watchdog: RwLock<SubAgentWatchdog>,
    timeline_buffer: RwLock<SubAgentTimelineBuffer>,
}

impl AgentRuntime {
    pub fn new(data_dir: PathBuf, control_plane: Option<ControlPlaneHandle>) -> Arc<Self> {
        let agents_dir = data_dir.join("agents");
        Arc::new(Self {
            tasks_path: agents_dir.join("tasks.json"),
            agents_dir,
            agents: RwLock::new(BTreeMap::new()),
            tasks: RwLock::new(BTreeMap::new()),
            control_plane,
            watchdog: RwLock::new(SubAgentWatchdog::new()),
            timeline_buffer: RwLock::new(SubAgentTimelineBuffer::default()),
        })
    }

    pub async fn poll_watchdog(&self) -> Vec<(String, Vec<WatchdogIssue>)> {
        let mut watchdog = self.watchdog.write().await;
        if !watchdog.should_poll() {
            return Vec::new();
        }
        drop(watchdog);
        let now = Utc::now().timestamp_millis();
        let tasks = self.tasks.read().await;
        tasks
            .iter()
            .map(|(task_id, ledger)| {
                let issues = SubAgentWatchdog::check(ledger, now);
                (task_id.clone(), issues)
            })
            .filter(|(_, issues)| !issues.is_empty())
            .collect()
    }

    pub async fn track_tool_call(&self, task_id: &str, call: &WatchdogToolCall) {
        let mut tasks = self.tasks.write().await;
        if let Some(ledger) = tasks.get_mut(task_id) {
            SubAgentWatchdog::track_repeated_tool(ledger, call);
            ledger.updated_at = now_ms();
        }
    }

    pub async fn record_tool_result(&self, task_id: &str, is_error: bool) {
        let mut tasks = self.tasks.write().await;
        if let Some(ledger) = tasks.get_mut(task_id) {
            SubAgentWatchdog::record_tool_error(ledger, &WatchdogToolOutput { is_error });
            ledger.updated_at = now_ms();
        }
    }

    pub async fn apply_watchdog_recovery(
        &self,
        task_id: &str,
        issue: &WatchdogIssue,
        settings: &WatchdogSettings,
    ) -> Option<WatchdogRecoveryPlan> {
        let warning = format!("{}: {}", issue.kind, issue.message);
        let old_ledger = {
            let tasks = self.tasks.read().await;
            let ledger = tasks.get(task_id)?;
            if ledger.status != "running" {
                return None;
            }
            if ledger.watchdog.warnings.iter().any(|existing| existing == &warning) {
                return None;
            }
            ledger.clone()
        };
        let recovery_hint = build_recovery_hint(issue);
        let now = now_ms();
        if !settings.auto_recover {
            let mut tasks = self.tasks.write().await;
            let ledger = tasks.get_mut(task_id)?;
            ledger.watchdog.warnings.push(warning.clone());
            ledger.status = "blocked".to_string();
            ledger.watchdog.blocked_reason = Some(warning.clone());
            ledger.updated_at = now;
            ledger.timeline.push(watchdog_timeline_event(ledger, &warning, &issue.kind, now));
            drop(tasks);
            let _ = self.flush_tasks().await;
            return Some(WatchdogRecoveryPlan {
                task_id: task_id.to_string(),
                new_task_id: None,
                action: WatchdogRecoveryAction::Block,
                reason: warning,
            });
        }
        {
            let mut tasks = self.tasks.write().await;
            let ledger = tasks.get_mut(task_id)?;
            ledger.watchdog.warnings.push(warning.clone());
            ledger.watchdog.retries = ledger.watchdog.retries.saturating_add(1);
            ledger.status = "blocked".to_string();
            ledger.watchdog.blocked_reason = Some(recovery_hint.clone());
            ledger.updated_at = now;
            ledger.timeline.push(watchdog_timeline_event(
                ledger,
                &warning,
                &issue.kind,
                now,
            ));
        }
        let mut metadata = old_ledger.task.metadata.clone();
        metadata.insert(
            "restartedFromTaskId".to_string(),
            json!(old_ledger.task_id),
        );
        metadata.insert("watchdogRecovery".to_string(), json!(true));
        let restart_input = SubAgentTaskInput {
            agent_id: old_ledger.agent_id.clone(),
            task: format!(
                "[Watchdog recovery attempt {}]\n{}\n\n{}",
                old_ledger.watchdog.retries.saturating_add(1),
                recovery_hint,
                old_ledger.task.task
            ),
            context: old_ledger.task.context.clone(),
            write_scope: old_ledger.task.write_scope.clone(),
            metadata,
        };
        let new_ledger = self
            .submit_task(
                restart_input,
                old_ledger.control_plane_session_id.clone(),
                old_ledger.control_plane_run_id.clone(),
            )
            .await
            .ok()?;
        let _ = self.flush_tasks().await;
        Some(WatchdogRecoveryPlan {
            task_id: task_id.to_string(),
            new_task_id: Some(new_ledger.task_id.clone()),
            action: WatchdogRecoveryAction::Retry,
            reason: recovery_hint,
        })
    }

    pub async fn append_timeline_reasoning(&self, task_id: &str, chunk: &str) {
        self.timeline_buffer
            .write()
            .await
            .append_reasoning(task_id, chunk);
    }

    pub async fn append_timeline_text(&self, task_id: &str, chunk: &str) {
        self.timeline_buffer.write().await.append_text(task_id, chunk);
    }

    pub async fn flush_timeline_event(
        &self,
        task_id: &str,
        event_type: &str,
        agent_id: &str,
    ) -> Option<SubAgentTimelineEvent> {
        let mut event = self
            .timeline_buffer
            .write()
            .await
            .flush(task_id, event_type)?;
        event.agent_id = agent_id.to_string();
        event.task_id = task_id.to_string();
        let mut tasks = self.tasks.write().await;
        if let Some(ledger) = tasks.get_mut(task_id) {
            ledger.timeline.push(event.clone());
            ledger.watchdog.last_event_at = event.timestamp;
            ledger.updated_at = event.timestamp;
        }
        Some(event)
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.agents_dir).await?;
        let loaded = load_custom_agents(&self.agents_dir).await?;
        let loaded_tasks = load_task_ledgers(&self.tasks_path).await?;
        let mut agents = self.agents.write().await;
        agents.clear();
        agents.extend(loaded.into_iter().map(|agent| (agent.id.clone(), agent)));
        let mut tasks = self.tasks.write().await;
        tasks.clear();
        tasks.extend(
            loaded_tasks
                .into_iter()
                .map(|ledger| (ledger.task_id.clone(), ledger)),
        );
        Ok(())
    }

    pub async fn list_agents(&self) -> Vec<AgentDefinition> {
        let custom = self.agents.read().await;
        let mut agents = builtin_agents();
        agents.extend(custom.values().cloned());
        agents
    }

    pub async fn create_agent(&self, input: AgentInput) -> AgentMutationResult {
        match self.create_agent_inner(input).await {
            Ok(()) => AgentMutationResult {
                ok: true,
                error: None,
            },
            Err(err) => AgentMutationResult {
                ok: false,
                error: Some(err.to_string()),
            },
        }
    }

    pub async fn update_agent(&self, id: String, input: AgentInput) -> AgentMutationResult {
        match self.update_agent_inner(id, input).await {
            Ok(()) => AgentMutationResult {
                ok: true,
                error: None,
            },
            Err(err) => AgentMutationResult {
                ok: false,
                error: Some(err.to_string()),
            },
        }
    }

    pub async fn delete_agent(&self, id: String) -> AgentMutationResult {
        match self.delete_agent_inner(id).await {
            Ok(()) => AgentMutationResult {
                ok: true,
                error: None,
            },
            Err(err) => AgentMutationResult {
                ok: false,
                error: Some(err.to_string()),
            },
        }
    }

    pub async fn team_status(&self, session_id: Option<String>) -> SubAgentTeamSnapshot {
        let generated_at = now_ms();
        let agents = self.list_agents().await;
        let tasks = self.tasks.read().await;
        let visible_tasks = filtered_tasks(&tasks, session_id.as_deref());
        let active_tasks = visible_tasks
            .iter()
            .filter(|task| task.status == "queued" || task.status == "running")
            .cloned()
            .collect::<Vec<_>>();
        let recent_tasks = visible_tasks.iter().take(20).cloned().collect::<Vec<_>>();
        let members = agents
            .into_iter()
            .map(|agent| team_member(agent, &visible_tasks, generated_at))
            .collect::<Vec<_>>();
        SubAgentTeamSnapshot {
            generated_at,
            members,
            running_count: active_tasks.len(),
            completed_count: visible_tasks
                .iter()
                .filter(|task| task.status == "completed")
                .count(),
            failed_count: visible_tasks
                .iter()
                .filter(|task| task.status == "failed" || task.status == "blocked")
                .count(),
            active_tasks,
            recent_tasks,
        }
    }

    pub async fn subagent_timeline(
        &self,
        session_id: Option<String>,
        task_id: String,
    ) -> (Option<SubAgentTaskLedger>, Vec<SubAgentTimelineEvent>) {
        let tasks = self.tasks.read().await;
        let ledger = tasks.get(&task_id).and_then(|ledger| {
            if session_matches(ledger, session_id.as_deref()) {
                Some(ledger.clone())
            } else {
                None
            }
        });
        let timeline = ledger
            .as_ref()
            .map(|ledger| ledger.timeline.clone())
            .unwrap_or_default();
        (ledger, timeline)
    }

    pub async fn list_tasks(
        &self,
        session_id: Option<String>,
        limit: usize,
    ) -> Vec<SubAgentTaskLedger> {
        let tasks = self.tasks.read().await;
        filtered_tasks(&tasks, session_id.as_deref())
            .into_iter()
            .take(limit)
            .collect()
    }

    pub async fn task_status(&self, task_id: String) -> Option<SubAgentTaskLedger> {
        self.tasks.read().await.get(&task_id).cloned()
    }

    pub async fn agent_definition(&self, agent_id: &str) -> Option<AgentDefinition> {
        self.list_agents()
            .await
            .into_iter()
            .find(|agent| agent.id == agent_id)
    }

    pub async fn submit_task(
        &self,
        input: SubAgentTaskInput,
        session_id: Option<String>,
        run_id: Option<String>,
    ) -> anyhow::Result<SubAgentTaskLedger> {
        if input.task.trim().is_empty() {
            anyhow::bail!("Sub-agent task is required");
        }
        let agents = self.list_agents().await;
        let Some(agent) = agents.into_iter().find(|agent| agent.id == input.agent_id) else {
            anyhow::bail!("Sub-agent {} not found", input.agent_id);
        };
        let now = now_ms();
        let task_id = format!("task_{}_{}", agent.id, Uuid::new_v4().simple());
        let plan = initial_task_plan(&task_id, &input, &agent, now);
        let mut payload = BTreeMap::new();
        payload.insert("task".to_string(), json!(input.task.clone()));
        if let Some(context) = &input.context {
            payload.insert("context".to_string(), json!(context));
        }
        if !input.write_scope.is_empty() {
            payload.insert("writeScope".to_string(), json!(input.write_scope.clone()));
        }
        let control_plane_task_id = metadata_string_any(
            &input.metadata,
            &["controlPlaneTaskId", "control_plane_task_id"],
        );
        let control_plane_run_id = run_id.or_else(|| {
            metadata_string_any(
                &input.metadata,
                &[
                    "controlPlaneRunId",
                    "control_plane_run_id",
                    "runId",
                    "run_id",
                ],
            )
        });
        let control_plane_session_id = session_id.or_else(|| {
            metadata_string_any(
                &input.metadata,
                &[
                    "controlPlaneSessionId",
                    "control_plane_session_id",
                    "sessionId",
                    "session_id",
                ],
            )
        });
        if let Some(control_plane_task_id) = &control_plane_task_id {
            payload.insert(
                "controlPlaneTaskId".to_string(),
                json!(control_plane_task_id),
            );
        }
        if let Some(control_plane_run_id) = &control_plane_run_id {
            payload.insert("controlPlaneRunId".to_string(), json!(control_plane_run_id));
        }
        if let Some(control_plane_session_id) = &control_plane_session_id {
            payload.insert(
                "controlPlaneSessionId".to_string(),
                json!(control_plane_session_id),
            );
        }
        let event = SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: task_id.clone(),
            agent_id: agent.id.clone(),
            event_type: "delegated".to_string(),
            timestamp: now,
            title: Some(format!("Task delegated to {}", agent.name)),
            status: Some("queued".to_string()),
            payload,
        };
        let ledger = SubAgentTaskLedger {
            task_id: task_id.clone(),
            agent_id: agent.id.clone(),
            agent_name: agent.name.clone(),
            role: agent.role.clone(),
            task: input,
            status: "queued".to_string(),
            provider: agent.provider.clone(),
            model: agent.model.clone(),
            control_plane_task_id,
            control_plane_run_id,
            control_plane_session_id,
            started_at: now,
            updated_at: now,
            completed_at: None,
            current_step_id: None,
            plan: Some(plan),
            progress: Vec::new(),
            timeline: vec![event],
            result: None,
            watchdog: SubAgentWatchdogState {
                last_event_at: now,
                last_progress_at: None,
                repeated_tool_count: 0,
                consecutive_errors: 0,
                retries: 0,
                warnings: Vec::new(),
                last_tool_fingerprint: None,
                blocked_reason: None,
            },
        };
        self.tasks.write().await.insert(task_id, ledger.clone());
        self.flush_tasks().await?;
        Ok(ledger)
    }

    pub async fn start_task(&self, task_id: &str) -> anyhow::Result<SubAgentTaskLedger> {
        let mut tasks = self.tasks.write().await;
        let Some(ledger) = tasks.get_mut(task_id) else {
            anyhow::bail!("Task {task_id} not found");
        };
        if ledger.status == "completed"
            || ledger.status == "failed"
            || ledger.status == "cancelled"
            || ledger.status == "blocked"
        {
            anyhow::bail!("Task {task_id} is already {}", ledger.status);
        }
        let now = now_ms();
        ledger.status = "running".to_string();
        ledger.updated_at = now;
        ledger.watchdog.last_event_at = now;
        ledger.watchdog.warnings.retain(|warning| {
            !warning.contains("Executor integration pending")
                && !warning.contains("queued in Rust ledger")
        });
        ledger
            .watchdog
            .warnings
            .push("Rust local executor active; governed tool execution is enabled.".to_string());
        mark_first_pending_step_running(ledger, now);
        ledger.timeline.push(SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            event_type: "progress".to_string(),
            timestamp: now,
            title: Some("Sub-agent task started".to_string()),
            status: Some(ledger.status.clone()),
            payload: BTreeMap::from([
                ("taskId".to_string(), json!(ledger.task_id.clone())),
                ("agentId".to_string(), json!(ledger.agent_id.clone())),
                ("stepTitle".to_string(), json!("Sub-agent task started")),
                (
                    "summary".to_string(),
                    json!("The task was claimed by the Rust local executor."),
                ),
            ]),
        });
        let task = ledger.clone();
        drop(tasks);
        self.flush_tasks().await?;
        Ok(task)
    }

    pub async fn report_progress(
        &self,
        task_id: &str,
        summary: String,
        evidence: Vec<String>,
        risks: Vec<String>,
        next_step: Option<String>,
    ) -> anyhow::Result<SubAgentTaskLedger> {
        let mut tasks = self.tasks.write().await;
        let Some(ledger) = tasks.get_mut(task_id) else {
            anyhow::bail!("Task {task_id} not found");
        };
        if ledger.status == "blocked" || ledger.status == "cancelled" {
            return Ok(ledger.clone());
        }
        let now = now_ms();
        if ledger.status == "queued" {
            ledger.status = "running".to_string();
        }
        let (step_id, step_title) = current_or_next_step(ledger, now)
            .unwrap_or_else(|| (None, "Sub-agent progress".to_string()));
        let progress = SubAgentProgressUpdate {
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            step_id: step_id.clone(),
            step_title: step_title.clone(),
            summary,
            evidence,
            risks,
            next_step,
            created_at: now,
        };
        apply_progress_to_plan(ledger, &progress, now);
        ledger.progress.push(progress.clone());
        ledger.updated_at = now;
        ledger.watchdog.last_progress_at = Some(now);
        ledger.watchdog.last_event_at = now;
        ledger.timeline.push(SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            event_type: "progress".to_string(),
            timestamp: now,
            title: Some(step_title),
            status: Some(ledger.status.clone()),
            payload: value_object_to_btree(serde_json::to_value(&progress)?),
        });
        let task = ledger.clone();
        drop(tasks);
        self.flush_tasks().await?;
        Ok(task)
    }

    pub async fn report_task_plan(
        &self,
        task_id: &str,
        title: Option<String>,
        steps: Vec<String>,
    ) -> anyhow::Result<SubAgentTaskLedger> {
        let clean_steps = normalize_reported_plan_steps(steps);
        if clean_steps.is_empty() {
            anyhow::bail!("Task plan requires at least one step");
        }
        let mut tasks = self.tasks.write().await;
        let Some(ledger) = tasks.get_mut(task_id) else {
            anyhow::bail!("Task {task_id} not found");
        };
        if ledger.status == "blocked" || ledger.status == "cancelled" {
            return Ok(ledger.clone());
        }
        let now = now_ms();
        let (plan, current_step_id) = reported_task_plan(
            task_id,
            title,
            clean_steps.clone(),
            now,
            ledger.status == "running",
        );
        ledger.plan = Some(plan.clone());
        ledger.current_step_id = current_step_id;
        ledger.updated_at = now;
        ledger.watchdog.last_event_at = now;
        ledger.timeline.push(SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            event_type: "progress".to_string(),
            timestamp: now,
            title: Some("Sub-agent task plan reported".to_string()),
            status: Some(ledger.status.clone()),
            payload: BTreeMap::from([
                ("title".to_string(), json!(plan.title.clone())),
                ("steps".to_string(), json!(clean_steps)),
            ]),
        });
        let task = ledger.clone();
        drop(tasks);
        self.flush_tasks().await?;
        Ok(task)
    }

    pub async fn complete_task(
        &self,
        task_id: &str,
        mut result: SubAgentTaskResult,
    ) -> anyhow::Result<SubAgentTaskLedger> {
        let mut tasks = self.tasks.write().await;
        let Some(ledger) = tasks.get_mut(task_id) else {
            anyhow::bail!("Task {task_id} not found");
        };
        if ledger.status == "blocked" || ledger.status == "cancelled" {
            return Ok(ledger.clone());
        }
        let now = now_ms();
        if result.completed_at <= 0 {
            result.completed_at = now;
        }
        if result.started_at <= 0 {
            result.started_at = ledger.started_at;
        }
        ledger.status = result.status.clone();
        ledger.result = Some(result.clone());
        ledger.completed_at = Some(result.completed_at);
        ledger.updated_at = result.completed_at;
        ledger.watchdog.last_event_at = result.completed_at;
        complete_plan_steps(ledger, &result.status, result.completed_at);
        ledger.timeline.push(SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            event_type: terminal_task_event_type(&result.status).to_string(),
            timestamp: result.completed_at,
            title: Some(terminal_task_title(&result.status).to_string()),
            status: Some(ledger.status.clone()),
            payload: BTreeMap::from([("result".to_string(), serde_json::to_value(&result)?)]),
        });
        let task = ledger.clone();
        drop(tasks);
        self.flush_tasks().await?;
        Ok(task)
    }

    pub async fn delegate_many(
        &self,
        inputs: Vec<SubAgentTaskInput>,
        session_id: Option<String>,
        run_id: Option<String>,
    ) -> anyhow::Result<Vec<SubAgentTaskLedger>> {
        let mut ledgers = Vec::with_capacity(inputs.len());
        for input in inputs {
            ledgers.push(
                self.submit_task(input, session_id.clone(), run_id.clone())
                    .await?,
            );
        }
        Ok(ledgers)
    }

    pub fn assess_merge_results(&self, results: &[SubAgentTaskResult]) -> SubAgentMergeAssessment {
        assess_merge(results)
    }

    pub async fn control_task(
        &self,
        task_id: String,
        action: String,
        reason: Option<String>,
    ) -> Value {
        let mut tasks = self.tasks.write().await;
        let Some(ledger) = tasks.get_mut(&task_id) else {
            return json!({
                "ok": false,
                "taskId": task_id,
                "action": action,
                "reason": "Task not found in Rust AgentRuntime."
            });
        };
        let now = now_ms();
        match action.as_str() {
            "pause" => {
                ledger.status = "blocked".to_string();
                ledger.watchdog.blocked_reason = reason.clone();
                mark_unfinished_plan_steps(ledger, "blocked", now);
            }
            "resume" => {
                ledger.status = "queued".to_string();
                ledger.completed_at = None;
                ledger.watchdog.blocked_reason = None;
                reset_controlled_plan_steps(ledger, now);
            }
            "cancel" => {
                let reason_text = reason
                    .clone()
                    .unwrap_or_else(|| "Task was cancelled.".to_string());
                ledger.status = "cancelled".to_string();
                ledger.completed_at = Some(now);
                ledger.watchdog.blocked_reason = Some(reason_text.clone());
                complete_plan_steps(ledger, "cancelled", now);
                ledger.result = Some(SubAgentTaskResult {
                    task_id: ledger.task_id.clone(),
                    agent_id: ledger.agent_id.clone(),
                    agent_name: ledger.agent_name.clone(),
                    status: "cancelled".to_string(),
                    conclusion: reason_text.clone(),
                    evidence: Vec::new(),
                    modified_files: Vec::new(),
                    risks: Vec::new(),
                    blockers: Vec::new(),
                    next_steps: Vec::new(),
                    output: reason_text.clone(),
                    started_at: ledger.started_at,
                    completed_at: now,
                    iterations: None,
                    error: Some(reason_text),
                    sandbox_id: None,
                    provider: ledger.provider.clone(),
                    model: ledger.model.clone(),
                    role: ledger.role.clone(),
                    write_scope: ledger.task.write_scope.clone(),
                    artifacts: Vec::new(),
                });
            }
            "retry" => {
                ledger.status = "queued".to_string();
                ledger.completed_at = None;
                ledger.result = None;
                ledger.watchdog.blocked_reason = None;
                ledger.watchdog.retries += 1;
                reset_controlled_plan_steps(ledger, now);
            }
            _ => {}
        }
        ledger.updated_at = now;
        ledger.timeline.push(SubAgentTimelineEvent {
            id: format!("evt_{}", Uuid::new_v4()),
            task_id: ledger.task_id.clone(),
            agent_id: ledger.agent_id.clone(),
            event_type: "watchdog".to_string(),
            timestamp: now,
            title: Some(format!("Task control: {action}")),
            status: Some(ledger.status.clone()),
            payload: BTreeMap::from([
                ("action".to_string(), json!(action)),
                ("reason".to_string(), json!(reason)),
            ]),
        });
        let task = ledger.clone();
        drop(tasks);
        if let Err(err) = self.flush_tasks().await {
            return json!({
                "ok": false,
                "taskId": task_id,
                "action": action,
                "reason": format!("Failed to persist task ledger: {err}")
            });
        }
        json!({
            "ok": true,
            "taskId": task_id,
            "action": action,
            "reason": reason,
            "task": task
        })
    }

    async fn flush_tasks(&self) -> anyhow::Result<()> {
        let data = PersistedAgentTasks {
            version: 1,
            updated_at: now_ms(),
            tasks: self.tasks.read().await.values().cloned().collect(),
        };
        if let Some(parent) = self.tasks_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = serde_json::to_string_pretty(&data)?;
        let tmp = self.tasks_path.with_extension("json.tmp");
        tokio::fs::write(&tmp, format!("{text}\n")).await?;
        tokio::fs::rename(tmp, &self.tasks_path).await?;
        if let Some(store) = &self.control_plane {
            let store = store.clone();
            let tasks = data.tasks.clone();
            tokio::task::spawn_blocking(move || {
                for task in tasks {
                    let ledger = match serde_json::to_value(&task) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let _ = store.upsert_task(UpsertTaskInput {
                        task_id: &task.task_id,
                        session_id: task.control_plane_session_id.as_deref(),
                        run_id: task.control_plane_run_id.as_deref(),
                        control_plane_task_id: task.control_plane_task_id.as_deref(),
                        agent_id: Some(task.agent_id.as_str()),
                        status: &task.status,
                        ledger: &ledger,
                        created_at: task.started_at,
                        updated_at: task.updated_at,
                    });
                }
            })
            .await?;
        }
        Ok(())
    }

    async fn create_agent_inner(&self, input: AgentInput) -> anyhow::Result<()> {
        let agent = normalize_agent_input(input, None, &self.agents_dir)?;
        ensure_custom_id(&agent.id)?;
        let mut agents = self.agents.write().await;
        if agents.contains_key(&agent.id) {
            anyhow::bail!("Agent {} already exists", agent.id);
        }
        write_agent_file(&self.agents_dir, &agent).await?;
        agents.insert(agent.id.clone(), agent);
        Ok(())
    }

    async fn update_agent_inner(&self, id: String, input: AgentInput) -> anyhow::Result<()> {
        ensure_custom_id(&id)?;
        let agent = normalize_agent_input(input, Some(id.clone()), &self.agents_dir)?;
        write_agent_file(&self.agents_dir, &agent).await?;
        self.agents.write().await.insert(id, agent);
        Ok(())
    }

    async fn delete_agent_inner(&self, id: String) -> anyhow::Result<()> {
        ensure_custom_id(&id)?;
        let path = self.agents_dir.join(format!("{id}.md"));
        if tokio::fs::metadata(&path).await.is_ok() {
            tokio::fs::remove_file(path).await?;
        }
        self.agents.write().await.remove(&id);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAgentTasks {
    version: u32,
    updated_at: i64,
    #[serde(default)]
    tasks: Vec<SubAgentTaskLedger>,
}

impl Default for PersistedAgentTasks {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: now_ms(),
            tasks: Vec::new(),
        }
    }
}

fn builtin_agents() -> Vec<AgentDefinition> {
    vec![
        builtin_agent(BuiltinAgentSpec {
            id: "architect",
            name: "Architect",
            description: "Requirements analysis, architecture design, technical plans",
            role: "architect",
            can_write: false,
            permission_mode: "plan",
            memory_scope: "project",
            model_profile: "strong reasoning, architecture",
            tools: &[
                "codebase_index",
                "file_read",
                "file_write",
                "file_edit",
                "apply_patch",
                "search",
                "glob",
                "list_files",
                "search_memory",
                "load_memories",
                "write_memory",
                "git_status",
                "git_diff",
                "git_log",
                "git_blame",
                "web_search",
                "web_fetch",
            ],
            max_turns: 40,
        }),
        builtin_agent(BuiltinAgentSpec {
            id: "developer",
            name: "Developer",
            description: "Code implementation, build verification, bug fixes",
            role: "developer",
            can_write: true,
            permission_mode: "auto",
            memory_scope: "session",
            model_profile: "strong coding model",
            tools: &[
                "file_read",
                "file_write",
                "file_edit",
                "apply_patch",
                "search",
                "glob",
                "list_files",
                "search_memory",
                "load_memories",
                "write_memory",
                "exec",
                "start_process",
                "read_process_output",
                "stop_process",
                "codebase_index",
                "git_status",
                "git_diff",
                "git_log",
                "git_blame",
                "git_branch",
                "git_commit",
                "git_patch_create",
                "git_patch_apply",
                "validation_discover",
                "validation_diagnose",
                "acceptance_check",
                "self_review",
            ],
            max_turns: 120,
        }),
        builtin_agent(BuiltinAgentSpec {
            id: "explorer",
            name: "Explorer",
            description: "Read-only codebase investigation and evidence collection",
            role: "explorer",
            can_write: false,
            permission_mode: "plan",
            memory_scope: "project",
            model_profile: "strong reasoning, investigation",
            tools: &[
                "codebase_index",
                "file_read",
                "search",
                "glob",
                "list_files",
                "search_memory",
                "load_memories",
                "git_status",
                "git_diff",
                "git_log",
                "git_blame",
                "web_search",
                "web_fetch",
            ],
            max_turns: 40,
        }),
        builtin_agent(BuiltinAgentSpec {
            id: "reviewer",
            name: "Reviewer",
            description: "Risk assessment, verification, security and acceptance review",
            role: "reviewer",
            can_write: false,
            permission_mode: "default",
            memory_scope: "project",
            model_profile: "strong reasoning, review",
            tools: &[
                "file_read",
                "file_write",
                "file_edit",
                "apply_patch",
                "search",
                "glob",
                "list_files",
                "search_memory",
                "load_memories",
                "write_memory",
                "codebase_index",
                "git_status",
                "git_diff",
                "git_log",
                "git_blame",
                "exec",
                "start_process",
                "read_process_output",
                "stop_process",
                "validation_discover",
                "validation_diagnose",
                "acceptance_check",
                "self_review",
            ],
            max_turns: 50,
        }),
        builtin_agent(BuiltinAgentSpec {
            id: "doc-writer",
            name: "Doc Writer",
            description: "Plans, documentation, changelogs",
            role: "doc-writer",
            can_write: true,
            permission_mode: "default",
            memory_scope: "project",
            model_profile: "clear documentation",
            tools: &[
                "file_read",
                "file_write",
                "file_edit",
                "apply_patch",
                "search",
                "glob",
                "list_files",
                "search_memory",
                "load_memories",
                "write_memory",
                "codebase_index",
                "git_status",
                "git_diff",
                "git_log",
                "git_blame",
                "web_search",
                "web_fetch",
            ],
            max_turns: 60,
        }),
    ]
}

fn normalize_permission_mode(value: String) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "plan" => "plan".to_string(),
        "auto" | "all" | "yolo" => "auto".to_string(),
        // default / ask / strict / 未知 → default
        _ => "default".to_string(),
    }
}

/// Static fields describing a built-in agent definition.
struct BuiltinAgentSpec<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    role: &'a str,
    can_write: bool,
    permission_mode: &'a str,
    memory_scope: &'a str,
    model_profile: &'a str,
    tools: &'a [&'a str],
    max_turns: u32,
}

fn builtin_agent(spec: BuiltinAgentSpec<'_>) -> AgentDefinition {
    let BuiltinAgentSpec {
        id,
        name,
        description,
        role,
        can_write,
        permission_mode,
        memory_scope,
        model_profile,
        tools,
        max_turns,
    } = spec;
    AgentDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        prompt: role_prompt(role),
        tools: tools.iter().map(|tool| tool.to_string()).collect(),
        permission_mode: permission_mode.to_string(),
        provider: None,
        model: None,
        base_url: None,
        api_key: None,
        max_tokens: None,
        temperature: None,
        model_profile: Some(model_profile.to_string()),
        can_write,
        can_talk_to_user: false,
        default_sandbox_mode: Some("worktree".to_string()),
        max_turns,
        memory_scope: memory_scope.to_string(),
        source: "builtin".to_string(),
        source_path: None,
        role: Some(role.to_string()),
    }
}

fn role_prompt(role: &str) -> String {
    match role {
        "architect" => [
            "Analyze requirements and produce implementable architecture and design docs.",
            "Do not modify business code. Deliver .md, .mmd, or .drawio design files via write tools.",
            "Docs must be engineering-ready: scope, modules, data model, APIs, deployment, risks.",
        ]
        .join("\n"),
        "developer" => [
            "Implement, verify, and fix within the assigned scope only.",
            "Read existing code before editing; prefer apply_patch for precise changes.",
            "Run validation after meaningful changes.",
            "Deliver code and config as files; do not paste large sources in chat.",
        ]
        .join("\n"),
        "explorer" => [
            "Investigate the codebase read-only: locate code, trace flows, collect evidence.",
            "Every conclusion must cite file paths, symbols, or command output. No speculation.",
        ]
        .join("\n"),
        "reviewer" => [
            "Review changes for regression, correctness, security, and acceptance criteria.",
            "Do not modify business code. Write findings to a Markdown review report file.",
            "Separate confirmed facts from open risks.",
        ]
        .join("\n"),
        "doc-writer" => [
            "Create and update plans, API docs, guides, and changelogs.",
            "Base content on actual code and verification results.",
            "Deliver documents as files; summarize paths in the final response, not full text.",
        ]
        .join("\n"),
        _ => "Work as a focused XuanYu sub-agent.".to_string(),
    }
}

const SUBAGENT_SHARED_RULE_LINES: &[&str] = &[
    "- Match the task language in all visible output.",
    "- Background worker only; do not address the user directly.",
    "- No nested delegation; stay within scope.",
    "- Ground conclusions in tool evidence; state uncertainty explicitly.",
    "- Final sections: Conclusion | Evidence | Modified files | Artifacts | Risks | Blockers | Next steps.",
];

const SUBAGENT_EXECUTION_RULE_ARTIFACT: &str =
    "Create the deliverable file with file_write or apply_patch before the final response. report_progress only after the file exists.";

const SUBAGENT_EXECUTION_RULE_PLAN: &str =
    "Call report_task_plan with task-specific stepN: steps before write/exec/git tools. Read-only tools are allowed before planning. Call report_progress after each step.";

pub fn subagent_requires_artifact(definition: &AgentDefinition) -> bool {
    let role = definition
        .role
        .as_deref()
        .unwrap_or(&definition.id)
        .to_ascii_lowercase();
    role.contains("architect")
        || role.contains("writer")
        || role.contains("doc-writer")
        || role.contains("reviewer")
}

pub fn build_subagent_system_prompt(definition: &AgentDefinition) -> String {
    let execution_rule = if subagent_requires_artifact(definition) {
        SUBAGENT_EXECUTION_RULE_ARTIFACT
    } else {
        SUBAGENT_EXECUTION_RULE_PLAN
    };
    let mut lines = vec![
        format!(
            "You are {}, a XuanYu SubAgent ({}).",
            definition.name, definition.description
        ),
        definition.prompt.clone(),
        String::new(),
        "Rules:".to_string(),
    ];
    lines.extend(
        SUBAGENT_SHARED_RULE_LINES
            .iter()
            .take(3)
            .map(|line| (*line).to_string()),
    );
    lines.push(format!("- {execution_rule}"));
    lines.extend(
        SUBAGENT_SHARED_RULE_LINES
            .iter()
            .skip(3)
            .map(|line| (*line).to_string()),
    );
    lines.join("\n")
}

fn normalize_reported_plan_steps(steps: Vec<String>) -> Vec<String> {
    let mut seen = BTreeMap::<String, ()>::new();
    steps
        .into_iter()
        .map(|step| step.trim().to_string())
        .filter(|step| !step.is_empty())
        .filter(|step| {
            let key = step.to_ascii_lowercase();
            seen.insert(key, ()).is_none()
        })
        .take(12)
        .collect()
}

fn reported_task_plan(
    task_id: &str,
    title: Option<String>,
    steps: Vec<String>,
    now: i64,
    mark_running: bool,
) -> (SubAgentTaskPlan, Option<String>) {
    let mut current_step_id = None;
    let steps = steps
        .into_iter()
        .enumerate()
        .map(|(index, title)| {
            let id = format!("{task_id}_worker_step_{}", index + 1);
            let status = if mark_running && index == 0 {
                current_step_id = Some(id.clone());
                "running"
            } else {
                "pending"
            };
            SubAgentTaskPlanStep {
                id,
                title,
                status: status.to_string(),
                summary: None,
                evidence: Vec::new(),
                risks: Vec::new(),
                next_step: None,
            }
        })
        .collect();
    (
        SubAgentTaskPlan {
            title: title
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Sub-agent task plan".to_string()),
            steps,
            created_at: now,
            updated_at: now,
        },
        current_step_id,
    )
}

fn normalize_agent_input(
    input: AgentInput,
    id_override: Option<String>,
    agents_dir: &std::path::Path,
) -> anyhow::Result<AgentDefinition> {
    let id = id_override
        .or(input.id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Agent id is required"))?;
    let name = input
        .name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Agent name is required"))?;
    let description = input
        .description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Agent description is required"))?;
    let prompt = input
        .prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Agent prompt is required"))?;
    Ok(AgentDefinition {
        id: id.clone(),
        name,
        description,
        prompt,
        tools: input.tools,
        permission_mode: normalize_permission_mode(
            input
                .permission_mode
                .unwrap_or_else(|| "default".to_string()),
        ),
        provider: empty_to_none(input.provider),
        model: empty_to_none(input.model),
        base_url: empty_to_none(input.base_url),
        api_key: empty_to_none(input.api_key),
        max_tokens: input.max_tokens,
        temperature: input.temperature,
        model_profile: empty_to_none(input.model_profile),
        can_write: input.can_write.unwrap_or(true),
        can_talk_to_user: input.can_talk_to_user.unwrap_or(false),
        default_sandbox_mode: empty_to_none(input.default_sandbox_mode),
        max_turns: input.max_turns.unwrap_or(64),
        memory_scope: input.memory_scope.unwrap_or_else(|| "session".to_string()),
        source: "project".to_string(),
        source_path: Some(path_to_string(&agents_dir.join(format!("{id}.md")))),
        role: empty_to_none(input.role),
    })
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn ensure_custom_id(id: &str) -> anyhow::Result<()> {
    if builtin_agents().iter().any(|agent| agent.id == id) {
        anyhow::bail!("Builtin agent {id} cannot be modified");
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        anyhow::bail!("Agent id contains invalid path characters");
    }
    Ok(())
}

async fn write_agent_file(
    agents_dir: &std::path::Path,
    agent: &AgentDefinition,
) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(agents_dir).await?;
    tokio::fs::write(
        agents_dir.join(format!("{}.md", agent.id)),
        serialize_agent_md(agent),
    )
    .await?;
    Ok(())
}

async fn load_custom_agents(agents_dir: &std::path::Path) -> anyhow::Result<Vec<AgentDefinition>> {
    let mut result = Vec::new();
    if tokio::fs::metadata(agents_dir).await.is_err() {
        return Ok(result);
    }
    let mut entries = tokio::fs::read_dir(agents_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = tokio::fs::read_to_string(&path).await?;
        if let Some(agent) = parse_agent_md(&text, path) {
            if !builtin_agents()
                .iter()
                .any(|builtin| builtin.id == agent.id)
            {
                result.push(agent);
            }
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

async fn load_task_ledgers(
    tasks_path: &std::path::Path,
) -> anyhow::Result<Vec<SubAgentTaskLedger>> {
    let Ok(text) = tokio::fs::read_to_string(tasks_path).await else {
        return Ok(Vec::new());
    };
    let data = serde_json::from_str::<PersistedAgentTasks>(text.trim_start_matches('\u{feff}'))?;
    let mut tasks = data.tasks;
    tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    Ok(tasks)
}

fn initial_task_plan(
    task_id: &str,
    input: &SubAgentTaskInput,
    agent: &AgentDefinition,
    now: i64,
) -> SubAgentTaskPlan {
    let role = agent
        .role
        .as_deref()
        .unwrap_or(&agent.id)
        .to_ascii_lowercase();
    let task = input.task.to_ascii_lowercase();
    let mut titles = Vec::new();
    if role.contains("architect") {
        titles.extend([
            "Analyze requirements and current architecture",
            "Design the target approach",
            "Document decisions, risks, and follow-up work",
        ]);
    } else if role.contains("review") {
        titles.extend([
            "Read and understand the target changes",
            "Review correctness, risks, and missing validation",
            "Summarize findings and recommended fixes",
        ]);
    } else if role.contains("doc") || role.contains("writer") {
        titles.extend([
            "Collect source context and audience requirements",
            "Draft the documentation or delivery artifact",
            "Check clarity, completeness, and handoff details",
        ]);
    } else if role.contains("explorer") {
        titles.extend([
            "Inspect the relevant source and runtime context",
            "Collect evidence without modifying files",
            "Summarize discoveries and open questions",
        ]);
    } else {
        titles.push("Explore and collect context");
        if contains_any(&task, &["design", "plan", "architecture", "方案", "设计"]) {
            titles.push("Prepare the implementation approach");
        }
        if contains_any(
            &task,
            &[
                "implement",
                "build",
                "fix",
                "migrate",
                "refactor",
                "实现",
                "修复",
                "迁移",
                "重构",
            ],
        ) {
            titles.push("Make the scoped code changes");
        }
        if contains_any(&task, &["test", "verify", "check", "验证", "测试", "检查"]) {
            titles.push("Run focused validation");
        }
        titles.push("Summarize results, risks, and next steps");
    }

    let mut seen = BTreeMap::new();
    let steps = titles
        .into_iter()
        .filter(|title| seen.insert(*title, ()).is_none())
        .enumerate()
        .map(|(index, title)| SubAgentTaskPlanStep {
            id: format!("{task_id}_step_{}", index + 1),
            title: title.to_string(),
            status: "pending".to_string(),
            summary: None,
            evidence: Vec::new(),
            risks: Vec::new(),
            next_step: None,
        })
        .collect();

    SubAgentTaskPlan {
        title: agent.name.clone(),
        steps,
        created_at: now,
        updated_at: now,
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn mark_first_pending_step_running(ledger: &mut SubAgentTaskLedger, now: i64) {
    let mut current_step_id = ledger.current_step_id.clone();
    if let Some(plan) = &mut ledger.plan {
        if !plan.steps.iter().any(|step| step.status == "running") {
            if let Some(step) = plan.steps.iter_mut().find(|step| step.status == "pending") {
                step.status = "running".to_string();
                current_step_id = Some(step.id.clone());
            }
        }
        plan.updated_at = now;
    }
    ledger.current_step_id = current_step_id;
}

fn current_or_next_step(
    ledger: &mut SubAgentTaskLedger,
    now: i64,
) -> Option<(Option<String>, String)> {
    let mut selected = None;
    if let Some(plan) = &mut ledger.plan {
        if let Some(step) = plan.steps.iter().find(|step| step.status == "running") {
            selected = Some((Some(step.id.clone()), step.title.clone()));
        } else if let Some(step) = plan.steps.iter_mut().find(|step| step.status == "pending") {
            step.status = "running".to_string();
            selected = Some((Some(step.id.clone()), step.title.clone()));
        }
        plan.updated_at = now;
    }
    if let Some((Some(step_id), _)) = &selected {
        ledger.current_step_id = Some(step_id.clone());
    }
    selected
}

fn apply_progress_to_plan(
    ledger: &mut SubAgentTaskLedger,
    progress: &SubAgentProgressUpdate,
    now: i64,
) {
    let mut next_current_step_id = None;
    if let Some(plan) = &mut ledger.plan {
        if let Some(step) = plan.steps.iter_mut().find(|step| {
            progress
                .step_id
                .as_deref()
                .map(|id| step.id == id)
                .unwrap_or(false)
                || step.title == progress.step_title
        }) {
            step.status = "completed".to_string();
            step.summary = Some(progress.summary.clone());
            step.evidence = progress.evidence.clone();
            step.risks = progress.risks.clone();
            step.next_step = progress.next_step.clone();
        }
        if let Some(step) = plan.steps.iter_mut().find(|step| step.status == "pending") {
            step.status = "running".to_string();
            next_current_step_id = Some(step.id.clone());
        }
        plan.updated_at = now;
    }
    ledger.current_step_id = next_current_step_id;
}

fn complete_plan_steps(ledger: &mut SubAgentTaskLedger, status: &str, now: i64) {
    if let Some(plan) = &mut ledger.plan {
        for step in &mut plan.steps {
            if step.status == "pending" || step.status == "running" {
                step.status = terminal_plan_step_status(status).to_string();
            }
        }
        plan.updated_at = now;
    }
    ledger.current_step_id = None;
}

fn mark_unfinished_plan_steps(ledger: &mut SubAgentTaskLedger, status: &str, now: i64) {
    if let Some(plan) = &mut ledger.plan {
        for step in &mut plan.steps {
            if step.status == "pending" || step.status == "running" {
                step.status = status.to_string();
            }
        }
        plan.updated_at = now;
    }
    ledger.current_step_id = None;
}

fn reset_controlled_plan_steps(ledger: &mut SubAgentTaskLedger, now: i64) {
    if let Some(plan) = &mut ledger.plan {
        for step in &mut plan.steps {
            if step.status == "blocked" || step.status == "cancelled" || step.status == "failed" {
                step.status = "pending".to_string();
            }
        }
        plan.updated_at = now;
    }
    ledger.current_step_id = None;
}

fn terminal_plan_step_status(status: &str) -> &'static str {
    match status {
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "completed",
    }
}

fn terminal_task_event_type(status: &str) -> &'static str {
    match status {
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "completed",
    }
}

fn terminal_task_title(status: &str) -> &'static str {
    match status {
        "failed" => "Sub-agent failed",
        "cancelled" => "Sub-agent cancelled",
        _ => "Sub-agent completed",
    }
}

fn value_object_to_btree(value: Value) -> BTreeMap<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

fn serialize_agent_md(agent: &AgentDefinition) -> String {
    let mut lines = vec!["---".to_string()];
    lines.push(format!("id: {}", agent.id));
    lines.push(format!("name: {}", agent.name));
    lines.push(format!("description: {}", agent.description));
    if let Some(role) = &agent.role {
        lines.push(format!("role: {role}"));
    }
    lines.push("tools:".to_string());
    for tool in &agent.tools {
        lines.push(format!("  - {tool}"));
    }
    lines.push(format!("permissionMode: {}", agent.permission_mode));
    optional_line(&mut lines, "provider", agent.provider.as_deref());
    optional_line(&mut lines, "model", agent.model.as_deref());
    optional_line(&mut lines, "baseUrl", agent.base_url.as_deref());
    optional_line(&mut lines, "apiKey", agent.api_key.as_deref());
    if let Some(max_tokens) = agent.max_tokens {
        lines.push(format!("maxTokens: {max_tokens}"));
    }
    if let Some(temperature) = agent.temperature {
        lines.push(format!("temperature: {temperature}"));
    }
    optional_line(&mut lines, "modelProfile", agent.model_profile.as_deref());
    lines.push(format!("maxTurns: {}", agent.max_turns));
    lines.push(format!("memoryScope: {}", agent.memory_scope));
    lines.push(format!("canWrite: {}", agent.can_write));
    lines.push(format!("canTalkToUser: {}", agent.can_talk_to_user));
    optional_line(
        &mut lines,
        "defaultSandboxMode",
        agent.default_sandbox_mode.as_deref(),
    );
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(agent.prompt.clone());
    lines.join("\n")
}

fn optional_line(lines: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        if !value.trim().is_empty() {
            lines.push(format!("{key}: {}", value.trim()));
        }
    }
}

fn parse_agent_md(text: &str, path: PathBuf) -> Option<AgentDefinition> {
    let mut parts = text.splitn(3, "---");
    parts.next()?;
    let header = parts.next()?;
    let prompt = parts.next().unwrap_or_default().trim().to_string();
    let mut map = BTreeMap::<String, String>::new();
    let mut tools = Vec::<String>::new();
    let mut in_tools = false;
    for line in header.lines() {
        let trimmed = line.trim();
        if trimmed == "tools:" {
            in_tools = true;
            continue;
        }
        if in_tools && trimmed.starts_with("- ") {
            tools.push(trimmed.trim_start_matches("- ").trim().to_string());
            continue;
        }
        in_tools = false;
        if let Some((key, value)) = trimmed.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let id = map.get("id")?.to_string();
    Some(AgentDefinition {
        id,
        name: map.get("name").cloned().unwrap_or_default(),
        description: map.get("description").cloned().unwrap_or_default(),
        prompt,
        tools,
        permission_mode: normalize_permission_mode(
            map
                .get("permissionMode")
                .cloned()
                .unwrap_or_else(|| "default".to_string()),
        ),
        provider: map.get("provider").cloned(),
        model: map.get("model").cloned(),
        base_url: map.get("baseUrl").cloned(),
        api_key: map.get("apiKey").cloned(),
        max_tokens: map.get("maxTokens").and_then(|value| value.parse().ok()),
        temperature: map.get("temperature").and_then(|value| value.parse().ok()),
        model_profile: map.get("modelProfile").cloned(),
        can_write: map
            .get("canWrite")
            .map(|value| value == "true")
            .unwrap_or(true),
        can_talk_to_user: map
            .get("canTalkToUser")
            .map(|value| value == "true")
            .unwrap_or(false),
        default_sandbox_mode: map.get("defaultSandboxMode").cloned(),
        max_turns: map
            .get("maxTurns")
            .and_then(|value| value.parse().ok())
            .unwrap_or(64),
        memory_scope: map
            .get("memoryScope")
            .cloned()
            .unwrap_or_else(|| "session".to_string()),
        source: "project".to_string(),
        source_path: Some(path_to_string(&path)),
        role: map.get("role").cloned(),
    })
}

fn team_member(
    agent: AgentDefinition,
    tasks: &[SubAgentTaskLedger],
    generated_at: i64,
) -> SubAgentTeamMember {
    let latest = tasks.iter().find(|task| task.agent_id == agent.id);
    let active = latest.filter(|task| task.status == "queued" || task.status == "running");
    SubAgentTeamMember {
        agent_id: agent.id,
        name: agent.name,
        role: agent.role.unwrap_or_else(|| "developer".to_string()),
        description: agent.description,
        status: active
            .map(|task| task.status.clone())
            .or_else(|| latest.map(|task| task.status.clone()))
            .unwrap_or_else(|| "idle".to_string()),
        current_task: active.map(|task| task.task.task.clone()),
        model: agent.model.unwrap_or_else(|| "inherit".to_string()),
        provider: agent.provider.unwrap_or_else(|| "inherit".to_string()),
        model_profile: agent.model_profile,
        tools: agent.tools,
        can_write: agent.can_write,
        can_talk_to_user: agent.can_talk_to_user,
        default_sandbox_mode: agent.default_sandbox_mode,
        source: agent.source,
        latest_result: latest.and_then(|task| task.result.clone()),
        latest_progress: latest.and_then(|task| task.progress.last().cloned()),
        active_task_id: active.map(|task| task.task_id.clone()),
        updated_at: latest.map(|task| task.updated_at).unwrap_or(generated_at),
    }
}

fn filtered_tasks(
    tasks: &BTreeMap<String, SubAgentTaskLedger>,
    session_id: Option<&str>,
) -> Vec<SubAgentTaskLedger> {
    let mut tasks = tasks
        .values()
        .filter(|task| session_matches(task, session_id))
        .cloned()
        .collect::<Vec<_>>();
    tasks.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    tasks
}

fn session_matches(task: &SubAgentTaskLedger, session_id: Option<&str>) -> bool {
    session_id
        .map(|session_id| task.control_plane_session_id.as_deref() == Some(session_id))
        .unwrap_or(true)
}

fn metadata_string_any(metadata: &BTreeMap<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

fn watchdog_timeline_event(
    ledger: &SubAgentTaskLedger,
    warning: &str,
    kind: &str,
    timestamp: i64,
) -> SubAgentTimelineEvent {
    SubAgentTimelineEvent {
        id: format!("evt_{}", Uuid::new_v4()),
        task_id: ledger.task_id.clone(),
        agent_id: ledger.agent_id.clone(),
        event_type: "watchdog".to_string(),
        timestamp,
        title: Some("Watchdog".to_string()),
        status: Some("warning".to_string()),
        payload: BTreeMap::from([
            ("warning".to_string(), json!(warning)),
            ("kind".to_string(), json!(kind)),
        ]),
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
