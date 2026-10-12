use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub goal: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    #[serde(rename = "type")]
    pub step_type: String,
    pub name: String,
    pub status: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    pub trace_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTimeline {
    pub run: RunRecord,
    pub steps: Vec<StepRecord>,
    pub events: Vec<RunEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_checkpoint: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceEvent {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_type: Option<String>,
    pub category: String,
    pub label: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceSummary {
    pub total_events: usize,
    pub model_requests: usize,
    pub tool_calls: usize,
    pub tool_failures: usize,
    pub permission_decisions: usize,
    pub memory_retrievals: usize,
    pub compactions: usize,
    pub errors: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceTimeline {
    pub trace_id: String,
    pub run: RunRecord,
    pub steps: Vec<StepRecord>,
    pub events: Vec<TraceEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_checkpoint: Option<Value>,
    pub summary: TraceSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptComparison {
    pub base_run_id: String,
    pub target_run_id: String,
    pub model_different: bool,
    pub context_different: bool,
    pub tool_results_different: bool,
    pub changes_different: bool,
    pub summary: Vec<String>,
    pub details: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceResourceUsage {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub model_requests: usize,
    pub tool_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePack {
    pub run_id: String,
    pub trace_id: String,
    pub generated_at: i64,
    pub user_goal: String,
    pub status: String,
    pub key_decisions: Vec<String>,
    pub modified_files: Vec<String>,
    pub diff_summary: Vec<String>,
    pub commands: Vec<String>,
    pub validation_results: Vec<String>,
    pub failures_and_recovery: Vec<String>,
    pub resource_usage: EvidenceResourceUsage,
    pub risks: Vec<String>,
    pub follow_ups: Vec<String>,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserFeedbackRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct CreateRunInput {
    pub session_id: String,
    pub goal: String,
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct StartStepInput {
    pub run_id: String,
    pub step_type: String,
    pub name: String,
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRunData {
    version: u32,
    updated_at: i64,
    #[serde(default)]
    runs: Vec<RunRecord>,
    #[serde(default)]
    steps: Vec<StepRecord>,
    #[serde(default)]
    events: Vec<RunEvent>,
    #[serde(default)]
    feedback: Vec<UserFeedbackRecord>,
}

#[derive(Debug, Default, Clone)]
struct RunData {
    runs: BTreeMap<String, RunRecord>,
    steps: BTreeMap<String, StepRecord>,
    events: Vec<RunEvent>,
    feedback: Vec<UserFeedbackRecord>,
}

pub struct RunStore {
    path: PathBuf,
    data: RwLock<RunData>,
}

impl RunStore {
    pub fn new(data_dir: PathBuf, project_name: impl Into<String>) -> Arc<Self> {
        let project_name = sanitize_segment(&project_name.into());
        Arc::new(Self {
            path: data_dir.join("runs").join(project_name).join("runs.json"),
            data: RwLock::new(RunData::default()),
        })
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        let loaded = load_run_data(&self.path).await?;
        *self.data.write().await = loaded;
        Ok(())
    }

    pub async fn create_run(&self, input: CreateRunInput) -> anyhow::Result<RunRecord> {
        let now = now_ms();
        let run = RunRecord {
            id: format!("run_{}", Uuid::new_v4().simple()),
            session_id: input.session_id,
            trace_id: format!("trace_{}", Uuid::new_v4().simple()),
            goal: input.goal,
            status: "running".to_string(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            error: None,
            current_step_id: None,
            checkpoint_id: None,
            metadata: input.metadata,
        };
        let mut data = self.data.write().await;
        data.runs.insert(run.id.clone(), run.clone());
        push_run_event(
            &mut data,
            &run,
            "run.created",
            Some(json!({ "run": run })),
            None,
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(run)
    }

    pub async fn start_step(&self, input: StartStepInput) -> anyhow::Result<StepRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let run = data
            .runs
            .get(&input.run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {} not found", input.run_id))?;
        let step = StepRecord {
            id: format!("step_{}", Uuid::new_v4().simple()),
            run_id: run.id.clone(),
            session_id: run.session_id.clone(),
            step_type: input.step_type,
            name: input.name,
            status: "running".to_string(),
            started_at: now,
            completed_at: None,
            error: None,
            metadata: input.metadata,
        };
        let mut updated_run = run.clone();
        updated_run.current_step_id = Some(step.id.clone());
        updated_run.updated_at = now;
        data.runs.insert(updated_run.id.clone(), updated_run);
        data.steps.insert(step.id.clone(), step.clone());
        push_run_event(
            &mut data,
            &run,
            "step.started",
            Some(json!({ "step": step })),
            Some(step.id.clone()),
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(step)
    }

    pub async fn complete_step(
        &self,
        step_id: &str,
        payload: BTreeMap<String, Value>,
    ) -> anyhow::Result<StepRecord> {
        self.finish_step(step_id, "completed", None, payload).await
    }

    pub async fn fail_step(&self, step_id: &str, error: String) -> anyhow::Result<StepRecord> {
        self.finish_step(step_id, "failed", Some(error), BTreeMap::new())
            .await
    }

    pub async fn record_event(
        &self,
        run_id: &str,
        event_type: &str,
        payload: Option<Value>,
        step_id: Option<String>,
    ) -> anyhow::Result<RunEvent> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut run = data
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))?;
        run.updated_at = now;
        let prev_hash = last_run_event_hash(&data.events, run_id);
        let event = run_event(&run, event_type, payload, step_id, now, prev_hash);
        data.runs.insert(run.id.clone(), run);
        data.events.push(event.clone());
        drop(data);
        self.flush().await?;
        Ok(event)
    }

    pub async fn set_run_plan(&self, run_id: &str, plan: Value) -> anyhow::Result<RunRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut run = data
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))?;
        run.updated_at = now;
        run.metadata.insert("plan".to_string(), plan.clone());
        data.runs.insert(run.id.clone(), run.clone());
        push_run_event(
            &mut data,
            &run,
            "plan.created",
            Some(json!({ "plan": plan })),
            None,
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(run)
    }

    pub async fn update_run_todo(
        &self,
        run_id: &str,
        todo_id: &str,
        status: &str,
    ) -> anyhow::Result<RunRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut run = data
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))?;
        let Some(plan_value) = run.metadata.get("plan").cloned() else {
            return Ok(run);
        };
        let Some(plan_obj) = plan_value.as_object() else {
            return Ok(run);
        };
        let mut plan = plan_obj.clone();
        let todos = plan
            .get("todos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let updated_todos = todos
            .into_iter()
            .map(|item| {
                let mut todo = item.as_object().cloned().unwrap_or_default();
                if todo.get("id").and_then(Value::as_str) == Some(todo_id) {
                    todo.insert("status".to_string(), json!(status));
                    todo.insert("updatedAt".to_string(), json!(now));
                }
                Value::Object(todo)
            })
            .collect::<Vec<_>>();
        plan.insert("todos".to_string(), Value::Array(updated_todos));
        plan.insert("updatedAt".to_string(), json!(now));
        let updated_plan = Value::Object(plan);
        run.updated_at = now;
        run.metadata.insert("plan".to_string(), updated_plan.clone());
        data.runs.insert(run.id.clone(), run.clone());
        push_run_event(
            &mut data,
            &run,
            "plan.updated",
            Some(json!({ "todoId": todo_id, "status": status, "plan": updated_plan })),
            None,
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(run)
    }

    pub async fn complete_run(
        &self,
        run_id: &str,
        payload: BTreeMap<String, Value>,
    ) -> anyhow::Result<RunRecord> {
        self.finish_run(run_id, "completed", None, payload).await
    }

    pub async fn fail_run(&self, run_id: &str, error: String) -> anyhow::Result<RunRecord> {
        self.finish_run(run_id, "failed", Some(error), BTreeMap::new())
            .await
    }

    pub async fn cancel_run(
        &self,
        run_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<RunRecord> {
        self.finish_run(
            run_id,
            "cancelled",
            reason.clone(),
            BTreeMap::from([(
                "reason".to_string(),
                json!(reason.unwrap_or_else(|| "cancelled by user".to_string())),
            )]),
        )
        .await
    }

    pub async fn resume_run(&self, run_id: &str) -> anyhow::Result<RunRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut run = data
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))?;
        run.status = "running".to_string();
        run.updated_at = now;
        run.completed_at = None;
        run.error = None;
        run.current_step_id = None;
        run.metadata.insert("lastResumedAt".to_string(), json!(now));
        data.runs.insert(run.id.clone(), run.clone());
        push_run_event(&mut data, &run, "run.resumed", Some(json!({})), None, now);
        drop(data);
        self.flush().await?;
        Ok(run)
    }

    pub async fn replay_run(&self, run_id: &str) -> anyhow::Result<RunTimeline> {
        let event_count = {
            let data = self.data.read().await;
            data.events
                .iter()
                .filter(|event| event.run_id == run_id)
                .count()
        };
        self.record_event(
            run_id,
            "run.replayed",
            Some(json!({ "eventCount": event_count })),
            None,
        )
        .await?;
        self.timeline(run_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))
    }

    pub async fn get_run(&self, run_id: &str) -> Option<RunRecord> {
        self.data.read().await.runs.get(run_id).cloned()
    }

    pub async fn latest_running_run_for_session(&self, session_id: &str) -> Option<RunRecord> {
        let mut runs = self
            .data
            .read()
            .await
            .runs
            .values()
            .filter(|run| run.session_id == session_id && run.status == "running")
            .cloned()
            .collect::<Vec<_>>();
        runs.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        runs.into_iter().next()
    }

    pub async fn list_runs(&self, limit: usize) -> Vec<RunRecord> {
        let mut runs = self
            .data
            .read()
            .await
            .runs
            .values()
            .cloned()
            .collect::<Vec<_>>();
        runs.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        runs.truncate(limit.max(1));
        runs
    }

    pub async fn timeline(&self, run_id: &str) -> Option<RunTimeline> {
        let data = self.data.read().await;
        let run = data.runs.get(run_id).cloned()?;
        Some(RunTimeline {
            steps: sorted_steps(data.steps.values(), &run.id),
            events: data
                .events
                .iter()
                .filter(|event| event.run_id == run.id)
                .cloned()
                .collect(),
            run,
            latest_checkpoint: None,
        })
    }

    pub async fn trace_timeline(&self, id: &str) -> Option<TraceTimeline> {
        let data = self.data.read().await;
        let run = data
            .runs
            .values()
            .find(|run| run.id == id || run.trace_id == id)
            .cloned()?;
        let steps = sorted_steps(data.steps.values(), &run.id);
        let raw_events = data
            .events
            .iter()
            .filter(|event| event.run_id == run.id)
            .cloned()
            .collect::<Vec<_>>();
        let step_by_id = steps
            .iter()
            .map(|step| (step.id.clone(), step.clone()))
            .collect::<HashMap<_, _>>();
        let events = raw_events
            .iter()
            .map(|event| {
                trace_event_from_run_event(
                    event,
                    step_by_id.get(event.step_id.as_deref().unwrap_or("")),
                )
            })
            .collect::<Vec<_>>();
        let summary = trace_summary(&run, &events);
        Some(TraceTimeline {
            trace_id: run.trace_id.clone(),
            run,
            steps,
            events,
            latest_checkpoint: None,
            summary,
        })
    }

    pub async fn evidence_pack(&self, run_id: &str) -> Option<EvidencePack> {
        let trace = self.trace_timeline(run_id).await?;
        Some(build_evidence_pack(trace))
    }

    pub async fn evidence_markdown(&self, run_id: &str) -> Option<String> {
        self.evidence_pack(run_id)
            .await
            .map(|pack| evidence_pack_to_markdown(&pack))
    }

    pub async fn compare_attempts(
        &self,
        base_run_id: &str,
        target_run_id: &str,
    ) -> Option<AttemptComparison> {
        let base = self.timeline(base_run_id).await?;
        let target = self.timeline(target_run_id).await?;
        Some(compare_timelines(&base, &target))
    }

    pub async fn submit_feedback(
        &self,
        run_id: Option<String>,
        session_id: Option<String>,
        status: String,
        note: Option<String>,
        tags: Vec<String>,
    ) -> anyhow::Result<UserFeedbackRecord> {
        let status = normalize_feedback_status(&status)?;
        let record = UserFeedbackRecord {
            id: format!("fb_{}", Uuid::new_v4().simple()),
            run_id,
            session_id,
            status,
            note,
            tags,
            created_at: now_ms(),
        };
        self.data.write().await.feedback.push(record.clone());
        self.flush().await?;
        Ok(record)
    }

    async fn finish_step(
        &self,
        step_id: &str,
        status: &str,
        error: Option<String>,
        payload: BTreeMap<String, Value>,
    ) -> anyhow::Result<StepRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut step = data
            .steps
            .get(step_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Step {step_id} not found"))?;
        let mut run = data
            .runs
            .get(&step.run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {} not found", step.run_id))?;
        step.status = status.to_string();
        step.completed_at = Some(now);
        step.error = error.clone();
        step.metadata.extend(payload.clone());
        run.updated_at = now;
        if run.current_step_id.as_deref() == Some(step_id) {
            run.current_step_id = None;
        }
        data.steps.insert(step.id.clone(), step.clone());
        data.runs.insert(run.id.clone(), run.clone());
        let event_type = match status {
            "failed" => "step.failed",
            "cancelled" => "step.cancelled",
            _ => "step.completed",
        };
        push_run_event(
            &mut data,
            &run,
            event_type,
            Some(json!({ "step": step, "error": error, "payload": payload })),
            Some(step_id.to_string()),
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(step)
    }

    async fn finish_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<String>,
        payload: BTreeMap<String, Value>,
    ) -> anyhow::Result<RunRecord> {
        let now = now_ms();
        let mut data = self.data.write().await;
        let mut run = data
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Run {run_id} not found"))?;
        run.status = status.to_string();
        run.updated_at = now;
        run.completed_at = Some(now);
        run.error = error.clone();
        run.metadata.extend(payload.clone());
        data.runs.insert(run.id.clone(), run.clone());
        let event_type = match status {
            "failed" => "run.failed",
            "cancelled" => "run.cancelled",
            _ => "run.completed",
        };
        push_run_event(
            &mut data,
            &run,
            event_type,
            Some(json!({ "run": run, "error": error, "payload": payload })),
            None,
            now,
        );
        drop(data);
        self.flush().await?;
        Ok(run)
    }

    async fn flush(&self) -> anyhow::Result<()> {
        let snapshot = {
            let data = self.data.read().await;
            PersistedRunData {
                version: 1,
                updated_at: now_ms(),
                runs: data.runs.values().cloned().collect(),
                steps: data.steps.values().cloned().collect(),
                events: data.events.clone(),
                feedback: data.feedback.clone(),
            }
        };
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = serde_json::to_string_pretty(&snapshot)?;
        let tmp = self.path.with_extension("json.tmp");
        {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(format!("{text}\n").as_bytes()).await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

async fn load_run_data(path: &PathBuf) -> anyhow::Result<RunData> {
    let Ok(text) = tokio::fs::read_to_string(path).await else {
        return Ok(RunData::default());
    };
    let persisted = serde_json::from_str::<PersistedRunData>(text.trim_start_matches('\u{feff}'))?;
    Ok(RunData {
        runs: persisted
            .runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect(),
        steps: persisted
            .steps
            .into_iter()
            .map(|step| (step.id.clone(), step))
            .collect(),
        events: persisted.events,
        feedback: persisted.feedback,
    })
}

fn last_run_event_hash(events: &[RunEvent], run_id: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event.run_id == run_id)
        .and_then(|event| event.event_hash.clone())
}

fn push_run_event(
    data: &mut RunData,
    run: &RunRecord,
    event_type: &str,
    payload: Option<Value>,
    step_id: Option<String>,
    timestamp: i64,
) -> RunEvent {
    let prev_hash = last_run_event_hash(&data.events, &run.id);
    let event = run_event(run, event_type, payload, step_id, timestamp, prev_hash);
    data.events.push(event.clone());
    event
}

fn run_event(
    run: &RunRecord,
    event_type: &str,
    payload: Option<Value>,
    step_id: Option<String>,
    timestamp: i64,
    prev_hash: Option<String>,
) -> RunEvent {
    let payload = payload.map(redact_json_value);
    let id = format!("evt_{}", Uuid::new_v4().simple());
    let prev = prev_hash.clone().unwrap_or_default();
    let payload_text = payload
        .as_ref()
        .map(|value| value.to_string())
        .unwrap_or_default();
    let event_hash = Some(sha256_hex(&format!(
        "{prev}|{event_type}|{id}|{timestamp}|{payload_text}"
    )));
    RunEvent {
        id,
        run_id: run.id.clone(),
        session_id: run.session_id.clone(),
        trace_id: run.trace_id.clone(),
        event_type: event_type.to_string(),
        timestamp,
        step_id,
        payload,
        prev_hash,
        event_hash,
    }
}

fn redact_json_value(mut value: Value) -> Value {
    match &mut value {
        Value::String(text) => {
            *text = xuanyu_governance::redact_string(text);
        }
        Value::Array(items) => {
            for item in items {
                *item = redact_json_value(item.clone());
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                *item = redact_json_value(item.clone());
            }
        }
        _ => {}
    }
    value
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sorted_steps<'a>(steps: impl Iterator<Item = &'a StepRecord>, run_id: &str) -> Vec<StepRecord> {
    let mut result = steps
        .filter(|step| step.run_id == run_id)
        .cloned()
        .collect::<Vec<_>>();
    result.sort_by_key(|a| a.started_at);
    result
}

fn trace_event_from_run_event(event: &RunEvent, step: Option<&StepRecord>) -> TraceEvent {
    let category = trace_category(&event.event_type, step);
    let status = trace_status(&event.event_type, step, event.payload.as_ref());
    TraceEvent {
        id: event.id.clone(),
        run_id: event.run_id.clone(),
        session_id: event.session_id.clone(),
        trace_id: event.trace_id.clone(),
        timestamp: event.timestamp,
        event_type: event.event_type.clone(),
        canonical_type: Some(event.event_type.clone()),
        category,
        label: trace_label(&event.event_type, event.payload.as_ref()),
        status,
        step_id: event.step_id.clone(),
        duration_ms: step.and_then(|step| {
            step.completed_at
                .map(|completed| completed.saturating_sub(step.started_at))
        }),
        tokens: event.payload.as_ref().and_then(trace_tokens),
        cost_usd: event.payload.as_ref().and_then(trace_cost_usd),
        error_code: event.payload.as_ref().and_then(trace_error_code),
        evidence: Vec::new(),
        summary: trace_payload_summary(&event.event_type, event.payload.as_ref()),
        metadata: event
            .payload
            .as_ref()
            .map(value_to_metadata)
            .unwrap_or_default(),
    }
}

fn trace_category(event_type: &str, step: Option<&StepRecord>) -> String {
    if event_type.contains("model") {
        "model"
    } else if event_type.contains("tool") {
        "tool"
    } else if event_type.contains("permission") {
        "permission"
    } else if event_type.contains("memory") {
        "memory"
    } else if event_type.contains("context") {
        "context"
    } else if event_type.contains("checkpoint") {
        "checkpoint"
    } else if event_type.contains("failed") || event_type.contains("error") {
        "error"
    } else if event_type.contains("agent")
        || step.map(|step| step.step_type.as_str()) == Some("agent")
    {
        "agent"
    } else if event_type.starts_with("run.") || event_type.starts_with("step.") {
        "run"
    } else {
        "system"
    }
    .to_string()
}

fn trace_status(event_type: &str, step: Option<&StepRecord>, payload: Option<&Value>) -> String {
    if matches!(event_type, "agent.tool.result" | "trace.tool.result") {
        if payload.and_then(payload_bool_is_error).unwrap_or(false) {
            "failed"
        } else {
            "completed"
        }
    } else if matches!(
        event_type,
        "agent.tool.requested" | "agent.tool.retry" | "trace.tool.call"
    ) {
        "running"
    } else if event_type.contains("failed")
        || step.map(|step| step.status.as_str()) == Some("failed")
    {
        "failed"
    } else if event_type.contains("cancelled") {
        "cancelled"
    } else if event_type.contains("completed") || event_type.contains("response") {
        "completed"
    } else if event_type.contains("denied") {
        "denied"
    } else if event_type.contains("allowed") {
        "allowed"
    } else if step.map(|step| step.status.as_str()) == Some("running") {
        "running"
    } else {
        "info"
    }
    .to_string()
}

fn trace_label(event_type: &str, payload: Option<&Value>) -> String {
    match event_type {
        "agent.tool.requested" | "trace.tool.call" => {
            format!("Tool requested: {}", tool_name(payload))
        }
        "agent.tool.result" | "trace.tool.result" => {
            format!("Tool result: {}", tool_name(payload))
        }
        "agent.tool.retry" => format!("Tool retry: {}", tool_name(payload)),
        "agent.tool.skipped" => format!("Tool skipped: {}", tool_name(payload)),
        "agent.policy.tools" => "Policy tool plan".to_string(),
        "agent.policy.plan" => "Main policy plan".to_string(),
        "agent.policy.fallback" => "Policy fallback".to_string(),
        _ => event_type.to_string(),
    }
}

fn trace_summary(run: &RunRecord, events: &[TraceEvent]) -> TraceSummary {
    TraceSummary {
        total_events: events.len(),
        model_requests: events
            .iter()
            .filter(|event| event.category == "model" && event.event_type.contains("request"))
            .count(),
        tool_calls: events
            .iter()
            .filter(|event| {
                event.category == "tool"
                    && matches!(
                        event.event_type.as_str(),
                        "agent.tool.requested" | "trace.tool.call"
                    )
            })
            .count(),
        tool_failures: events
            .iter()
            .filter(|event| event.category == "tool" && event.status == "failed")
            .count(),
        permission_decisions: events
            .iter()
            .filter(|event| event.category == "permission")
            .count(),
        memory_retrievals: events
            .iter()
            .filter(|event| event.event_type == "trace.memory.retrieved")
            .count(),
        compactions: events
            .iter()
            .filter(|event| event.event_type == "context.compacted")
            .count(),
        errors: events
            .iter()
            .filter(|event| event.status == "failed" || event.category == "error")
            .count(),
        duration_ms: run
            .completed_at
            .map(|completed| completed.saturating_sub(run.created_at)),
    }
}

fn build_evidence_pack(trace: TraceTimeline) -> EvidencePack {
    let usage = resource_usage(&trace);
    EvidencePack {
        run_id: trace.run.id.clone(),
        trace_id: trace.trace_id.clone(),
        generated_at: now_ms(),
        user_goal: trace.run.goal.clone(),
        status: trace.run.status.clone(),
        key_decisions: collect_key_decisions(&trace.events),
        modified_files: collect_string_set(
            &trace.events,
            &["file", "path", "changedFile", "changedFiles", "files"],
        ),
        diff_summary: collect_string_set(&trace.events, &["diffSummary", "diff", "summary"]),
        commands: collect_string_set(&trace.events, &["command", "cmd", "script"]),
        validation_results: trace
            .events
            .iter()
            .filter(|event| {
                event.event_type.contains("validation") || event.event_type.contains("test")
            })
            .filter_map(|event| event.summary.clone().or_else(|| Some(event.label.clone())))
            .collect(),
        failures_and_recovery: trace
            .events
            .iter()
            .filter(|event| event.status == "failed" || event.category == "error")
            .filter_map(|event| event.summary.clone().or_else(|| Some(event.label.clone())))
            .collect(),
        resource_usage: usage,
        risks: collect_string_set(&trace.events, &["risk", "risks"]),
        follow_ups: collect_string_set(
            &trace.events,
            &["followUp", "followUps", "nextStep", "nextSteps"],
        ),
        events: trace.events,
    }
}

fn evidence_pack_to_markdown(pack: &EvidencePack) -> String {
    [
        format!("# Evidence Pack: {}", pack.user_goal),
        String::new(),
        format!("- Run: {}", pack.run_id),
        format!("- Trace: {}", pack.trace_id),
        format!("- Status: {}", pack.status),
        format!("- Generated: {}", pack.generated_at),
        String::new(),
        "## Key Decisions".to_string(),
        markdown_list(&pack.key_decisions),
        String::new(),
        "## Modified Files".to_string(),
        markdown_list(&pack.modified_files),
        String::new(),
        "## Commands".to_string(),
        markdown_list(&pack.commands),
        String::new(),
        "## Validation Results".to_string(),
        markdown_list(&pack.validation_results),
        String::new(),
        "## Failures And Recovery".to_string(),
        markdown_list(&pack.failures_and_recovery),
        String::new(),
        "## Resource Usage".to_string(),
        format!(
            "- Tokens: {} total ({} prompt / {} completion)",
            pack.resource_usage.total_tokens,
            pack.resource_usage.prompt_tokens,
            pack.resource_usage.completion_tokens
        ),
        format!("- Model requests: {}", pack.resource_usage.model_requests),
        format!("- Tool calls: {}", pack.resource_usage.tool_calls),
        String::new(),
        "## Risks".to_string(),
        markdown_list(&pack.risks),
        String::new(),
        "## Follow Ups".to_string(),
        markdown_list(&pack.follow_ups),
    ]
    .join("\n")
}

fn markdown_list(items: &[String]) -> String {
    if items.is_empty() {
        "- None recorded.".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn compare_timelines(base: &RunTimeline, target: &RunTimeline) -> AttemptComparison {
    let base_model = metadata_string(&base.run.metadata, "model");
    let target_model = metadata_string(&target.run.metadata, "model");
    let base_context_hash = timeline_fingerprint(base, "context");
    let target_context_hash = timeline_fingerprint(target, "context");
    let base_tool_results = event_fingerprints(base, "tool");
    let target_tool_results = event_fingerprints(target, "tool");
    let base_changed_files = changed_files(base);
    let target_changed_files = changed_files(target);
    let model_different = base_model != target_model;
    let context_different = base_context_hash != target_context_hash;
    let tool_results_different = base_tool_results != target_tool_results;
    let changes_different = base_changed_files != target_changed_files;
    let mut summary = Vec::new();
    if model_different {
        summary.push(format!(
            "Model changed: {} -> {}",
            base_model.as_deref().unwrap_or("-"),
            target_model.as_deref().unwrap_or("-")
        ));
    }
    if context_different {
        summary.push("Context events differ.".to_string());
    }
    if tool_results_different {
        summary.push("Tool results differ.".to_string());
    }
    if changes_different {
        summary.push("Changed files differ.".to_string());
    }
    if summary.is_empty() {
        summary
            .push("No model, context, tool result, or changed-file differences found.".to_string());
    }
    AttemptComparison {
        base_run_id: base.run.id.clone(),
        target_run_id: target.run.id.clone(),
        model_different,
        context_different,
        tool_results_different,
        changes_different,
        summary,
        details: BTreeMap::from([
            ("baseModel".to_string(), json!(base_model)),
            ("targetModel".to_string(), json!(target_model)),
            ("baseContextHash".to_string(), json!(base_context_hash)),
            ("targetContextHash".to_string(), json!(target_context_hash)),
            ("baseToolResults".to_string(), json!(base_tool_results)),
            ("targetToolResults".to_string(), json!(target_tool_results)),
            ("baseChangedFiles".to_string(), json!(base_changed_files)),
            (
                "targetChangedFiles".to_string(),
                json!(target_changed_files),
            ),
        ]),
    }
}

fn resource_usage(trace: &TraceTimeline) -> EvidenceResourceUsage {
    let mut usage = EvidenceResourceUsage {
        duration_ms: trace.summary.duration_ms,
        model_requests: trace.summary.model_requests,
        tool_calls: trace.summary.tool_calls,
        ..EvidenceResourceUsage::default()
    };
    for event in &trace.events {
        let Some(tokens) = event.tokens.as_ref().and_then(Value::as_object) else {
            continue;
        };
        usage.prompt_tokens += tokens.get("prompt").and_then(Value::as_u64).unwrap_or(0);
        usage.completion_tokens += tokens
            .get("completion")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        usage.total_tokens += tokens.get("total").and_then(Value::as_u64).unwrap_or(0);
    }
    usage
}

fn collect_key_decisions(events: &[TraceEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            event.event_type.contains("plan")
                || event.event_type.contains("decision")
                || event.category == "permission"
        })
        .filter_map(|event| event.summary.clone().or_else(|| Some(event.label.clone())))
        .take(40)
        .collect()
}

fn collect_string_set(events: &[TraceEvent], keys: &[&str]) -> Vec<String> {
    let mut values = BTreeSet::new();
    for event in events {
        for key in keys {
            collect_value_strings(event.metadata.get(*key), &mut values);
        }
    }
    values.into_iter().collect()
}

fn collect_value_strings(value: Option<&Value>, values: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            values.insert(text.trim().to_string());
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_value_strings(Some(item), values);
            }
        }
        Some(Value::Object(map)) => {
            for value in map.values() {
                collect_value_strings(Some(value), values);
            }
        }
        _ => {}
    }
}

fn event_fingerprints(timeline: &RunTimeline, needle: &str) -> Vec<String> {
    timeline
        .events
        .iter()
        .filter(|event| event.event_type.contains(needle))
        .map(|event| {
            format!(
                "{}:{}",
                event.event_type,
                event
                    .payload
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default()
            )
        })
        .collect()
}

fn changed_files(timeline: &RunTimeline) -> Vec<String> {
    let trace_events = timeline
        .events
        .iter()
        .map(|event| trace_event_from_run_event(event, None))
        .collect::<Vec<_>>();
    collect_string_set(
        &trace_events,
        &["file", "path", "changedFile", "changedFiles", "files"],
    )
}

fn timeline_fingerprint(timeline: &RunTimeline, needle: &str) -> String {
    event_fingerprints(timeline, needle).join("\n")
}

fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn trace_payload_summary(event_type: &str, payload: Option<&Value>) -> Option<String> {
    let payload = payload?;
    match event_type {
        "agent.tool.requested" | "trace.tool.call" => join_summary_parts(vec![
            payload_field_string(payload, &["source"]).map(|value| format!("source {value}")),
            payload_field_string(payload, &["agentId"]).map(|value| format!("agent {value}")),
            payload_field_string(payload, &["taskId"]).map(|value| format!("task {value}")),
            payload_field_string(payload, &["iteration"]).map(|value| format!("iteration {value}")),
            payload_field_string(payload, &["reason"])
                .map(|value| format!("reason {}", compact_text(&value, 140))),
            payload
                .get("args")
                .map(|value| format!("args {}", compact_json(value, 220))),
        ]),
        "agent.tool.result" | "trace.tool.result" => {
            let status = if payload_bool_is_error(payload).unwrap_or(false) {
                "failed"
            } else {
                "ok"
            };
            join_summary_parts(vec![
                Some(status.to_string()),
                payload_field_string(payload, &["source"]).map(|value| format!("source {value}")),
                payload_field_string(payload, &["iteration"])
                    .map(|value| format!("iteration {value}")),
                payload_field_string(payload, &["content", "summary", "error", "message"])
                    .map(|value| compact_text(&value, 220)),
                payload_field_string(payload, &["affectedFiles", "changedFiles", "files"])
                    .map(|value| format!("files {}", compact_text(&value, 160))),
                payload_field_string(payload, &["requiresApproval"])
                    .filter(|value| value == "true")
                    .map(|_| "requires approval".to_string()),
            ])
        }
        "agent.tool.retry" => join_summary_parts(vec![
            payload_field_string(payload, &["source"]).map(|value| format!("source {value}")),
            payload_field_string(payload, &["iteration"]).map(|value| format!("iteration {value}")),
        ]),
        "agent.tool.skipped" => join_summary_parts(vec![
            payload_field_string(payload, &["source"]).map(|value| format!("source {value}")),
            payload_field_string(payload, &["reason"]).map(|value| format!("reason {value}")),
        ]),
        "agent.policy.tools" => join_summary_parts(vec![
            payload_field_string(payload, &["executed"]).map(|value| format!("executed {value}")),
            payload_field_string(payload, &["skipped"]).map(|value| format!("skipped {value}")),
            payload_field_string(payload, &["failed"]).map(|value| format!("failed {value}")),
            first_array_text(payload, "evidence")
                .map(|value| format!("evidence {}", compact_text(&value, 180))),
            first_array_text(payload, "risks")
                .map(|value| format!("risk {}", compact_text(&value, 180))),
        ]),
        "agent.policy.plan" => join_summary_parts(vec![
            payload_field_string(payload, &["intent"]).map(|value| format!("intent {value}")),
            payload_field_string(payload, &["responseStyle"]).map(|value| format!("style {value}")),
            payload_field_string(payload, &["modelPreference"])
                .map(|value| format!("model {value}")),
        ])
        .or_else(|| payload_summary(payload)),
        _ => payload_summary(payload),
    }
}

fn trace_tokens(payload: &Value) -> Option<Value> {
    if let Some(tokens) = payload.get("tokens") {
        return normalized_token_value(tokens).or_else(|| Some(tokens.clone()));
    }
    if let Some(usage) = payload.get("usage") {
        if let Some(tokens) = normalized_token_value(usage) {
            return Some(tokens);
        }
        return Some(usage.clone());
    }
    normalized_token_value(payload)
}

fn normalized_token_value(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let prompt = payload_u64(
        object,
        &["prompt", "promptTokens", "prompt_tokens", "inputTokens"],
    );
    let completion = payload_u64(
        object,
        &[
            "completion",
            "completionTokens",
            "completion_tokens",
            "outputTokens",
        ],
    );
    let total = payload_u64(object, &["total", "totalTokens", "total_tokens"]);
    let cache = payload_u64(object, &["cache", "cacheTokens", "cachedTokens"]);
    if prompt.is_none() && completion.is_none() && total.is_none() && cache.is_none() {
        return None;
    }
    let mut tokens = serde_json::Map::new();
    if let Some(value) = prompt {
        tokens.insert("prompt".to_string(), json!(value));
    }
    if let Some(value) = completion {
        tokens.insert("completion".to_string(), json!(value));
    }
    if let Some(value) = total {
        tokens.insert("total".to_string(), json!(value));
    }
    if let Some(value) = cache {
        tokens.insert("cache".to_string(), json!(value));
    }
    Some(Value::Object(tokens))
}

fn trace_cost_usd(payload: &Value) -> Option<f64> {
    payload_f64(payload, &["costUsd", "cost_usd", "estimatedCostUsd"])
}

fn trace_error_code(payload: &Value) -> Option<String> {
    payload_field_string(payload, &["errorCode", "code"]).or_else(|| {
        payload
            .get("error")
            .and_then(|error| payload_field_string(error, &["code", "errorCode"]))
    })
}

fn payload_bool_is_error(payload: &Value) -> Option<bool> {
    if let Some(value) = payload.get("isError").and_then(Value::as_bool) {
        return Some(value);
    }
    if let Some(status) = payload_field_string(payload, &["status"]) {
        if matches!(status.as_str(), "failed" | "error") {
            return Some(true);
        }
    }
    if payload.get("error").is_some() {
        return Some(true);
    }
    None
}

fn tool_name(payload: Option<&Value>) -> String {
    payload
        .and_then(|value| {
            payload_field_string(value, &["tool", "toolName", "name", "requestedTool"])
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn payload_field_string(payload: &Value, keys: &[&str]) -> Option<String> {
    let object = payload.as_object()?;
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(text) = value_to_summary_string(value) {
            return Some(text);
        }
    }
    None
}

fn value_to_summary_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(compact_json(value, 180)),
        _ => None,
    }
}

fn payload_u64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(number) = value
            .as_u64()
            .or_else(|| value.as_f64().map(|number| number.max(0.0).round() as u64))
        {
            return Some(number);
        }
    }
    None
}

fn payload_f64(payload: &Value, keys: &[&str]) -> Option<f64> {
    let object = payload.as_object()?;
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(number) = value.as_f64() {
            return Some(number);
        }
    }
    None
}

fn first_array_text(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(value_to_summary_string)
}

fn join_summary_parts(parts: Vec<Option<String>>) -> Option<String> {
    let joined = parts
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    (!joined.is_empty()).then_some(joined)
}

fn compact_json(value: &Value, max_chars: usize) -> String {
    compact_text(
        &serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
        max_chars,
    )
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let take = max_chars.saturating_sub(3).max(1);
    format!("{}...", normalized.chars().take(take).collect::<String>())
}

fn payload_summary(value: &Value) -> Option<String> {
    if let Some(summary) = value.get("summary").and_then(Value::as_str) {
        return Some(summary.to_string());
    }
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(content.chars().take(180).collect());
    }
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Some(error.to_string());
    }
    None
}

fn value_to_metadata(value: &Value) -> BTreeMap<String, Value> {
    match value {
        Value::Object(map) => map.clone().into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

fn sanitize_segment(value: &str) -> String {
    let clean = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    clean.trim_matches('_').chars().take(80).collect::<String>()
}

fn normalize_feedback_status(status: &str) -> anyhow::Result<String> {
    let normalized = status.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "success" | "failed" | "partial" | "extra_changes" | "forgot_constraint" => {
            Ok(normalized)
        }
        "" => Err(anyhow::anyhow!(
            "feedback status required (success|failed|partial|extra_changes|forgot_constraint)"
        )),
        other => Err(anyhow::anyhow!(
            "invalid feedback status '{other}' (expected success|failed|partial|extra_changes|forgot_constraint)"
        )),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
