use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

pub mod worker_session;

pub use worker_session::{persistent_workers_enabled, prepare_worker_command, WorkerSessionSlot};
use uuid::Uuid;

#[derive(Debug, Clone)]
enum AgentWorkerLaunch {
    Binary(PathBuf),
    Python { python: String, script: PathBuf },
}

#[derive(Debug, Clone)]
pub struct AgentWorkerBridge {
    python: String,
    worker_binary: Option<PathBuf>,
    script_path: Option<PathBuf>,
    slot: WorkerSessionSlot,
    status_cache: Arc<Mutex<Option<AgentWorkerStatus>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkerState {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkerStatus {
    pub state: AgentWorkerState,
    pub configured: bool,
    pub python: String,
    pub script_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_binary: Option<PathBuf>,
    pub detail: String,
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyRequest {
    pub session_id: Option<String>,
    pub mode: Option<String>,
    pub user_input: String,
    #[serde(default)]
    pub recent_messages: Vec<AgentPolicyMessage>,
    #[serde(default)]
    pub available_tools: Vec<String>,
    #[serde(default)]
    pub memory_hits: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub budget: Option<AgentPolicyBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyBudget {
    pub max_context_tokens: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyDecision {
    pub intent: String,
    #[serde(default)]
    pub system_prompt_patch: Vec<String>,
    #[serde(default)]
    pub tool_plan: Vec<SubAgentToolRequest>,
    #[serde(default)]
    pub memory_write_candidates: Vec<AgentPolicyMemoryCandidate>,
    #[serde(default)]
    pub context_hints: Vec<String>,
    pub model_preference: Option<String>,
    pub response_style: Option<String>,
    #[serde(default)]
    pub safety_notes: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyMemoryCandidate {
    pub content: String,
    pub category: Option<String>,
    pub importance: Option<u8>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StimulusClassificationRequest {
    pub source: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StimulusClassification {
    pub brain_kind: String,
    pub intensity: f64,
    pub attention: String,
    pub world_key: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentPlanRequest {
    pub task_id: String,
    pub agent_id: String,
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub task: String,
    pub context: Option<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub artifact_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentPlanResponse {
    pub title: String,
    pub steps: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub tool_requests: Vec<SubAgentToolRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentToolRequest {
    pub name: String,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkerRequest {
    id: String,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct WorkerResponse {
    id: String,
    ok: bool,
    #[serde(default)]
    result: Value,
    error: Option<String>,
}

impl AgentWorkerBridge {
    pub fn discover() -> Self {
        let python =
            std::env::var("XUANYU_AGENT_WORKER_PYTHON").unwrap_or_else(|_| "python".to_string());
        let worker_binary = std::env::var("XUANYU_AGENT_WORKER_BIN")
            .ok()
            .map(PathBuf::from)
            .or_else(default_worker_binary);
        let script_path = if worker_binary.as_ref().is_some_and(|path| path.is_file()) {
            None
        } else {
            std::env::var("XUANYU_AGENT_WORKER_SCRIPT")
                .ok()
                .map(PathBuf::from)
                .or_else(default_worker_script)
        };
        Self {
            python,
            worker_binary,
            script_path,
            slot: WorkerSessionSlot::new(),
            status_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_script(python: impl Into<String>, script_path: impl Into<PathBuf>) -> Self {
        Self {
            python: python.into(),
            worker_binary: None,
            script_path: Some(script_path.into()),
            slot: WorkerSessionSlot::new(),
            status_cache: Arc::new(Mutex::new(None)),
        }
    }

    fn worker_launch(&self) -> Option<AgentWorkerLaunch> {
        if let Some(binary) = self.worker_binary.clone().filter(|path| path.is_file()) {
            return Some(AgentWorkerLaunch::Binary(binary));
        }
        let script = self.script_path.clone().filter(|path| path.is_file())?;
        Some(AgentWorkerLaunch::Python {
            python: self.python.clone(),
            script,
        })
    }

    fn spawn_worker_command(launch: &AgentWorkerLaunch) -> Command {
        let mut command = match launch {
            AgentWorkerLaunch::Binary(binary) => Command::new(binary),
            AgentWorkerLaunch::Python { python, script } => {
                let mut command = Command::new(python);
                command.arg(script);
                command
            }
        };
        command.arg("--serve");
        prepare_worker_command(&mut command);
        command
    }

    pub fn is_configured(&self) -> bool {
        self.worker_launch().is_some()
    }

    pub fn status(&self) -> AgentWorkerStatus {
        if WorkerSessionSlot::cache_fresh(self.slot.last_ok_ms(), Duration::from_secs(30)) {
            if let Ok(guard) = self.status_cache.lock() {
                if let Some(status) = guard.as_ref() {
                    return status.clone();
                }
            }
        }
        self.refresh_status()
    }

    pub fn warm(&self) -> anyhow::Result<Value> {
        self.ensure_persistent_session()?;
        let result = self.health()?;
        let _ = self.refresh_status();
        Ok(result)
    }

    pub fn ensure_persistent_session(&self) -> anyhow::Result<()> {
        let launch = self
            .worker_launch()
            .ok_or_else(|| anyhow!("agent worker was not found"))?;
        self.slot
            .ensure(move || Ok(Self::spawn_worker_command(&launch)))
    }

    fn refresh_status(&self) -> AgentWorkerStatus {
        let Some(launch) = self.worker_launch() else {
            let expected = self
                .worker_binary
                .clone()
                .or_else(|| self.script_path.clone())
                .map(|path| display_path(&path))
                .unwrap_or_else(|| "xuanyu-agent-worker.exe".to_string());
            return AgentWorkerStatus {
                state: AgentWorkerState::Unavailable,
                configured: false,
                python: self.python.clone(),
                script_path: self.script_path.clone(),
                worker_binary: self.worker_binary.clone(),
                detail: format!(
                    "agent worker not found (expected {expected}); reinstall or set XUANYU_AGENT_WORKER_BIN"
                ),
                methods: Vec::new(),
            };
        };
        let (script_path, worker_binary) = match &launch {
            AgentWorkerLaunch::Binary(path) => (None, Some(path.clone())),
            AgentWorkerLaunch::Python { script, .. } => (Some(script.clone()), None),
        };
        let status = match self.health() {
            Ok(result) => status_from_ping(
                script_path.clone(),
                worker_binary.clone(),
                self.python.clone(),
                result,
            ),
            Err(err) => AgentWorkerStatus {
                state: AgentWorkerState::Degraded,
                configured: true,
                python: self.python.clone(),
                script_path,
                worker_binary,
                detail: format!(
                    "agent worker configured but ping failed: {}",
                    compact_error(&err.to_string())
                ),
                methods: Vec::new(),
            },
        };
        if let Ok(mut guard) = self.status_cache.lock() {
            *guard = Some(status.clone());
        }
        status
    }

    pub fn status_snapshot(&self) -> AgentWorkerStatus {
        self.status()
    }

    pub fn health(&self) -> anyhow::Result<Value> {
        self.request("ping", json!({}))
    }

    pub fn plan_subagent_task(
        &self,
        input: SubAgentPlanRequest,
    ) -> anyhow::Result<SubAgentPlanResponse> {
        let value = self.request("subagent.plan", serde_json::to_value(input)?)?;
        let plan = serde_json::from_value::<SubAgentPlanResponse>(value)
            .context("agent worker returned invalid subagent plan")?;
        if plan.steps.is_empty() {
            bail!("agent worker returned an empty subagent plan");
        }
        Ok(plan)
    }

    pub fn plan_turn(&self, input: AgentPolicyRequest) -> anyhow::Result<AgentPolicyDecision> {
        let value = self.request("policy.plan_turn", serde_json::to_value(input)?)?;
        let decision = serde_json::from_value::<AgentPolicyDecision>(value)
            .context("agent worker returned invalid policy decision")?;
        if decision.intent.trim().is_empty() {
            bail!("agent worker returned a policy decision without intent");
        }
        Ok(decision)
    }

    pub fn classify_stimulus(
        &self,
        input: StimulusClassificationRequest,
    ) -> anyhow::Result<StimulusClassification> {
        let value = self.request("policy.classify_stimulus", serde_json::to_value(input)?)?;
        let classification = serde_json::from_value::<StimulusClassification>(value)
            .context("agent worker returned invalid stimulus classification")?;
        if classification.summary.trim().is_empty() {
            bail!("agent worker returned an empty stimulus summary");
        }
        Ok(classification)
    }

    fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let _launch = self
            .worker_launch()
            .ok_or_else(|| anyhow!("agent worker was not found"))?;
        let request = WorkerRequest {
            id: format!("req_{}", Uuid::new_v4().simple()),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&request)?;
        if persistent_workers_enabled() {
            if let Ok(response_line) = self.request_persistent(&line) {
                return parse_worker_response_line(&response_line, &request.id);
            }
        }
        self.request_once(&line, &request.id)
    }

    fn request_persistent(&self, line: &str) -> anyhow::Result<String> {
        let launch = self
            .worker_launch()
            .ok_or_else(|| anyhow!("agent worker was not found"))?;
        self.slot
            .request_persistent(move || Ok(Self::spawn_worker_command(&launch)), line)
    }

    fn request_once(&self, line: &str, request_id: &str) -> anyhow::Result<Value> {
        use std::io::Write;
        let launch = self
            .worker_launch()
            .ok_or_else(|| anyhow!("agent worker was not found"))?;
        let mut cmd = Self::spawn_worker_command(&launch);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| match &launch {
            AgentWorkerLaunch::Binary(path) => {
                format!("failed to start agent worker at {}", display_path(path))
            }
            AgentWorkerLaunch::Python { python, .. } => {
                format!("failed to start agent worker via {python}")
            }
        })?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow!("agent worker stdin is unavailable"))?;
            stdin.write_all(line.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let output = child
            .wait_with_output()
            .context("failed waiting for agent worker")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            bail!(
                "agent worker exited with {:?}: {}",
                output.status.code(),
                stderr
            );
        }
        parse_worker_stdout(&output.stdout, request_id)
    }
}

fn parse_worker_response_line(response_line: &str, request_id: &str) -> anyhow::Result<Value> {
    let line = response_line.trim();
    let response = serde_json::from_str::<WorkerResponse>(line)
        .with_context(|| format!("agent worker response was not JSON: {line}"))?;
    if response.id != request_id {
        bail!(
            "agent worker response id mismatch: expected {request_id}, got {}",
            response.id
        );
    }
    if !response.ok {
        bail!(
            "agent worker request failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
    Ok(response.result)
}

fn parse_worker_stdout(stdout: &[u8], request_id: &str) -> anyhow::Result<Value> {
    let text = String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| anyhow!("agent worker returned no response"))?;
    let response = serde_json::from_str::<WorkerResponse>(line)
        .with_context(|| format!("agent worker response was not JSON: {line}"))?;
    if response.id != request_id {
        bail!(
            "agent worker response id mismatch: expected {request_id}, got {}",
            response.id
        );
    }
    if !response.ok {
        bail!(
            "agent worker request failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
    Ok(response.result)
}

fn status_from_ping(
    script_path: Option<PathBuf>,
    worker_binary: Option<PathBuf>,
    python: String,
    result: Value,
) -> AgentWorkerStatus {
    let runtime_status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let worker = result
        .get("worker")
        .and_then(Value::as_str)
        .unwrap_or("agent worker");
    let methods = result
        .get("methods")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let state = if runtime_status == "ready" {
        AgentWorkerState::Healthy
    } else {
        AgentWorkerState::Degraded
    };
    let method_detail = if methods.is_empty() {
        "no methods reported".to_string()
    } else {
        format!("methods: {}", methods.join(", "))
    };
    let location = worker_binary
        .as_ref()
        .or(script_path.as_ref())
        .map(|path| display_path(path.as_path()))
        .unwrap_or_else(|| "agent worker".to_string());
    AgentWorkerStatus {
        state,
        configured: true,
        python,
        script_path,
        worker_binary,
        detail: format!("{worker} {runtime_status} at {location} ({method_detail})"),
        methods,
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn compact_error(error: &str) -> String {
    const LIMIT_CHARS: usize = 220;
    if error.chars().count() <= LIMIT_CHARS {
        return error.to_string();
    }
    format!("{}...", error.chars().take(LIMIT_CHARS).collect::<String>())
}

fn default_worker_binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "xuanyu-agent-worker.exe"
    } else {
        "xuanyu-agent-worker"
    };
    let mut candidates = Vec::new();
    if let Some(dir) = worker_session::executable_dir() {
        candidates.push(dir.join(exe));
        candidates.push(dir.join("workers").join(exe));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("workers/agent-python/dist").join(exe));
        candidates.push(cwd.join(exe));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn default_worker_script() -> Option<PathBuf> {
    let rel = PathBuf::from("workers/agent-python/xuanyu_agent_worker/main.py");
    let mut candidates = Vec::new();
    if let Some(dir) = worker_session::executable_dir() {
        candidates.push(dir.join(&rel));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(&rel));
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join(&rel));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}
