use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Component, Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::fs as async_fs;
use tokio::sync::RwLock;
use xuanyu_agents::{AgentRuntime, SubAgentTaskInput, SubAgentTaskLedger};
use xuanyu_governance::{
    GovernanceAction, GovernanceDecision, GovernanceEngine, GovernanceRequest, PermissionMode,
};
use xuanyu_memory::{
    kind_for_category, with_memory_session_tag, MemoryIngestInput, MemorySearchQuery,
    MemoryService, MemoryUpdate, NewMemoryEntry,
};
use xuanyu_governance::{RecordWriteInput, TurnLedgerStore};

mod mcp;
mod mcp_remote;
mod os_sandbox;
mod output_bound;
mod validation_store;
mod session_work;
mod tool_surface;
pub use mcp::McpLoadSummary;
pub use session_work::todos_prompt_block;
pub use output_bound::{EXEC_MAX_OUTPUT_BYTES, TOOL_INLINE_MAX_BYTES};
pub use tool_surface::{
    format_deferred_tool_catalog_for_prompt, initial_active_tool_names, is_meta_tool_name,
    should_report_meta_load_as_error, AGENT_CORE_PRELOAD_TOOLS, META_TOOL_NAMES,
    DeferredCatalogOptions,
};
use mcp::McpRuntime;
use session_work::{complete_session_goal, sync_todos, TodoItem, TodoStatus};
use xuanyu_extensions::{
    load_extensions, load_plugins, run_plugin_tool_hooks, ExtensionLoadConfig, PluginToolHookInput,
};
use xuanyu_storage::{project_data_root, runtime_data_root};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub aliases: Vec<String>,
    pub is_write: bool,
    pub is_execute: bool,
    pub is_concurrent: bool,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogEntry {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub source: ToolSource,
    pub is_write: bool,
    pub is_execute: bool,
    pub is_concurrent: bool,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Builtin,
    Mcp,
    Plugin,
    Internal,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub args: BTreeMap<String, serde_json::Value>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundProcessSnapshot {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub command: String,
    pub cwd: String,
    pub log_path: String,
    pub status: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    pub output_bytes: usize,
}

struct BackgroundProcess {
    id: String,
    pid: Option<u32>,
    name: Option<String>,
    command: String,
    cwd: PathBuf,
    log_path: PathBuf,
    status: String,
    started_at: i64,
    completed_at: Option<i64>,
    exit_code: Option<i32>,
    signal: Option<String>,
    output: Vec<u8>,
    child: Arc<Mutex<std::process::Child>>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    stop_requested: bool,
}

const COMPLETED_PROCESS_RETENTION_MS: i64 = 60 * 60 * 1000;

#[derive(Default)]
pub struct ProcessSupervisor {
    processes: Mutex<BTreeMap<String, Arc<Mutex<BackgroundProcess>>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApplyPatchInput {
    patches: Vec<FilePatch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct FilePatch {
    path: String,
    action: PatchAction,
    content: Option<String>,
    old_text: Option<String>,
    new_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PatchAction {
    Add,
    Delete,
    Update,
}

const MAX_DELEGATION_CONTEXT_FIELD_CHARS: usize = 8_000;
const MAX_DELEGATION_TOTAL_CONTEXT_CHARS: isize = 12_000;
const DELEGATION_CONTEXT_TRUNCATED_MARKER: &str = "\n...[context truncated]";

#[derive(Debug, Clone)]
struct ParsedSubAgentTaskInputs {
    inputs: Vec<SubAgentTaskInput>,
    total_context_chars: usize,
    context_truncated: bool,
    duplicate_subtask_count: usize,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDefinition>,
    aliases: BTreeMap<String, String>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: ToolDefinition) -> anyhow::Result<()> {
        if self.tools.contains_key(&tool.name) || self.aliases.contains_key(&tool.name) {
            anyhow::bail!("tool name already registered: {}", tool.name);
        }
        for alias in &tool.aliases {
            if self.tools.contains_key(alias) || self.aliases.contains_key(alias) {
                anyhow::bail!("tool alias conflicts with existing tool: {alias}");
            }
        }
        for alias in &tool.aliases {
            self.aliases.insert(alias.clone(), tool.name.clone());
        }
        self.tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    pub fn register_all(
        &mut self,
        tools: impl IntoIterator<Item = ToolDefinition>,
    ) -> anyhow::Result<()> {
        for tool in tools {
            self.register(tool)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        let resolved = self.aliases.get(name).map(String::as_str).unwrap_or(name);
        self.tools.get(resolved)
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn catalog(&self, source: ToolSource) -> Vec<ToolCatalogEntry> {
        self.tools
            .values()
            .map(|tool| ToolCatalogEntry {
                name: tool.name.clone(),
                description: tool.description.clone(),
                category: tool.category.clone(),
                source: infer_tool_source(&tool.name, source),
                is_write: tool.is_write,
                is_execute: tool.is_execute,
                is_concurrent: tool.is_concurrent,
                aliases: tool.aliases.clone(),
            })
            .collect()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().cloned().collect()
    }

    fn remove_mcp_tools(&mut self) {
        let names = self
            .tools
            .keys()
            .filter(|name| name.starts_with("mcp__"))
            .cloned()
            .collect::<Vec<_>>();
        for name in names {
            self.tools.remove(&name);
        }
        self.aliases
            .retain(|alias, target| !alias.starts_with("mcp__") && !target.starts_with("mcp__"));
    }
}

impl ProcessSupervisor {
    pub fn start(
        &self,
        command: String,
        cwd: PathBuf,
        name: Option<String>,
    ) -> anyhow::Result<BackgroundProcessSnapshot> {
        let id = format!("proc_{}", uuid::Uuid::new_v4().simple());
        let log_path = std::env::temp_dir().join(format!("{id}.log"));
        let mut child = match os_sandbox::shell_command_with_os_sandbox(&command, &cwd) {
            Ok(mut cmd) => cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::piped())
                .spawn()?,
            Err(err) => anyhow::bail!("{err}"),
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child
            .stdin
            .take()
            .map(|handle| Arc::new(Mutex::new(handle)));
        let child = Arc::new(Mutex::new(child));
        let process = Arc::new(Mutex::new(BackgroundProcess {
            id: id.clone(),
            pid: child.lock().ok().map(|child| child.id()),
            name,
            command,
            cwd,
            log_path,
            status: "running".to_string(),
            started_at: now_ms(),
            completed_at: None,
            exit_code: None,
            signal: None,
            output: Vec::new(),
            child: child.clone(),
            stdin,
            stop_requested: false,
        }));
        self.processes
            .lock()
            .expect("process table lock")
            .insert(id, process.clone());
        if let Some(stdout) = stdout {
            spawn_output_reader(stdout, process.clone());
        }
        if let Some(stderr) = stderr {
            spawn_output_reader(stderr, process.clone());
        }
        spawn_waiter(process.clone());
        Ok(snapshot(&process))
    }

    pub fn list(&self) -> Vec<BackgroundProcessSnapshot> {
        self.cleanup_completed_older_than(COMPLETED_PROCESS_RETENTION_MS);
        let mut processes = self
            .processes
            .lock()
            .expect("process table lock")
            .values()
            .map(snapshot)
            .collect::<Vec<_>>();
        processes.sort_by_key(|p| std::cmp::Reverse(p.started_at));
        processes
    }

    pub fn get(&self, process_id: &str) -> Option<BackgroundProcessSnapshot> {
        self.processes
            .lock()
            .expect("process table lock")
            .get(process_id)
            .map(snapshot)
    }

    pub fn wait_for_exit(
        &self,
        process_id: &str,
        timeout_ms: u64,
    ) -> Option<BackgroundProcessSnapshot> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let process = self.get(process_id)?;
            if process.status != "running" || std::time::Instant::now() >= deadline {
                return Some(process);
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    pub fn wait_for_output_or_exit(
        &self,
        process_id: &str,
        timeout_ms: u64,
    ) -> Option<BackgroundProcessSnapshot> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let process = self.get(process_id)?;
            if process.status != "running"
                || process.output_bytes > 0
                || std::time::Instant::now() >= deadline
            {
                return Some(process);
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    pub fn read(&self, process_id: &str, offset: usize, max_bytes: usize) -> Option<ProcessRead> {
        let process = self
            .processes
            .lock()
            .expect("process table lock")
            .get(process_id)
            .cloned()?;
        let max_bytes = max_bytes.clamp(1, 1024 * 1024);
        let (content, next_offset) = {
            let process_guard = process.lock().expect("process lock");
            read_process_log(&process_guard, offset, max_bytes)
        };
        Some(ProcessRead {
            process_id: process_id.to_string(),
            content: if content.is_empty() {
                "(no new output)".to_string()
            } else {
                content
            },
            offset: next_offset,
            process: snapshot(&process),
        })
    }

    pub fn stop(&self, process_id: &str) -> anyhow::Result<BackgroundProcessSnapshot> {
        let process = self
            .processes
            .lock()
            .expect("process table lock")
            .get(process_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown background process: {process_id}"))?;
        {
            let mut guard = process.lock().expect("process lock");
            if guard.status != "running" {
                return Ok(snapshot_from_guard(&guard));
            }
            guard.stop_requested = true;
            guard.status = "stopped".to_string();
            guard.completed_at = Some(now_ms());
            guard.stdin = None;
            terminate_child(&guard);
        }
        Ok(snapshot(&process))
    }

    pub fn write_stdin(
        &self,
        process_id: &str,
        data: &str,
        eof: bool,
    ) -> anyhow::Result<BackgroundProcessSnapshot> {
        let process = self
            .processes
            .lock()
            .expect("process table lock")
            .get(process_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown background process: {process_id}"))?;
        {
            let mut guard = process.lock().expect("process lock");
            if guard.status != "running" {
                anyhow::bail!("Process {process_id} is not running");
            }
            let Some(stdin) = guard.stdin.clone() else {
                anyhow::bail!("Process {process_id} stdin is closed");
            };
            {
                let mut handle = stdin.lock().expect("process stdin lock");
                handle.write_all(data.as_bytes())?;
                if !eof {
                    handle.flush()?;
                }
            }
            if eof {
                guard.stdin = None;
            }
        }
        Ok(snapshot(&process))
    }

    pub fn cleanup_completed_older_than(&self, retention_ms: i64) -> usize {
        let now = now_ms();
        let retention_ms = retention_ms.max(0);
        let expired = {
            let processes = self.processes.lock().expect("process table lock");
            processes
                .iter()
                .filter_map(|(id, process)| {
                    let proc = process.lock().expect("process lock");
                    let expired =
                        process_is_expired(&proc.status, proc.completed_at, now, retention_ms);
                    expired.then(|| (id.clone(), proc.log_path.clone()))
                })
                .collect::<Vec<_>>()
        };

        if expired.is_empty() {
            return 0;
        }

        let mut removed = 0;
        let mut processes = self.processes.lock().expect("process table lock");
        for (id, log_path) in expired {
            if processes.remove(&id).is_some() {
                let _ = std::fs::remove_file(log_path);
                removed += 1;
            }
        }
        removed
    }
}

fn process_is_expired(
    status: &str,
    completed_at: Option<i64>,
    now: i64,
    retention_ms: i64,
) -> bool {
    let retention_ms = retention_ms.max(0);
    status != "running"
        && completed_at
            .map(|completed_at| now.saturating_sub(completed_at) > retention_ms)
            .unwrap_or(false)
}

pub struct ProcessRead {
    pub process_id: String,
    pub content: String,
    pub offset: usize,
    pub process: BackgroundProcessSnapshot,
}

struct TurnLedgerCapture {
    store: Arc<TurnLedgerStore>,
    session_id: String,
    turn_id: String,
    user_message_preview: String,
    tool_name: String,
    cwd: PathBuf,
    paths: Vec<String>,
}

pub struct ToolRuntime {
    registry: RwLock<ToolRegistry>,
    governance: Arc<GovernanceEngine>,
    workspace_roots: std::sync::RwLock<Vec<PathBuf>>,
    processes: Arc<ProcessSupervisor>,
    mcp: Arc<McpRuntime>,
    memory: Option<Arc<MemoryService>>,
    turn_ledger: Option<Arc<TurnLedgerStore>>,
    agents: Option<Arc<AgentRuntime>>,
}

impl ToolRuntime {
    pub fn with_builtins(governance: Arc<GovernanceEngine>, workspace_roots: Vec<PathBuf>) -> Self {
        let mut registry = ToolRegistry::default();
        registry
            .register_all(builtin_tools())
            .expect("builtin tool definitions must be valid");
        Self {
            registry: RwLock::new(registry),
            governance,
            workspace_roots: std::sync::RwLock::new(workspace_roots),
            processes: Arc::new(ProcessSupervisor::default()),
            mcp: Arc::new(McpRuntime::default()),
            memory: None,
            turn_ledger: None,
            agents: None,
        }
    }

    pub fn with_memory(mut self, memory: Arc<MemoryService>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_turn_ledger(mut self, turn_ledger: Arc<TurnLedgerStore>) -> Self {
        self.turn_ledger = Some(turn_ledger);
        self
    }

    pub fn with_agents(mut self, agents: Arc<AgentRuntime>) -> Self {
        self.agents = Some(agents);
        self
    }

    pub async fn register(&self, tool: ToolDefinition) -> anyhow::Result<()> {
        self.registry.write().await.register(tool)
    }

    pub async fn refresh_mcp_tools(&self, project_dir: &Path) -> anyhow::Result<McpLoadSummary> {
        let (tools, summary) = self.mcp.load_from_project(project_dir);
        let mut registry = self.registry.write().await;
        registry.remove_mcp_tools();
        registry.register_all(tools)?;
        Ok(summary)
    }

    pub async fn catalog(&self) -> Vec<ToolCatalogEntry> {
        self.registry.read().await.catalog(ToolSource::Builtin)
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry.read().await.definitions()
    }

    pub fn list_processes(&self) -> Vec<BackgroundProcessSnapshot> {
        self.processes.list()
    }

    pub fn read_process(
        &self,
        process_id: &str,
        offset: usize,
        max_bytes: usize,
    ) -> Option<ProcessRead> {
        self.processes.read(process_id, offset, max_bytes)
    }

    pub fn stop_process(&self, process_id: &str) -> anyhow::Result<BackgroundProcessSnapshot> {
        self.processes.stop(process_id)
    }

    pub async fn execute(&self, call: ToolCall) -> ToolOutput {
        self.execute_with_approval(call, None).await
    }

    pub async fn execute_approved(&self, call: ToolCall, approval_id: String) -> ToolOutput {
        self.execute_with_approval(call, Some(approval_id)).await
    }

    async fn execute_with_approval(
        &self,
        call: ToolCall,
        approved_approval_id: Option<String>,
    ) -> ToolOutput {
        let tool = {
            let registry = self.registry.read().await;
            registry.get(&call.name).cloned()
        };
        let Some(tool) = tool else {
            let names = self.registry.read().await.names().join(", ");
            return ToolOutput {
                content: format!("Unknown tool: \"{}\". Available tools: {names}", call.name),
                is_error: true,
                data: None,
            };
        };

        let mut governance_args = call.args.clone();
        governance_args
            .entry("_toolCwd".to_string())
            .or_insert_with(|| serde_json::Value::String(call.cwd.to_string_lossy().into_owned()));
        let mut request = GovernanceRequest::tool(tool.name.clone(), governance_args.clone());
        request.action = GovernanceAction::Tool {
            tool_name: tool.name.clone(),
            args: governance_args,
            writes_files: tool.is_write,
            runs_command: tool.is_execute,
            external_network: tool.name == "web_fetch"
                || tool.name == "web_search"
                || tool.name.starts_with("mcp__"),
            touches_secrets: args_touch_secret(&call.args),
        };
        let decision = self.governance.evaluate(request).await;
        if decision.decision == GovernanceDecision::Deny {
            return ToolOutput {
                content: format!("Denied: {}", decision.reason),
                is_error: true,
                data: Some(json!({ "auditId": decision.audit_id })),
            };
        }
        if decision.decision == GovernanceDecision::NeedsApproval && approved_approval_id.is_none()
        {
            return ToolOutput {
                content: format!("Approval required: {}", decision.reason),
                is_error: true,
                data: Some(json!({
                    "approvalId": decision.approval_id,
                    "auditId": decision.audit_id,
                    "risk": decision.risk,
                })),
            };
        }

        let turn_ledger_capture = self.prepare_turn_ledger_capture(&tool.name, &call);
        if let Some(capture) = turn_ledger_capture.as_ref() {
            if let Err(err) = capture.store.snapshot_before_write(
                &capture.session_id,
                &capture.turn_id,
                &capture.cwd,
                &capture.paths,
            ) {
                return error(format!("turn ledger snapshot failed: {err}"));
            }
        }

        let plugin_hook_ctx = self.plugin_tool_hook_context(&call);
        if let Some((ext_config, plugins)) = plugin_hook_ctx.as_ref() {
            let before = run_plugin_tool_hooks(
                ext_config,
                plugins,
                &PluginToolHookInput {
                    tool_name: tool.name.clone(),
                    args: serde_json::to_value(&call.args).unwrap_or(json!({})),
                    cwd: call.cwd.to_string_lossy().into_owned(),
                    phase: "before".to_string(),
                    is_error: None,
                    result_preview: None,
                },
            );
            if before.deny {
                return ToolOutput {
                    content: before
                        .reason
                        .unwrap_or_else(|| "tool.before hook denied this call".to_string()),
                    is_error: true,
                    data: Some(json!({ "hook": "tool.before", "notes": before.notes })),
                };
            }
        }

        let mut output = match tool.name.as_str() {
            "file_read" => self.file_read(&call).await,
            "file_write" => self.file_write(&call).await,
            "file_edit" => self.file_edit(&call).await,
            "apply_patch" => self.apply_patch(&call).await,
            "list_files" => self.list_files(&call).await,
            "search" => self.search(&call).await,
            "glob" => self.glob(&call).await,
            "codebase_index" => self.codebase_index(&call).await,
            "git_status" => self.git_status(&call).await,
            "git_diff" => self.git_diff(&call).await,
            "git_log" => self.git_log(&call).await,
            "git_blame" => self.git_blame(&call).await,
            "git_branch" => self.git_branch(&call).await,
            "git_commit" => self.git_commit(&call).await,
            "git_patch_create" => self.git_patch_create(&call).await,
            "git_patch_apply" => self.git_patch_apply(&call).await,
            "validation_discover" => self.validation_discover(&call).await,
            "validation_diagnose" => self.validation_diagnose(&call).await,
            "validation_replay" => self.validation_replay(&call).await,
            "acceptance_check" => self.acceptance_check(&call).await,
            "self_review" => self.self_review(&call).await,
            "write_memory" => self.write_memory(&call).await,
            "web_search" => self.web_search(&call).await,
            "web_fetch" => self.web_fetch(&call).await,
            "sandbox_modes" => self.sandbox_modes(&call).await,
            "sandbox_create" => self.sandbox_create(&call).await,
            "sandbox_list" => self.sandbox_list(&call).await,
            "sandbox_diff" => self.sandbox_diff(&call).await,
            "sandbox_apply" => self.sandbox_apply(&call).await,
            "sandbox_rollback" => self.sandbox_rollback(&call).await,
            "workspace_checkpoint_create" => self.workspace_checkpoint_create(&call).await,
            "workspace_checkpoint_list" => self.workspace_checkpoint_list(&call).await,
            "workspace_checkpoint_restore" => self.workspace_checkpoint_restore(&call).await,
            "submit_task" => self.submit_task(&call).await,
            "todo_write" => self.todo_write(&call).await,
            "update_goal" => self.update_goal(&call).await,
            "enter_plan_mode" => self.enter_plan_mode(&call).await,
            "exit_plan_mode" => self.exit_plan_mode(&call).await,
            "activate_skill" => self.activate_skill(&call).await,
            "get_task_status" => self.get_task_status(&call).await,
            "list_tasks" => self.list_tasks(&call).await,
            "cancel_task" => self.cancel_task(&call).await,
            "delivery_create_local_pr" => self.delivery_create_local_pr(&call).await,
            "git_platform_status" => self.git_platform_status(&call).await,
            "git_issue_task_draft" => self.git_issue_task_draft(&call).await,
            "search_tools" => self.search_tools(&call).await,
            "load_tools" => self.load_tools(&call).await,
            "search_memory" => self.search_memory(&call).await,
            "load_memories" => self.load_memories(&call).await,
            "ask_user" => ToolOutput {
                content: "ask_user unavailable: no interactive user channel in this runtime. Ask in your final reply instead.".to_string(),
                is_error: true,
                data: None,
            },
            "exec" => self.exec(&call).await,
            "start_process" => self.start_process(&call).await,
            "list_processes" => self.list_processes_tool(&call).await,
            "read_process_output" => self.read_process_output(&call).await,
            "write_process_stdin" => self.write_process_stdin(&call).await,
            "stop_process" => self.stop_process_tool(&call).await,
            name if name.starts_with("mcp__") => self.mcp_call(&call).await,
            _ => ToolOutput {
                content: format!(
                    "Tool is registered but has no runtime handler: {}",
                    tool.name
                ),
                is_error: true,
                data: None,
            },
        };

        if !output.is_error {
            if let Some(capture) = turn_ledger_capture {
                let affected_files = affected_files_for_ledger(&output, &capture.paths);
                if !affected_files.is_empty() {
                    if let Err(err) = capture.store.record_write_result(RecordWriteInput {
                        session_id: &capture.session_id,
                        turn_id: &capture.turn_id,
                        user_message_preview: &capture.user_message_preview,
                        tool_name: &capture.tool_name,
                        cwd: &capture.cwd,
                        affected_files: &affected_files,
                    }) {
                        append_turn_ledger_warning(&mut output, err);
                    }
                }
            }
        }

        if let Some((ext_config, plugins)) = plugin_hook_ctx.as_ref() {
            let preview = truncate_text(&output.content, 2_000);
            let after = run_plugin_tool_hooks(
                ext_config,
                plugins,
                &PluginToolHookInput {
                    tool_name: tool.name.clone(),
                    args: serde_json::to_value(&call.args).unwrap_or(json!({})),
                    cwd: call.cwd.to_string_lossy().into_owned(),
                    phase: "after".to_string(),
                    is_error: Some(output.is_error),
                    result_preview: Some(preview),
                },
            );
            if !after.notes.is_empty() {
                let note_block = after.notes.join("\n");
                if output.content.trim().is_empty() {
                    output.content = note_block;
                } else {
                    output.content.push_str("\n\n[tool.after]\n");
                    output.content.push_str(&note_block);
                }
            }
        }

        output_bound::apply_model_facing_bound(&tool.name, output)
    }

    fn plugin_tool_hook_context(
        &self,
        call: &ToolCall,
    ) -> Option<(ExtensionLoadConfig, Vec<xuanyu_extensions::PluginManifest>)> {
        let project_dir = self
            .workspace_roots()
            .into_iter()
            .next()
            .unwrap_or_else(|| call.cwd.clone());
        let app_config = load_nearby_app_config(&project_dir);
        let ext_config = ExtensionLoadConfig::from_app_config(project_dir, &app_config);
        if !ext_config.plugins_enabled {
            return None;
        }
        let plugins = load_plugins(&ext_config);
        if plugins.is_empty() {
            return None;
        }
        Some((ext_config, plugins))
    }

    pub fn workspace_roots(&self) -> Vec<PathBuf> {
        self.workspace_roots
            .read()
            .map(|roots| roots.clone())
            .unwrap_or_default()
    }

    fn prepare_turn_ledger_capture(
        &self,
        tool_name: &str,
        call: &ToolCall,
    ) -> Option<TurnLedgerCapture> {
        let store = self.turn_ledger.clone()?;
        if !xuanyu_governance::is_write_tool_name(tool_name) {
            return None;
        }
        let session_id = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        )?
        .trim();
        let turn_id =
            string_arg_any(&call.args, &["turnId", "turn_id", "_turnId", "_turn_id"])?.trim();
        if session_id.is_empty() || turn_id.is_empty() {
            return None;
        }
        let args = serde_json::to_value(&call.args).ok()?;
        let paths = xuanyu_governance::extract_write_paths(tool_name, &args);
        if paths.is_empty() {
            return None;
        }
        let user_message_preview = string_arg_any(
            &call.args,
            &[
                "userMessagePreview",
                "user_message_preview",
                "turnUserMessage",
                "turn_user_message",
                "_userMessagePreview",
                "_user_message_preview",
            ],
        )
        .unwrap_or_default()
        .chars()
        .take(240)
        .collect::<String>();
        Some(TurnLedgerCapture {
            store,
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            user_message_preview,
            tool_name: tool_name.to_string(),
            cwd: call.cwd.clone(),
            paths,
        })
    }

    async fn file_read(&self, call: &ToolCall) -> ToolOutput {
        let Some(path) = string_arg(&call.args, "path") else {
            return error("file_read requires a string path");
        };
        let start_line = usize_arg_any(&call.args, &["start_line", "startLine"]);
        let end_line = usize_arg_any(&call.args, &["end_line", "endLine"]);
        let path = match self.resolve_existing_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        match tokio::fs::read(&path).await {
            Ok(bytes) => match detect_media_kind(&path, &bytes) {
                MediaKind::Image => {
                    const MAX_IMAGE_READ_BYTES: usize = 5 * 1024 * 1024;
                    if bytes.len() > MAX_IMAGE_READ_BYTES {
                        return error(format!(
                            "image too large ({:.1} MB > {} MB). Use a smaller image.",
                            bytes.len() as f64 / 1_048_576.0,
                            MAX_IMAGE_READ_BYTES / 1_048_576
                        ));
                    }
                    let Some(mime) = resolve_image_mime(&path, &bytes) else {
                        return error(format!(
                            "unrecognized image format for {}. Supported: png, jpeg, gif, webp.",
                            path.display()
                        ));
                    };
                    let b64 = base64_encode(&bytes);
                    let line_note = if start_line.is_some() || end_line.is_some() {
                        "\nNote: start_line/end_line are ignored for images."
                    } else {
                        ""
                    };
                    ToolOutput {
                        content: format!(
                            "Image: {}\nMIME: {mime}\nSize: {} bytes\nPixels attached for vision — describe or reason about the image contents.{line_note}",
                            path.display(),
                            bytes.len()
                        ),
                        is_error: false,
                        data: Some(json!({
                            "path": path,
                            "bytes": bytes.len(),
                            "mediaKind": "image",
                            "mimeType": mime,
                            "attachment": {
                                "type": "image",
                                "mimeType": mime,
                                "encoding": "base64",
                                "data": b64
                            }
                        })),
                    }
                }
                MediaKind::Pdf => {
                    const MAX_PDF_READ_BYTES: usize = 8 * 1024 * 1024;
                    if bytes.len() > MAX_PDF_READ_BYTES {
                        return error(format!(
                            "PDF too large ({:.1} MB > {} MB).",
                            bytes.len() as f64 / 1_048_576.0,
                            MAX_PDF_READ_BYTES / 1_048_576
                        ));
                    }
                    let extracted = extract_pdf_text_from_bytes(&bytes);
                    let lines: Vec<&str> = extracted.lines().collect();
                    let start = start_line.unwrap_or(1).max(1);
                    let end = end_line.unwrap_or(lines.len()).min(lines.len());
                    if lines.is_empty() {
                        return ToolOutput {
                            content: format!(
                                "PDF: {}\n(no extractable text)",
                                path.display()
                            ),
                            is_error: false,
                            data: Some(json!({
                                "path": path,
                                "bytes": bytes.len(),
                                "mediaKind": "pdf",
                                "lineCount": 0
                            })),
                        };
                    }
                    if start > lines.len() {
                        return error(format!(
                            "start_line {start} is out of range (extracted text has {} lines)",
                            lines.len()
                        ));
                    }
                    let slice = lines[(start - 1)..end].join("\n");
                    ToolOutput {
                        content: format!(
                            "PDF text ({path}):\n{slice}",
                            path = path.display()
                        ),
                        is_error: false,
                        data: Some(json!({
                            "path": path,
                            "bytes": bytes.len(),
                            "mediaKind": "pdf",
                            "lineCount": lines.len(),
                            "startLine": start,
                            "endLine": end
                        })),
                    }
                }
                MediaKind::Text | MediaKind::Binary => {
                    // Read windowing: default 2000 lines, 2000 chars/line, ~25k-token
                    // output page with PARTIAL continue (file_read owns its caps).
                    const FILE_READ_MAX_OUTPUT_TOKENS: usize = 25_000;
                    const FILE_READ_MAX_OUTPUT_CHARS: usize = FILE_READ_MAX_OUTPUT_TOKENS * 4;
                    const FILE_READ_DEFAULT_LINE_LIMIT: usize = 2_000;
                    const FILE_READ_MAX_LINE_CHARS: usize = 2_000;
                    const FILE_READ_DEFAULT_MAX_BYTES: usize = 256 * 1024;

                    let max_bytes = usize_arg(&call.args, "maxBytes")
                        .unwrap_or(FILE_READ_DEFAULT_MAX_BYTES)
                        .max(1);
                    let offset = start_line
                        .or_else(|| usize_arg_any(&call.args, &["offset"]))
                        .unwrap_or(1)
                        .max(1);
                    let line_limit = if let Some(end) = end_line {
                        end.saturating_add(1).saturating_sub(offset).max(1)
                    } else if let Some(limit) =
                        usize_arg_any(&call.args, &["limit", "max_lines", "maxLines"])
                    {
                        limit.clamp(1, FILE_READ_DEFAULT_LINE_LIMIT)
                    } else {
                        FILE_READ_DEFAULT_LINE_LIMIT
                    };

                    let clipped_bytes = bytes.len() > max_bytes;
                    let slice = &bytes[..bytes.len().min(max_bytes)];
                    let text = String::from_utf8_lossy(slice);
                    let lines: Vec<&str> = text.lines().collect();
                    let total_lines = lines.len();
                    if total_lines > 0 && offset > total_lines {
                        return error(format!(
                            "start_line/offset {offset} is out of range (file has {total_lines} lines)"
                        ));
                    }
                    let end = total_lines
                        .min(offset.saturating_add(line_limit).saturating_sub(1))
                        .max(if total_lines == 0 { 0 } else { offset });

                    let mut page = String::new();
                    let mut used_chars = 0usize;
                    let mut last_line = offset.saturating_sub(1);
                    let mut hit_char_cap = false;
                    if total_lines > 0 {
                        for (idx, raw_line) in lines[(offset - 1)..end].iter().enumerate() {
                            let line_no = offset + idx;
                            let mut line = raw_line.to_string();
                            if line.chars().count() > FILE_READ_MAX_LINE_CHARS {
                                line = line.chars().take(FILE_READ_MAX_LINE_CHARS).collect();
                                line.push_str("... (line truncated)");
                            }
                            let numbered = format!("{line_no:>6}\t{line}");
                            let add = numbered.chars().count() + 1;
                            if used_chars > 0 && used_chars + add > FILE_READ_MAX_OUTPUT_CHARS {
                                hit_char_cap = true;
                                break;
                            }
                            if !page.is_empty() {
                                page.push('\n');
                            }
                            page.push_str(&numbered);
                            used_chars += add;
                            last_line = line_no;
                        }
                    }

                    let truncated = hit_char_cap
                        || clipped_bytes
                        || (total_lines > 0 && last_line < total_lines);
                    if truncated {
                        let next = last_line.saturating_add(1);
                        let mut notice = format!(
                            "\n\n[PARTIAL view] Showing lines {offset}-{last_line} of {total_lines} (~{used_chars} chars, ~{FILE_READ_MAX_OUTPUT_TOKENS} token page)."
                        );
                        if next <= total_lines {
                            notice.push_str(&format!(
                                " Continue with file_read path={path:?} start_line={next} (optional end_line / limit).",
                                path = display_relative(&path, &call.cwd)
                            ));
                        }
                        if clipped_bytes {
                            notice.push_str(&format!(
                                " File exceeds maxBytes={max_bytes} (file={} bytes); raise maxBytes or continue by line range.",
                                bytes.len()
                            ));
                        }
                        page.push_str(&notice);
                    }

                    ToolOutput {
                        content: page,
                        is_error: false,
                        data: Some(json!({
                            "path": path,
                            "bytes": bytes.len(),
                            "truncated": truncated,
                            "partial": truncated,
                            "mediaKind": "text",
                            "lineCount": total_lines,
                            "startLine": offset,
                            "endLine": last_line.max(offset.saturating_sub(1)),
                            "maxOutputTokens": FILE_READ_MAX_OUTPUT_TOKENS
                        })),
                    }
                }
            },
            Err(err) => error(format!("failed to read file: {err}")),
        }
    }

    async fn file_write(&self, call: &ToolCall) -> ToolOutput {
        let Some(path) = string_arg(&call.args, "path") else {
            return error("file_write requires a string path");
        };
        let Some(content) = string_arg(&call.args, "content") else {
            return error("file_write requires string content");
        };
        let path = match self.resolve_write_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        if let Err(err) = self.ensure_write_scope(call, &path).await {
            return error(err);
        }
        let before = async_fs::read_to_string(&path).await.unwrap_or_default();
        if let Some(parent) = path.parent() {
            if let Err(err) = async_fs::create_dir_all(parent).await {
                return error(format!("failed to create parent directory: {err}"));
            }
        }
        if let Err(err) = atomic_write_file(&path, content).await {
            return error(format!("failed to write file: {err}"));
        }
        let diff = create_unified_diff(&path, &before, content, &call.cwd);
        let changes = count_line_changes(&before, content);
        ToolOutput {
            content: format!(
                "Successfully wrote {}.\nLines changed: +{} / -{}\n\n{}",
                display_relative(&path, &call.cwd),
                changes.0,
                changes.1,
                diff
            ),
            is_error: false,
            data: Some(json!({
                "path": path,
                "diff": diff,
                "affectedFiles": [display_relative(&path, &call.cwd)],
                "lineChanges": { "added": changes.0, "removed": changes.1 }
            })),
        }
    }

    async fn file_edit(&self, call: &ToolCall) -> ToolOutput {
        let Some(path) = string_arg(&call.args, "path") else {
            return error("file_edit requires a string path");
        };
        let Some(old_text) = string_arg_any(&call.args, &["old_text", "oldText"]) else {
            return error("file_edit requires old_text");
        };
        let Some(new_text) = string_arg_any(&call.args, &["new_text", "newText"]) else {
            return error("file_edit requires new_text");
        };
        let path = match self.resolve_existing_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        if let Err(err) = self.ensure_write_scope(call, &path).await {
            return error(err);
        }
        let before = match async_fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(err) => return error(format!("failed to read file: {err}")),
        };
        let occurrences = before.matches(old_text).count();
        if occurrences == 0 {
            return error(format!(
                "Could not find the exact text in {}",
                display_relative(&path, &call.cwd)
            ));
        }
        if occurrences > 1 {
            return error(format!(
                "Found {occurrences} occurrences in {}; provide a more specific old_text",
                display_relative(&path, &call.cwd)
            ));
        }
        let after = before.replacen(old_text, new_text, 1);
        if let Err(err) = atomic_write_file(&path, &after).await {
            return error(format!("failed to edit file: {err}"));
        }
        let diff = create_unified_diff(&path, &before, &after, &call.cwd);
        let changes = count_line_changes(&before, &after);
        ToolOutput {
            content: format!(
                "Successfully edited {}.\nLines changed: +{} / -{}\n\n{}",
                display_relative(&path, &call.cwd),
                changes.0,
                changes.1,
                diff
            ),
            is_error: false,
            data: Some(json!({
                "path": path,
                "diff": diff,
                "affectedFiles": [display_relative(&path, &call.cwd)],
                "lineChanges": { "added": changes.0, "removed": changes.1 }
            })),
        }
    }

    async fn apply_patch(&self, call: &ToolCall) -> ToolOutput {
        let input = match serde_json::from_value::<ApplyPatchInput>(
            json!({ "patches": call.args.get("patches").cloned().unwrap_or_default() }),
        ) {
            Ok(input) if !input.patches.is_empty() => input,
            Ok(_) => return error("apply_patch requires at least one patch"),
            Err(err) => return error(format!("invalid apply_patch input: {err}")),
        };
        let mut diffs = Vec::new();
        let mut affected = Vec::new();
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut resolved_patches = Vec::new();

        for patch in input.patches {
            let path = match patch.action {
                PatchAction::Add => match self.resolve_write_path(&call.cwd, &patch.path).await {
                    Ok(path) => path,
                    Err(err) => return error(err),
                },
                PatchAction::Update | PatchAction::Delete => {
                    match self.resolve_existing_path(&call.cwd, &patch.path).await {
                        Ok(path) => path,
                        Err(err) => return error(err),
                    }
                }
            };
            if let Err(err) = self.ensure_write_scope(call, &path).await {
                return error(err);
            }
            resolved_patches.push((patch, path));
        }

        let mut planned_patches = Vec::new();
        for (patch, path) in resolved_patches {
            let before = async_fs::read_to_string(&path).await.unwrap_or_default();
            let after = match patch.action {
                PatchAction::Add => {
                    if path.exists() {
                        return error(format!(
                            "Cannot add {}: file already exists",
                            display_relative(&path, &call.cwd)
                        ));
                    }
                    patch.content.unwrap_or_default()
                }
                PatchAction::Delete => String::new(),
                PatchAction::Update => {
                    let Some(old_text) = patch.old_text.as_deref() else {
                        return error(format!(
                            "Patch update for {} requires old_text",
                            display_relative(&path, &call.cwd)
                        ));
                    };
                    let Some(new_text) = patch.new_text.as_deref() else {
                        return error(format!(
                            "Patch update for {} requires new_text",
                            display_relative(&path, &call.cwd)
                        ));
                    };
                    let occurrences = before.matches(old_text).count();
                    if occurrences != 1 {
                        return error(format!(
                            "Patch update for {} expected exactly one old_text match, found {occurrences}",
                            display_relative(&path, &call.cwd)
                        ));
                    }
                    before.replacen(old_text, new_text, 1)
                }
            };

            planned_patches.push((path, patch.action, before, after));
        }

        for (path, action, before, after) in planned_patches {
            match action {
                PatchAction::Delete => {
                    if let Err(err) = async_fs::remove_file(&path).await {
                        return error(format!("failed to delete file: {err}"));
                    }
                }
                PatchAction::Add | PatchAction::Update => {
                    if let Some(parent) = path.parent() {
                        if let Err(err) = async_fs::create_dir_all(parent).await {
                            return error(format!("failed to create parent directory: {err}"));
                        }
                    }
                    if let Err(err) = atomic_write_file(&path, &after).await {
                        return error(format!("failed to write patch target: {err}"));
                    }
                }
            }

            let diff = create_unified_diff(&path, &before, &after, &call.cwd);
            let changes = count_line_changes(&before, &after);
            added += changes.0;
            removed += changes.1;
            affected.push(display_relative(&path, &call.cwd));
            diffs.push(diff);
        }

        let diff = diffs.join("\n");
        ToolOutput {
            content: format!(
                "Applied {} patch(es).\nAffected files: {}\nLines changed: +{} / -{}\n\n{}",
                affected.len(),
                affected.join(", "),
                added,
                removed,
                diff
            ),
            is_error: false,
            data: Some(json!({
                "diff": diff,
                "affectedFiles": affected,
                "lineChanges": { "added": added, "removed": removed }
            })),
        }
    }

    async fn list_files(&self, call: &ToolCall) -> ToolOutput {
        let path = string_arg(&call.args, "path").unwrap_or(".");
        let limit = usize_arg(&call.args, "limit").unwrap_or(200);
        let root = match self.resolve_existing_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        let mut files = Vec::new();
        collect_files(&root, &root, limit, &mut files);
        let rendered = files
            .iter()
            .map(|path| normalize_path(path))
            .collect::<Vec<_>>();
        let content = if rendered.is_empty() {
            format!(
                "(empty directory) {}",
                normalize_path(&root)
            )
        } else {
            rendered.join("\n")
        };
        ToolOutput {
            content,
            is_error: false,
            data: Some(json!({ "root": path_to_string(&root), "count": files.len() })),
        }
    }

    async fn search(&self, call: &ToolCall) -> ToolOutput {
        let pattern_arg = string_arg(&call.args, "pattern");
        let query_arg = string_arg(&call.args, "query");
        let Some(raw_pattern) = pattern_arg.or(query_arg) else {
            return error("search requires pattern or query");
        };
        let path = string_arg(&call.args, "path").unwrap_or(".");
        let limit = usize_arg_any(&call.args, &["max_results", "maxResults", "limit"])
            .unwrap_or(100)
            .clamp(1, 500);
        let include = string_arg(&call.args, "include");
        let case_insensitive = bool_arg(&call.args, "case_insensitive")
            .or_else(|| bool_arg(&call.args, "caseInsensitive"))
            .unwrap_or(false);
        let root = match self.resolve_existing_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        let regex_pattern = if pattern_arg.is_some() {
            raw_pattern.to_string()
        } else {
            regex::escape(raw_pattern)
        };
        let matcher = match regex::RegexBuilder::new(&regex_pattern)
            .case_insensitive(case_insensitive)
            .build()
        {
            Ok(matcher) => matcher,
            Err(err) => return error(format!("invalid search pattern: {err}")),
        };
        let (base, files) = collect_search_files(&root, 10_000);
        let mut matches = Vec::new();
        for relative in files {
            if matches.len() >= limit {
                break;
            }
            let normalized = normalize_path(&relative);
            if let Some(include) = include {
                if !wildcard_match(include, &normalized) {
                    continue;
                }
            }
            let full = base.join(&relative);
            let Ok(text) = std::fs::read_to_string(&full) else {
                continue;
            };
            for (idx, line) in text.lines().enumerate() {
                if matcher.is_match(line) {
                    matches.push(format!("{normalized}:{}:{}", idx + 1, line.trim()));
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
        }
        let content = if matches.is_empty() {
            format!("No matches found for pattern: {raw_pattern}")
        } else {
            let truncated = if matches.len() >= limit {
                format!("\n\n(Showing first {limit} results, there may be more)")
            } else {
                String::new()
            };
            format!(
                "Found {} matches:\n{}{}",
                matches.len(),
                matches.join("\n"),
                truncated
            )
        };
        ToolOutput {
            content,
            is_error: false,
            data: Some(json!({
                "root": root,
                "count": matches.len(),
                "pattern": raw_pattern,
                "caseInsensitive": case_insensitive,
                "include": include
            })),
        }
    }

    async fn glob(&self, call: &ToolCall) -> ToolOutput {
        let Some(pattern) = string_arg(&call.args, "pattern") else {
            return error("glob requires a string pattern");
        };
        let path = string_arg(&call.args, "path").unwrap_or(".");
        let limit = usize_arg(&call.args, "limit").unwrap_or(200);
        let root = match self.resolve_existing_path(&call.cwd, path).await {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        let mut files = Vec::new();
        collect_files(&root, &root, 2_000, &mut files);
        let matches: Vec<String> = files
            .into_iter()
            .filter_map(|path| {
                let text = normalize_path(&path);
                wildcard_match(pattern, &text).then_some(text)
            })
            .take(limit)
            .collect();
        ToolOutput {
            content: matches.join("\n"),
            is_error: false,
            data: Some(json!({ "root": root, "count": matches.len() })),
        }
    }

    async fn codebase_index(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.workspace_cwd_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let include = string_vec_arg(&call.args, "include");
        for pattern in &include {
            if contains_traversal(pattern) {
                return error(format!(
                    "Include pattern rejected: path traversal is not allowed: {pattern}"
                ));
            }
        }
        let options = CodebaseIndexOptions {
            max_files: usize_arg_any(&call.args, &["max_files", "maxFiles"]).unwrap_or(2000),
            include,
            refresh: bool_arg(&call.args, "refresh").unwrap_or(false),
        };
        let summary_limit =
            usize_arg_any(&call.args, &["summary_limit", "summaryLimit"]).unwrap_or(40);
        match build_codebase_index(&root, options) {
            Ok(index) => {
                let index_path = codebase_index_path(&root);
                ToolOutput {
                    content: format_codebase_index(&index, summary_limit),
                    is_error: false,
                    data: Some(json!({
                        "indexPath": path_to_string(&index_path),
                        "generatedAt": index.generated_at,
                        "fileCount": index.file_count,
                        "symbolCount": index.symbols.len(),
                        "referenceCount": index.references.len(),
                        "directoryCount": index.directories.len(),
                        "changeCount": index.changes.len(),
                        "incremental": index.incremental
                    })),
                }
            }
            Err(err) => error(format!("codebase_index failed: {err}")),
        }
    }

    async fn git_status(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let status = match git_status_snapshot(&root) {
            Ok(status) => status,
            Err(err) => return error(err),
        };
        ToolOutput {
            content: format_git_status_content(&status),
            is_error: false,
            data: Some(json!({ "status": status })),
        }
    }

    async fn git_diff(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let mut args = vec!["diff".to_string(), "--binary".to_string()];
        if bool_arg(&call.args, "stat").unwrap_or(false) {
            args.push("--stat".to_string());
        }
        if bool_arg(&call.args, "staged").unwrap_or(false) {
            args.push("--staged".to_string());
        }
        if let Some(context) = usize_arg(&call.args, "context") {
            args.push(format!("--unified={}", context.min(100)));
        }
        if let Some(from) = string_arg(&call.args, "from") {
            match safe_git_ref_arg(from) {
                Ok(from) => args.push(from),
                Err(err) => return error(err),
            }
        }
        if let Some(to) = string_arg(&call.args, "to") {
            match safe_git_ref_arg(to) {
                Ok(to) => args.push(to),
                Err(err) => return error(err),
            }
        }
        let paths = match safe_git_pathspecs(string_vec_arg(&call.args, "paths")) {
            Ok(paths) => paths,
            Err(err) => return error(err),
        };
        if !paths.is_empty() {
            args.push("--".to_string());
            args.extend(paths);
        }
        match git_output_owned_raw(&root, &args) {
            Ok(diff) => ToolOutput {
                content: if diff.trim().is_empty() {
                    "(no diff)".to_string()
                } else {
                    truncate_text(&diff, 512 * 1024)
                },
                is_error: false,
                data: Some(json!({ "diff": diff })),
            },
            Err(err) => error(format!("git diff failed: {err}")),
        }
    }

    async fn git_log(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let limit = usize_arg(&call.args, "limit").unwrap_or(20).clamp(1, 100);
        let mut args = vec![
            "log".to_string(),
            format!("-n{limit}"),
            "--pretty=format:%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%s".to_string(),
        ];
        if let Some(reference) = string_arg(&call.args, "ref") {
            match safe_git_ref_arg(reference) {
                Ok(reference) => args.push(reference),
                Err(err) => return error(err),
            }
        }
        let paths = match safe_git_pathspecs(string_vec_arg(&call.args, "paths")) {
            Ok(paths) => paths,
            Err(err) => return error(err),
        };
        if !paths.is_empty() {
            args.push("--".to_string());
            args.extend(paths);
        }
        match git_output_owned(&root, &args) {
            Ok(raw) => {
                let entries = parse_git_log(&raw);
                ToolOutput {
                    content: if entries.is_empty() {
                        "(no commits)".to_string()
                    } else {
                        entries
                            .iter()
                            .map(|entry| {
                                format!(
                                    "{} {} ({})",
                                    entry["shortHash"].as_str().unwrap_or_default(),
                                    entry["subject"].as_str().unwrap_or_default(),
                                    entry["authorName"].as_str().unwrap_or_default()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    },
                    is_error: false,
                    data: Some(json!({ "entries": entries })),
                }
            }
            Err(err) => error(format!("git log failed: {err}")),
        }
    }

    async fn git_blame(&self, call: &ToolCall) -> ToolOutput {
        let Some(file) = string_arg(&call.args, "file") else {
            return error("git_blame requires a file");
        };
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let file = match safe_git_pathspecs(vec![file.to_string()]) {
            Ok(mut paths) => paths.pop().unwrap_or_default(),
            Err(err) => return error(err),
        };
        let mut args = vec!["blame".to_string(), "--line-porcelain".to_string()];
        if usize_arg(&call.args, "startLine").is_some()
            || usize_arg(&call.args, "endLine").is_some()
        {
            let start = usize_arg(&call.args, "startLine").unwrap_or(1).max(1);
            let end = usize_arg(&call.args, "endLine").unwrap_or(start).max(start);
            args.push("-L".to_string());
            args.push(format!("{start},{end}"));
        }
        args.push("--".to_string());
        args.push(file);
        match git_output_owned(&root, &args) {
            Ok(raw) => {
                let lines = parse_git_blame(&raw);
                ToolOutput {
                    content: if lines.is_empty() {
                        "(no blame lines)".to_string()
                    } else {
                        lines
                            .iter()
                            .map(|line| {
                                format!(
                                    "{} {} {}: {}",
                                    line["finalLine"].as_u64().unwrap_or_default(),
                                    line["commit"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .chars()
                                        .take(8)
                                        .collect::<String>(),
                                    line["author"].as_str().unwrap_or_default(),
                                    line["content"].as_str().unwrap_or_default()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    },
                    is_error: false,
                    data: Some(json!({ "lines": lines })),
                }
            }
            Err(err) => error(format!("git blame failed: {err}")),
        }
    }

    async fn git_branch(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let action = string_arg(&call.args, "action").unwrap_or("list");
        match action {
            "list" => match list_git_branches(&root) {
                Ok(branches) => ToolOutput {
                    content: if branches.is_empty() {
                        "(no branches)".to_string()
                    } else {
                        branches
                            .iter()
                            .map(format_git_branch)
                            .collect::<Vec<_>>()
                            .join("\n")
                    },
                    is_error: false,
                    data: Some(json!({ "branches": branches })),
                },
                Err(err) => error(format!("git branch failed: {err}")),
            },
            "create" => {
                let Some(name) = string_arg(&call.args, "name") else {
                    return error("Branch name is required for create.");
                };
                let branch = match validate_branch_name(name) {
                    Ok(branch) => branch,
                    Err(err) => return error(err),
                };
                let base_ref = match string_arg(&call.args, "baseRef") {
                    Some(value) => match safe_git_ref_arg(value) {
                        Ok(value) => Some(value),
                        Err(err) => return error(err),
                    },
                    None => None,
                };
                let mut args = vec!["branch".to_string(), branch.clone()];
                if let Some(base_ref) = &base_ref {
                    args.push(base_ref.clone());
                }
                match git_output_owned(&root, &args) {
                    Ok(_) => match git_status_snapshot(&root) {
                        Ok(status) => ToolOutput {
                            content: format!("Created branch {branch}."),
                            is_error: false,
                            data: Some(json!({
                                "name": branch,
                                "baseRef": base_ref,
                                "status": status
                            })),
                        },
                        Err(err) => error(err),
                    },
                    Err(err) => error(format!("git branch failed: {err}")),
                }
            }
            _ => error("git_branch action must be list or create"),
        }
    }

    async fn git_commit(&self, call: &ToolCall) -> ToolOutput {
        let Some(message) = string_arg(&call.args, "message").map(str::trim) else {
            return error("git_commit requires a message");
        };
        if message.is_empty() {
            return error("Commit message is required");
        }
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };

        if bool_arg(&call.args, "all").unwrap_or(false) {
            if let Err(err) = git_output(&root, &["add", "-A"]) {
                return error(format!("git add failed: {err}"));
            }
        } else {
            let files = match safe_git_pathspecs(string_vec_arg(&call.args, "files")) {
                Ok(files) => files,
                Err(err) => return error(err),
            };
            if !files.is_empty() {
                let mut args = vec!["add".to_string(), "--".to_string()];
                args.extend(files);
                if let Err(err) = git_output_owned(&root, &args) {
                    return error(format!("git add failed: {err}"));
                }
            }
        }

        let mut args = vec!["commit".to_string(), "-m".to_string(), message.to_string()];
        if bool_arg(&call.args, "allowEmpty").unwrap_or(false) {
            args.push("--allow-empty".to_string());
        }
        match git_output_owned(&root, &args) {
            Ok(_) => {
                let commit = match git_output(&root, &["rev-parse", "HEAD"]) {
                    Ok(commit) => commit,
                    Err(err) => return error(format!("git rev-parse failed: {err}")),
                };
                let status = match git_status_snapshot(&root) {
                    Ok(status) => status,
                    Err(err) => return error(err),
                };
                ToolOutput {
                    content: format!(
                        "Created commit {}.",
                        commit.chars().take(12).collect::<String>()
                    ),
                    is_error: false,
                    data: Some(json!({ "commit": commit, "status": status })),
                }
            }
            Err(err) => error(format!("git commit failed: {err}")),
        }
    }

    async fn git_patch_create(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let paths = match safe_git_pathspecs(string_vec_arg(&call.args, "paths")) {
            Ok(paths) => paths,
            Err(err) => return error(err),
        };
        let diff = if bool_arg(&call.args, "includeUntracked").unwrap_or(false) {
            match get_working_tree_diff(&root, &paths) {
                Ok(diff) => diff,
                Err(err) => return error(format!("git patch create failed: {err}")),
            }
        } else {
            let mut args = vec!["diff".to_string(), "--binary".to_string()];
            if bool_arg(&call.args, "stat").unwrap_or(false) {
                args.push("--stat".to_string());
            }
            if bool_arg(&call.args, "staged").unwrap_or(false) {
                args.push("--staged".to_string());
            }
            if let Some(context) = usize_arg(&call.args, "context") {
                args.push(format!("--unified={}", context.min(100)));
            }
            if let Some(from) = string_arg(&call.args, "from") {
                match safe_git_ref_arg(from) {
                    Ok(from) => args.push(from),
                    Err(err) => return error(err),
                }
            }
            if let Some(to) = string_arg(&call.args, "to") {
                match safe_git_ref_arg(to) {
                    Ok(to) => args.push(to),
                    Err(err) => return error(err),
                }
            }
            if !paths.is_empty() {
                args.push("--".to_string());
                args.extend(paths);
            }
            match git_output_owned_raw(&root, &args) {
                Ok(diff) => diff,
                Err(err) => return error(format!("git patch create failed: {err}")),
            }
        };
        match write_git_patch_file(&root, &diff, "patch") {
            Ok((_patch_path, patch)) => ToolOutput {
                content: [
                    format!(
                        "Created patch {}.",
                        patch["id"].as_str().unwrap_or_default()
                    ),
                    format!("Path: {}", patch["patchPath"].as_str().unwrap_or_default()),
                    if diff.trim().is_empty() {
                        "Patch is empty.".to_string()
                    } else {
                        String::new()
                    },
                ]
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
                is_error: false,
                data: Some(json!({ "patch": patch })),
            },
            Err(err) => error(format!("git patch create failed: {err}")),
        }
    }

    async fn git_patch_apply(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.git_root_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let patch_path = match resolve_git_patch_input(&root, &call.args) {
            Ok(path) => path,
            Err(err) => return error(err),
        };
        let reverse = bool_arg(&call.args, "reverse").unwrap_or(false);
        let check_only = bool_arg(&call.args, "checkOnly").unwrap_or(false);
        let mut check_args = vec!["apply".to_string()];
        if reverse {
            check_args.push("--reverse".to_string());
        }
        check_args.push("--check".to_string());
        check_args.push(path_to_string(&patch_path));
        if let Err(err) = git_output_owned(&root, &check_args) {
            return error(format!("git patch apply failed: {err}"));
        }

        if !check_only {
            let mut apply_args = vec!["apply".to_string()];
            if reverse {
                apply_args.push("--reverse".to_string());
            }
            apply_args.push(path_to_string(&patch_path));
            if let Err(err) = git_output_owned(&root, &apply_args) {
                return error(format!("git patch apply failed: {err}"));
            }
        }

        let status = match git_status_snapshot(&root) {
            Ok(status) => status,
            Err(err) => return error(err),
        };
        ToolOutput {
            content: if check_only {
                format!("Patch check passed for {}.", path_to_string(&patch_path))
            } else if reverse {
                format!("Reversed patch {}.", path_to_string(&patch_path))
            } else {
                format!("Applied patch {}.", path_to_string(&patch_path))
            },
            is_error: false,
            data: Some(json!({
                "checked": true,
                "applied": !check_only,
                "reverse": reverse,
                "patchPath": path_to_string(&patch_path),
                "status": status
            })),
        }
    }

    async fn validation_discover(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.workspace_cwd_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let commands = discover_validation_commands(&root);
        ToolOutput {
            content: if commands.is_empty() {
                "No validation commands discovered.".to_string()
            } else {
                commands
                    .iter()
                    .map(|command| {
                        format!(
                            "{}: {} ({}:{})",
                            command["type"].as_str().unwrap_or("unknown"),
                            command["command"].as_str().unwrap_or_default(),
                            command["source"].as_str().unwrap_or_default(),
                            command["sourcePath"].as_str().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            is_error: false,
            data: Some(json!({ "commands": commands })),
        }
    }

    async fn validation_diagnose(&self, call: &ToolCall) -> ToolOutput {
        let Some(command) = string_arg(&call.args, "command") else {
            return error("validation_diagnose requires command");
        };
        let Some(output) = string_arg(&call.args, "output") else {
            return error("validation_diagnose requires output");
        };
        let diagnosis =
            diagnose_validation_failure(command, output, string_arg(&call.args, "type"));
        let memory_result = match &self.memory {
            Some(memory) => ingest_validation_failure_memory(memory, call, command, &diagnosis)
                .await
                .unwrap_or_else(|err| json!({ "status": "error", "error": err })),
            None => json!({ "status": "disabled" }),
        };
        let mut content = format_validation_diagnosis(&diagnosis);
        if let Some(status) = memory_result.get("status").and_then(Value::as_str) {
            content.push_str(&format!("\nMemory: {status}"));
        }
        ToolOutput {
            content,
            is_error: false,
            data: Some(json!({ "diagnosis": diagnosis, "memory": memory_result })),
        }
    }

    async fn validation_replay(&self, call: &ToolCall) -> ToolOutput {
        let root = match self.workspace_cwd_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        if let Some(reason) = os_sandbox::os_sandbox_exec_block_reason() {
            return error(reason);
        }
        let since_ms = call
            .args
            .get("sinceMs")
            .or_else(|| call.args.get("since_ms"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let max_commands = usize_arg_any(&call.args, &["maxCommands", "max_commands"]).unwrap_or(4);
        let include_build =
            bool_arg_any(&call.args, &["includeBuild", "include_build"]).unwrap_or(false);
        let stop_on_failure =
            bool_arg_any(&call.args, &["stopOnFailure", "stop_on_failure"]).unwrap_or(true);
        let timeout_ms = usize_arg_any(&call.args, &["timeout", "timeoutMs", "timeout_ms"])
            .unwrap_or(60_000)
            .clamp(5_000, 600_000);
        let store = validation_store::ValidationRuntimeStore::for_project(&root);
        let turn_runs = store.recent_runs_since(since_ms);
        if since_ms == 0 {
            return ToolOutput {
                content: "Auto-validation skipped: sinceMs is required (turn window).".to_string(),
                is_error: false,
                data: Some(json!({
                    "ran": false,
                    "skippedReason": "no turn window for replay",
                    "runs": []
                })),
            };
        }
        let selected = validation_store::select_replay_validation_commands(
            &turn_runs,
            max_commands,
            include_build,
        );
        if selected.is_empty() {
            return ToolOutput {
                content: "Auto-validation skipped: no typecheck/lint/test commands recorded in this turn.".to_string(),
                is_error: false,
                data: Some(json!({
                    "ran": false,
                    "skippedReason": "no agent verification commands in this turn",
                    "runs": []
                })),
            };
        }
        let mut runs = Vec::new();
        let mut failed = false;
        for cmd in selected {
            let started = validation_store::now_ms_public();
            let cwd = if cmd.cwd.trim().is_empty() {
                root.clone()
            } else {
                PathBuf::from(&cmd.cwd)
            };
            let (success, exit_code, content) =
                match os_sandbox::shell_command_with_os_sandbox(&cmd.command, &cwd)
                    .and_then(|mut command| {
                        command
                            .output()
                            .map_err(|err| format!("failed to execute: {err}"))
                    }) {
                    Ok(output) => {
                        let mut content = String::new();
                        content.push_str(&String::from_utf8_lossy(&output.stdout));
                        content.push_str(&String::from_utf8_lossy(&output.stderr));
                        (output.status.success(), output.status.code(), content)
                    }
                    Err(err) => (false, None, err),
                };
            let run = validation_store::ValidationCommandRun {
                command: cmd.command.clone(),
                kind: cmd.kind.clone(),
                cwd: cwd.to_string_lossy().replace('\\', "/"),
                related_files: cmd.related_files.clone(),
                started_at: started,
                duration_ms: validation_store::now_ms_public().saturating_sub(started),
                success,
                exit_code,
                output: truncate_text(&content, EXEC_MAX_OUTPUT_BYTES),
            };
            store.record_run(run.clone());
            if !success {
                failed = true;
            }
            runs.push(run);
            if !success && stop_on_failure {
                break;
            }
            let _ = timeout_ms;
        }
        let summary = runs
            .iter()
            .map(|run| {
                format!(
                    "{} [{}] {} ({}ms)",
                    if run.success { "✓" } else { "✗" },
                    run.kind,
                    run.command,
                    run.duration_ms
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ToolOutput {
            content: format!("Auto-validation:\n{summary}"),
            is_error: failed,
            data: Some(json!({
                "ran": true,
                "failed": failed,
                "runs": runs,
                "summary": summary
            })),
        }
    }

    async fn acceptance_check(&self, call: &ToolCall) -> ToolOutput {
        let criteria = string_vec_arg(&call.args, "criteria");
        let modified_files = string_vec_arg(&call.args, "modifiedFiles");
        let manual_evidence = string_vec_arg(&call.args, "manualEvidence");
        let report = build_acceptance_report(criteria, modified_files, manual_evidence);
        let passed = report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ToolOutput {
            content: format_acceptance_report(&report),
            is_error: !passed,
            data: Some(json!({ "report": report })),
        }
    }

    async fn self_review(&self, call: &ToolCall) -> ToolOutput {
        let Some(goal) = string_arg(&call.args, "goal") else {
            return error("self_review requires goal");
        };
        let modified_files = string_vec_arg(&call.args, "modifiedFiles");
        let report = build_self_review_report(goal, modified_files);
        let passed = report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ToolOutput {
            content: format_self_review_report(&report),
            is_error: !passed,
            data: Some(json!({ "report": report })),
        }
    }

    async fn write_memory(&self, call: &ToolCall) -> ToolOutput {
        let Some(kind) = string_arg(&call.args, "kind") else {
            return error("write_memory requires kind");
        };
        let Some(content) = string_arg(&call.args, "content") else {
            return error("write_memory requires content");
        };
        if content.trim().is_empty() {
            return error("write_memory content cannot be empty");
        }
        let root = match self.workspace_cwd_for_call(&call.cwd).await {
            Ok(root) => root,
            Err(err) => return error(err),
        };
        let Some(memory) = &self.memory else {
            return error("Memory manager is not configured.");
        };
        if let Err(err) = memory.initialize().await {
            return error(format!("memory initialization failed: {err}"));
        }
        let profile = match write_memory_profile(memory, &root, kind, content).await {
            Ok(profile) => profile,
            Err(err) => return error(err),
        };
        ToolOutput {
            content: format!(
                "Updated memory profile {} ({}).",
                kind,
                profile
                    .get("relativePath")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ),
            is_error: false,
            data: Some(json!({ "profile": profile })),
        }
    }

    async fn web_search(&self, call: &ToolCall) -> ToolOutput {
        let Some(query) = string_arg(&call.args, "query") else {
            return error("web_search requires query");
        };
        let query = query.trim();
        if query.is_empty() {
            return error("web_search query cannot be empty");
        }
        let provider = string_arg(&call.args, "provider")
            .map(str::to_string)
            .or_else(|| std::env::var("XY_WEB_SEARCH_PROVIDER").ok())
            .unwrap_or_else(|| "brave".to_string())
            .to_ascii_lowercase();
        if !matches!(provider.as_str(), "brave" | "bing" | "serpapi") {
            return ToolOutput {
                content: format!(
                    "Unsupported web search provider \"{provider}\". Expected brave, bing, or serpapi."
                ),
                is_error: true,
                data: Some(json!({ "provider": provider, "unsupportedProvider": true })),
            };
        }
        let limit = usize_arg(&call.args, "num_results")
            .unwrap_or(5)
            .clamp(1, 20);
        let Some(api_key) = search_provider_api_key(&provider, |name| std::env::var(name).ok())
        else {
            return ToolOutput {
                content: [
                    format!("Web search is not configured for provider \"{provider}\"."),
                    "Set XY_WEB_SEARCH_API_KEY or provider-specific env vars: BRAVE_SEARCH_API_KEY, BING_SEARCH_API_KEY, SERPAPI_API_KEY.".to_string(),
                ]
                .join("\n"),
                is_error: true,
                data: Some(json!({ "provider": provider, "missingApiKey": true })),
            };
        };
        match run_web_search(&provider, &api_key, query, limit) {
            Ok(results) => ToolOutput {
                content: format_search_results(query, &provider, &results),
                is_error: false,
                data: Some(json!({
                    "provider": provider,
                    "query": query,
                    "results": results
                })),
            },
            Err(err) => ToolOutput {
                content: format!("Web search failed: {err}"),
                is_error: true,
                data: Some(json!({ "provider": provider, "query": query })),
            },
        }
    }

    async fn web_fetch(&self, call: &ToolCall) -> ToolOutput {
        let Some(raw_url) = string_arg(&call.args, "url") else {
            return error("web_fetch requires url");
        };
        let max_chars = usize_arg(&call.args, "max_chars")
            .or_else(|| usize_arg(&call.args, "maxChars"))
            .unwrap_or(TOOL_INLINE_MAX_BYTES)
            .clamp(1_000, TOOL_INLINE_MAX_BYTES);
        match fetch_web_document(raw_url, max_chars) {
            Ok(document) => ToolOutput {
                content: [
                    format!("Source: {}", document.url),
                    format!(
                        "Content-Type: {}",
                        if document.content_type.is_empty() {
                            "unknown"
                        } else {
                            document.content_type.as_str()
                        }
                    ),
                    if document.truncated {
                        format!("Extracted text truncated to {max_chars} characters.")
                    } else {
                        format!("Extracted {} characters.", document.chars)
                    },
                    String::new(),
                    if document.text.is_empty() {
                        "(no text extracted)".to_string()
                    } else {
                        document.text.clone()
                    },
                ]
                .join("\n"),
                is_error: false,
                data: Some(json!({
                    "url": document.url,
                    "contentType": document.content_type,
                    "truncated": document.truncated,
                    "chars": document.chars,
                    "redirects": document.redirects
                })),
            },
            Err(err) => error(format!("Fetch failed for {raw_url}: {err}")),
        }
    }

    async fn sandbox_modes(&self, _call: &ToolCall) -> ToolOutput {
        let modes = [
            "readonly",
            "workspace-write",
            "worktree-write",
            "container",
        ];
        let policies = modes
            .iter()
            .map(|mode| describe_sandbox_mode(mode))
            .collect::<Vec<_>>();
        ToolOutput {
            content: policies
                .iter()
                .map(|policy| {
                    [
                        format!("{}: {}", policy.mode, policy.runner),
                        format!("  read: {}", policy.read_scope),
                        format!("  write: {}", policy.write_scope),
                        format!("  network: {}", policy.network),
                        format!("  background: {}", policy.background_processes),
                        format!("  env: {}", policy.environment),
                        format!("  approval: {}", policy.requires_approval),
                    ]
                    .join("\n")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            is_error: false,
            data: Some(json!({ "policies": policies })),
        }
    }

    async fn sandbox_create(&self, call: &ToolCall) -> ToolOutput {
        let name = string_arg(&call.args, "name").map(str::to_string);
        let base_ref = string_arg(&call.args, "baseRef")
            .or_else(|| string_arg(&call.args, "base_ref"))
            .unwrap_or("HEAD");
        let root_dir =
            string_arg(&call.args, "rootDir").or_else(|| string_arg(&call.args, "root_dir"));
        match create_sandbox(&call.cwd, name.as_deref(), base_ref, root_dir) {
            Ok(sandbox) => ToolOutput {
                content: [
                    format!("Created sandbox {}", sandbox.id),
                    format!("Worktree: {}", sandbox.worktree_path),
                    format!("Branch: {}", sandbox.branch),
                    format!("Base: {}", sandbox.base_commit),
                ]
                .join("\n"),
                is_error: false,
                data: Some(json!(sandbox)),
            },
            Err(err) => error(format!("Error creating sandbox: {err}")),
        }
    }

    async fn sandbox_list(&self, call: &ToolCall) -> ToolOutput {
        let root_dir =
            string_arg(&call.args, "rootDir").or_else(|| string_arg(&call.args, "root_dir"));
        match list_sandboxes(&call.cwd, root_dir) {
            Ok(sandboxes) => ToolOutput {
                content: if sandboxes.is_empty() {
                    "No sandboxes.".to_string()
                } else {
                    sandboxes
                        .iter()
                        .map(|sandbox| {
                            [
                                format!("{} [{}]", sandbox.id, sandbox.status),
                                format!("  worktree: {}", sandbox.worktree_path),
                                format!("  branch: {}", sandbox.branch),
                            ]
                            .join("\n")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                },
                is_error: false,
                data: Some(json!({ "sandboxes": sandboxes })),
            },
            Err(err) => error(format!("Error listing sandboxes: {err}")),
        }
    }

    async fn sandbox_diff(&self, call: &ToolCall) -> ToolOutput {
        let Some(sandbox_id) = string_arg_any(&call.args, &["sandboxId", "sandbox_id"]) else {
            return error("sandbox_diff requires sandboxId");
        };
        let root_dir =
            string_arg(&call.args, "rootDir").or_else(|| string_arg(&call.args, "root_dir"));
        match diff_sandbox(&call.cwd, sandbox_id, root_dir) {
            Ok((record, diff, patch_path)) => {
                let change_set: Option<serde_json::Value> = None;
                ToolOutput {
                    content: [
                        if diff.trim().is_empty() {
                            format!("Sandbox {sandbox_id} has no changes.")
                        } else {
                            diff.clone()
                        },
                        String::new(),
                        format!(
                            "Patch: {}",
                            display_relative(&patch_path, Path::new(&record.project_root))
                        ),
                    ]
                    .join("\n"),
                    is_error: false,
                    data: Some(json!({
                        "sandbox": record,
                        "patchPath": path_to_string(&patch_path),
                        "changeSet": change_set
                    })),
                }
            }
            Err(err) => error(format!("Error generating sandbox diff: {err}")),
        }
    }

    async fn sandbox_apply(&self, call: &ToolCall) -> ToolOutput {
        let Some(sandbox_id) = string_arg_any(&call.args, &["sandboxId", "sandbox_id"]) else {
            return error("sandbox_apply requires sandboxId");
        };
        let root_dir =
            string_arg(&call.args, "rootDir").or_else(|| string_arg(&call.args, "root_dir"));
        match apply_sandbox(&call.cwd, sandbox_id, root_dir) {
            Ok((record, diff, patch_path)) => ToolOutput {
                content: [
                    format!("Applied sandbox {sandbox_id} to {}.", record.project_root),
                    format!(
                        "Patch: {}",
                        display_relative(&patch_path, Path::new(&record.project_root))
                    ),
                    String::new(),
                    diff.clone(),
                ]
                .join("\n"),
                is_error: false,
                data: Some(json!({
                    "sandbox": record,
                    "patchPath": path_to_string(&patch_path)
                })),
            },
            Err(err) => error(format!("Error applying sandbox: {err}")),
        }
    }

    async fn sandbox_rollback(&self, call: &ToolCall) -> ToolOutput {
        let Some(sandbox_id) = string_arg_any(&call.args, &["sandboxId", "sandbox_id"]) else {
            return error("sandbox_rollback requires sandboxId");
        };
        let remove_worktree = bool_arg(&call.args, "removeWorktree")
            .or_else(|| bool_arg(&call.args, "remove_worktree"))
            .unwrap_or(true);
        let root_dir =
            string_arg(&call.args, "rootDir").or_else(|| string_arg(&call.args, "root_dir"));
        match rollback_sandbox(&call.cwd, sandbox_id, remove_worktree, root_dir) {
            Ok(sandbox) => ToolOutput {
                content: format!(
                    "Rolled back sandbox {sandbox_id}. Status: {}",
                    sandbox.status
                ),
                is_error: false,
                data: Some(json!(sandbox)),
            },
            Err(err) => error(format!("Error rolling back sandbox: {err}")),
        }
    }

    async fn workspace_checkpoint_create(&self, call: &ToolCall) -> ToolOutput {
        let session_id = string_arg_any(&call.args, &["sessionId", "session_id"])
            .unwrap_or("default")
            .trim();
        let turn_id = string_arg_any(&call.args, &["turnId", "turn_id"])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("turn_{}", uuid::Uuid::new_v4().simple()));
        match create_workspace_checkpoint(&call.cwd, session_id, &turn_id) {
            Ok(record) => ToolOutput {
                content: format!(
                    "Created workspace checkpoint {} (head {}, changes: {}, files: {}).",
                    record.turn_id,
                    record.git_head,
                    record.has_changes,
                    record.files.len()
                ),
                is_error: false,
                data: Some(json!({ "checkpoint": record })),
            },
            Err(err) => error(format!("Error creating workspace checkpoint: {err}")),
        }
    }

    async fn workspace_checkpoint_list(&self, call: &ToolCall) -> ToolOutput {
        let session_id = string_arg_any(&call.args, &["sessionId", "session_id"])
            .unwrap_or("default")
            .trim();
        match list_workspace_checkpoints(&call.cwd, session_id) {
            Ok(records) => ToolOutput {
                content: if records.is_empty() {
                    format!("No workspace checkpoints for session {session_id}.")
                } else {
                    records
                        .iter()
                        .map(|record| {
                            format!(
                                "{} head={} changes={} files={}",
                                record.turn_id,
                                record.git_head,
                                record.has_changes,
                                record.files.len()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                },
                is_error: false,
                data: Some(json!({ "checkpoints": records })),
            },
            Err(err) => error(format!("Error listing workspace checkpoints: {err}")),
        }
    }

    async fn workspace_checkpoint_restore(&self, call: &ToolCall) -> ToolOutput {
        let Some(session_id) = string_arg_any(&call.args, &["sessionId", "session_id"]) else {
            return error("workspace_checkpoint_restore requires sessionId");
        };
        let Some(turn_id) = string_arg_any(&call.args, &["turnId", "turn_id"]) else {
            return error("workspace_checkpoint_restore requires turnId");
        };
        match restore_workspace_checkpoint(&call.cwd, session_id, turn_id) {
            Ok(record) => ToolOutput {
                content: format!(
                    "Restored workspace checkpoint {} (files: {}).",
                    record.turn_id,
                    record.files.len()
                ),
                is_error: false,
                data: Some(json!({ "checkpoint": record })),
            },
            Err(err) => error(format!("Error restoring workspace checkpoint: {err}")),
        }
    }

    async fn submit_task(&self, call: &ToolCall) -> ToolOutput {
        let Some(agents) = &self.agents else {
            return error("Task control plane is not configured");
        };
        let Some(session_id) = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        ) else {
            return error("submit_task requires sessionId or session context");
        };
        let run_id = string_arg_any(&call.args, &["runId", "run_id", "_runId", "_run_id"])
            .map(str::to_string);
        let parsed = match subagent_task_inputs_from_args(&call.args) {
            Ok(parsed) => parsed,
            Err(err) => return error(err),
        };
        if parsed.inputs.is_empty() {
            return error("submit_task requires agentId/task or a non-empty subTasks array");
        }

        let mut tasks = Vec::new();
        for input in parsed.inputs {
            match agents
                .submit_task(input, Some(session_id.to_string()), run_id.clone())
                .await
            {
                Ok(task) => tasks.push(task),
                Err(err) => return error(format!("submit_task failed: {err}")),
            }
        }
        let task_ids = tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>();
        let lines = tasks
            .iter()
            .map(|task| format!("{} [{}] {}", task.task_id, task.status, task.task.task))
            .collect::<Vec<_>>();
        let deduped_subtask_count = tasks.len();
        ToolOutput {
            content: if lines.len() == 1 {
                format!("Background task submitted: {}", lines[0])
            } else {
                format!(
                    "{} background tasks submitted:\n{}",
                    lines.len(),
                    lines.join("\n")
                )
            },
            is_error: false,
            data: Some(json!({
                "tasks": tasks,
                "taskIds": task_ids,
                "dedupedSubTaskCount": deduped_subtask_count,
                "duplicateSubTaskCount": parsed.duplicate_subtask_count,
                "delegationContextChars": parsed.total_context_chars,
                "delegationContextTruncated": parsed.context_truncated
            })),
        }
    }

    async fn get_task_status(&self, call: &ToolCall) -> ToolOutput {
        let Some(agents) = &self.agents else {
            return error("Task control plane is not configured");
        };
        let Some(task_id) = string_arg_any(&call.args, &["taskId", "task_id"]) else {
            return error("get_task_status requires taskId");
        };
        let Some(task) = agents.task_status(task_id.to_string()).await else {
            return error(format!("Task not found: {task_id}"));
        };
        ToolOutput {
            content: format_task_status_content(&task),
            is_error: false,
            data: Some(json!({
                "task": task.clone(),
                "timeline": task.timeline.clone(),
                "result": task.result.clone()
            })),
        }
    }

    async fn list_tasks(&self, call: &ToolCall) -> ToolOutput {
        let Some(agents) = &self.agents else {
            return error("Task control plane is not configured");
        };
        let session_id = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        )
        .map(str::to_string);
        let limit = usize_arg(&call.args, "limit").unwrap_or(20).clamp(1, 200);
        let tasks = agents.list_tasks(session_id, limit).await;
        ToolOutput {
            content: if tasks.is_empty() {
                "No tasks found.".to_string()
            } else {
                tasks
                    .iter()
                    .map(|task| format!("{} [{}] {}", task.task_id, task.status, task.task.task))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            is_error: false,
            data: Some(json!({ "tasks": tasks })),
        }
    }

    async fn cancel_task(&self, call: &ToolCall) -> ToolOutput {
        let Some(agents) = &self.agents else {
            return error("Task control plane is not configured");
        };
        let Some(task_id) = string_arg_any(&call.args, &["taskId", "task_id"]) else {
            return error("cancel_task requires taskId");
        };
        let reason = string_arg(&call.args, "reason").map(str::to_string);
        let result = agents
            .control_task(task_id.to_string(), "cancel".to_string(), reason)
            .await;
        let ok = result.get("ok").and_then(Value::as_bool) == Some(true);
        ToolOutput {
            content: if ok {
                format!("Task cancelled: {task_id}")
            } else {
                result
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("Task cancel failed")
                    .to_string()
            },
            is_error: !ok,
            data: Some(result),
        }
    }

    async fn delivery_create_local_pr(&self, call: &ToolCall) -> ToolOutput {
        match create_delivery_local_pr(&call.cwd, &call.args) {
            Ok(local_pr) => {
                let title = local_pr
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Local PR package");
                let markdown_path = local_pr
                    .get("markdownPath")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let html_path = local_pr
                    .get("htmlPath")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let patch_path = local_pr
                    .get("patchPath")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let commit_message = local_pr
                    .get("commitMessage")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                ToolOutput {
                    content: [
                        format!("Local PR package created: {title}"),
                        format!("Markdown: {markdown_path}"),
                        format!("HTML: {html_path}"),
                        format!("Patch: {patch_path}"),
                        String::new(),
                        format!("Commit message:\n{commit_message}"),
                    ]
                    .join("\n"),
                    is_error: false,
                    data: Some(json!({ "localPr": local_pr })),
                }
            }
            Err(err) => error(format!("Error creating local PR package: {err}")),
        }
    }

    async fn git_platform_status(&self, call: &ToolCall) -> ToolOutput {
        match inspect_git_platform_status(&call.cwd, string_arg(&call.args, "platform")) {
            Ok(integrations) => ToolOutput {
                content: integrations
                    .iter()
                    .map(|item| {
                        [
                            format!(
                                "{}: {}",
                                item["platform"].as_str().unwrap_or("git"),
                                if item["configured"].as_bool().unwrap_or(false) {
                                    "configured"
                                } else {
                                    "not configured"
                                }
                            ),
                            item.get("remote")
                                .and_then(Value::as_str)
                                .map(|remote| format!("  remote: {remote}"))
                                .unwrap_or_default(),
                            item.get("authHint")
                                .and_then(Value::as_str)
                                .map(|hint| format!("  hint: {hint}"))
                                .unwrap_or_default(),
                        ]
                        .into_iter()
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                is_error: false,
                data: Some(json!({ "integrations": integrations })),
            },
            Err(err) => error(format!("Error inspecting Git platform integration: {err}")),
        }
    }

    async fn git_issue_task_draft(&self, call: &ToolCall) -> ToolOutput {
        match create_git_issue_task_draft(&call.cwd, &call.args) {
            Ok(draft) => ToolOutput {
                content: [
                    format!(
                        "{} task draft: {}",
                        draft["platform"].as_str().unwrap_or("git"),
                        draft["title"].as_str().unwrap_or("Issue task")
                    ),
                    format!("Issue: {}", draft["issue"].as_str().unwrap_or_default()),
                    format!("Branch: {}", draft["branch"].as_str().unwrap_or_default()),
                    "Commands:".to_string(),
                    draft["commands"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|command| format!("- {command}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ]
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
                is_error: false,
                data: Some(json!({ "draft": draft })),
            },
            Err(err) => error(format!("Error creating issue task draft: {err}")),
        }
    }

    async fn search_tools(&self, call: &ToolCall) -> ToolOutput {
        let Some(query) = string_arg(&call.args, "query") else {
            return error("search_tools requires query");
        };
        let query = query.trim();
        if query.is_empty() {
            return error("search_tools query cannot be empty");
        }
        let limit = usize_arg(&call.args, "limit").unwrap_or(10).clamp(1, 20);
        let registry = self.registry.read().await;
        let mut hits = search_tool_catalog(registry.catalog(ToolSource::Builtin), query, limit);
        drop(registry);
        if hits.is_empty() {
            return ToolOutput {
                content: format!(
                    "No tools matched \"{query}\". Try broader keywords (file, git, exec, memory, task)."
                ),
                is_error: false,
                data: Some(json!({ "hits": [] })),
            };
        }
        hits.truncate(limit);
        ToolOutput {
            content: [
                format!("Found {} tool(s):", hits.len()),
                hits.iter()
                    .map(|hit| {
                        let flags = [
                            hit.get("isWrite")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                                .then_some("write"),
                            hit.get("isExecute")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                                .then_some("execute"),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(", ");
                        format!(
                            "- {}{}: {}{}",
                            hit["name"].as_str().unwrap_or_default(),
                            hit.get("category")
                                .and_then(Value::as_str)
                                .map(|category| format!(" [{category}]"))
                                .unwrap_or_default(),
                            hit["description"].as_str().unwrap_or_default(),
                            if flags.is_empty() {
                                String::new()
                            } else {
                                format!(" ({flags})")
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                String::new(),
                "Call load_tools with names before using them.".to_string(),
            ]
            .join("\n"),
            is_error: false,
            data: Some(json!({ "hits": hits })),
        }
    }

    async fn load_tools(&self, call: &ToolCall) -> ToolOutput {
        let names = string_vec_arg(&call.args, "names");
        if names.is_empty() {
            return error("load_tools requires names");
        }
        let registry = self.registry.read().await;
        let mut loaded = Vec::new();
        let mut already_active = Vec::new();
        let mut rejected = Vec::new();
        let active_hint: BTreeSet<String> = string_vec_arg_any(
            &call.args,
            &["_activeTools", "_active_tools", "activeTools"],
        )
        .into_iter()
        .collect();
        for name in names.into_iter().take(10) {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            if is_meta_tool_name(trimmed) {
                rejected.push(json!({
                    "name": trimmed,
                    "reason": "meta tools activate automatically when enabled"
                }));
                continue;
            }
            let Some(tool) = registry.get(trimmed) else {
                rejected.push(json!({ "name": trimmed, "reason": "unknown_tool" }));
                continue;
            };
            let canonical = tool.name.clone();
            if active_hint.contains(&canonical)
                || active_hint.contains(trimmed)
                || loaded.contains(&canonical)
            {
                if !already_active.contains(&canonical) {
                    already_active.push(canonical);
                }
                continue;
            }
            loaded.push(canonical);
        }
        let mut active_tools = active_hint;
        for name in &loaded {
            active_tools.insert(name.clone());
        }
        for name in META_TOOL_NAMES {
            active_tools.insert((*name).to_string());
        }
        for name in AGENT_CORE_PRELOAD_TOOLS {
            active_tools.insert((*name).to_string());
        }
        let mut active_list = active_tools.into_iter().collect::<Vec<_>>();
        active_list.sort();
        let is_error = should_report_meta_load_as_error(&loaded, &already_active, &rejected);
        let mut lines = Vec::new();
        if loaded.is_empty() && !already_active.is_empty() && rejected.is_empty() {
            lines.push(
                "All requested tools were already active — schemas unchanged for the next model iteration."
                    .to_string(),
            );
        }
        if !loaded.is_empty() {
            lines.push(format!("Loaded: {}", loaded.join(", ")));
        }
        if !already_active.is_empty() {
            lines.push(format!("Already active: {}", already_active.join(", ")));
        }
        if !rejected.is_empty() {
            lines.push(format!(
                "Rejected: {}",
                rejected
                    .iter()
                    .map(|item| format!(
                        "{} ({})",
                        item["name"].as_str().unwrap_or_default(),
                        item["reason"].as_str().unwrap_or_default()
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        lines.push(String::new());
        lines.push(format!(
            "Active tools ({}): {}",
            active_list.len(),
            active_list.join(", ")
        ));
        ToolOutput {
            content: lines.join("\n"),
            is_error,
            data: Some(json!({
                "loaded": loaded,
                "alreadyActive": already_active,
                "rejected": rejected,
                "activeTools": active_list,
            })),
        }
    }

    async fn mcp_call(&self, call: &ToolCall) -> ToolOutput {
        let mcp = self.mcp.clone();
        let name = call.name.clone();
        let args = call.args.clone();
        match tokio::task::spawn_blocking(move || mcp.call_tool(&name, args)).await {
            Ok(output) => output,
            Err(err) => error(format!("MCP runtime task failed: {err}")),
        }
    }

    async fn search_memory(&self, call: &ToolCall) -> ToolOutput {
        let Some(query) = string_arg(&call.args, "query") else {
            return error("search_memory requires query");
        };
        let query = query.trim();
        if query.is_empty() {
            return error("search_memory query cannot be empty");
        }
        let Some(memory) = &self.memory else {
            return error("Memory manager is not configured.");
        };
        if let Err(err) = memory.initialize().await {
            return error(format!("memory initialization failed: {err}"));
        }
        let limit = usize_arg(&call.args, "limit").unwrap_or(8).clamp(1, 20);
        let min_score = 0.35_f32;
        let session_id = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        )
        .map(str::to_string);
        let include_cross_session = bool_arg_any(
            &call.args,
            &["includeCrossSession", "include_cross_session"],
        )
        .unwrap_or(false);
        let hits = memory
            .search(MemorySearchQuery {
                text: Some(query.to_string()),
                session_id,
                include_cross_session,
                limit,
                ..MemorySearchQuery::default()
            })
            .await
            .into_iter()
            .filter(|result| result.score > min_score)
            .take(limit)
            .map(|result| {
                json!({
                    "id": result.entry.id,
                    "category": result.entry.category,
                    "snippet": snippet_text(&result.entry.content, 160),
                    "score": (result.score * 1000.0).round() / 1000.0
                })
            })
            .collect::<Vec<_>>();
        if hits.is_empty() {
            return ToolOutput {
                content: format!(
                    "No memories matched \"{query}\" above inject threshold (minScore={min_score}). Try broader technical keywords or project terms."
                ),
                is_error: false,
                data: Some(json!({ "hits": [] })),
            };
        }
        ToolOutput {
            content: [
                format!("Found {} memory match(es):", hits.len()),
                hits.iter()
                    .map(|hit| {
                        format!(
                            "- {} [{}] score={}: {}",
                            hit["id"].as_str().unwrap_or_default(),
                            hit["category"].as_str().unwrap_or_default(),
                            hit["score"],
                            hit["snippet"].as_str().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                String::new(),
                "Call load_memories([\"memory\"]) to inject relevant memories on the next model iteration.".to_string(),
            ]
            .join("\n"),
            is_error: false,
            data: Some(json!({ "hits": hits })),
        }
    }

    async fn load_memories(&self, call: &ToolCall) -> ToolOutput {
        let sections = string_vec_arg(&call.args, "sections");
        if sections.is_empty() {
            return error("load_memories requires sections");
        }
        let mut loaded = Vec::new();
        let mut rejected = Vec::new();
        for section in sections.into_iter().take(4) {
            if section == "skills" {
                rejected.push(json!({
                    "section": section,
                    "reason": "use_activate_skill"
                }));
            } else if is_context_section_name(&section) {
                loaded.push(section);
            } else {
                rejected.push(json!({ "section": section, "reason": "unknown_section" }));
            }
        }
        let available = context_section_names();
        let mut lines = Vec::new();
        if !loaded.is_empty() {
            lines.push(format!(
                "Loaded sections: {}",
                loaded
                    .iter()
                    .map(|section| format!("{section} ({})", context_section_label(section)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !rejected.is_empty() {
            lines.push(format!(
                "Rejected: {}",
                rejected
                    .iter()
                    .map(|item| format!(
                        "{} ({})",
                        item["section"].as_str().unwrap_or_default(),
                        item["reason"].as_str().unwrap_or_default()
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        lines.push(format!(
            "Active context sections ({}): {}",
            loaded.len(),
            if loaded.is_empty() {
                "(none)".to_string()
            } else {
                loaded.join(", ")
            }
        ));
        lines.push(
            "Note: requested sections are hydrated below when backing data is available. Skills use activate_skill, not load_memories."
                .to_string(),
        );
        lines.push(format!("Available: {}", available.join(", ")));
        let mut section_payloads = BTreeMap::new();
        if loaded.iter().any(|section| section == "memory") {
            if let Some(memory) = &self.memory {
                let query = string_arg(&call.args, "query").unwrap_or_default();
                let limit = usize_arg(&call.args, "limit").unwrap_or(8).clamp(1, 20);
                let session_id = string_arg_any(
                    &call.args,
                    &["sessionId", "session_id", "_sessionId", "_session_id"],
                )
                .map(str::to_string);
                let include_cross_session = bool_arg_any(
                    &call.args,
                    &["includeCrossSession", "include_cross_session"],
                )
                .unwrap_or(false);
                let entries = if query.trim().is_empty() {
                    memory
                        .recent(limit)
                        .await
                        .into_iter()
                        .map(|entry| json!({ "entry": entry, "score": null, "reasons": [] }))
                        .collect::<Vec<_>>()
                } else {
                    memory
                        .search(MemorySearchQuery {
                            text: Some(query.to_string()),
                            session_id,
                            include_cross_session,
                            limit,
                            ..MemorySearchQuery::default()
                        })
                        .await
                        .into_iter()
                        .map(|result| json!(result))
                        .collect::<Vec<_>>()
                };
                lines.push(format!(
                    "Hydrated {} long-term memory entr{}.",
                    entries.len(),
                    if entries.len() == 1 { "y" } else { "ies" }
                ));
                let memory_text = entries
                    .iter()
                    .filter_map(format_loaded_memory_entry)
                    .collect::<Vec<_>>()
                    .join("\n");
                if !memory_text.is_empty() {
                    lines.push(String::new());
                    lines.push(memory_text.clone());
                }
                section_payloads.insert(
                    "memory".to_string(),
                    json!({ "query": query, "entries": entries, "text": memory_text }),
                );
            } else {
                lines.push("Verified: memory manager is not configured.".to_string());
            }
        }
        if loaded.iter().any(|section| section == "memory_profile") {
            if let Some(memory) = &self.memory {
                let project_dir = find_project_root(&call.cwd);
                let mut profiles = Vec::new();
                let mut profile_lines = Vec::new();
                for kind in memory_profile_kinds() {
                    match read_memory_profile(memory, &project_dir, kind).await {
                        Ok(profile) => {
                            if let Some(text) = format_loaded_memory_profile(&profile) {
                                profile_lines.push(text);
                            }
                            profiles.push(profile);
                        }
                        Err(err) => rejected.push(json!({
                            "section": "memory_profile",
                            "kind": kind,
                            "reason": err
                        })),
                    }
                }
                let profile_text = profile_lines.join("\n\n");
                if profile_text.is_empty() {
                    lines.push(
                        "Verified: no explicit memory profiles have stored content.".to_string(),
                    );
                } else {
                    lines.push(format!(
                        "Hydrated {} explicit memory profile(s).",
                        profile_lines.len()
                    ));
                    lines.push(String::new());
                    lines.push(profile_text.clone());
                }
                section_payloads.insert(
                    "memory_profile".to_string(),
                    json!({ "profiles": profiles, "text": profile_text }),
                );
            } else {
                lines.push("Verified: memory manager is not configured.".to_string());
            }
        }
        if loaded
            .iter()
            .any(|section| section == "project_instructions")
        {
            let instructions = load_project_instructions(&call.cwd, true);
            if instructions.sources.is_empty() {
                lines.push(format!(
                    "Verified: no project instruction files found for {}.",
                    instructions.root_dir
                ));
            } else {
                lines.push(format!(
                    "Hydrated project instructions from {} source(s): {}",
                    instructions.sources.len(),
                    instructions
                        .sources
                        .iter()
                        .map(|source| format!("{}:{}", source.scope, source.relative_path))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                lines.push(String::new());
                lines.push(instructions.text.clone());
            }
            section_payloads.insert("project_instructions".to_string(), json!(instructions));
        }
        let is_error = loaded.is_empty() && !rejected.is_empty();
        ToolOutput {
            content: lines.join("\n"),
            is_error,
            data: Some(json!({
                "loaded": loaded,
                "alreadyActive": [],
                "rejected": rejected,
                "activeSections": loaded,
                "availableSections": available,
                "sections": section_payloads
            })),
        }
    }

    async fn exec(&self, call: &ToolCall) -> ToolOutput {
        let Some(command) = string_arg(&call.args, "command") else {
            return error("exec requires a string command");
        };
        if let Some(reason) = os_sandbox::os_sandbox_exec_block_reason() {
            return error(reason);
        }
        if let Some(blocked) = exec_readonly_block(call, command) {
            return blocked;
        }
        let mut cwd = match command_cwd(call).await {
            Ok(cwd) => cwd,
            Err(err) => return error(err),
        };
        // Risky commands can auto-create a git worktree sandbox when configured.
        // 与参考 maybeCreateCommandSandbox(auto) 一致：创建失败则留在原 cwd，不硬拒。
        if auto_for_risky_commands_enabled() && looks_like_risky_exec(command) {
            if let Ok(sandbox) = create_sandbox(&call.cwd, Some("risky-exec"), "HEAD", None) {
                cwd = PathBuf::from(&sandbox.worktree_path);
            }
        }
        let timeout_ms = usize_arg_any(&call.args, &["timeout", "timeoutMs", "timeout_ms"]);
        let background_on_timeout = bool_arg_any(
            &call.args,
            &["backgroundOnTimeout", "background_on_timeout"],
        )
        .unwrap_or(false);
        if timeout_ms.is_some() || background_on_timeout {
            let output = self.exec_with_supervisor(
                command,
                cwd.clone(),
                timeout_ms.unwrap_or(30_000).clamp(1, 86_400_000),
                background_on_timeout,
            );
            self.maybe_record_validation_run(call, command, &cwd, &output);
            return output;
        }
        let started = validation_store::now_ms_public();
        let mut cmd = match os_sandbox::shell_command_with_os_sandbox(command, &cwd) {
            Ok(cmd) => cmd,
            Err(err) => return error(err),
        };
        match cmd.output() {
            Ok(output) => {
                let mut content = String::new();
                content.push_str(&String::from_utf8_lossy(&output.stdout));
                content.push_str(&String::from_utf8_lossy(&output.stderr));
                let content = if content.trim().is_empty() {
                    format!("Command exited with code {:?}", output.status.code())
                } else {
                    content
                };
                let tool_output = ToolOutput {
                    content,
                    is_error: !output.status.success(),
                    data: Some(json!({
                        "exitCode": output.status.code(),
                        "success": output.status.success(),
                        "cwd": cwd,
                        "startedAt": started,
                        "durationMs": validation_store::now_ms_public().saturating_sub(started)
                    })),
                };
                self.maybe_record_validation_run(call, command, &cwd, &tool_output);
                tool_output
            }
            Err(err) => error(format!("failed to execute command: {err}")),
        }
    }

    fn maybe_record_validation_run(
        &self,
        call: &ToolCall,
        command: &str,
        cwd: &Path,
        output: &ToolOutput,
    ) {
        let kind = classify_validation_command(command);
        if kind == "unknown" {
            return;
        }
        let started_at = output
            .data
            .as_ref()
            .and_then(|value| value.get("startedAt"))
            .and_then(Value::as_u64)
            .unwrap_or_else(validation_store::now_ms_public);
        let duration_ms = output
            .data
            .as_ref()
            .and_then(|value| value.get("durationMs"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let exit_code = output
            .data
            .as_ref()
            .and_then(|value| value.get("exitCode"))
            .and_then(Value::as_i64)
            .map(|value| value as i32);
        let store = validation_store::ValidationRuntimeStore::for_project(
            if call.cwd.as_os_str().is_empty() {
                cwd
            } else {
                call.cwd.as_path()
            },
        );
        store.record_run(validation_store::ValidationCommandRun {
            command: command.to_string(),
            kind,
            cwd: cwd.to_string_lossy().replace('\\', "/"),
            related_files: Vec::new(),
            started_at,
            duration_ms,
            success: !output.is_error,
            exit_code,
            output: truncate_text(&output.content, EXEC_MAX_OUTPUT_BYTES),
        });
    }

    fn exec_with_supervisor(
        &self,
        command: &str,
        cwd: PathBuf,
        timeout_ms: usize,
        background_on_timeout: bool,
    ) -> ToolOutput {
        let process = match self.processes.start(command.to_string(), cwd, None) {
            Ok(process) => process,
            Err(err) => return error(format!("failed to execute command: {err}")),
        };
        let snapshot = self
            .processes
            .wait_for_exit(&process.id, timeout_ms as u64)
            .unwrap_or_else(|| process.clone());
        if snapshot.status == "running" {
            if background_on_timeout {
                return ToolOutput {
                    content: format!(
                        "Command is still running after {timeout_ms}ms. Continuing in background as {}.\nLog: {}",
                        snapshot.id, snapshot.log_path
                    ),
                    is_error: true,
                    data: Some(json!({
                        "errorType": "timeout_background",
                        "timeout": timeout_ms,
                        "processId": snapshot.id,
                        "pid": snapshot.pid,
                        "logPath": snapshot.log_path,
                        "process": snapshot
                    })),
                };
            }
            let stopped = self
                .processes
                .stop(&snapshot.id)
                .unwrap_or_else(|_| snapshot.clone());
            let read = self
                .processes
                .read(&stopped.id, 0, output_bound::EXEC_MAX_OUTPUT_BYTES.saturating_mul(4))
                .map(|read| read.content)
                .unwrap_or_default();
            let content = empty_process_output(read, stopped.exit_code);
            return ToolOutput {
                content: format!("Command timed out after {timeout_ms}ms:\n{content}"),
                is_error: true,
                data: Some(json!({
                    "errorType": "timeout",
                    "timeout": timeout_ms,
                    "processId": stopped.id,
                    "exitCode": stopped.exit_code,
                    "logPath": stopped.log_path,
                    "process": stopped
                })),
            };
        }

        let read = self
            .processes
            .read(
                &snapshot.id,
                0,
                output_bound::EXEC_MAX_OUTPUT_BYTES.saturating_mul(4),
            )
            .map(|read| read.content)
            .unwrap_or_default();
        ToolOutput {
            content: empty_process_output(read, snapshot.exit_code),
            is_error: snapshot.status == "failed",
            data: Some(json!({
                "exitCode": snapshot.exit_code,
                "success": snapshot.status == "completed",
                "processId": snapshot.id,
                "logPath": snapshot.log_path,
                "process": snapshot
            })),
        }
    }

    async fn start_process(&self, call: &ToolCall) -> ToolOutput {
        let Some(command) = string_arg(&call.args, "command") else {
            return error("start_process requires a string command");
        };
        if let Some(reason) = os_sandbox::os_sandbox_exec_block_reason() {
            return error(reason);
        }
        if let Some(blocked) = exec_readonly_block(call, command) {
            return blocked;
        }
        let cwd = match command_cwd(call).await {
            Ok(cwd) => cwd,
            Err(err) => return error(err),
        };
        let name = string_arg(&call.args, "name").map(str::to_string);
        let startup_timeout_ms = usize_arg_any(&call.args, &["startupTimeout", "startup_timeout"])
            .unwrap_or(500)
            .clamp(0, 10_000);
        match self.processes.start(command.to_string(), cwd, name) {
            Ok(process) => {
                let process = self
                    .processes
                    .wait_for_output_or_exit(&process.id, startup_timeout_ms as u64)
                    .unwrap_or(process);
                let read = self.processes.read(&process.id, 0, 50 * 1024);
                let initial_output = read
                    .as_ref()
                    .map(|read| read.content.as_str())
                    .filter(|content| *content != "(no new output)")
                    .unwrap_or("");
                let mut lines = vec![
                    format!(
                        "Started background process {} (pid {:?}).",
                        process.id, process.pid
                    ),
                    format!("Status: {}", process.status),
                    format!("Log: {}", process.log_path),
                ];
                if !initial_output.is_empty() {
                    lines.push(format!("\nInitial output:\n{initial_output}"));
                }
                ToolOutput {
                    content: lines.join("\n"),
                    is_error: process.status == "failed",
                    data: Some(json!({
                        "processId": process.id,
                        "offset": read.as_ref().map(|read| read.offset).unwrap_or(0),
                        "process": process
                    })),
                }
            }
            Err(err) => error(format!("failed to start process: {err}")),
        }
    }

    async fn list_processes_tool(&self, call: &ToolCall) -> ToolOutput {
        let filter = string_arg(&call.args, "status")
            .unwrap_or("running")
            .to_ascii_lowercase();
        let processes = self
            .processes
            .list()
            .into_iter()
            .filter(|process| filter == "all" || process.status.eq_ignore_ascii_case(&filter))
            .collect::<Vec<_>>();
        if processes.is_empty() {
            return ToolOutput {
                content: if filter == "running" {
                    "No running background processes.".to_string()
                } else {
                    "No tracked processes.".to_string()
                },
                is_error: false,
                data: Some(json!({ "processes": [], "status": filter })),
            };
        }
        let now = now_ms();
        let lines = processes
            .iter()
            .map(|process| {
                let label = process
                    .name
                    .clone()
                    .unwrap_or_else(|| process.command.chars().take(80).collect::<String>());
                let age_sec = (now - process.started_at).max(0) / 1000 ;
                format!(
                    "- {}  pid={}  {}  {}s  bytes={}  {label}",
                    process.id,
                    process
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    process.status,
                    age_sec,
                    process.output_bytes
                )
            })
            .collect::<Vec<_>>();
        ToolOutput {
            content: format!("Processes ({filter}):\n{}", lines.join("\n")),
            is_error: false,
            data: Some(json!({ "processes": processes, "status": filter })),
        }
    }

    async fn read_process_output(&self, call: &ToolCall) -> ToolOutput {
        let Some(process_id) = string_arg_any(&call.args, &["processId", "process_id"]) else {
            return error("read_process_output requires processId");
        };
        let offset = usize_arg(&call.args, "offset").unwrap_or(0);
        let max_bytes = usize_arg_any(&call.args, &["maxBytes", "max_bytes"])
            .unwrap_or(EXEC_MAX_OUTPUT_BYTES);
        match self.processes.read(process_id, offset, max_bytes) {
            Some(read) => ToolOutput {
                content: read.content.clone(),
                is_error: false,
                data: Some(json!({
                    "processId": read.process_id,
                    "offset": read.offset,
                    "running": read.process.status == "running",
                    "status": read.process.status,
                    "exitCode": read.process.exit_code,
                    "process": read.process
                })),
            },
            None => error(format!("Unknown background process: {process_id}")),
        }
    }

    async fn stop_process_tool(&self, call: &ToolCall) -> ToolOutput {
        let Some(process_id) = string_arg_any(&call.args, &["processId", "process_id"]) else {
            return error("stop_process requires processId");
        };
        match self.processes.stop(process_id) {
            Ok(process) => ToolOutput {
                content: format!("Stopped background process {process_id}"),
                is_error: false,
                data: Some(json!({ "processId": process_id, "stopped": true, "process": process })),
            },
            Err(err) => error(err.to_string()),
        }
    }

    async fn write_process_stdin(&self, call: &ToolCall) -> ToolOutput {
        let Some(process_id) = string_arg_any(&call.args, &["processId", "process_id"]) else {
            return error("write_process_stdin requires processId");
        };
        let data = string_arg_any(&call.args, &["data", "input", "text"]).unwrap_or("");
        let eof = bool_arg(&call.args, "eof").unwrap_or(false);
        match self.processes.write_stdin(process_id, data, eof) {
            Ok(process) => ToolOutput {
                content: if eof {
                    format!(
                        "Wrote {} bytes to {process_id} stdin and closed input",
                        data.len()
                    )
                } else {
                    format!("Wrote {} bytes to {process_id} stdin", data.len())
                },
                is_error: false,
                data: Some(json!({
                    "processId": process_id,
                    "bytesWritten": data.len(),
                    "eof": eof,
                    "process": process
                })),
            },
            Err(err) => error(format!("write_process_stdin failed: {err}")),
        }
    }

    async fn todo_write(&self, call: &ToolCall) -> ToolOutput {
        let Some(session_id) = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        ) else {
            return error("todo_write requires sessionId or session context");
        };
        let merge = bool_arg(&call.args, "merge").unwrap_or(true);
        let Some(todos_value) = call.args.get("todos").and_then(Value::as_array) else {
            return error("todo_write requires todos array");
        };
        let mut incoming = Vec::new();
        for item in todos_value {
            let Some(object) = item.as_object() else {
                return error("todo_write todos items must be objects");
            };
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("");
            let content = object
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("");
            let status = object
                .get("status")
                .and_then(Value::as_str)
                .and_then(TodoStatus::from_str);
            if id.is_empty() || content.is_empty() || status.is_none() {
                return error(
                    "todo_write items require id, content, and status (pending|in_progress|completed|cancelled)",
                );
            }
            incoming.push(TodoItem {
                id: id.to_string(),
                content: content.to_string(),
                status: status.expect("checked"),
            });
        }
        if incoming.is_empty() {
            return error("todo_write requires at least one todo");
        }
        let in_progress = incoming
            .iter()
            .filter(|todo| todo.status == TodoStatus::InProgress)
            .count();
        if in_progress > 1 {
            return error(format!(
                "todo_write rejected: at most one todo may be in_progress (got {in_progress})"
            ));
        }
        match sync_todos(&call.cwd, session_id, incoming, merge) {
            Ok(written) => {
                let lines = written
                    .iter()
                    .map(|todo| {
                        format!(
                            "- [{}] {}: {}",
                            todo.status.as_str(),
                            todo.id,
                            todo.content
                        )
                    })
                    .collect::<Vec<_>>();
                ToolOutput {
                    content: format!(
                        "Todo list {} ({} items):\n{}",
                        if merge { "updated" } else { "replaced" },
                        written.len(),
                        lines.join("\n")
                    ),
                    is_error: false,
                    data: Some(json!({ "todos": written, "merge": merge })),
                }
            }
            Err(err) => error(format!("todo_write failed: {err}")),
        }
    }

    async fn update_goal(&self, call: &ToolCall) -> ToolOutput {
        let Some(session_id) = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        ) else {
            return error("update_goal requires sessionId or session context");
        };
        let status = string_arg(&call.args, "status").unwrap_or("complete");
        if status != "complete" {
            return error("update_goal only supports status=complete from the model");
        }
        let Some(evidence) =
            string_arg_any(&call.args, &["evidence_summary", "evidenceSummary", "evidence"])
        else {
            return error("update_goal requires evidence_summary");
        };
        let progress_note = string_arg_any(&call.args, &["progress_note", "progressNote"]);
        match complete_session_goal(&call.cwd, session_id, evidence, progress_note) {
            Ok(goal) => ToolOutput {
                content: format!("Objective complete.\n{evidence}"),
                is_error: false,
                data: Some(json!({ "goal": goal })),
            },
            Err(err) => error(err),
        }
    }

    async fn enter_plan_mode(&self, call: &ToolCall) -> ToolOutput {
        let Some(session_id) = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        ) else {
            return error("enter_plan_mode requires sessionId or session context");
        };
        let reason = string_arg(&call.args, "reason").unwrap_or("enter plan mode");
        let previous = self
            .governance
            .session_permission_mode(session_id)
            .await
            .unwrap_or(PermissionMode::Default);
        if previous == PermissionMode::Plan {
            return ToolOutput {
                content: format!("Already in plan mode. Reason noted:\n{reason}"),
                is_error: false,
                data: Some(json!({ "previous": "plan", "unchanged": true, "reason": reason })),
            };
        }
        self.governance
            .set_session_permission_mode(session_id, PermissionMode::Plan)
            .await;
        ToolOutput {
            content: format!(
                "Entered plan mode (was: {:?}).\n\nReason:\n{reason}\n\nWrite/exec tools are blocked. Call exit_plan_mode with a plan summary after the user approves.",
                previous
            ),
            is_error: false,
            data: Some(json!({
                "previous": format!("{previous:?}").to_ascii_lowercase(),
                "next": "plan",
                "reason": reason
            })),
        }
    }

    async fn exit_plan_mode(&self, call: &ToolCall) -> ToolOutput {
        let Some(session_id) = string_arg_any(
            &call.args,
            &["sessionId", "session_id", "_sessionId", "_session_id"],
        ) else {
            return error("exit_plan_mode requires sessionId or session context");
        };
        let plan = string_arg_any(&call.args, &["plan", "summary", "reason"]).unwrap_or("");
        self.governance
            .clear_session_permission_mode(session_id)
            .await;
        ToolOutput {
            content: if plan.is_empty() {
                "Exited plan mode. Write/exec tools follow normal permissions again.".to_string()
            } else {
                format!("Exited plan mode.\n\nPlan summary:\n{plan}")
            },
            is_error: false,
            data: Some(json!({ "next": "default", "plan": plan })),
        }
    }

    async fn activate_skill(&self, call: &ToolCall) -> ToolOutput {
        let Some(skill_id) = string_arg_any(&call.args, &["id", "skillId", "skill_id", "name"])
        else {
            return error("activate_skill requires id");
        };
        let config = ExtensionLoadConfig::from_app_config(call.cwd.clone(), &json!({}));
        let extensions = load_extensions(&config);
        let Some(skill) = extensions
            .skills
            .iter()
            .find(|skill| skill.id == skill_id || skill.name == skill_id)
        else {
            return error(format!("Skill not found: {skill_id}"));
        };
        let prompt = skill
            .prompt
            .clone()
            .unwrap_or_else(|| skill.description.clone());
        let tools = skill.tools.clone();
        ToolOutput {
            content: format!(
                "Activated skill `{}` ({})\n\nDeclared tools: {}\n\n{}",
                skill.id,
                skill.name,
                if tools.is_empty() {
                    "(none)".to_string()
                } else {
                    tools.join(", ")
                },
                prompt
            ),
            is_error: false,
            data: Some(json!({
                "skill": skill,
                "tools": tools,
            })),
        }
    }

    async fn resolve_existing_path(&self, cwd: &Path, input: &str) -> Result<PathBuf, String> {
        let candidate = if Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            cwd.join(input)
        };
        let canonical = tokio::fs::canonicalize(&candidate)
            .await
            .map_err(|err| format!("path does not exist: {} ({err})", candidate.display()))?;
        if self.is_path_allowed(cwd, &canonical).await {
            Ok(canonical)
        } else {
            Err(format!(
                "path is outside the allowed workspace: {}",
                canonical.display()
            ))
        }
    }

    async fn resolve_write_path(&self, cwd: &Path, input: &str) -> Result<PathBuf, String> {
        let input_path = Path::new(input);
        if input_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("write paths may not contain parent directory segments".to_string());
        }
        let candidate = if input_path.is_absolute() {
            input_path.to_path_buf()
        } else {
            cwd.join(input_path)
        };
        let anchor = nearest_existing_ancestor(&candidate).unwrap_or(cwd);
        let canonical_anchor = tokio::fs::canonicalize(anchor)
            .await
            .map_err(|err| format!("path anchor does not exist: {} ({err})", anchor.display()))?;
        if self.is_path_allowed(cwd, &canonical_anchor).await {
            Ok(candidate)
        } else {
            Err(format!(
                "path is outside the allowed workspace: {}",
                candidate.display()
            ))
        }
    }

    async fn ensure_write_scope(&self, call: &ToolCall, path: &Path) -> Result<(), String> {
        if bool_arg_any(
            &call.args,
            &["readonly", "readOnly", "_readonly", "_readOnly"],
        )
        .unwrap_or(false)
        {
            return Err("write denied: read-only tool context".to_string());
        }
        let write_scope = string_vec_arg_any(&call.args, &["writeScope", "write_scope"]);
        if write_scope.is_empty() {
            return Ok(());
        }
        let relative = write_scope_relative_path(&call.cwd, path).await;
        if write_scope
            .iter()
            .any(|pattern| glob_scope_matches(pattern, &relative))
        {
            return Ok(());
        }
        Err(format!(
            "write denied: {} is outside writeScope. Allowed: {}",
            relative,
            write_scope.join(", ")
        ))
    }

    async fn git_root_for_call(&self, cwd: &Path) -> Result<PathBuf, String> {
        let root = git_root(cwd)?;
        let canonical = tokio::fs::canonicalize(&root)
            .await
            .map_err(|err| format!("git root does not exist: {} ({err})", root.display()))?;
        if self.is_path_allowed(cwd, &canonical).await {
            Ok(canonical)
        } else {
            Err(format!(
                "git repository is outside the allowed workspace: {}",
                canonical.display()
            ))
        }
    }

    async fn workspace_cwd_for_call(&self, cwd: &Path) -> Result<PathBuf, String> {
        let canonical = tokio::fs::canonicalize(cwd)
            .await
            .map_err(|err| format!("cwd does not exist: {} ({err})", cwd.display()))?;
        if self.is_path_allowed(cwd, &canonical).await {
            Ok(canonical)
        } else {
            Err(format!(
                "cwd is outside the allowed workspace: {}",
                canonical.display()
            ))
        }
    }

    /** 允许根 = 会话 cwd（及 git 仓库根）+ 引擎配置的 workspace_roots（如 data_dir）。 */
    async fn allowed_roots_for_cwd(&self, cwd: &Path) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Ok(git) = git_root(cwd) {
            push_unique_path(&mut roots, git);
        }
        push_unique_path(&mut roots, cwd.to_path_buf());
        for root in self.workspace_roots() {
            push_unique_path(&mut roots, root);
        }
        roots
    }

    async fn is_path_allowed(&self, cwd: &Path, path: &Path) -> bool {
        let roots = self.allowed_roots_for_cwd(cwd).await;
        for root in roots {
            if let Ok(canonical_root) = tokio::fs::canonicalize(&root).await {
                if path_is_within(path, &canonical_root) {
                    return true;
                }
            }
            if path_is_within(path, &root) {
                return true;
            }
        }
        false
    }
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read a file with line numbers. Default window: 2000 lines / ~25k-token page (PARTIAL + continue). Images/PDF supported.".to_string(),
            category: Some("filesystem".to_string()),
            aliases: vec!["read_file".to_string()],
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 1 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 2000 },
                    "maxBytes": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Create or overwrite a UTF-8 file inside an allowed workspace."
                .to_string(),
            category: Some("filesystem".to_string()),
            aliases: vec!["write_file".to_string()],
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "file_edit".to_string(),
            description: "Edit a UTF-8 file by replacing one exact text match.".to_string(),
            category: Some("filesystem".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["path", "old_text", "new_text"],
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply structured add, update, or delete patches to workspace files."
                .to_string(),
            category: Some("filesystem".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["patches"],
                "properties": {
                    "patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "action"],
                            "properties": {
                                "path": { "type": "string" },
                                "action": { "type": "string", "enum": ["add", "delete", "update"] },
                                "content": { "type": "string" },
                                "old_text": { "type": "string" },
                                "new_text": { "type": "string" }
                            }
                        }
                    }
                }
            }),
        },
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files recursively inside an allowed workspace.".to_string(),
            category: Some("filesystem".to_string()),
            aliases: vec!["ls".to_string()],
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "search".to_string(),
            description: "Search text files for a regex pattern or literal query.".to_string(),
            category: Some("filesystem".to_string()),
            aliases: vec!["grep".to_string()],
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "query": { "type": "string" },
                    "path": { "type": "string" },
                    "include": { "type": "string" },
                    "case_insensitive": { "type": "boolean" },
                    "caseInsensitive": { "type": "boolean" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 500 },
                    "maxResults": { "type": "integer", "minimum": 1, "maximum": 500 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
                }
            }),
        },
        ToolDefinition {
            name: "glob".to_string(),
            description: "Find files by a simple wildcard pattern.".to_string(),
            category: Some("filesystem".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "codebase_index".to_string(),
            description:
                "Build or read a codebase index with files, symbols, references, and changes."
                    .to_string(),
            category: Some("search".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "refresh": { "type": "boolean" },
                    "max_files": { "type": "integer", "minimum": 1 },
                    "include": { "type": "array", "items": { "type": "string" } },
                    "summary_limit": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "git_status".to_string(),
            description: "Read structured Git status for the current repository.".to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "git_diff".to_string(),
            description: "Read Git diff output, optionally scoped by refs, paths, or stat mode."
                .to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "staged": { "type": "boolean" },
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "stat": { "type": "boolean" },
                    "context": { "type": "integer", "minimum": 0, "maximum": 100 }
                }
            }),
        },
        ToolDefinition {
            name: "git_log".to_string(),
            description: "Read recent Git commits with structured metadata.".to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
                    "ref": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
        ToolDefinition {
            name: "git_blame".to_string(),
            description: "Read Git blame metadata for a file or line range.".to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["file"],
                "properties": {
                    "file": { "type": "string" },
                    "startLine": { "type": "integer", "minimum": 1 },
                    "endLine": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "git_branch".to_string(),
            description: "List or create Git branches without switching the working tree."
                .to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create"] },
                    "name": { "type": "string" },
                    "baseRef": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "git_commit".to_string(),
            description: "Create a Git commit from selected files or all changes.".to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["message"],
                "properties": {
                    "message": { "type": "string" },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "all": { "type": "boolean" },
                    "allowEmpty": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "git_patch_create".to_string(),
            description: "Create a patch file from Git diff output for review or delivery."
                .to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "staged": { "type": "boolean" },
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "stat": { "type": "boolean" },
                    "context": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "includeUntracked": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "git_patch_apply".to_string(),
            description: "Apply or check a Git patch file or patch text.".to_string(),
            category: Some("git".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patchPath": { "type": "string" },
                    "patchText": { "type": "string" },
                    "reverse": { "type": "boolean" },
                    "checkOnly": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "validation_discover".to_string(),
            description: "Discover project typecheck, lint, and build commands.".to_string(),
            category: Some("validation".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "validation_diagnose".to_string(),
            description: "Parse a failed validation log into structured diagnosis.".to_string(),
            category: Some("validation".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["command", "output"],
                "properties": {
                    "command": { "type": "string" },
                    "output": { "type": "string" },
                    "type": { "type": "string", "enum": ["typecheck", "lint", "build", "unknown"] }
                }
            }),
        },
        ToolDefinition {
            name: "validation_replay".to_string(),
            description: "Replay typecheck/lint/test commands that were executed earlier in this turn.".to_string(),
            category: Some("validation".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: true,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["sinceMs"],
                "properties": {
                    "sinceMs": { "type": "number" },
                    "maxCommands": { "type": "number" },
                    "includeBuild": { "type": "boolean" },
                    "stopOnFailure": { "type": "boolean" },
                    "timeoutMs": { "type": "number" }
                }
            }),
        },
        ToolDefinition {
            name: "acceptance_check".to_string(),
            description: "Bind acceptance criteria to evidence and report missing proof."
                .to_string(),
            category: Some("validation".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["criteria"],
                "properties": {
                    "criteria": { "type": "array", "items": { "type": "string" } },
                    "modifiedFiles": { "type": "array", "items": { "type": "string" } },
                    "manualEvidence": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
        ToolDefinition {
            name: "self_review".to_string(),
            description: "Run a final self-review for goal fit, scope, validation, and file risk."
                .to_string(),
            category: Some("validation".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": { "type": "string" },
                    "modifiedFiles": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
        ToolDefinition {
            name: "write_memory".to_string(),
            description: "Create or replace a durable memory profile (identity, user, project, workflow, agents, lessons). Use write_memory only — not file_write/file_edit. To read profiles, call load_memories([\"memory_profile\"]).".to_string(),
            category: Some("memory".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["kind", "content"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["identity", "user", "project", "agents", "workflow", "lessons"]
                    },
                    "content": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web through a configured Brave, Bing, or SerpAPI provider."
                .to_string(),
            category: Some("web".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "num_results": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "provider": { "type": "string", "enum": ["brave", "bing", "serpapi"] }
                }
            }),
        },
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch an HTTP(S) page or PDF URL and extract readable text.".to_string(),
            category: Some("web".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "format": "uri" },
                    "max_chars": { "type": "integer", "minimum": 1000, "maximum": 64000 },
                    "maxChars": { "type": "integer", "minimum": 1000, "maximum": 64000 }
                }
            }),
        },
        ToolDefinition {
            name: "sandbox_modes".to_string(),
            description: "Describe standardized local sandbox modes and policies.".to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "sandbox_create".to_string(),
            description: "Create an isolated Git worktree sandbox for risky or complex edits."
                .to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "baseRef": { "type": "string" },
                    "rootDir": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "sandbox_list".to_string(),
            description: "List Git worktree sandboxes for this project.".to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "rootDir": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "sandbox_diff".to_string(),
            description: "Generate a patch diff from a sandbox against its base commit."
                .to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["sandboxId"],
                "properties": {
                    "sandboxId": { "type": "string" },
                    "rootDir": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "sandbox_apply".to_string(),
            description: "Apply a sandbox patch to the main worktree after review.".to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["sandboxId"],
                "properties": {
                    "sandboxId": { "type": "string" },
                    "rootDir": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "sandbox_rollback".to_string(),
            description: "Roll back an applied sandbox patch and optionally remove the worktree."
                .to_string(),
            category: Some("sandbox".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["sandboxId"],
                "properties": {
                    "sandboxId": { "type": "string" },
                    "removeWorktree": { "type": "boolean" },
                    "rootDir": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "workspace_checkpoint_create".to_string(),
            description:
                "Capture the current Git workspace state for a session turn as a restorable checkpoint."
                    .to_string(),
            category: Some("checkpoint".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "session_id": { "type": "string" },
                    "turnId": { "type": "string" },
                    "turn_id": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "workspace_checkpoint_list".to_string(),
            description: "List stored workspace checkpoints for a session.".to_string(),
            category: Some("checkpoint".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "session_id": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "workspace_checkpoint_restore".to_string(),
            description:
                "Restore a session turn checkpoint by resetting to its Git head and applying the saved patch."
                    .to_string(),
            category: Some("checkpoint".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["sessionId", "turnId"],
                "properties": {
                    "sessionId": { "type": "string" },
                    "session_id": { "type": "string" },
                    "turnId": { "type": "string" },
                    "turn_id": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "submit_task".to_string(),
            description:
                "Submit one or more background sub-agent tasks (also available as alias `agent`)."
                    .to_string(),
            category: Some("system".to_string()),
            aliases: vec!["agent".to_string()],
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "session_id": { "type": "string" },
                    "type": { "type": "string", "enum": ["coding", "shell", "review", "explore", "plan"] },
                    "mainBrainIntent": { "type": "string" },
                    "main_brain_intent": { "type": "string" },
                    "agentId": { "type": "string" },
                    "agent_id": { "type": "string" },
                    "task": { "type": "string" },
                    "goal": { "type": "string" },
                    "context": { "type": "string" },
                    "acceptance": { "type": "array", "items": { "type": "string" } },
                    "strategy": { "type": "string" },
                    "priority": { "type": "number" },
                    "timeoutMs": { "type": "number" },
                    "timeout_ms": { "type": "number" },
                    "writeScope": { "type": "array", "items": { "type": "string" } },
                    "write_scope": { "type": "array", "items": { "type": "string" } },
                    "agents": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["agentId", "task"],
                            "properties": {
                                "agentId": { "type": "string" },
                                "task": { "type": "string" },
                                "brief": { "type": "string" },
                                "context": { "type": "string" },
                                "writeScope": { "type": "array", "items": { "type": "string" } },
                                "readScope": { "type": "array", "items": { "type": "string" } }
                            }
                        }
                    },
                    "subTasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["agentId", "task"],
                            "properties": {
                                "agentId": { "type": "string" },
                                "agent_id": { "type": "string" },
                                "task": { "type": "string" },
                                "context": { "type": "string" },
                                "writeScope": { "type": "array", "items": { "type": "string" } },
                                "write_scope": { "type": "array", "items": { "type": "string" } }
                            }
                        }
                    },
                    "sub_tasks": { "type": "array" }
                }
            }),
        },
        ToolDefinition {
            name: "todo_write".to_string(),
            description:
                "Work checklist for this task (steps). Use for multi-step boards; user-pinned objectives use /goal and update_goal."
                    .to_string(),
            category: Some("runtime".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["todos"],
                "properties": {
                    "merge": { "type": "boolean" },
                    "todos": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "required": ["id", "content", "status"],
                            "properties": {
                                "id": { "type": "string" },
                                "content": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed", "cancelled"]
                                }
                            }
                        }
                    }
                }
            }),
        },
        ToolDefinition {
            name: "update_goal".to_string(),
            description:
                "Mark the user-pinned objective (/goal) complete after verification with evidence_summary."
                    .to_string(),
            category: Some("runtime".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["status", "evidence_summary"],
                "properties": {
                    "status": { "type": "string", "enum": ["complete"] },
                    "evidence_summary": { "type": "string" },
                    "evidenceSummary": { "type": "string" },
                    "progress_note": { "type": "string" },
                    "progressNote": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "enter_plan_mode".to_string(),
            description:
                "Enter plan mode for read-only exploration. Write/exec stay blocked until exit_plan_mode."
                    .to_string(),
            category: Some("runtime".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["reason"],
                "properties": {
                    "reason": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "exit_plan_mode".to_string(),
            description: "Exit plan mode after the user approves a plan summary.".to_string(),
            category: Some("runtime".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "plan": { "type": "string" },
                    "summary": { "type": "string" },
                    "reason": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "activate_skill".to_string(),
            description:
                "Load a skill package by id and return its full prompt plus declared tools."
                    .to_string(),
            category: Some("meta".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "skillId": { "type": "string" },
                    "name": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "get_task_status".to_string(),
            description: "Get a sub-agent task ledger from the Rust task control plane.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["taskId"],
                "properties": {
                    "taskId": { "type": "string" },
                    "task_id": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "List sub-agent task ledgers from the Rust task control plane.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "session_id": { "type": "string" },
                    "limit": { "type": "number" }
                }
            }),
        },
        ToolDefinition {
            name: "cancel_task".to_string(),
            description: "Cancel a sub-agent task in the Rust task control plane.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["taskId"],
                "properties": {
                    "taskId": { "type": "string" },
                    "task_id": { "type": "string" },
                    "reason": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "delivery_create_local_pr".to_string(),
            description:
                "Generate a local PR-style delivery package with markdown, HTML, patch, and commit message."
                    .to_string(),
            category: Some("delivery".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "background": { "type": "string" },
                    "summary": { "type": "array", "items": { "type": "string" } },
                    "commitMessage": { "type": "string" },
                    "diff": { "type": "string" },
                    "includeUntracked": { "type": "boolean" },
                    "issue": { "type": "string" },
                    "platform": { "type": "string", "enum": ["github", "gitlab"] },
                    "branch": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "git_platform_status".to_string(),
            description:
                "Inspect optional GitHub/GitLab integration readiness for delivery workflows."
                    .to_string(),
            category: Some("delivery".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "platform": { "type": "string", "enum": ["github", "gitlab"] }
                }
            }),
        },
        ToolDefinition {
            name: "git_issue_task_draft".to_string(),
            description:
                "Create a local task draft from a GitHub/GitLab issue with branch and command suggestions."
                    .to_string(),
            category: Some("delivery".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["platform", "issue"],
                "properties": {
                    "platform": { "type": "string", "enum": ["github", "gitlab"] },
                    "issue": { "type": "string" },
                    "title": { "type": "string" },
                    "branch": { "type": "string" },
                    "body": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "ask_user".to_string(),
            description: "Ask the user one or more structured clarifying questions and wait for answers before continuing. Prefer options when choices are discrete. Do not use for routine narration.".to_string(),
            category: Some("meta".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["questions"],
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "required": ["id", "prompt"],
                            "properties": {
                                "id": { "type": "string", "minLength": 1 },
                                "prompt": { "type": "string", "minLength": 1 },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "maxItems": 8,
                                    "items": { "type": "string", "minLength": 1 }
                                },
                                "allow_multiple": { "type": "boolean" },
                                "allowMultiple": { "type": "boolean" }
                            }
                        }
                    }
                }
            }),
        },
        ToolDefinition {
            name: "search_tools".to_string(),
            description:
                "Search deferred tools by keyword and return lightweight metadata. Prefer the Tool catalog + load_tools for normal work."
                    .to_string(),
            category: Some("discovery".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "includeCrossSession": {
                        "type": "boolean",
                        "description": "Search memories from other sessions as well as current-session and global memories."
                    }
                }
            }),
        },
        ToolDefinition {
            name: "load_tools".to_string(),
            description:
                "Activate deferred tools by exact name from the Tool catalog so their full schemas are available on the next model iteration. Required before calling non-core tools."
                    .to_string(),
            category: Some("meta".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["names"],
                "properties": {
                    "names": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 10,
                        "items": { "type": "string" }
                    }
                }
            }),
        },
        ToolDefinition {
            name: "search_memory".to_string(),
            description:
                "Search long-term memory by keyword and return lightweight matches."
                    .to_string(),
            category: Some("meta".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                }
            }),
        },
        ToolDefinition {
            name: "load_memories".to_string(),
            description:
                "Load context sections: memory, memory_profile, or project_instructions. Skills use activate_skill instead."
                    .to_string(),
            category: Some("meta".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["sections"],
                "properties": {
                    "sections": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "string",
                            "enum": ["memory", "memory_profile", "project_instructions"]
                        }
                    },
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "includePrompt": { "type": "boolean" },
                    "include_prompt": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "exec".to_string(),
            description: "Run a short shell command and return combined stdout/stderr.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: true,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "timeout": { "type": "integer", "minimum": 1 },
                    "backgroundOnTimeout": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "start_process".to_string(),
            description: "Start a long-running background process and keep its output readable."
                .to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: true,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "name": { "type": "string" },
                    "startupTimeout": { "type": "integer", "minimum": 0 }
                }
            }),
        },
        ToolDefinition {
            name: "list_processes".to_string(),
            description: "List background processes started by exec (background=true or auto-background on timeout) or start_process. Prefer status=running.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["running", "exited", "failed", "all"]
                    }
                }
            }),
        },
        ToolDefinition {
            name: "read_process_output".to_string(),
            description: "Read buffered output from a background process.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: false,
            is_execute: false,
            is_concurrent: true,
            input_schema: json!({
                "type": "object",
                "required": ["processId"],
                "properties": {
                    "processId": { "type": "string" },
                    "offset": { "type": "integer", "minimum": 0 },
                    "maxBytes": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDefinition {
            name: "write_process_stdin".to_string(),
            description: "Write to a background process stdin; set eof to close input.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: true,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["processId"],
                "properties": {
                    "processId": { "type": "string" },
                    "data": { "type": "string" },
                    "input": { "type": "string" },
                    "text": { "type": "string" },
                    "eof": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: "stop_process".to_string(),
            description: "Stop a background process started by start_process.".to_string(),
            category: Some("system".to_string()),
            aliases: Vec::new(),
            is_write: true,
            is_execute: true,
            is_concurrent: false,
            input_schema: json!({
                "type": "object",
                "required": ["processId"],
                "properties": {
                    "processId": { "type": "string" }
                }
            }),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WebSearchResult {
    title: String,
    url: String,
    snippet: String,
    source: String,
}

#[derive(Debug, Clone)]
struct WebDocument {
    url: String,
    content_type: String,
    text: String,
    truncated: bool,
    chars: usize,
    redirects: usize,
}

#[derive(Debug, Clone)]
struct CurlResponse {
    status: u16,
    content_type: String,
    redirect_url: Option<String>,
    body: Vec<u8>,
    truncated: bool,
}

const WEB_FETCH_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const WEB_ERROR_BODY_LIMIT: usize = 16 * 1024;
const WEB_MAX_REDIRECTS: usize = 5;
const CURL_TIMEOUT_SECONDS: &str = "15";

fn run_web_search(
    provider: &str,
    api_key: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchResult>, String> {
    match provider {
        "bing" => search_bing(api_key, query, limit),
        "serpapi" => search_serpapi(api_key, query, limit),
        "brave" => search_brave(api_key, query, limit),
        _ => Err(format!("unsupported provider: {provider}")),
    }
}

fn search_brave(api_key: &str, query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let mut url = url::Url::parse("https://api.search.brave.com/res/v1/web/search")
        .map_err(|err| format!("invalid Brave search URL: {err}"))?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("q", query);
        params.append_pair("count", &limit.to_string());
    }
    let data = fetch_json(url, &[("X-Subscription-Token", api_key)])?;
    Ok(data
        .pointer("/web/results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit)
        .map(|item| WebSearchResult {
            title: json_string(item.get("title")),
            url: json_string(item.get("url")),
            snippet: strip_html(&json_string(item.get("description"))),
            source: "brave".to_string(),
        })
        .collect())
}

fn search_bing(api_key: &str, query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let mut url = url::Url::parse("https://api.bing.microsoft.com/v7.0/search")
        .map_err(|err| format!("invalid Bing search URL: {err}"))?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("q", query);
        params.append_pair("count", &limit.to_string());
    }
    let data = fetch_json(url, &[("Ocp-Apim-Subscription-Key", api_key)])?;
    Ok(data
        .pointer("/webPages/value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit)
        .map(|item| WebSearchResult {
            title: json_string(item.get("name")),
            url: json_string(item.get("url")),
            snippet: strip_html(&json_string(item.get("snippet"))),
            source: "bing".to_string(),
        })
        .collect())
}

fn search_serpapi(
    api_key: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let mut url = url::Url::parse("https://serpapi.com/search.json")
        .map_err(|err| format!("invalid SerpAPI search URL: {err}"))?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("q", query);
        params.append_pair("api_key", api_key);
        params.append_pair("num", &limit.to_string());
    }
    let data = fetch_json(url, &[])?;
    Ok(data
        .get("organic_results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit)
        .map(|item| WebSearchResult {
            title: json_string(item.get("title")),
            url: json_string(item.get("link")),
            snippet: strip_html(&json_string(item.get("snippet"))),
            source: "serpapi".to_string(),
        })
        .collect())
}

fn fetch_json(url: url::Url, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut request_headers = vec![("Accept", "application/json")];
    request_headers.extend_from_slice(headers);
    let response = curl_http_request(&url, &request_headers, WEB_FETCH_MAX_BODY_BYTES)?;
    if !http_success(response.status) {
        let text = String::from_utf8_lossy(&response.body).to_string();
        return Err(format!(
            "HTTP {}: {}",
            response.status,
            truncate_text(&text, 300)
        ));
    }
    serde_json::from_slice::<Value>(&response.body)
        .map_err(|err| format!("invalid JSON response: {err}"))
}

fn curl_http_request(
    url: &url::Url,
    headers: &[(&str, &str)],
    max_bytes: usize,
) -> Result<CurlResponse, String> {
    let body_path =
        std::env::temp_dir().join(format!("xuanyu-web-{}.body", uuid::Uuid::new_v4().simple()));
    let mut command = Command::new("curl");
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg(CURL_TIMEOUT_SECONDS)
        .arg("--proto")
        .arg("=http,https")
        .arg("--max-filesize")
        .arg(max_bytes.to_string())
        .arg("--user-agent")
        .arg("XuanYu-Agent/1.0 (+https://local)")
        .arg("--output")
        .arg(&body_path)
        .arg("--write-out")
        .arg("%{http_code}\n%{content_type}\n%{redirect_url}");
    for (name, value) in headers {
        command.arg("--header").arg(format!("{name}: {value}"));
    }
    command.arg(url.as_str());

    let output = command
        .output()
        .map_err(|err| format!("failed to run curl: {err}"))?;
    let body = std::fs::read(&body_path).unwrap_or_default();
    let _ = std::fs::remove_file(&body_path);
    let exit_code = output.status.code();
    let truncated = exit_code == Some(63);
    if !output.status.success() && !truncated {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "curl failed{}{}",
            exit_code
                .map(|code| format!(" with exit code {code}"))
                .unwrap_or_default(),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }

    let metadata = String::from_utf8_lossy(&output.stdout);
    let mut lines = metadata.lines();
    let status = lines
        .next()
        .and_then(|line| line.trim().parse::<u16>().ok())
        .ok_or_else(|| "curl did not report an HTTP status".to_string())?;
    let content_type = lines.next().unwrap_or_default().trim().to_string();
    let redirect_url = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string);

    Ok(CurlResponse {
        status,
        content_type,
        redirect_url,
        body,
        truncated,
    })
}

fn http_success(status: u16) -> bool {
    (200..=299).contains(&status)
}

fn http_redirect(status: u16) -> bool {
    (300..=399).contains(&status)
}

fn search_provider_api_key<F>(provider: &str, mut lookup: F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    lookup("XY_WEB_SEARCH_API_KEY")
        .and_then(non_empty_string)
        .or_else(|| {
            let provider_key = match provider {
                "bing" => "BING_SEARCH_API_KEY",
                "serpapi" => "SERPAPI_API_KEY",
                _ => "BRAVE_SEARCH_API_KEY",
            };
            lookup(provider_key).and_then(non_empty_string)
        })
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_search_results(query: &str, provider: &str, results: &[WebSearchResult]) -> String {
    if results.is_empty() {
        return format!("No web results found for \"{query}\" via {provider}.");
    }
    let mut lines = vec![
        format!("Web search results for \"{query}\" via {provider}:"),
        String::new(),
    ];
    for (index, result) in results.iter().enumerate() {
        lines.push(format!("{}. {}", index + 1, result.title));
        lines.push(format!("   URL: {}", result.url));
        lines.push(format!(
            "   Snippet: {}",
            if result.snippet.is_empty() {
                "(none)"
            } else {
                result.snippet.as_str()
            }
        ));
    }
    lines.join("\n")
}

fn fetch_web_document(raw_url: &str, max_chars: usize) -> Result<WebDocument, String> {
    let mut url = validate_public_web_url(raw_url)?;
    let mut redirects = 0;

    loop {
        let response = curl_http_request(
            &url,
            &[(
                "Accept",
                "text/html,application/pdf,text/plain;q=0.9,*/*;q=0.8",
            )],
            WEB_FETCH_MAX_BODY_BYTES,
        )?;
        if http_redirect(response.status) {
            redirects += 1;
            if redirects > WEB_MAX_REDIRECTS {
                return Err(format!("too many redirects (>{WEB_MAX_REDIRECTS})"));
            }
            let location = response.redirect_url.ok_or_else(|| {
                format!("redirect HTTP {} missing Location header", response.status)
            })?;
            let redirect_url = url
                .join(&location)
                .map_err(|err| format!("invalid redirect URL: {err}"))?;
            url = validate_public_web_url(redirect_url.as_str())
                .map_err(|err| format!("Redirect blocked (SSRF): {err}"))?;
            continue;
        }
        if !http_success(response.status) {
            let text = String::from_utf8_lossy(
                &response.body[..response.body.len().min(WEB_ERROR_BODY_LIMIT)],
            )
            .to_string();
            return Err(format!(
                "HTTP {} for {}{}",
                response.status,
                url,
                if text.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", truncate_text(&text, 300))
                }
            ));
        }

        let content_type_lower = response.content_type.to_ascii_lowercase();
        let raw = String::from_utf8_lossy(&response.body).to_string();
        let extracted = if content_type_lower.contains("pdf") {
            extract_pdf_text(&raw)
        } else if content_type_lower.contains("html") {
            extract_html_text(&raw)
        } else {
            compress_whitespace(&raw)
        };
        let (text, char_truncated, chars) = limit_chars(&extracted, max_chars);
        return Ok(WebDocument {
            url: url.to_string(),
            content_type: response.content_type,
            text,
            truncated: response.truncated || char_truncated,
            chars,
            redirects,
        });
    }
}

fn validate_public_web_url(raw_url: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw_url).map_err(|_| "Invalid URL format".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "Blocked protocol: {} (only http/https allowed)",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "Blocked URL without hostname".to_string())?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(&host)
        .to_string();
    if matches!(
        host.as_str(),
        "localhost" | "metadata.google.internal" | "instance-data"
    ) {
        return Err(format!("Blocked hostname: {host}"));
    }
    if host == "169.254.169.254" {
        return Err("Blocked: cloud metadata endpoint".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if let Some(reason) = blocked_ip_reason(ip) {
            return Err(format!("Blocked: {reason} ({host})"));
        }
    }
    Ok(parsed)
}

fn blocked_ip_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => blocked_ipv4_reason(ip),
        IpAddr::V6(ip) => blocked_ipv6_reason(ip),
    }
}

fn blocked_ipv4_reason(ip: Ipv4Addr) -> Option<&'static str> {
    let octets = ip.octets();
    if octets[0] == 10
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 168)
    {
        return Some("private IP range");
    }
    if octets[0] == 127 {
        return Some("loopback IP range");
    }
    if octets[0] == 0 {
        return Some("unspecified IP range");
    }
    if octets[0] == 169 && octets[1] == 254 {
        return Some("link-local IP range");
    }
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some("shared address space");
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return Some("documentation IP range");
    }
    if octets[0] == 198 && octets[1] == 51 && octets[2] == 100 {
        return Some("documentation IP range");
    }
    if octets[0] == 203 && octets[1] == 0 && octets[2] == 113 {
        return Some("documentation IP range");
    }
    if octets[0] >= 224 {
        return Some("multicast/reserved IP range");
    }
    None
}

fn blocked_ipv6_reason(ip: Ipv6Addr) -> Option<&'static str> {
    let segments = ip.segments();
    let first = segments[0];
    if ip.is_loopback() {
        return Some("IPv6 loopback");
    }
    if ip.is_unspecified() {
        return Some("IPv6 unspecified address");
    }
    if (first & 0xfe00) == 0xfc00 {
        return Some("IPv6 unique local address");
    }
    if (first & 0xffc0) == 0xfe80 {
        return Some("IPv6 link-local address");
    }
    if (first & 0xff00) == 0xff00 {
        return Some("IPv6 multicast address");
    }
    None
}

fn extract_html_text(html: &str) -> String {
    let without_scripts = remove_html_block(html, "script");
    let without_styles = remove_html_block(&without_scripts, "style");
    let without_nav = remove_html_block(&without_styles, "nav");
    let without_footer = remove_html_block(&without_nav, "footer");
    strip_html(&without_footer)
}

fn remove_html_block(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative_start) = lower[index..].find(&open) {
        let start = index + relative_start;
        output.push_str(&input[index..start]);
        if let Some(relative_end) = lower[start..].find(&close) {
            index = start + relative_end + close.len();
        } else {
            index = input.len();
        }
        output.push(' ');
    }
    output.push_str(&input[index..]);
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaKind {
    Image,
    Pdf,
    Text,
    Binary,
}

fn detect_media_kind(path: &Path, bytes: &[u8]) -> MediaKind {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_pdf_magic(bytes) || ext == "pdf" {
        return MediaKind::Pdf;
    }
    if resolve_image_mime(path, bytes).is_some() {
        return MediaKind::Image;
    }
    if looks_binary_bytes(bytes) {
        return MediaKind::Binary;
    }
    MediaKind::Text
}

fn is_pdf_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
}

fn resolve_image_mime(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png");
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn looks_binary_bytes(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8_192)];
    if sample.contains(&0) {
        return true;
    }
    let non_text = sample
        .iter()
        .filter(|b| {
            let ch = **b;
            !(ch == b'\n' || ch == b'\r' || ch == b'\t' || (0x20..=0x7e).contains(&ch) || ch >= 0x80)
        })
        .count();
    non_text * 10 > sample.len()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn extract_pdf_text_from_bytes(bytes: &[u8]) -> String {
    extract_pdf_text(&String::from_utf8_lossy(bytes))
}

fn extract_pdf_text(raw: &str) -> String {
    let printable = raw
        .chars()
        .map(|ch| if is_printable_pdf_char(ch) { ch } else { ' ' })
        .collect::<String>();
    compress_whitespace(&printable)
}

fn strip_html(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => {
                in_tag = true;
                output.push(' ');
            }
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    compress_whitespace(&decode_html_entities(&output))
}

fn decode_html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn compress_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_printable_pdf_char(ch: char) -> bool {
    let code = ch as u32;
    code == 9
        || code == 10
        || code == 13
        || (0x20..=0x7e).contains(&code)
        || (0x4e00..=0x9fff).contains(&code)
}

fn json_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn limit_chars(value: &str, max_chars: usize) -> (String, bool, usize) {
    let mut output = String::new();
    let mut count = 0;
    for ch in value.chars() {
        if count >= max_chars {
            return (output, true, count);
        }
        output.push(ch);
        count += 1;
    }
    (output, false, count)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SandboxModePolicy {
    mode: String,
    read_scope: String,
    write_scope: String,
    network: String,
    background_processes: bool,
    environment: String,
    requires_approval: bool,
    runner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SandboxRecord {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    project_root: String,
    worktree_path: String,
    branch: String,
    base_ref: String,
    base_commit: String,
    status: String,
    created_at: i64,
    updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    applied_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rolled_back_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SandboxIndex {
    version: u32,
    sandboxes: Vec<SandboxRecord>,
}

impl Default for SandboxIndex {
    fn default() -> Self {
        Self {
            version: 1,
            sandboxes: Vec::new(),
        }
    }
}

fn describe_sandbox_mode(mode: &str) -> SandboxModePolicy {
    match mode {
        "readonly" => SandboxModePolicy {
            mode: "readonly".to_string(),
            read_scope: "workspace and configured allowlist".to_string(),
            write_scope: "none".to_string(),
            network: "off".to_string(),
            background_processes: false,
            environment: "filtered".to_string(),
            requires_approval: false,
            runner: "local".to_string(),
        },
        "worktree-write" => SandboxModePolicy {
            mode: "worktree-write".to_string(),
            read_scope: "workspace and isolated worktree".to_string(),
            write_scope: "isolated git worktree".to_string(),
            network: "package-registry-only".to_string(),
            background_processes: true,
            environment: "filtered".to_string(),
            requires_approval: true,
            runner: "worktree".to_string(),
        },
        "container" => SandboxModePolicy {
            mode: "container".to_string(),
            read_scope: "mounted workspace".to_string(),
            write_scope: "container filesystem or mounted worktree".to_string(),
            network: "off".to_string(),
            background_processes: true,
            environment: "filtered".to_string(),
            requires_approval: true,
            runner: "container".to_string(),
        },
        _ => SandboxModePolicy {
            mode: "workspace-write".to_string(),
            read_scope: "workspace and configured allowlist".to_string(),
            write_scope: "workspace".to_string(),
            network: "package-registry-only".to_string(),
            background_processes: true,
            environment: "filtered".to_string(),
            requires_approval: true,
            runner: "local".to_string(),
        },
    }
}

fn create_sandbox(
    cwd: &Path,
    name: Option<&str>,
    base_ref: &str,
    root_dir: Option<&str>,
) -> Result<SandboxRecord, String> {
    let project_root = ensure_git_repository(&resolve_sandbox_project_root(cwd)?)?;
    let base_ref = safe_git_ref_arg(base_ref)?;
    let base_commit = git_output(&project_root, &["rev-parse", &base_ref])?;
    let id = create_sandbox_id(name);
    let branch = format!("xy-sandbox/{id}");
    let sandboxes_root = sandbox_root(&project_root, root_dir);
    std::fs::create_dir_all(&sandboxes_root)
        .map_err(|err| format!("failed to create sandbox root: {err}"))?;
    let worktree_path = sandboxes_root.join(&id);

    let args = vec![
        "worktree".to_string(),
        "add".to_string(),
        "-b".to_string(),
        branch.clone(),
        path_to_string(&worktree_path),
        base_ref.clone(),
    ];
    if let Err(err) = git_output_owned(&project_root, &args) {
        let _ = git_output_owned(
            &project_root,
            &[
                "worktree".to_string(),
                "remove".to_string(),
                "--force".to_string(),
                path_to_string(&worktree_path),
            ],
        );
        return Err(err);
    }

    let now = now_ms();
    let record = SandboxRecord {
        id,
        name: name.map(str::to_string),
        project_root: path_to_string(&project_root),
        worktree_path: path_to_string(&worktree_path),
        branch,
        base_ref,
        base_commit,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        applied_at: None,
        rolled_back_at: None,
        patch_path: None,
    };
    upsert_sandbox(&record, root_dir)?;
    Ok(record)
}

fn list_sandboxes(cwd: &Path, root_dir: Option<&str>) -> Result<Vec<SandboxRecord>, String> {
    let project_root = resolve_sandbox_project_root(cwd)?;
    let mut records = read_sandbox_index(&project_root, root_dir).sandboxes;
    records.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    Ok(records)
}

fn get_sandbox(
    cwd: &Path,
    sandbox_id: &str,
    root_dir: Option<&str>,
) -> Result<SandboxRecord, String> {
    let project_root = resolve_sandbox_project_root(cwd)?;
    let sandbox_id = safe_sandbox_id(sandbox_id)?;
    read_sandbox_index(&project_root, root_dir)
        .sandboxes
        .into_iter()
        .find(|record| record.id == sandbox_id)
        .ok_or_else(|| format!("Sandbox not found: {sandbox_id}"))
}

fn diff_sandbox(
    cwd: &Path,
    sandbox_id: &str,
    root_dir: Option<&str>,
) -> Result<(SandboxRecord, String, PathBuf), String> {
    let mut record = get_sandbox(cwd, sandbox_id, root_dir)?;
    let worktree_path = PathBuf::from(&record.worktree_path);
    if !worktree_path.exists() {
        return Err(format!(
            "Sandbox worktree is missing: {}",
            record.worktree_path
        ));
    }
    git_output(&worktree_path, &["add", "-A"])?;
    let diff = git_output_owned_raw(
        &worktree_path,
        &[
            "diff".to_string(),
            "--cached".to_string(),
            "--binary".to_string(),
            record.base_commit.clone(),
            "--".to_string(),
        ],
    )?;
    let patch_path = write_sandbox_patch(&record, &diff, root_dir)?;
    record.patch_path = Some(path_to_string(&patch_path));
    record.updated_at = now_ms();
    upsert_sandbox(&record, root_dir)?;
    Ok((record, diff, patch_path))
}

fn apply_sandbox(
    cwd: &Path,
    sandbox_id: &str,
    root_dir: Option<&str>,
) -> Result<(SandboxRecord, String, PathBuf), String> {
    let (mut record, diff, patch_path) = diff_sandbox(cwd, sandbox_id, root_dir)?;
    if diff.trim().is_empty() {
        return Err(format!("Sandbox {sandbox_id} has no changes to apply"));
    }
    let project_root = PathBuf::from(&record.project_root);
    let patch = path_to_string(&patch_path);
    git_output(&project_root, &["apply", "--check", &patch])?;
    git_output(&project_root, &["apply", &patch])?;
    let now = now_ms();
    record.status = "applied".to_string();
    record.applied_at = Some(now);
    record.updated_at = now;
    record.patch_path = Some(patch);
    upsert_sandbox(&record, root_dir)?;
    Ok((record, diff, patch_path))
}

fn rollback_sandbox(
    cwd: &Path,
    sandbox_id: &str,
    remove_worktree: bool,
    root_dir: Option<&str>,
) -> Result<SandboxRecord, String> {
    let mut record = get_sandbox(cwd, sandbox_id, root_dir)?;
    let patch_path = match record.patch_path.clone() {
        Some(path) => PathBuf::from(path),
        None => {
            let (_, _, patch_path) = diff_sandbox(cwd, sandbox_id, root_dir)?;
            patch_path
        }
    };
    if record.status == "applied" && patch_path.exists() {
        let project_root = PathBuf::from(&record.project_root);
        let patch = path_to_string(&patch_path);
        git_output(&project_root, &["apply", "--reverse", "--check", &patch])?;
        git_output(&project_root, &["apply", "--reverse", &patch])?;
    }
    if remove_worktree {
        remove_sandbox_worktree(&record)?;
    }
    let now = now_ms();
    record.status = if remove_worktree {
        "removed".to_string()
    } else {
        "rolled_back".to_string()
    };
    record.rolled_back_at = Some(now);
    record.updated_at = now;
    record.patch_path = Some(path_to_string(&patch_path));
    upsert_sandbox(&record, root_dir)?;
    Ok(record)
}

fn resolve_sandbox_project_root(cwd: &Path) -> Result<PathBuf, String> {
    let root = git_root(cwd)?;
    if is_sandbox_worktree_path(&root) {
        primary_worktree_root(&root).unwrap_or(Ok(root))
    } else {
        Ok(root)
    }
}

fn primary_worktree_root(cwd: &Path) -> Option<Result<PathBuf, String>> {
    let output = git_output(cwd, &["worktree", "list", "--porcelain"]).ok()?;
    let marker = normalize_fs_path_text(&runtime_data_root().join("sandboxes"));
    for line in output.lines() {
        let Some(path) = line.strip_prefix("worktree ") else {
            continue;
        };
        let path = PathBuf::from(path.trim());
        if !normalize_fs_path_text(&path).contains(&marker) {
            return Some(Ok(path));
        }
    }
    None
}

fn is_sandbox_worktree_path(path: &Path) -> bool {
    let normalized = normalize_fs_path_text(path);
    let marker = normalize_fs_path_text(&runtime_data_root().join("sandboxes"));
    normalized.contains(&marker)
}

fn ensure_git_repository(project_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(project_dir)
        .map_err(|err| format!("failed to create project directory: {err}"))?;
    if git_root(project_dir).is_err() {
        git_output(project_dir, &["init"])?;
    }
    let root = git_root(project_dir)?;
    configure_git_identity(&root);
    if git_output(&root, &["rev-parse", "--verify", "HEAD"]).is_err() {
        git_output(&root, &["add", "-A"])?;
        let staged = git_output(&root, &["diff", "--cached", "--name-only"]).unwrap_or_default();
        if staged.trim().is_empty() {
            git_output(
                &root,
                &[
                    "commit",
                    "--allow-empty",
                    "-m",
                    "chore: initialize sandbox repository",
                ],
            )?;
        } else {
            git_output(
                &root,
                &["commit", "-m", "chore: initialize sandbox repository"],
            )?;
        }
    }
    Ok(root)
}

fn configure_git_identity(root: &Path) {
    let _ = git_output(root, &["config", "user.name", "XuanYu Agent"]);
    let _ = git_output(root, &["config", "user.email", "xuanyu@local"]);
}

fn sandbox_root(project_root: &Path, root_dir: Option<&str>) -> PathBuf {
    match root_dir.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                project_root.join(path)
            }
        }
        None => runtime_data_root().join("sandboxes"),
    }
}

fn sandbox_index_path(project_root: &Path, root_dir: Option<&str>) -> PathBuf {
    sandbox_root(project_root, root_dir).join("index.json")
}

fn sandbox_patch_dir(project_root: &Path, root_dir: Option<&str>) -> PathBuf {
    sandbox_root(project_root, root_dir).join("patches")
}

fn read_sandbox_index(project_root: &Path, root_dir: Option<&str>) -> SandboxIndex {
    let path = sandbox_index_path(project_root, root_dir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<SandboxIndex>(&text).ok())
        .unwrap_or_default()
}

fn write_sandbox_index(
    project_root: &Path,
    index: &SandboxIndex,
    root_dir: Option<&str>,
) -> Result<(), String> {
    let path = sandbox_index_path(project_root, root_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create sandbox index directory: {err}"))?;
    }
    let text = serde_json::to_string_pretty(index)
        .map_err(|err| format!("failed to serialize sandbox index: {err}"))?;
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|err| format!("failed to write sandbox index {}: {err}", path.display()))
}

fn upsert_sandbox(record: &SandboxRecord, root_dir: Option<&str>) -> Result<(), String> {
    let project_root = PathBuf::from(&record.project_root);
    let mut index = read_sandbox_index(&project_root, root_dir);
    index.sandboxes.retain(|item| item.id != record.id);
    index.sandboxes.push(record.clone());
    write_sandbox_index(&project_root, &index, root_dir)
}

fn write_sandbox_patch(
    record: &SandboxRecord,
    diff: &str,
    root_dir: Option<&str>,
) -> Result<PathBuf, String> {
    let project_root = PathBuf::from(&record.project_root);
    let patch_dir = sandbox_patch_dir(&project_root, root_dir);
    std::fs::create_dir_all(&patch_dir)
        .map_err(|err| format!("failed to create sandbox patch directory: {err}"))?;
    let patch_path = patch_dir.join(format!("{}.patch", record.id));
    std::fs::write(&patch_path, diff).map_err(|err| {
        format!(
            "failed to write sandbox patch {}: {err}",
            patch_path.display()
        )
    })?;
    Ok(patch_path)
}

fn remove_sandbox_worktree(record: &SandboxRecord) -> Result<(), String> {
    let project_root = PathBuf::from(&record.project_root);
    let worktree_path = PathBuf::from(&record.worktree_path);
    if !worktree_path.exists() {
        return Ok(());
    }
    let result = git_output_owned(
        &project_root,
        &[
            "worktree".to_string(),
            "remove".to_string(),
            "--force".to_string(),
            path_to_string(&worktree_path),
        ],
    );
    if result.is_err() {
        std::fs::remove_dir_all(&worktree_path).map_err(|err| {
            format!(
                "failed to remove sandbox worktree {}: {err}",
                worktree_path.display()
            )
        })?;
        let _ = git_output(&project_root, &["worktree", "prune"]);
    }
    Ok(())
}

fn delete_sandbox_branch(record: &SandboxRecord) {
    let project_root = PathBuf::from(&record.project_root);
    let _ = git_output(&project_root, &["branch", "-D", &record.branch]);
}

/** 物化结果：拷/删文件与冲突。 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxMaterializeResult {
    pub record: serde_json::Value,
    pub diff: String,
    pub patch_path: String,
    pub copied_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub conflicts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual_conflicts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_diffs: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_retried_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxMaterializeOptions {
    pub files: Option<Vec<String>>,
    pub write_scope: Option<Vec<String>>,
    pub conflict_policy: Option<String>,
    pub force_files: Option<Vec<String>>,
}

/** 谁调用：子 Agent 收尾 / 内部合并。把沙箱变更按冲突策略拷回主树。 */
pub fn materialize_sandbox_files(
    project_dir: &Path,
    sandbox_id: &str,
    options: SandboxMaterializeOptions,
) -> Result<SandboxMaterializeResult, String> {
    let (record, diff, patch_path) = diff_sandbox(project_dir, sandbox_id, None)?;
    let changed = list_sandbox_changed_files(&record)?;
    let requested = normalize_relative_files(options.files.unwrap_or_default());
    let scoped = normalize_relative_files(options.write_scope.unwrap_or_default());
    let force_set: BTreeSet<String> =
        normalize_relative_files(options.force_files.unwrap_or_default())
            .into_iter()
            .collect();
    let prefer = options
        .conflict_policy
        .as_deref()
        .unwrap_or("skip")
        == "prefer-sandbox";

    let candidates = changed
        .into_iter()
        .filter(|file| {
            if !requested.is_empty() && !requested.iter().any(|item| item == file) {
                return false;
            }
            scoped.is_empty() || scoped.iter().any(|scope| matches_sandbox_scope(file, scope))
        })
        .collect::<Vec<_>>();

    let mut copied_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut skipped_files = Vec::new();
    let mut conflicts = Vec::new();
    let mut conflict_diffs = BTreeMap::new();

    for file in candidates {
        if is_runtime_or_git_path(&file) {
            skipped_files.push(file);
            continue;
        }
        let source_path = safe_resolve_inside(&record.worktree_path, &file)?;
        let target_path = safe_resolve_inside(&record.project_root, &file)?;
        let base = read_base_file(&record, &file);
        let current = if target_path.exists() {
            Some(std::fs::read(&target_path).map_err(|err| format!("read {file}: {err}"))?)
        } else {
            None
        };
        let source_exists = source_path.exists();
        let next = if source_exists {
            Some(std::fs::read(&source_path).map_err(|err| format!("read sandbox {file}: {err}"))?)
        } else {
            None
        };
        let force = prefer && (force_set.is_empty() || force_set.contains(&file));
        if !force && !can_overwrite(current.as_deref(), base.as_deref(), next.as_deref()) {
            conflicts.push(file.clone());
            conflict_diffs.insert(
                file.clone(),
                build_conflict_unified_diff(&file, current.as_deref(), next.as_deref()),
            );
            continue;
        }
        if source_exists {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("mkdir {}: {err}", parent.display()))?;
            }
            std::fs::copy(&source_path, &target_path)
                .map_err(|err| format!("copy {file}: {err}"))?;
            copied_files.push(file);
        } else if target_path.exists() {
            std::fs::remove_file(&target_path)
                .map_err(|err| format!("delete {file}: {err}"))?;
            deleted_files.push(file);
        }
    }

    let mut updated = record;
    let now = now_ms();
    updated.patch_path = Some(path_to_string(&patch_path));
    if conflicts.is_empty() {
        updated.status = "applied".to_string();
        updated.applied_at = Some(now);
    }
    updated.updated_at = now;
    upsert_sandbox(&updated, None)?;

    Ok(SandboxMaterializeResult {
        record: serde_json::to_value(&updated).unwrap_or(Value::Null),
        diff,
        patch_path: path_to_string(&patch_path),
        copied_files,
        deleted_files,
        skipped_files,
        conflicts,
        residual_conflicts: None,
        conflict_diffs: if conflict_diffs.is_empty() {
            None
        } else {
            Some(conflict_diffs)
        },
        auto_retried_files: None,
    })
}

/** 先 skip 合并，冲突文件再 prefer-sandbox 重试。 */
pub fn materialize_sandbox_files_with_auto_retry(
    project_dir: &Path,
    sandbox_id: &str,
    options: SandboxMaterializeOptions,
) -> Result<SandboxMaterializeResult, String> {
    let first = materialize_sandbox_files(
        project_dir,
        sandbox_id,
        SandboxMaterializeOptions {
            files: options.files.clone(),
            write_scope: options.write_scope.clone(),
            conflict_policy: Some("skip".to_string()),
            force_files: None,
        },
    )?;
    if first.conflicts.is_empty() {
        return Ok(SandboxMaterializeResult {
            residual_conflicts: Some(Vec::new()),
            auto_retried_files: Some(Vec::new()),
            ..first
        });
    }
    let retry = materialize_sandbox_files(
        project_dir,
        sandbox_id,
        SandboxMaterializeOptions {
            files: options.files,
            write_scope: options.write_scope,
            conflict_policy: Some("prefer-sandbox".to_string()),
            force_files: Some(first.conflicts.clone()),
        },
    )?;
    let auto_retried = retry
        .copied_files
        .iter()
        .chain(retry.deleted_files.iter())
        .filter(|file| first.conflicts.iter().any(|item| item == *file))
        .cloned()
        .collect::<Vec<_>>();
    let residual = retry.conflicts.clone();
    let mut copied = first.copied_files;
    copied.extend(retry.copied_files);
    copied.sort();
    copied.dedup();
    let mut deleted = first.deleted_files;
    deleted.extend(retry.deleted_files);
    deleted.sort();
    deleted.dedup();
    let mut skipped = first.skipped_files;
    skipped.extend(retry.skipped_files);
    skipped.sort();
    skipped.dedup();
    Ok(SandboxMaterializeResult {
        record: retry.record,
        diff: retry.diff,
        patch_path: retry.patch_path,
        copied_files: copied,
        deleted_files: deleted,
        skipped_files: skipped,
        conflicts: residual.clone(),
        residual_conflicts: Some(residual),
        conflict_diffs: retry.conflict_diffs.or(first.conflict_diffs),
        auto_retried_files: Some(auto_retried),
    })
}

/** 收尾前落盘当前 diff/patch（子 Agent finally 用）。 */
pub fn record_sandbox_diff(project_dir: &Path, sandbox_id: &str) -> Result<String, String> {
    let (_, diff, _) = diff_sandbox(project_dir, sandbox_id, None)?;
    Ok(diff)
}

/** 移除 worktree + 分支，索引标 removed。 */
pub fn dispose_sandbox(project_dir: &Path, sandbox_id: &str) -> Result<serde_json::Value, String> {
    let mut record = get_sandbox(project_dir, sandbox_id, None)?;
    remove_sandbox_worktree(&record)?;
    delete_sandbox_branch(&record);
    record.status = "removed".to_string();
    record.updated_at = now_ms();
    upsert_sandbox(&record, None)?;
    serde_json::to_value(record).map_err(|err| err.to_string())
}

/// 按年龄清理过期沙箱与 `xy-sandbox/*` 分支。
pub fn prune_stale_sandboxes(
    project_dir: &Path,
    max_age_ms: Option<i64>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let project_root = resolve_sandbox_project_root(project_dir)?;
    let max_age_ms = max_age_ms.unwrap_or(0);
    let now = now_ms();
    let mut removed = Vec::new();
    let index = read_sandbox_index(&project_root, None);
    for record in index.sandboxes {
        if record.status == "removed" {
            continue;
        }
        let age = now.saturating_sub(record.updated_at);
        if max_age_ms > 0 && age < max_age_ms {
            continue;
        }
        let mut next = record.clone();
        if remove_sandbox_worktree(&record).is_ok() {
            delete_sandbox_branch(&record);
            next.status = "removed".to_string();
            next.updated_at = now;
            let _ = upsert_sandbox(&next, None);
            removed.push(record.id);
        }
    }
    let _ = git_output(&project_root, &["worktree", "prune"]);
    let mut branches = Vec::new();
    if let Ok(listed) = git_output(&project_root, &["branch", "--list", "xy-sandbox/*"]) {
        for line in listed.lines() {
            let branch = line.trim().trim_start_matches('*').trim();
            if !branch.starts_with("xy-sandbox/") {
                continue;
            }
            if git_output(&project_root, &["branch", "-D", branch]).is_ok() {
                branches.push(branch.to_string());
            }
        }
    }
    Ok((removed, branches))
}

fn list_sandbox_changed_files(record: &SandboxRecord) -> Result<Vec<String>, String> {
    let worktree = PathBuf::from(&record.worktree_path);
    git_output(&worktree, &["add", "-A"])?;
    let names = git_output_owned(
        &worktree,
        &[
            "diff".to_string(),
            "--cached".to_string(),
            "--name-only".to_string(),
            record.base_commit.clone(),
            "--".to_string(),
        ],
    )?;
    Ok(normalize_relative_files(
        names.lines().map(str::to_string).collect(),
    ))
}

fn normalize_relative_files(files: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        let trimmed = file
            .trim()
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
            .replace('\\', "/");
        if trimmed.is_empty() {
            continue;
        }
        if Path::new(&trimmed).is_absolute() || trimmed.starts_with("../") || trimmed == ".." {
            continue;
        }
        if !out.iter().any(|item| item == &trimmed) {
            out.push(trimmed);
        }
    }
    out
}

fn matches_sandbox_scope(file: &str, scope: &str) -> bool {
    let file = file.replace('\\', "/");
    let scope = scope.replace('\\', "/").trim_end_matches('/').to_string();
    if scope.is_empty() || scope == "." {
        return true;
    }
    if !scope.contains(['*', '?', '[', ']', '{', '}']) {
        return file == scope || file.starts_with(&format!("{scope}/"));
    }
    // 简化 glob：仅支持 * / ** / ?
    let mut pattern = String::new();
    pattern.push('^');
    let mut chars = scope.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    pattern.push_str(".*");
                } else {
                    pattern.push_str("[^/]*");
                }
            }
            '?' => pattern.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            other => pattern.push(other),
        }
    }
    pattern.push('$');
    Regex::new(&pattern)
        .map(|re| re.is_match(&file))
        .unwrap_or(false)
}

fn is_runtime_or_git_path(file: &str) -> bool {
    let normalized = file.replace('\\', "/").to_lowercase();
    normalized == ".git"
        || normalized.starts_with(".git/")
        || (normalized.contains("/workspaces/") && normalized.contains("/sandboxes/"))
}

fn safe_resolve_inside(root: &str, relative: &str) -> Result<PathBuf, String> {
    let root_path = PathBuf::from(root);
    let resolved = root_path.join(relative);
    let resolved = if resolved.is_absolute() {
        resolved
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&resolved)
    };
    let root_canon = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.clone());
    let check = if resolved.exists() {
        resolved.canonicalize().unwrap_or(resolved.clone())
    } else {
        resolved.clone()
    };
    let relative_check = check.strip_prefix(&root_canon);
    if relative_check.is_err() {
        // 文件尚不存在时用 components 防逃逸
        let mut ok = true;
        for component in Path::new(relative).components() {
            if matches!(component, Component::ParentDir) {
                ok = false;
                break;
            }
        }
        if !ok {
            return Err(format!("Path escapes sandbox root: {relative}"));
        }
        return Ok(root_path.join(relative));
    }
    Ok(check)
}

fn read_base_file(record: &SandboxRecord, file: &str) -> Option<Vec<u8>> {
    let worktree = PathBuf::from(&record.worktree_path);
    let mut cmd = Command::new("git");
    cmd.args(["show", &format!("{}:{file}", record.base_commit)])
        .current_dir(&worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

fn can_overwrite(current: Option<&[u8]>, base: Option<&[u8]>, next: Option<&[u8]>) -> bool {
    let Some(current) = current else {
        return true;
    };
    if let Some(base) = base {
        if current == base {
            return true;
        }
    }
    if let Some(next) = next {
        if current == next {
            return true;
        }
    }
    false
}

fn build_conflict_unified_diff(file: &str, current: Option<&[u8]>, next: Option<&[u8]>) -> String {
    let a = current
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let b = next
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let a_lines: Vec<&str> = a.split('\n').collect();
    let b_lines: Vec<&str> = b.split('\n').collect();
    let mut lines = vec![
        format!("--- a/{file} (workspace)"),
        format!("+++ b/{file} (sandbox)"),
    ];
    let max = a_lines.len().max(b_lines.len());
    let show = max.min(80);
    for i in 0..show {
        let left = a_lines.get(i).copied();
        let right = b_lines.get(i).copied();
        if left == right {
            if let Some(value) = left {
                lines.push(format!(" {value}"));
            }
            continue;
        }
        if let Some(value) = left {
            lines.push(format!("-{value}"));
        }
        if let Some(value) = right {
            lines.push(format!("+{value}"));
        }
    }
    if max > 80 {
        lines.push(format!("... ({} more lines truncated)", max - 80));
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceCheckpointRecord {
    id: String,
    session_id: String,
    turn_id: String,
    project_root: String,
    git_head: String,
    patch_path: String,
    has_changes: bool,
    files: Vec<String>,
    created_at: i64,
}

fn create_workspace_checkpoint(
    cwd: &Path,
    session_id: &str,
    turn_id: &str,
) -> Result<WorkspaceCheckpointRecord, String> {
    let project_root = git_root(cwd)?;
    let session_id = safe_checkpoint_segment(session_id)?;
    let turn_id = safe_checkpoint_segment(turn_id)?;
    let dir = workspace_checkpoint_dir(&project_root, &session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("failed to create checkpoint directory: {err}"))?;
    let patch_path = dir.join(format!("{turn_id}.patch"));
    let git_head = git_output(&project_root, &["rev-parse", "HEAD"])?;
    let patch = get_workspace_checkpoint_diff(&project_root).unwrap_or_default();
    std::fs::write(&patch_path, &patch)
        .map_err(|err| format!("failed to write checkpoint patch: {err}"))?;
    let record = WorkspaceCheckpointRecord {
        id: format!("checkpoint_{turn_id}"),
        session_id,
        turn_id,
        project_root: path_to_string(&project_root),
        git_head,
        patch_path: path_to_string(&patch_path),
        has_changes: !patch.trim().is_empty(),
        files: files_in_checkpoint_patch_text(&patch),
        created_at: now_ms(),
    };
    write_workspace_checkpoint_record(&record)?;
    Ok(record)
}

fn list_workspace_checkpoints(
    cwd: &Path,
    session_id: &str,
) -> Result<Vec<WorkspaceCheckpointRecord>, String> {
    let project_root = git_root(cwd)?;
    let session_id = safe_checkpoint_segment(session_id)?;
    let dir = workspace_checkpoint_dir(&project_root, &session_id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut records = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|item| item.to_str()) == Some("json"))
        .filter_map(|path| read_workspace_checkpoint_record(&path))
        .collect::<Vec<_>>();
    records.sort_by_key(|a| a.created_at);
    Ok(records)
}

fn restore_workspace_checkpoint(
    cwd: &Path,
    session_id: &str,
    turn_id: &str,
) -> Result<WorkspaceCheckpointRecord, String> {
    let project_root = git_root(cwd)?;
    let session_id = safe_checkpoint_segment(session_id)?;
    let turn_id = safe_checkpoint_segment(turn_id)?;
    let path = workspace_checkpoint_dir(&project_root, &session_id).join(format!("{turn_id}.json"));
    let record = read_workspace_checkpoint_record(&path)
        .ok_or_else(|| format!("workspace checkpoint not found: {turn_id}"))?;
    git_output(&project_root, &["reset", "--hard", &record.git_head])?;
    let patch_path = PathBuf::from(&record.patch_path);
    let patch = std::fs::read_to_string(&patch_path).unwrap_or_default();
    if !patch.trim().is_empty() {
        let temp_patch = patch_path.with_file_name(format!("_restore_{}.patch", now_ms()));
        std::fs::write(&temp_patch, patch)
            .map_err(|err| format!("failed to write restore patch: {err}"))?;
        let apply_result = git_output(
            &project_root,
            &["apply", "--whitespace=nowarn", &path_to_string(&temp_patch)],
        );
        let _ = std::fs::remove_file(&temp_patch);
        apply_result?;
    }
    Ok(record)
}

fn workspace_checkpoint_dir(project_root: &Path, session_id: &str) -> PathBuf {
    project_data_root(project_root)
        .join("turn-checkpoints")
        .join(session_id)
}

fn write_workspace_checkpoint_record(record: &WorkspaceCheckpointRecord) -> Result<(), String> {
    let path = workspace_checkpoint_dir(Path::new(&record.project_root), &record.session_id)
        .join(format!("{}.json", record.turn_id));
    let text = serde_json::to_string_pretty(record)
        .map_err(|err| format!("failed to serialize checkpoint: {err}"))?;
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|err| format!("failed to write checkpoint record: {err}"))
}

fn read_workspace_checkpoint_record(path: &Path) -> Option<WorkspaceCheckpointRecord> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<WorkspaceCheckpointRecord>(&text).ok())
}

fn files_in_checkpoint_patch_text(patch: &str) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            if path != "/dev/null" {
                files.insert(path.to_string());
            }
        }
    }
    files.into_iter().collect()
}

fn get_workspace_checkpoint_diff(root: &Path) -> Result<String, String> {
    let mut diff = git_output_owned_raw(
        root,
        &[
            "diff".to_string(),
            "--binary".to_string(),
            "HEAD".to_string(),
            "--".to_string(),
            ".".to_string(),
            ":(exclude).xuanyu/**".to_string(),
        ],
    )
    .or_else(|_| {
        git_output_owned_raw(
            root,
            &[
                "diff".to_string(),
                "--binary".to_string(),
                "--".to_string(),
                ".".to_string(),
                ":(exclude).xuanyu/**".to_string(),
            ],
        )
    })?;

    let status = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    let synthetic = status
        .lines()
        .filter_map(parse_git_status_line)
        .filter(|file| file.get("status").and_then(Value::as_str) == Some("untracked"))
        .filter_map(|file| file.get("path").and_then(Value::as_str).map(str::to_string))
        .filter(|path| !is_xuanyu_internal_path(path))
        .map(|path| build_untracked_file_diff(root, &path))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if !synthetic.trim().is_empty() {
        diff = [diff.trim_end(), synthetic.trim_end()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        diff.push('\n');
    }
    Ok(diff)
}

fn is_xuanyu_internal_path(path: &str) -> bool {
    let normalized = normalize_git_pathspec(path);
    normalized == ".xuanyu" || normalized.starts_with(".xuanyu/")
}

fn safe_checkpoint_segment(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("checkpoint segment cannot be empty".to_string());
    }
    if value.contains("..")
        || value.contains('/')
        || value.contains('\\')
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("unsafe checkpoint segment: {value}"));
    }
    Ok(value.to_string())
}

fn create_sandbox_id(name: Option<&str>) -> String {
    let slug = name
        .unwrap_or("sandbox")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(32)
        .collect::<String>();
    let slug = if slug.is_empty() {
        "sandbox".to_string()
    } else {
        slug
    };
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{slug}-{}-{}", now_ms(), &random[..6])
}

fn safe_sandbox_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("sandbox id cannot be empty".to_string());
    }
    if value.contains("..")
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("unsafe sandbox id: {value}"));
    }
    Ok(value.to_string())
}

fn normalize_fs_path_text(path: &Path) -> String {
    let mut text = path_to_string(path).replace('/', "\\");
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_string();
    } else if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.replace('/', "\\");
    }
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text.replace('\\', "/")
    }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path_key = normalize_fs_path_text(path);
    let root_key = normalize_fs_path_text(root);
    if path_key == root_key {
        return true;
    }
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let prefix = format!("{}{sep}", root_key.trim_end_matches(['/', '\\']));
    path_key.starts_with(&prefix)
}

fn push_unique_path(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if roots.iter().any(|existing| same_path(existing, &candidate)) {
        return;
    }
    roots.push(candidate);
}

fn create_delivery_local_pr(
    cwd: &Path,
    args: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    let root = git_root(cwd)?;
    let status = git_status_snapshot(&root)?;
    let now = now_ms();
    let id = format!("local-pr-{now}");
    let title = string_arg(args, "title")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Local delivery package".to_string());
    let background = string_arg(args, "background")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            "Generated by XuanYu delivery tools from current Git state.".to_string()
        });
    let issue = string_arg(args, "issue").map(str::to_string);
    let platform = string_arg(args, "platform").map(str::to_string);
    let diff = string_arg(args, "diff")
        .map(str::to_string)
        .unwrap_or_else(|| {
            git_output_owned_raw(
                &root,
                &[
                    "diff".to_string(),
                    "--binary".to_string(),
                    "HEAD".to_string(),
                    "--".to_string(),
                ],
            )
            .unwrap_or_default()
        });
    let files = status
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let clean = status.get("clean").and_then(Value::as_bool).unwrap_or(true);
    let mut summary = string_vec_arg(args, "summary");
    if summary.is_empty() {
        summary = if clean {
            vec!["Workspace is clean.".to_string()]
        } else {
            vec![format!("{} changed files detected.", files.len())]
        };
    }
    let risks = if clean {
        Vec::<Value>::new()
    } else {
        vec![json!({
            "severity": "medium",
            "message": "Workspace contains uncommitted changes; review diff before publishing.",
            "evidence": "git.status"
        })]
    };
    let commit_message = string_arg(args, "commitMessage")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title.to_ascii_lowercase());
    let package_dir = project_data_root(&root).join("delivery").join(&id);
    std::fs::create_dir_all(&package_dir)
        .map_err(|err| format!("failed to create delivery package directory: {err}"))?;
    let markdown_path = package_dir.join("PR.md");
    let html_path = package_dir.join("PR.html");
    let patch_path = package_dir.join("changes.patch");
    let markdown = render_delivery_local_pr_markdown(&title, &background, &summary, &status, &diff);
    let html = render_delivery_local_pr_html(&title, &markdown);
    std::fs::write(&markdown_path, markdown)
        .map_err(|err| format!("failed to write delivery markdown: {err}"))?;
    std::fs::write(&html_path, html)
        .map_err(|err| format!("failed to write delivery html: {err}"))?;
    std::fs::write(&patch_path, &diff)
        .map_err(|err| format!("failed to write delivery patch: {err}"))?;
    let change_set: Option<serde_json::Value> = None;
    Ok(json!({
        "id": id,
        "title": title,
        "background": background,
        "summary": summary,
        "files": files,
        "checks": [],
        "risks": risks,
        "rollback": ["Use git restore/checkout or discard the local PR package after review."],
        "commitMessage": commit_message,
        "diff": diff,
        "status": status,
        "createdAt": now,
        "markdownPath": path_to_string(&markdown_path),
        "htmlPath": path_to_string(&html_path),
        "patchPath": path_to_string(&patch_path),
        "changeSet": change_set,
        "external": {
            "platform": platform,
            "issue": issue,
            "branch": string_arg(args, "branch")
                .map(str::to_string)
                .or_else(|| status.get("branch").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string()),
            "remote": git_output(&root, &["remote", "get-url", "origin"]).ok(),
            "state": "draft_ready",
            "nextSteps": ["Review PR.md and changes.patch, run validation, then publish manually."]
        }
    }))
}

fn inspect_git_platform_status(cwd: &Path, platform: Option<&str>) -> Result<Vec<Value>, String> {
    if let Some(platform) = platform {
        if !matches!(platform, "github" | "gitlab") {
            return Err(format!("unsupported git platform: {platform}"));
        }
    }
    let root = git_root(cwd)?;
    let status = git_status_snapshot(&root)?;
    let remote = git_output(&root, &["remote", "get-url", "origin"]).ok();
    let branch = status
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    Ok(["github", "gitlab"]
        .into_iter()
        .filter(|name| platform.map(|wanted| wanted == *name).unwrap_or(true))
        .map(|name| {
            let configured = remote
                .as_deref()
                .map(|remote| remote.to_ascii_lowercase().contains(name))
                .unwrap_or(false);
            json!({
                "platform": name,
                "configured": configured,
                "remote": remote,
                "currentBranch": branch,
                "authHint": if configured {
                    Value::Null
                } else {
                    json!(format!("No {name} remote detected. Configure origin or use the local PR package."))
                },
                "nextSteps": if configured {
                    json!(["Use the generated local PR package, then open a PR on the configured remote."])
                } else {
                    json!(["Add a Git remote for this platform or keep using local delivery packages."])
                }
            })
        })
        .collect())
}

fn create_git_issue_task_draft(
    cwd: &Path,
    args: &BTreeMap<String, Value>,
) -> Result<Value, String> {
    let platform = string_arg(args, "platform").unwrap_or("github");
    if !matches!(platform, "github" | "gitlab") {
        return Err(format!("unsupported git platform: {platform}"));
    }
    let Some(issue) = string_arg(args, "issue") else {
        return Err("git_issue_task_draft requires issue".to_string());
    };
    let issue = issue.trim();
    if issue.is_empty() {
        return Err("git_issue_task_draft issue cannot be empty".to_string());
    }
    let status = git_root(cwd)
        .ok()
        .and_then(|root| git_status_snapshot(&root).ok())
        .unwrap_or_else(|| json!({}));
    let branch = string_arg(args, "branch")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| delivery_draft_branch_name(issue));
    let title = string_arg(args, "title")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Implement issue {issue}"));
    let current_branch = status
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let body = string_arg(args, "body")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Task draft generated from XuanYu delivery tools.\n\nCurrent branch: {current_branch}\nIssue: {issue}\n\nSuggested workflow:\n1. Review the local PR package.\n2. Create or switch to `{branch}`.\n3. Apply changes and run validation.\n4. Open the PR against the configured remote."
            )
        });
    Ok(json!({
        "platform": platform,
        "issue": issue,
        "title": title,
        "branch": branch,
        "body": body,
        "commands": [
            format!("git checkout -b {branch}"),
            "pnpm build".to_string(),
            "cargo test --workspace".to_string()
        ]
    }))
}

fn render_delivery_local_pr_markdown(
    title: &str,
    background: &str,
    summary: &[String],
    status: &Value,
    diff: &str,
) -> String {
    let files = status
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut markdown = String::new();
    markdown.push_str(&format!("# {title}\n\n"));
    markdown.push_str("## Background\n\n");
    markdown.push_str(background);
    markdown.push_str("\n\n## Summary\n\n");
    for item in summary {
        markdown.push_str(&format!("- {item}\n"));
    }
    markdown.push_str("\n## Git Status\n\n");
    markdown.push_str(&format!(
        "- Branch: `{}`\n- Head: `{}`\n- Ahead/behind: `{}/{}`\n",
        status.get("branch").and_then(Value::as_str).unwrap_or("-"),
        status.get("head").and_then(Value::as_str).unwrap_or("-"),
        status.get("ahead").and_then(Value::as_u64).unwrap_or(0),
        status.get("behind").and_then(Value::as_u64).unwrap_or(0)
    ));
    markdown.push_str("\n## Files\n\n");
    if files.is_empty() {
        markdown.push_str("- No workspace changes detected.\n");
    } else {
        for file in files {
            markdown.push_str(&format!(
                "- `{}` ({})\n",
                file.get("path").and_then(Value::as_str).unwrap_or("-"),
                file.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("modified")
            ));
        }
    }
    markdown.push_str("\n## Diff\n\n```diff\n");
    markdown.push_str(diff);
    markdown.push_str("\n```\n");
    markdown
}

fn render_delivery_local_pr_html(title: &str, markdown: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body><pre>{}</pre></body></html>",
        delivery_html_escape(title),
        delivery_html_escape(markdown)
    )
}

fn delivery_html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn delivery_draft_branch_name(issue: &str) -> String {
    let slug = issue
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "task/generated-issue".to_string()
    } else {
        format!("task/{slug}")
    }
}

fn search_tool_catalog(catalog: Vec<ToolCatalogEntry>, query: &str, limit: usize) -> Vec<Value> {
    let tokens = query
        .to_ascii_lowercase()
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut scored = catalog
        .into_iter()
        .filter_map(|tool| {
            let haystack = [
                tool.name.clone(),
                tool.description.clone(),
                tool.category.clone().unwrap_or_default(),
                tool.aliases.join(" "),
            ]
            .join(" ")
            .to_ascii_lowercase();
            let mut score = 0_u32;
            for token in &tokens {
                if tool.name.to_ascii_lowercase().contains(token) {
                    score += 5;
                }
                if tool
                    .category
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(token)
                {
                    score += 3;
                }
                if tool.description.to_ascii_lowercase().contains(token) {
                    score += 2;
                }
                if haystack.contains(token) {
                    score += 1;
                }
            }
            (score > 0).then_some((
                score,
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "category": tool.category,
                    "source": tool.source,
                    "isWrite": tool.is_write,
                    "isExecute": tool.is_execute,
                    "isConcurrent": tool.is_concurrent,
                    "aliases": tool.aliases
                }),
            ))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(score_a, tool_a), (score_b, tool_b)| {
        score_b.cmp(score_a).then_with(|| {
            tool_a["name"]
                .as_str()
                .unwrap_or_default()
                .cmp(tool_b["name"].as_str().unwrap_or_default())
        })
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, tool)| tool)
        .collect()
}

fn snippet_text(text: &str, max: usize) -> String {
    let normalized = compress_whitespace(text);
    if normalized.chars().count() <= max {
        normalized
    } else {
        format!("{}...", normalized.chars().take(max).collect::<String>())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInstructionSource {
    path: String,
    relative_path: String,
    scope: String,
    name: String,
    priority: usize,
    content: String,
    size: u64,
    mtime_ms: i64,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInstructionsLoadResult {
    root_dir: String,
    cwd: String,
    sources: Vec<ProjectInstructionSource>,
    text: String,
}

const PROJECT_INSTRUCTION_FILES: [&str; 2] = ["XUANYU.md", "AGENTS.md"];
const PROJECT_INSTRUCTION_MAX_FILE_BYTES: usize = 64 * 1024;
const PROJECT_INSTRUCTION_MAX_TOTAL_BYTES: usize = 160 * 1024;
const PROJECT_INSTRUCTIONS_HEADER: &str = "[Project instructions]\nApply these repository/user instructions before planning, editing, running commands, or delegating sub-agents. Later directory-scoped files override broader guidance when they conflict.";
const PROJECT_INSTRUCTIONS_FOOTER: &str = "[End project instructions]";

pub fn project_instructions_context_text(cwd: &Path) -> Option<String> {
    let loaded = load_project_instructions(cwd, true);
    let text = loaded.text.trim();
    if text.is_empty() {
        None
    } else {
        Some(loaded.text)
    }
}

fn load_project_instructions(cwd: &Path, include_global: bool) -> ProjectInstructionsLoadResult {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let root_dir = find_project_root(&cwd);
    let mut sources = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut total_bytes = 0usize;

    for candidate in collect_instruction_candidates(&root_dir, &cwd, include_global) {
        let path = candidate.dir.join(candidate.name);
        let key = normalize_fs_path_text(&path).to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || total_bytes >= PROJECT_INSTRUCTION_MAX_TOTAL_BYTES {
            continue;
        }
        let remaining = PROJECT_INSTRUCTION_MAX_TOTAL_BYTES.saturating_sub(total_bytes);
        let allowed = PROJECT_INSTRUCTION_MAX_FILE_BYTES
            .min(remaining)
            .min(metadata.len() as usize);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let truncated = bytes.len() > allowed;
        let mut content = String::from_utf8_lossy(&bytes[..bytes.len().min(allowed)]).to_string();
        if truncated {
            content.push_str(&format!(
                "\n\n[Instruction file truncated at {allowed} bytes]"
            ));
        }
        let content = content.trim().to_string();
        total_bytes = total_bytes.saturating_add(content.len());
        sources.push(ProjectInstructionSource {
            path: path_to_string(&path),
            relative_path: display_path_from_root(&path, &root_dir),
            scope: candidate.scope.to_string(),
            name: candidate.name.to_string(),
            priority: candidate.priority,
            content,
            size: metadata.len(),
            mtime_ms: metadata_mtime_ms(&metadata),
            truncated,
        });
    }

    let text = format_project_instructions(&sources);
    ProjectInstructionsLoadResult {
        root_dir: path_to_string(&root_dir),
        cwd: path_to_string(&cwd),
        sources,
        text,
    }
}

struct ProjectInstructionCandidate {
    dir: PathBuf,
    scope: &'static str,
    name: &'static str,
    priority: usize,
}

fn collect_instruction_candidates(
    root_dir: &Path,
    cwd: &Path,
    include_global: bool,
) -> Vec<ProjectInstructionCandidate> {
    let mut candidates = Vec::new();
    let mut priority = 0usize;
    if include_global {
        for dir in global_instruction_dirs() {
            for name in PROJECT_INSTRUCTION_FILES {
                candidates.push(ProjectInstructionCandidate {
                    dir: dir.clone(),
                    scope: "global",
                    name,
                    priority,
                });
                priority += 1;
            }
        }
    }
    for dir in path_chain(root_dir, cwd) {
        let scope = if same_path(&dir, root_dir) {
            "project"
        } else {
            "directory"
        };
        for name in PROJECT_INSTRUCTION_FILES {
            candidates.push(ProjectInstructionCandidate {
                dir: dir.clone(),
                scope,
                name,
                priority,
            });
            priority += 1;
        }
    }
    candidates
}

fn find_project_root(start_dir: &Path) -> PathBuf {
    let mut current = if start_dir.is_file() {
        start_dir.parent().unwrap_or(start_dir).to_path_buf()
    } else {
        start_dir.to_path_buf()
    };
    loop {
        if current.join(".git").exists()
            || current.join("package.json").exists()
            || current.join("pyproject.toml").exists()
            || current.join("Cargo.toml").exists()
        {
            return current;
        }
        let Some(parent) = current.parent() else {
            return start_dir.to_path_buf();
        };
        if parent == current {
            return start_dir.to_path_buf();
        }
        current = parent.to_path_buf();
    }
}

fn path_chain(root_dir: &Path, cwd: &Path) -> Vec<PathBuf> {
    let root = root_dir.to_path_buf();
    let mut current = if cwd.is_file() {
        cwd.parent().unwrap_or(cwd).to_path_buf()
    } else {
        cwd.to_path_buf()
    };
    let mut chain = Vec::new();
    loop {
        chain.push(current.clone());
        if same_path(&current, &root) {
            break;
        }
        let Some(parent) = current.parent() else {
            if !chain.iter().any(|dir| same_path(dir, &root)) {
                chain.push(root.clone());
            }
            break;
        };
        if parent == current {
            if !chain.iter().any(|dir| same_path(dir, &root)) {
                chain.push(root.clone());
            }
            break;
        }
        current = parent.to_path_buf();
    }
    chain.reverse();
    let mut seen = std::collections::BTreeSet::new();
    chain
        .into_iter()
        .filter(|dir| seen.insert(normalize_fs_path_text(dir).to_ascii_lowercase()))
        .collect()
}

fn global_instruction_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(runtime_data_root());
    if let Some(app_data) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(app_data).join("xuan-yu-agent"));
    }
    dirs
}

fn format_project_instructions(sources: &[ProjectInstructionSource]) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let mut lines = vec![PROJECT_INSTRUCTIONS_HEADER.to_string(), String::new()];
    for source in sources {
        lines.push(format!("## {}: {}", source.scope, source.relative_path));
        if source.content.is_empty() {
            lines.push("(empty instruction file)".to_string());
        } else {
            lines.push(source.content.clone());
        }
        lines.push(String::new());
    }
    lines.push(PROJECT_INSTRUCTIONS_FOOTER.to_string());
    lines.join("\n").trim().to_string()
}

fn display_path_from_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(normalize_fs_path_text)
        .unwrap_or_else(|| normalize_fs_path_text(path))
}

fn same_path(left: &Path, right: &Path) -> bool {
    normalize_fs_path_text(left).eq_ignore_ascii_case(&normalize_fs_path_text(right))
}

fn context_section_names() -> Vec<String> {
    ["memory", "memory_profile", "project_instructions"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn is_context_section_name(section: &str) -> bool {
    matches!(
        section,
        "memory" | "memory_profile" | "project_instructions"
    )
}

fn context_section_label(section: &str) -> &'static str {
    match section {
        "memory" => "Retrieved long-term memories",
        "memory_profile" => "Explicit memory profiles",
        "project_instructions" => "Project instruction files",
        _ => "Unknown section",
    }
}

fn format_loaded_memory_entry(item: &Value) -> Option<String> {
    let entry = item.get("entry")?;
    let category = entry
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("memory");
    let content = entry.get("content").and_then(Value::as_str)?;
    let score = item
        .get("score")
        .and_then(Value::as_f64)
        .map(|score| format!(" score={score:.3}"))
        .unwrap_or_default();
    Some(format!(
        "- [{category}]{score} {}",
        snippet_text(content, 220)
    ))
}

fn memory_profile_kinds() -> &'static [&'static str] {
    &["identity", "user", "project", "agents", "workflow", "lessons"]
}

fn format_loaded_memory_profile(profile: &Value) -> Option<String> {
    let content = profile
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())?;
    let title = profile
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| profile.get("kind").and_then(Value::as_str))
        .unwrap_or("Memory Profile");
    let relative_path = profile
        .get("relativePath")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(format!(
        "### {title} ({relative_path})\n{}",
        truncate_text(content, 1200)
    ))
}

fn collect_files(root: &Path, current: &Path, limit: usize, out: &mut Vec<PathBuf>) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= limit {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == "node_modules" || name == "target" {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, limit, out);
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.to_path_buf());
        }
    }
}

fn collect_search_files(root: &Path, limit: usize) -> (PathBuf, Vec<PathBuf>) {
    if root.is_file() {
        let base = root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let file = root
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(root));
        return (base, vec![file]);
    }
    let mut files = Vec::new();
    collect_files(root, root, limit, &mut files);
    (root.to_path_buf(), files)
}

#[derive(Debug, Clone)]
struct CodebaseIndexOptions {
    max_files: usize,
    include: Vec<String>,
    refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebaseFileEntry {
    path: String,
    size: u64,
    language: String,
    updated_at: i64,
    fingerprint: String,
    summary: String,
    imports: Vec<String>,
    referenced_symbols: Vec<String>,
    entrypoint: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodebaseSymbolEntry {
    name: String,
    kind: String,
    path: String,
    line: usize,
    signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodebaseReferenceEntry {
    name: String,
    kind: String,
    path: String,
    line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebaseDirectorySummary {
    path: String,
    file_count: usize,
    languages: BTreeMap<String, usize>,
    entry_files: Vec<String>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebaseChangeEntry {
    path: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    added_lines: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed_lines: Option<u64>,
    impact_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebaseIncremental {
    enabled: bool,
    reused_files: usize,
    updated_files: usize,
    deleted_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebaseIndex {
    version: u8,
    root: String,
    generated_at: i64,
    file_count: usize,
    total_bytes: u64,
    files: Vec<CodebaseFileEntry>,
    symbols: Vec<CodebaseSymbolEntry>,
    references: Vec<CodebaseReferenceEntry>,
    directories: Vec<CodebaseDirectorySummary>,
    changes: Vec<CodebaseChangeEntry>,
    incremental: CodebaseIncremental,
}

struct FileAnalysis {
    summary: String,
    imports: Vec<String>,
    referenced_symbols: Vec<String>,
    entrypoint: bool,
    symbols: Vec<CodebaseSymbolEntry>,
    references: Vec<CodebaseReferenceEntry>,
}

fn build_codebase_index(
    root: &Path,
    options: CodebaseIndexOptions,
) -> Result<CodebaseIndex, String> {
    let files = discover_codebase_files(root, &options)?;
    let previous = if options.refresh {
        None
    } else {
        load_codebase_index(root)
    };
    let previous_files = previous
        .as_ref()
        .map(|index| {
            index
                .files
                .iter()
                .map(|file| (file.path.clone(), file.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let previous_symbols = previous
        .as_ref()
        .map(|index| group_symbols_by_path(&index.symbols))
        .unwrap_or_default();
    let previous_references = previous
        .as_ref()
        .map(|index| group_references_by_path(&index.references))
        .unwrap_or_default();

    let mut file_entries = Vec::new();
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut total_bytes = 0u64;
    let mut reused_files = 0usize;
    let mut updated_files = 0usize;

    for rel_path in &files {
        let abs_path = root.join(rel_path);
        let Ok(metadata) = std::fs::metadata(&abs_path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        let normalized_path = normalize_codebase_path(rel_path);
        let fingerprint = fingerprint_for_metadata(&metadata);
        if let Some(cached) = previous_files.get(&normalized_path) {
            if cached.fingerprint == fingerprint {
                file_entries.push(cached.clone());
                symbols.extend(
                    previous_symbols
                        .get(&normalized_path)
                        .cloned()
                        .unwrap_or_default(),
                );
                references.extend(
                    previous_references
                        .get(&normalized_path)
                        .cloned()
                        .unwrap_or_default(),
                );
                reused_files += 1;
                continue;
            }
        }

        let language = codebase_language_for_path(rel_path);
        let content = read_codebase_text_file(&abs_path, metadata.len())?;
        let analysis = analyze_codebase_file(rel_path, &language, &content, metadata.len());
        file_entries.push(CodebaseFileEntry {
            path: normalized_path,
            size: metadata.len(),
            language,
            updated_at: metadata_mtime_ms(&metadata),
            fingerprint,
            summary: analysis.summary,
            imports: analysis.imports,
            referenced_symbols: analysis.referenced_symbols,
            entrypoint: analysis.entrypoint,
        });
        symbols.extend(analysis.symbols);
        references.extend(analysis.references);
        updated_files += 1;
    }

    file_entries.sort_by(|a, b| a.path.cmp(&b.path));
    let changes = enrich_codebase_changes(read_codebase_git_changes(root), &file_entries);
    let index = CodebaseIndex {
        version: 2,
        root: path_to_string(root),
        generated_at: now_ms(),
        file_count: file_entries.len(),
        total_bytes,
        directories: summarize_codebase_directories(&file_entries),
        changes,
        files: file_entries,
        symbols: symbols.into_iter().take(5000).collect(),
        references: references.into_iter().take(10000).collect(),
        incremental: CodebaseIncremental {
            enabled: !options.refresh,
            reused_files,
            updated_files,
            deleted_files: previous
                .as_ref()
                .map(|index| index.files.len().saturating_sub(files.len()))
                .unwrap_or(0),
        },
    };
    save_codebase_index(root, &index)?;
    Ok(index)
}

fn discover_codebase_files(
    root: &Path,
    options: &CodebaseIndexOptions,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    collect_codebase_files(root, root, options, &mut out)?;
    out.sort();
    out.truncate(options.max_files.max(1));
    Ok(out)
}

fn collect_codebase_files(
    root: &Path,
    current: &Path,
    options: &CodebaseIndexOptions,
    out: &mut Vec<String>,
) -> Result<(), String> {
    if out.len() >= options.max_files.max(1) {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(current) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_codebase_dir(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_codebase_files(root, &path, options, out)?;
        } else {
            let rel = normalize_rel(root, &path);
            if rel != ".xuanyu/codebase-index.json"
                && codebase_file_included(&rel, &options.include)
            {
                out.push(rel);
            }
        }
        if out.len() >= options.max_files.max(1) {
            break;
        }
    }
    Ok(())
}

fn should_skip_codebase_dir(name: &str) -> bool {
    matches!(
        name,
        ".cache"
            | ".git"
            | ".hg"
            | ".svn"
            | ".xuanyu"
            | "node_modules"
            | "dist"
            | "build"
            | ".next"
            | "coverage"
            | "target"
            | "out"
    )
}

fn codebase_file_included(path: &str, include: &[String]) -> bool {
    if include.is_empty() {
        return matches!(
            Path::new(path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "ts" | "tsx"
                | "js"
                | "jsx"
                | "json"
                | "md"
                | "py"
                | "yml"
                | "yaml"
                | "css"
                | "scss"
                | "html"
                | "rs"
                | "toml"
        );
    }
    include
        .iter()
        .flat_map(|pattern| expand_brace_pattern(pattern))
        .any(|pattern| wildcard_match(&pattern.replace('\\', "/"), path))
}

fn expand_brace_pattern(pattern: &str) -> Vec<String> {
    let Some(start) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(end_offset) = pattern[start + 1..].find('}') else {
        return vec![pattern.to_string()];
    };
    let end = start + 1 + end_offset;
    let prefix = &pattern[..start];
    let suffix = &pattern[end + 1..];
    pattern[start + 1..end]
        .split(',')
        .map(|part| format!("{prefix}{part}{suffix}"))
        .collect()
}

fn read_codebase_text_file(path: &Path, size: u64) -> Result<String, String> {
    if size > 256 * 1024 {
        return Ok(String::new());
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if bytes.iter().take(512).any(|byte| *byte == 0) {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn analyze_codebase_file(path: &str, language: &str, content: &str, size: u64) -> FileAnalysis {
    let imports = extract_codebase_imports(path, language, content);
    let symbols = extract_codebase_symbols(path, language, content);
    let references = extract_codebase_references(path, language, content, &imports);
    let entrypoint = is_codebase_entrypoint(path, content);
    let referenced_symbols = references
        .iter()
        .filter(|reference| reference.kind == "call" || reference.kind == "type")
        .map(|reference| reference.name.clone())
        .take(100)
        .collect::<Vec<_>>();
    FileAnalysis {
        summary: summarize_codebase_file(path, language, content, size, &symbols, entrypoint),
        imports,
        referenced_symbols,
        entrypoint,
        symbols,
        references,
    }
}

fn summarize_codebase_file(
    _path: &str,
    language: &str,
    content: &str,
    size: u64,
    symbols: &[CodebaseSymbolEntry],
    entrypoint: bool,
) -> String {
    if content.is_empty() {
        return if size > 256 * 1024 {
            "Large file skipped for content summary".to_string()
        } else {
            "Unreadable or empty file".to_string()
        };
    }
    if language == "markdown" {
        if let Some(heading) = content.lines().find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("# ")
                .or_else(|| trimmed.strip_prefix("## "))
                .or_else(|| trimmed.strip_prefix("### "))
                .map(str::trim)
        }) {
            return format!("Documentation: {heading}");
        }
    }
    if language == "json" {
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            let keys = value
                .as_object()
                .map(|object| object.keys().take(8).cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            return format!(
                "JSON config/data with keys: {}",
                if keys.is_empty() {
                    "(none)".to_string()
                } else {
                    keys.join(", ")
                }
            );
        }
        return "JSON-like file".to_string();
    }
    let imports = content
        .lines()
        .filter(|line| line.trim_start().starts_with("import "))
        .count();
    let exports = content
        .lines()
        .filter(|line| line.trim_start().starts_with("export "))
        .count();
    let mut parts = Vec::new();
    if entrypoint {
        parts.push("entrypoint".to_string());
    }
    parts.push("source file".to_string());
    if imports > 0 {
        parts.push(format!("{imports} imports"));
    }
    if exports > 0 {
        parts.push(format!("{exports} exports"));
    }
    let exported_symbols = symbols
        .iter()
        .filter(|symbol| symbol.kind != "method")
        .take(5)
        .map(|symbol| symbol.name.clone())
        .collect::<Vec<_>>();
    if !exported_symbols.is_empty() {
        parts.push(format!("symbols: {}", exported_symbols.join(", ")));
    }
    parts.join(", ")
}

fn extract_codebase_symbols(path: &str, language: &str, content: &str) -> Vec<CodebaseSymbolEntry> {
    if !matches!(language, "typescript" | "javascript" | "python" | "rust") {
        return Vec::new();
    }
    let mut symbols = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = line.trim();
        let candidates = if language == "python" {
            vec![
                ("function", name_after_prefix(trimmed, "def ")),
                ("function", name_after_prefix(trimmed, "async def ")),
                ("class", name_after_prefix(trimmed, "class ")),
            ]
        } else if language == "rust" {
            vec![
                ("function", name_after_prefix(trimmed, "fn ")),
                ("function", name_after_prefix(trimmed, "pub fn ")),
                ("type", name_after_prefix(trimmed, "struct ")),
                ("type", name_after_prefix(trimmed, "pub struct ")),
                ("type", name_after_prefix(trimmed, "enum ")),
                ("type", name_after_prefix(trimmed, "pub enum ")),
                ("interface", name_after_prefix(trimmed, "trait ")),
                ("interface", name_after_prefix(trimmed, "pub trait ")),
                ("constant", name_after_prefix(trimmed, "const ")),
                ("constant", name_after_prefix(trimmed, "pub const ")),
            ]
        } else {
            vec![
                ("function", name_after_prefix(trimmed, "function ")),
                ("function", name_after_prefix(trimmed, "export function ")),
                ("function", name_after_prefix(trimmed, "async function ")),
                (
                    "function",
                    name_after_prefix(trimmed, "export async function "),
                ),
                ("class", name_after_prefix(trimmed, "class ")),
                ("class", name_after_prefix(trimmed, "export class ")),
                ("interface", name_after_prefix(trimmed, "interface ")),
                ("interface", name_after_prefix(trimmed, "export interface ")),
                ("type", name_after_prefix(trimmed, "type ")),
                ("type", name_after_prefix(trimmed, "export type ")),
                ("constant", name_after_prefix(trimmed, "const ")),
                ("constant", name_after_prefix(trimmed, "export const ")),
            ]
        };
        for (kind, name) in candidates {
            if let Some(name) = name {
                symbols.push(CodebaseSymbolEntry {
                    name,
                    kind: kind.to_string(),
                    path: normalize_codebase_path(path),
                    line: line_no,
                    signature: trimmed.chars().take(240).collect(),
                });
                break;
            }
        }
        if let Some(exports) = trimmed
            .strip_prefix("export {")
            .and_then(|rest| rest.split('}').next())
        {
            for name in exports
                .split(',')
                .filter_map(|item| item.split_whitespace().next())
            {
                if !name.is_empty() {
                    symbols.push(CodebaseSymbolEntry {
                        name: name.to_string(),
                        kind: "export".to_string(),
                        path: normalize_codebase_path(path),
                        line: line_no,
                        signature: trimmed.chars().take(240).collect(),
                    });
                }
            }
        }
    }
    symbols
}

fn name_after_prefix(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name = rest
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

fn extract_codebase_imports(path: &str, language: &str, content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if language == "python" {
            if let Some(rest) = trimmed.strip_prefix("from ") {
                if let Some(module) = rest.split_whitespace().next() {
                    push_unique(&mut imports, module.to_string());
                }
            } else if let Some(rest) = trimmed.strip_prefix("import ") {
                for module in rest
                    .split(',')
                    .filter_map(|item| item.split_whitespace().next())
                {
                    push_unique(&mut imports, module.to_string());
                }
            }
            continue;
        }
        if language == "rust" {
            if let Some(rest) = trimmed.strip_prefix("use ") {
                push_unique(&mut imports, rest.trim_end_matches(';').to_string());
            }
            continue;
        }
        for quote in ['"', '\''] {
            if ((trimmed.starts_with("import ") || trimmed.starts_with("export "))
                && trimmed.contains(" from "))
                || trimmed.starts_with("import ")
                || trimmed.contains("require(")
            {
                if let Some(target) = quoted_segment(trimmed, quote) {
                    push_unique(&mut imports, normalize_import_target(path, &target));
                }
            }
        }
    }
    imports.into_iter().take(100).collect()
}

fn quoted_segment(value: &str, quote: char) -> Option<String> {
    let start = value.find(quote)?;
    let rest = &value[start + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn extract_codebase_references(
    path: &str,
    language: &str,
    content: &str,
    imports: &[String],
) -> Vec<CodebaseReferenceEntry> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut refs = imports
        .iter()
        .take(80)
        .map(|import| CodebaseReferenceEntry {
            name: import.clone(),
            kind: "import".to_string(),
            path: normalize_codebase_path(path),
            line: find_line(&lines, import),
            target: Some(import.clone()),
        })
        .collect::<Vec<_>>();
    if !matches!(language, "typescript" | "javascript" | "python" | "rust") {
        return refs;
    }
    for (index, line) in lines.iter().enumerate() {
        let line_no = index + 1;
        for name in call_like_names(line) {
            if !REFERENCE_STOP_WORDS.contains(&name.as_str()) {
                refs.push(CodebaseReferenceEntry {
                    name,
                    kind: "call".to_string(),
                    path: normalize_codebase_path(path),
                    line: line_no,
                    target: None,
                });
            }
        }
        for name in type_like_names(line) {
            refs.push(CodebaseReferenceEntry {
                name,
                kind: "type".to_string(),
                path: normalize_codebase_path(path),
                line: line_no,
                target: None,
            });
        }
    }
    refs.into_iter().take(300).collect()
}

fn call_like_names(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    let chars = line.chars().collect::<Vec<_>>();
    for index in 0..chars.len() {
        if chars[index] != '(' || index == 0 {
            continue;
        }
        let mut start = index;
        while start > 0 {
            let ch = chars[start - 1];
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < index {
            names.push(chars[start..index].iter().collect::<String>());
        }
    }
    unique_strings(names)
}

fn type_like_names(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    for marker in [": ", "<"] {
        let mut rest = line;
        while let Some(pos) = rest.find(marker) {
            let after = &rest[pos + marker.len()..];
            let name = after
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
                .collect::<String>();
            if name.chars().next().map(char::is_uppercase).unwrap_or(false) {
                names.push(name);
            }
            rest = after;
        }
    }
    unique_strings(names)
}

fn summarize_codebase_directories(files: &[CodebaseFileEntry]) -> Vec<CodebaseDirectorySummary> {
    let mut dirs: BTreeMap<String, Vec<&CodebaseFileEntry>> = BTreeMap::new();
    for file in files {
        dirs.entry(dirname_text(&file.path)).or_default().push(file);
    }
    let mut summaries = dirs
        .into_iter()
        .map(|(dir, entries)| {
            let mut languages = BTreeMap::new();
            for entry in &entries {
                *languages.entry(entry.language.clone()).or_insert(0) += 1;
            }
            let entry_files = entries
                .iter()
                .filter(|entry| entry.entrypoint)
                .map(|entry| entry.path.clone())
                .take(8)
                .collect::<Vec<_>>();
            let dependencies = unique_strings(
                entries
                    .iter()
                    .flat_map(|entry| {
                        entry
                            .imports
                            .iter()
                            .map(|import| directory_for_import(import))
                    })
                    .filter(|dep| !dep.is_empty() && dep != &dir)
                    .take(12)
                    .collect(),
            );
            let dependents = unique_strings(
                files
                    .iter()
                    .filter(|file| {
                        file.imports
                            .iter()
                            .any(|target| directory_for_import(target) == dir)
                    })
                    .map(|file| dirname_text(&file.path))
                    .filter(|dep| !dep.is_empty() && dep != &dir)
                    .take(12)
                    .collect(),
            );
            let top_languages = languages
                .iter()
                .map(|(language, count)| format!("{count} {language}"))
                .collect::<Vec<_>>()
                .join(", ");
            CodebaseDirectorySummary {
                path: dir,
                file_count: entries.len(),
                languages,
                entry_files: entry_files.clone(),
                dependencies,
                dependents,
                summary: format!(
                    "{} files ({}){}",
                    entries.len(),
                    top_languages,
                    if entry_files.is_empty() {
                        String::new()
                    } else {
                        format!(", {} entry files", entry_files.len())
                    }
                ),
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then_with(|| a.path.cmp(&b.path))
    });
    summaries
}

fn enrich_codebase_changes(
    changes: Vec<CodebaseChangeEntry>,
    files: &[CodebaseFileEntry],
) -> Vec<CodebaseChangeEntry> {
    changes
        .into_iter()
        .map(|mut change| {
            let normalized = normalize_codebase_path(&change.path);
            change.path = normalized.clone();
            change.impact_paths = files
                .iter()
                .filter(|file| {
                    file.imports
                        .iter()
                        .any(|target| import_could_target_path(target, &normalized))
                })
                .map(|file| file.path.clone())
                .filter(|path| path != &normalized)
                .take(12)
                .collect();
            change
        })
        .collect()
}

fn read_codebase_git_changes(root: &Path) -> Vec<CodebaseChangeEntry> {
    let mut changes = Vec::new();
    if let Ok(status) = git_output(root, &["status", "--short"]) {
        for line in status.lines().filter(|line| !line.trim().is_empty()) {
            let status = line.get(0..2).unwrap_or("").trim();
            let path = line.get(3..).unwrap_or("").trim_matches('"');
            changes.push(CodebaseChangeEntry {
                path: normalize_codebase_path(path),
                status: if status.is_empty() {
                    "modified".to_string()
                } else {
                    status.to_string()
                },
                added_lines: None,
                removed_lines: None,
                impact_paths: Vec::new(),
            });
        }
    }
    if let Ok(numstat) = git_output(root, &["diff", "--numstat"]) {
        for line in numstat.lines().filter(|line| !line.trim().is_empty()) {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 3 {
                continue;
            }
            let path = normalize_codebase_path(parts[2]);
            let added = parts[0].parse::<u64>().ok();
            let removed = parts[1].parse::<u64>().ok();
            if let Some(existing) = changes.iter_mut().find(|change| change.path == path) {
                existing.added_lines = added;
                existing.removed_lines = removed;
            } else {
                changes.push(CodebaseChangeEntry {
                    path,
                    status: "modified".to_string(),
                    added_lines: added,
                    removed_lines: removed,
                    impact_paths: Vec::new(),
                });
            }
        }
    }
    changes.into_iter().take(200).collect()
}

fn format_codebase_index(index: &CodebaseIndex, limit: usize) -> String {
    let mut lines = vec![
        format!(
            "Codebase index: {} files, {}",
            index.file_count,
            format_bytes(index.total_bytes)
        ),
        format!("Generated: {}", index.generated_at),
        format!(
            "Incremental: {} ({} reused, {} updated, {} deleted)",
            if index.incremental.enabled {
                "on"
            } else {
                "off"
            },
            index.incremental.reused_files,
            index.incremental.updated_files,
            index.incremental.deleted_files
        ),
        String::new(),
        "Top directories:".to_string(),
    ];
    lines.extend(index.directories.iter().take(12).map(|dir| {
        let deps = if dir.dependencies.is_empty() {
            String::new()
        } else {
            format!(
                " deps -> {}",
                dir.dependencies
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let entries = if dir.entry_files.is_empty() {
            String::new()
        } else {
            format!(
                " entries: {}",
                dir.entry_files
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "- {}: {}{}{}",
            if dir.path.is_empty() { "." } else { &dir.path },
            dir.summary,
            entries,
            deps
        )
    }));
    lines.push(String::new());
    lines.push("Files:".to_string());
    lines.extend(index.files.iter().take(limit).map(|file| {
        format!(
            "- {} ({}, {}): {}",
            file.path,
            file.language,
            format_bytes(file.size),
            file.summary
        )
    }));
    if !index.symbols.is_empty() {
        lines.push(String::new());
        lines.push("Symbols:".to_string());
        lines.extend(index.symbols.iter().take(limit).map(|symbol| {
            format!(
                "- {} {} @ {}:{}",
                symbol.kind, symbol.name, symbol.path, symbol.line
            )
        }));
    }
    if !index.references.is_empty() {
        lines.push(String::new());
        lines.push("References/imports:".to_string());
        lines.extend(index.references.iter().take(limit).map(|reference| {
            format!(
                "- {} {} @ {}:{}{}",
                reference.kind,
                reference.name,
                reference.path,
                reference.line,
                reference
                    .target
                    .as_ref()
                    .map(|target| format!(" -> {target}"))
                    .unwrap_or_default()
            )
        }));
    }
    if !index.changes.is_empty() {
        lines.push(String::new());
        lines.push("Recent changes:".to_string());
        lines.extend(index.changes.iter().take(limit).map(|change| {
            format!(
                "- {} {}{}{}",
                change.status,
                change.path,
                change
                    .added_lines
                    .map(|added| format!(" (+{}/-{})", added, change.removed_lines.unwrap_or(0)))
                    .unwrap_or_default(),
                if change.impact_paths.is_empty() {
                    String::new()
                } else {
                    format!(" impact: {}", change.impact_paths.join(", "))
                }
            )
        }));
    }
    lines.join("\n")
}

fn load_codebase_index(root: &Path) -> Option<CodebaseIndex> {
    let text = std::fs::read_to_string(codebase_index_path(root)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_codebase_index(root: &Path, index: &CodebaseIndex) -> Result<(), String> {
    let path = codebase_index_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(index).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| err.to_string())
}

fn codebase_index_path(root: &Path) -> PathBuf {
    project_data_root(root).join("codebase-index.json")
}

fn contains_traversal(pattern: &str) -> bool {
    let normalized = pattern.replace('\\', "/");
    normalized.contains("../") || normalized == ".." || normalized.ends_with("/..")
}

fn group_symbols_by_path(
    symbols: &[CodebaseSymbolEntry],
) -> BTreeMap<String, Vec<CodebaseSymbolEntry>> {
    let mut grouped = BTreeMap::new();
    for symbol in symbols {
        grouped
            .entry(symbol.path.clone())
            .or_insert_with(Vec::new)
            .push(symbol.clone());
    }
    grouped
}

fn group_references_by_path(
    references: &[CodebaseReferenceEntry],
) -> BTreeMap<String, Vec<CodebaseReferenceEntry>> {
    let mut grouped = BTreeMap::new();
    for reference in references {
        grouped
            .entry(reference.path.clone())
            .or_insert_with(Vec::new)
            .push(reference.clone());
    }
    grouped
}

fn fingerprint_for_metadata(metadata: &std::fs::Metadata) -> String {
    format!("{}:{}", metadata.len(), metadata_mtime_ms(metadata))
}

fn metadata_mtime_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn codebase_language_for_path(path: &str) -> String {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "ts" | "tsx" => "typescript".to_string(),
        "js" | "jsx" => "javascript".to_string(),
        "py" => "python".to_string(),
        "rs" => "rust".to_string(),
        "md" => "markdown".to_string(),
        "json" => "json".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        "css" | "scss" => "css".to_string(),
        "html" => "html".to_string(),
        "toml" => "toml".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "text".to_string(),
    }
}

fn is_codebase_entrypoint(path: &str, content: &str) -> bool {
    let base = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "index.ts"
            | "index.tsx"
            | "index.js"
            | "main.ts"
            | "main.tsx"
            | "main.js"
            | "main.rs"
            | "lib.rs"
            | "cli.ts"
            | "server.ts"
            | "app.tsx"
    ) || content.contains("commander")
        || content.contains("createServer")
        || content.contains("vite")
        || content.contains("render(")
}

fn normalize_import_target(from_path: &str, target: &str) -> String {
    if !target.starts_with('.') {
        return target.to_string();
    }
    let dir = dirname_text(from_path);
    let mut parts = dir
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for part in target.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value.to_string()),
        }
    }
    strip_code_extension(&parts.join("/"))
}

fn directory_for_import(target: &str) -> String {
    let normalized = normalize_codebase_path(target);
    if normalized.is_empty() || normalized.starts_with('.') {
        return String::new();
    }
    if !normalized.contains('/') {
        return if normalized.starts_with('@') {
            normalized
        } else {
            String::new()
        };
    }
    if normalized.starts_with('@') {
        let parts = normalized.split('/').collect::<Vec<_>>();
        return parts.iter().take(2).cloned().collect::<Vec<_>>().join("/");
    }
    dirname_text(&normalized)
}

fn dirname_text(path: &str) -> String {
    let normalized = normalize_codebase_path(path);
    normalized
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn import_could_target_path(target: &str, path: &str) -> bool {
    let target = strip_code_extension(&normalize_codebase_path(target));
    let path = strip_code_extension(&normalize_codebase_path(path));
    path.ends_with(&target) || target.ends_with(&path)
}

fn strip_code_extension(path: &str) -> String {
    for extension in [".ts", ".tsx", ".js", ".jsx", ".json", ".rs"] {
        if let Some(stripped) = path.strip_suffix(extension) {
            return stripped.to_string();
        }
    }
    path.to_string()
}

fn find_line(lines: &[&str], needle: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or(1)
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

const REFERENCE_STOP_WORDS: &[&str] = &[
    "if", "for", "while", "switch", "catch", "function", "return", "console", "require",
];

fn string_arg<'a>(args: &'a BTreeMap<String, serde_json::Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(serde_json::Value::as_str)
}

fn string_arg_any<'a>(
    args: &'a BTreeMap<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter().find_map(|key| string_arg(args, key))
}

fn bool_arg(args: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<bool> {
    args.get(key).and_then(serde_json::Value::as_bool)
}

fn bool_arg_any(args: &BTreeMap<String, serde_json::Value>, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| bool_arg(args, key))
}

fn format_task_status_content(task: &SubAgentTaskLedger) -> String {
    let mut lines = vec![
        format!("taskId: {}", task.task_id),
        format!("status: {}", task.status),
        format!("agent: {}", task.agent_name),
        format!("task: {}", task.task.task),
        format!("events: {}", task.timeline.len()),
    ];
    if let Some(result) = &task.result {
        lines.push(format!("result: {}", result.status));
        if !result.conclusion.trim().is_empty() {
            lines.push(format!(
                "conclusion: {}",
                truncate_task_status_line(&result.conclusion, 500)
            ));
        }
        if !result.modified_files.is_empty() {
            lines.push(format!(
                "changedFiles:\n{}",
                format_task_status_list(&result.modified_files)
            ));
        }
        if !result.artifacts.is_empty() {
            lines.push(format!(
                "artifacts:\n{}",
                format_task_status_list(&result.artifacts)
            ));
        }
        if !result.risks.is_empty() {
            lines.push(format!(
                "risks:\n{}",
                format_task_status_list(&result.risks)
            ));
        }
        if !result.next_steps.is_empty() {
            lines.push(format!(
                "nextSteps:\n{}",
                format_task_status_list(&result.next_steps)
            ));
        }
    } else {
        lines.push("result: pending".to_string());
    }
    lines.join("\n")
}

fn format_task_status_list(items: &[String]) -> String {
    items
        .iter()
        .take(12)
        .map(|item| format!("- {}", truncate_task_status_line(item, 500)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_task_status_line(value: &str, max_len: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_len {
        normalized
    } else {
        let take = max_len.saturating_sub(3);
        format!("{}...", normalized.chars().take(take).collect::<String>())
    }
}

fn subagent_task_inputs_from_args(
    args: &BTreeMap<String, serde_json::Value>,
) -> Result<ParsedSubAgentTaskInputs, String> {
    let fallback_context = string_arg(args, "context").map(str::to_string);
    let fallback_write_scope = string_vec_arg_any(args, &["writeScope", "write_scope"]);
    let fallback_metadata = metadata_arg(args);
    if let Some(items) = args
        .get("subTasks")
        .or_else(|| args.get("sub_tasks"))
        .or_else(|| args.get("agents"))
        .and_then(Value::as_array)
    {
        let inputs = items
            .iter()
            .map(|item| {
                let mut value = item.clone();
                if let Some(object) = value.as_object_mut() {
                    if !object.contains_key("context") {
                        if let Some(brief) = object.get("brief").cloned() {
                            object.insert("context".to_string(), brief);
                        }
                    }
                }
                subagent_task_input_from_value(
                    &value,
                    fallback_context.as_deref(),
                    &fallback_write_scope,
                    &fallback_metadata,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (inputs, duplicate_subtask_count) = dedupe_subagent_task_inputs(inputs);
        return Ok(apply_subagent_context_budget(
            inputs,
            duplicate_subtask_count,
        ));
    }

    let agent_id = string_arg_any(args, &["agentId", "agent_id"])
        .map(str::to_string)
        .or_else(|| default_agent_id_for_submit_task(args))
        .ok_or_else(|| {
            "submit_task requires agentId or a routable type/mainBrainIntent when subTasks is omitted"
                .to_string()
        })?;
    let task = string_arg(args, "task")
        .or_else(|| string_arg(args, "goal"))
        .ok_or_else(|| "submit_task requires task or goal".to_string())?;
    Ok(apply_subagent_context_budget(
        vec![SubAgentTaskInput {
            agent_id,
            task: task.to_string(),
            context: fallback_context,
            write_scope: fallback_write_scope,
            metadata: fallback_metadata,
        }],
        0,
    ))
}

fn default_agent_id_for_submit_task(args: &BTreeMap<String, Value>) -> Option<String> {
    let intent = string_arg_any(
        args,
        &["mainBrainIntent", "main_brain_intent", "_mainBrainIntent"],
    )
    .map(|value| value.trim().to_ascii_lowercase());
    match intent.as_deref() {
        Some("review") => return Some("reviewer".to_string()),
        Some("chat" | "coding" | "task" | "debug") => return Some("developer".to_string()),
        _ => {}
    }
    let task_type = string_arg_any(args, &["type", "taskType", "task_type"])
        .map(|value| value.trim().to_ascii_lowercase());
    match task_type.as_deref() {
        Some("review") => Some("reviewer".to_string()),
        Some("explore") => Some("explorer".to_string()),
        Some("plan") => Some("architect".to_string()),
        Some("coding" | "shell") => Some("developer".to_string()),
        None => Some("developer".to_string()),
        _ => None,
    }
}

fn dedupe_subagent_task_inputs(inputs: Vec<SubAgentTaskInput>) -> (Vec<SubAgentTaskInput>, usize) {
    let original_len = inputs.len();
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for input in inputs {
        let key = [
            input.agent_id.trim().to_ascii_lowercase(),
            normalize_delegation_text_key(&input.task),
            normalize_delegation_text_key(input.context.as_deref().unwrap_or_default()),
            normalize_delegation_scope_key(&input.write_scope),
        ]
        .join("\0");
        if seen.insert(key) {
            deduped.push(input);
        }
    }
    let duplicate_count = original_len.saturating_sub(deduped.len());
    (deduped, duplicate_count)
}

fn apply_subagent_context_budget(
    mut inputs: Vec<SubAgentTaskInput>,
    duplicate_subtask_count: usize,
) -> ParsedSubAgentTaskInputs {
    let mut remaining = MAX_DELEGATION_TOTAL_CONTEXT_CHARS;
    let mut truncated = false;
    for input in &mut inputs {
        input.context =
            take_delegation_context(input.context.as_deref(), &mut remaining).inspect(|context| {
                if context.contains(DELEGATION_CONTEXT_TRUNCATED_MARKER) {
                    truncated = true;
                }
            });
        if input.context.is_none() && remaining <= 0 {
            truncated = true;
        }
    }
    ParsedSubAgentTaskInputs {
        inputs,
        total_context_chars: (MAX_DELEGATION_TOTAL_CONTEXT_CHARS - remaining).max(0) as usize,
        context_truncated: truncated,
        duplicate_subtask_count,
    }
}

fn take_delegation_context(value: Option<&str>, remaining: &mut isize) -> Option<String> {
    let trimmed = value.map(str::trim).filter(|value| !value.is_empty())?;
    if *remaining <= 0 {
        return None;
    }
    let field_cap = MAX_DELEGATION_CONTEXT_FIELD_CHARS.min(*remaining as usize);
    let char_count = trimmed.chars().count();
    let text = if char_count > field_cap {
        format!(
            "{}{}",
            take_chars(trimmed, field_cap),
            DELEGATION_CONTEXT_TRUNCATED_MARKER
        )
    } else {
        trimmed.to_string()
    };
    *remaining -= text.chars().count() as isize;
    Some(text)
}

fn take_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn normalize_delegation_text_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn normalize_delegation_scope_key(write_scope: &[String]) -> String {
    let mut normalized = write_scope
        .iter()
        .map(|item| item.trim().replace('\\', "/").to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized.join("|")
}

fn subagent_task_input_from_value(
    value: &Value,
    fallback_context: Option<&str>,
    fallback_write_scope: &[String],
    fallback_metadata: &BTreeMap<String, Value>,
) -> Result<SubAgentTaskInput, String> {
    let Some(object) = value.as_object() else {
        return Err("submit_task subTasks items must be objects".to_string());
    };
    let agent_id = value_string_any(object, &["agentId", "agent_id"])
        .ok_or_else(|| "submit_task subTasks item requires agentId".to_string())?;
    let task = value_string_any(object, &["task", "goal"])
        .ok_or_else(|| "submit_task subTasks item requires task".to_string())?;
    let context = value_string(object, "context")
        .map(str::to_string)
        .or_else(|| fallback_context.map(str::to_string));
    let write_scope = value_string_vec_any(object, &["writeScope", "write_scope"])
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| fallback_write_scope.to_vec());
    let mut metadata = fallback_metadata.clone();
    metadata.extend(value_metadata(object));
    Ok(SubAgentTaskInput {
        agent_id: agent_id.to_string(),
        task: task.to_string(),
        context,
        write_scope,
        metadata,
    })
}

fn value_string<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn value_string_any<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter().find_map(|key| value_string(object, key))
}

fn value_string_vec_any(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<Vec<String>> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
    })
}

fn metadata_arg(args: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut metadata = args
        .get("metadata")
        .and_then(Value::as_object)
        .map(value_map_to_btree)
        .unwrap_or_default();
    insert_metadata_string_any(
        args,
        &mut metadata,
        "controlPlaneTaskId",
        &["controlPlaneTaskId", "control_plane_task_id"],
    );
    insert_metadata_string_any(
        args,
        &mut metadata,
        "controlPlaneRunId",
        &[
            "controlPlaneRunId",
            "control_plane_run_id",
            "runId",
            "run_id",
            "_runId",
            "_run_id",
        ],
    );
    insert_metadata_string_any(
        args,
        &mut metadata,
        "controlPlaneSessionId",
        &[
            "controlPlaneSessionId",
            "control_plane_session_id",
            "sessionId",
            "session_id",
            "_sessionId",
            "_session_id",
        ],
    );
    insert_metadata_value_any(
        args,
        &mut metadata,
        "taskType",
        &["type", "taskType", "task_type"],
    );
    insert_metadata_value_any(
        args,
        &mut metadata,
        "mainBrainIntent",
        &["mainBrainIntent", "main_brain_intent", "_mainBrainIntent"],
    );
    insert_metadata_value_any(args, &mut metadata, "acceptance", &["acceptance"]);
    insert_metadata_value_any(args, &mut metadata, "strategy", &["strategy"]);
    insert_metadata_value_any(args, &mut metadata, "priority", &["priority"]);
    insert_metadata_value_any(
        args,
        &mut metadata,
        "timeoutMs",
        &["timeoutMs", "timeout_ms"],
    );
    metadata
}

fn insert_metadata_string_any(
    args: &BTreeMap<String, Value>,
    metadata: &mut BTreeMap<String, Value>,
    output_key: &str,
    input_keys: &[&str],
) {
    if metadata.contains_key(output_key) {
        return;
    }
    if let Some(value) = input_keys.iter().find_map(|key| {
        args.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }) {
        metadata.insert(output_key.to_string(), json!(value));
    }
}

fn insert_metadata_value_any(
    args: &BTreeMap<String, Value>,
    metadata: &mut BTreeMap<String, Value>,
    output_key: &str,
    input_keys: &[&str],
) {
    if metadata.contains_key(output_key) {
        return;
    }
    if let Some(value) = input_keys.iter().find_map(|key| args.get(*key).cloned()) {
        metadata.insert(output_key.to_string(), value);
    }
}

fn value_metadata(object: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .get("metadata")
        .and_then(Value::as_object)
        .map(value_map_to_btree)
        .unwrap_or_default()
}

fn value_map_to_btree(object: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn string_vec_arg(args: &BTreeMap<String, serde_json::Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_vec_arg_any(args: &BTreeMap<String, serde_json::Value>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            let values = string_vec_arg(args, key);
            (!values.is_empty()).then_some(values)
        })
        .unwrap_or_default()
}

fn usize_arg(args: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn usize_arg_any(args: &BTreeMap<String, serde_json::Value>, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| usize_arg(args, key))
}

fn readonly_context_active(call: &ToolCall) -> bool {
    if bool_arg_any(
        &call.args,
        &["readonly", "readOnly", "_readonly", "_readOnly"],
    )
    .unwrap_or(false)
    {
        return true;
    }
    string_arg_any(&call.args, &["sandboxMode", "sandbox_mode", "sandbox"])
        .map(|value| value.eq_ignore_ascii_case("readonly"))
        .unwrap_or(false)
}

fn exec_readonly_block(call: &ToolCall, command: &str) -> Option<ToolOutput> {
    if !readonly_context_active(call) {
        return None;
    }
    static WRITE_PATTERNS: &[&str] = &[
        r">\s*[^&|]",
        r"\btouch\b",
        r"\bcp\b.*\s+\S+$",
        r"\bmv\b.*\s+\S+$",
        r"\bmkdir\b",
        r"\brm\b",
        r"\becho\b.*>\s*\S+",
    ];
    for pattern in WRITE_PATTERNS {
        if Regex::new(pattern)
            .ok()
            .is_some_and(|regex| regex.is_match(command))
        {
            return Some(ToolOutput {
                content: format!("Command blocked in readonly mode: {command}"),
                is_error: true,
                data: Some(json!({ "errorType": "readonly_violation" })),
            });
        }
    }
    None
}

fn error(message: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: message.into(),
        is_error: true,
        data: None,
    }
}

fn git_root(cwd: &Path) -> Result<PathBuf, String> {
    git_output(cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|err| format!("not a git repository: {err}"))
}

fn git_status_snapshot(root: &Path) -> Result<Value, String> {
    let branch = git_output(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "HEAD".to_string());
    let head =
        git_output(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|_| "".to_string());
    let upstream = git_output(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok();
    let (ahead, behind) = upstream
        .as_ref()
        .and_then(|_| {
            git_output(
                root,
                &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            )
            .ok()
        })
        .and_then(|text| {
            let mut parts = text.split_whitespace();
            Some((
                parts.next()?.parse::<u64>().ok()?,
                parts.next()?.parse::<u64>().ok()?,
            ))
        })
        .unwrap_or((0, 0));
    let raw = git_output(root, &["status", "--porcelain=v1"])?;
    let files = raw
        .lines()
        .filter_map(parse_git_status_line)
        .collect::<Vec<_>>();
    Ok(json!({
        "root": path_to_string(root),
        "branch": branch,
        "head": head,
        "upstream": upstream,
        "ahead": ahead,
        "behind": behind,
        "clean": files.is_empty(),
        "files": files
    }))
}

fn format_git_status_content(status: &Value) -> String {
    let branch = status
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("HEAD");
    let head = status.get("head").and_then(Value::as_str).unwrap_or("");
    let files = status
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if files.is_empty() {
        return format!("{branch} @ {head}\nWorking tree clean.");
    }

    let mut lines = vec![format!("{branch} @ {head}")];
    if let Some(upstream) = status.get("upstream").and_then(Value::as_str) {
        let ahead = status.get("ahead").and_then(Value::as_u64).unwrap_or(0);
        let behind = status.get("behind").and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!(
            "tracking {upstream}; ahead {ahead}, behind {behind}"
        ));
    }
    lines.extend(files.iter().map(|file| {
        format!(
            "{}{} {}",
            file.get("index").and_then(Value::as_str).unwrap_or(" "),
            file.get("workingTree")
                .and_then(Value::as_str)
                .unwrap_or(" "),
            file.get("path").and_then(Value::as_str).unwrap_or_default()
        )
    }));
    lines.join("\n")
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    git_output_owned(cwd, &args)
}

fn git_output_owned(cwd: &Path, args: &[String]) -> Result<String, String> {
    git_output_owned_raw(cwd, args).map(|output| output.trim_end().to_string())
}

fn git_output_owned_raw(cwd: &Path, args: &[String]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with {:?}", output.status.code())
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn safe_git_pathspecs(paths: Vec<String>) -> Result<Vec<String>, String> {
    paths
        .into_iter()
        .map(|path| normalize_git_pathspec(&path))
        .filter(|path| !path.is_empty())
        .map(|path| {
            if path.starts_with('-') {
                return Err(format!("Unsafe git pathspec: {path}"));
            }
            if Path::new(&path).is_absolute() {
                return Err(format!("Git pathspec must be relative: {path}"));
            }
            if path.split('/').any(|part| part == "..") {
                return Err(format!("Git pathspec cannot escape the repository: {path}"));
            }
            if path.contains('\0') {
                return Err("Git pathspec contains a null byte".to_string());
            }
            Ok(path)
        })
        .collect()
}

fn list_git_branches(root: &Path) -> Result<Vec<Value>, String> {
    let raw = git_output(
        root,
        &[
            "branch",
            "--format=%(HEAD)%09%(refname:short)%09%(upstream:short)",
        ],
    )?;
    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            let head = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            let upstream = parts.next().filter(|value| !value.is_empty());
            json!({
                "name": name,
                "current": head.trim() == "*",
                "upstream": upstream
            })
        })
        .collect())
}

fn format_git_branch(branch: &Value) -> String {
    let marker = if branch
        .get("current")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "* "
    } else {
        "  "
    };
    let name = branch
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match branch.get("upstream").and_then(Value::as_str) {
        Some(upstream) => format!("{marker}{name} -> {upstream}"),
        None => format!("{marker}{name}"),
    }
}

fn validate_branch_name(name: &str) -> Result<String, String> {
    let branch = name.trim();
    let safe_chars = branch
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-'));
    if branch.is_empty()
        || !safe_chars
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.ends_with('/')
        || branch.contains("//")
        || branch.contains('\0')
    {
        return Err(format!("Unsafe branch name: {name}"));
    }
    Ok(branch.to_string())
}

fn safe_git_ref_arg(value: &str) -> Result<String, String> {
    let reference = value.trim();
    if reference.is_empty() {
        return Err("Git ref cannot be empty".to_string());
    }
    if reference.starts_with('-') {
        return Err(format!("Unsafe git ref: {reference}"));
    }
    if reference.contains('\0') {
        return Err("Git ref contains a null byte".to_string());
    }
    Ok(reference.to_string())
}

fn normalize_git_pathspec(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .to_string()
}

fn get_working_tree_diff(root: &Path, paths: &[String]) -> Result<String, String> {
    let mut args = vec![
        "diff".to_string(),
        "--binary".to_string(),
        "HEAD".to_string(),
        "--".to_string(),
    ];
    args.extend(paths.iter().cloned());
    let mut diff = git_output_owned_raw(root, &args).or_else(|_| {
        let mut fallback = vec!["diff".to_string(), "--binary".to_string(), "--".to_string()];
        fallback.extend(paths.iter().cloned());
        git_output_owned_raw(root, &fallback)
    })?;

    let selected = if paths.is_empty() { None } else { Some(paths) };
    let status = git_output(root, &["status", "--porcelain=v1"])?;
    let synthetic = status
        .lines()
        .filter_map(parse_git_status_line)
        .filter(|file| file.get("status").and_then(Value::as_str) == Some("untracked"))
        .filter_map(|file| {
            let path = file.get("path").and_then(Value::as_str)?;
            let selected = selected
                .map(|paths| paths.iter().any(|item| item == path))
                .unwrap_or(true);
            selected.then_some(path.to_string())
        })
        .map(|path| build_untracked_file_diff(root, &path))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if !synthetic.trim().is_empty() {
        diff = [diff.trim_end(), synthetic.trim_end()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        diff.push('\n');
    }
    Ok(diff)
}

fn build_untracked_file_diff(root: &Path, rel_path: &str) -> Result<String, String> {
    let safe = safe_git_pathspecs(vec![rel_path.to_string()])?
        .pop()
        .unwrap_or_default();
    let abs = root.join(&safe);
    let metadata = match std::fs::metadata(&abs) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(String::new()),
    };
    if !metadata.is_file() {
        return Ok(String::new());
    }
    if metadata.len() > 1024 * 1024 {
        return Ok(binary_untracked_diff(&safe));
    }
    let buffer = std::fs::read(&abs).map_err(|err| err.to_string())?;
    if buffer.contains(&0) {
        return Ok(binary_untracked_diff(&safe));
    }

    let content = String::from_utf8_lossy(&buffer).replace("\r\n", "\n");
    let mut body_lines = if content.is_empty() {
        Vec::new()
    } else {
        content.split('\n').collect::<Vec<_>>()
    };
    if body_lines.last() == Some(&"") {
        body_lines.pop();
    }
    let line_count = body_lines.len().max(1);
    Ok([
        format!("diff --git a/{safe} b/{safe}"),
        "new file mode 100644".to_string(),
        "index 0000000..0000000".to_string(),
        "--- /dev/null".to_string(),
        format!("+++ b/{safe}"),
        format!("@@ -0,0 +1,{line_count} @@"),
    ]
    .into_iter()
    .chain(body_lines.into_iter().map(|line| format!("+{line}")))
    .chain(std::iter::once(String::new()))
    .collect::<Vec<_>>()
    .join("\n"))
}

fn binary_untracked_diff(path: &str) -> String {
    [
        format!("diff --git a/{path} b/{path}"),
        "new file mode 100644".to_string(),
        "index 0000000..0000000".to_string(),
        format!("Binary files /dev/null and b/{path} differ"),
        String::new(),
    ]
    .join("\n")
}

fn write_git_patch_file(root: &Path, diff: &str, prefix: &str) -> Result<(PathBuf, Value), String> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("{prefix}_{}_{}", now_ms(), &nonce[..6]);
    let patch_dir = project_data_root(root).join("patches");
    std::fs::create_dir_all(&patch_dir).map_err(|err| err.to_string())?;
    let patch_path = patch_dir.join(format!("{id}.patch"));
    std::fs::write(&patch_path, diff).map_err(|err| err.to_string())?;
    let metadata = json!({
        "id": id,
        "root": path_to_string(root),
        "patchPath": path_to_string(&patch_path),
        "diff": diff,
        "createdAt": now_ms()
    });
    Ok((patch_path, metadata))
}

fn resolve_git_patch_input(
    root: &Path,
    args: &BTreeMap<String, serde_json::Value>,
) -> Result<PathBuf, String> {
    if let Some(patch_text) = string_arg(args, "patchText") {
        return write_git_patch_file(root, patch_text, "inline").map(|(path, _)| path);
    }
    let Some(raw_path) = string_arg(args, "patchPath") else {
        return Err("patchPath or patchText is required".to_string());
    };
    let patch_path = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        root.join(raw_path)
    };
    let resolved_root = std::fs::canonicalize(root).map_err(|err| err.to_string())?;
    let resolved_patch = std::fs::canonicalize(&patch_path).map_err(|err| {
        format!(
            "Patch file does not exist: {} ({err})",
            patch_path.display()
        )
    })?;
    if !resolved_patch.starts_with(&resolved_root) {
        return Err(format!(
            "Patch path must be inside the repository: {}",
            patch_path.display()
        ));
    }
    Ok(resolved_patch)
}

fn parse_git_status_line(line: &str) -> Option<Value> {
    if line.len() < 4 {
        return None;
    }
    let bytes = line.as_bytes();
    let index = bytes[0] as char;
    let working_tree = bytes[1] as char;
    let mut path = normalize_git_pathspec(&line[3..]);
    let mut from = None;
    if (index == 'R' || index == 'C') && path.contains(" -> ") {
        let (old, new) = path.split_once(" -> ")?;
        from = Some(normalize_git_pathspec(old));
        path = normalize_git_pathspec(new);
    }
    Some(json!({
        "path": path,
        "from": from,
        "index": index.to_string(),
        "workingTree": working_tree.to_string(),
        "status": git_status_kind(index, working_tree),
        "staged": index != ' ' && index != '?',
        "unstaged": working_tree != ' ' && working_tree != '?'
    }))
}

fn git_status_kind(index: char, working_tree: char) -> &'static str {
    if index == '?' || working_tree == '?' {
        "untracked"
    } else if index == 'U'
        || working_tree == 'U'
        || (index == 'A' && working_tree == 'A')
        || (index == 'D' && working_tree == 'D')
    {
        "conflicted"
    } else if index == 'R' || working_tree == 'R' {
        "renamed"
    } else if index == 'A' || working_tree == 'A' {
        "added"
    } else if index == 'D' || working_tree == 'D' {
        "deleted"
    } else {
        "modified"
    }
}

fn parse_git_log(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\x1f');
            let hash = parts.next().unwrap_or_default();
            let short_hash = parts.next().unwrap_or_default();
            let author_name = parts.next().unwrap_or_default();
            let author_email = parts.next().unwrap_or_default();
            let timestamp = parts
                .next()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_default()
                * 1000;
            let subject = parts.next().unwrap_or_default();
            json!({
                "hash": hash,
                "shortHash": short_hash,
                "authorName": author_name,
                "authorEmail": author_email,
                "timestamp": timestamp,
                "subject": subject
            })
        })
        .collect()
}

fn parse_git_blame(raw: &str) -> Vec<Value> {
    let mut result = Vec::new();
    let mut current = serde_json::Map::new();
    for line in raw.lines() {
        let mut header_parts = line.split_whitespace();
        if let (Some(commit), Some(original), Some(final_line)) = (
            header_parts.next(),
            header_parts.next(),
            header_parts.next(),
        ) {
            if commit.len() == 40 && commit.chars().all(|ch| ch.is_ascii_hexdigit()) {
                current = serde_json::Map::new();
                current.insert("commit".to_string(), json!(commit));
                current.insert(
                    "originalLine".to_string(),
                    json!(original.parse::<u64>().unwrap_or_default()),
                );
                current.insert(
                    "finalLine".to_string(),
                    json!(final_line.parse::<u64>().unwrap_or_default()),
                );
                continue;
            }
        }
        if let Some(author) = line.strip_prefix("author ") {
            current.insert("author".to_string(), json!(author));
        } else if let Some(author_time) = line.strip_prefix("author-time ") {
            current.insert(
                "authorTime".to_string(),
                json!(author_time.parse::<i64>().unwrap_or_default() * 1000),
            );
        } else if let Some(summary) = line.strip_prefix("summary ") {
            current.insert("summary".to_string(), json!(summary));
        } else if let Some(content) = line.strip_prefix('\t') {
            current.insert("content".to_string(), json!(content));
            result.push(Value::Object(current.clone()));
            current.clear();
        }
    }
    result
}

#[derive(Clone)]
struct ValidationCommandCandidate {
    command: String,
    kind: String,
    source: String,
    source_path: String,
    related_files: Vec<String>,
}

fn discover_validation_commands(project_dir: &Path) -> Vec<Value> {
    let mut commands = Vec::new();
    commands.extend(discover_package_validation_commands(project_dir));
    commands.extend(discover_readme_validation_commands(project_dir));
    commands.extend(discover_ci_validation_commands(project_dir));
    commands.extend(discover_instruction_validation_commands(project_dir));
    dedupe_validation_commands(commands)
        .into_iter()
        .map(|command| validation_command_to_value(&command))
        .collect()
}

fn discover_package_validation_commands(project_dir: &Path) -> Vec<ValidationCommandCandidate> {
    let mut result = Vec::new();
    for package_path in find_validation_files(project_dir, "package.json", 4) {
        let Ok(text) = std::fs::read_to_string(&package_path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(scripts) = parsed.get("scripts").and_then(Value::as_object) else {
            continue;
        };
        let source_path = normalize_rel(project_dir, &package_path);
        for (name, script) in scripts {
            let Some(script) = script.as_str() else {
                continue;
            };
            let kind = classify_validation_command(&format!("{name} {script}"));
            if kind == "unknown" {
                continue;
            }
            result.push(ValidationCommandCandidate {
                command: format!("npm run {name}"),
                kind,
                source: "package.json".to_string(),
                source_path: source_path.clone(),
                related_files: infer_validation_related_files(project_dir, &source_path),
            });
        }
    }
    result
}

fn discover_readme_validation_commands(project_dir: &Path) -> Vec<ValidationCommandCandidate> {
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(project_dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if lower != "readme" && !lower.starts_with("readme.") {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let source_path = normalize_rel(project_dir, &path);
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        for command in extract_validation_commands_from_text(&content) {
            let kind = classify_validation_command(&command);
            if kind == "unknown" {
                continue;
            }
            result.push(ValidationCommandCandidate {
                command,
                kind,
                source: "readme".to_string(),
                source_path: source_path.clone(),
                related_files: infer_validation_related_files(project_dir, &source_path),
            });
        }
    }
    result
}

fn discover_ci_validation_commands(project_dir: &Path) -> Vec<ValidationCommandCandidate> {
    let mut result = Vec::new();
    let github_dir = project_dir.join(".github");
    for ci_path in find_validation_files(&github_dir, "*", 5) {
        let extension = ci_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extension != "yml" && extension != "yaml" {
            continue;
        }
        let source_path = normalize_rel(project_dir, &ci_path);
        let content = std::fs::read_to_string(&ci_path).unwrap_or_default();
        for line in content.lines() {
            let trimmed = line.trim();
            let Some(command) = trimmed
                .strip_prefix("run:")
                .or_else(|| trimmed.strip_prefix("- run:"))
                .map(unquote)
            else {
                continue;
            };
            let kind = classify_validation_command(&command);
            if kind == "unknown" {
                continue;
            }
            result.push(ValidationCommandCandidate {
                command,
                kind,
                source: "ci".to_string(),
                source_path: source_path.clone(),
                related_files: infer_validation_related_files(project_dir, &source_path),
            });
        }
    }
    result
}

fn discover_instruction_validation_commands(project_dir: &Path) -> Vec<ValidationCommandCandidate> {
    let instruction_paths = [
        "AGENTS.md",
        "CODEX.md",
        "CLAUDE.md",
        ".cursorrules",
        ".github/copilot-instructions.md",
        "REFACTOR_PLAN.md",
    ];
    let mut result = Vec::new();
    for relative in instruction_paths {
        let path = project_dir.join(relative);
        if !path.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        for command in extract_validation_commands_from_text(&content) {
            let kind = classify_validation_command(&command);
            if kind == "unknown" {
                continue;
            }
            result.push(ValidationCommandCandidate {
                command,
                kind,
                source: "project_instruction".to_string(),
                source_path: relative.to_string(),
                related_files: Vec::new(),
            });
        }
    }
    result
}

fn classify_validation_command(command: &str) -> String {
    let lower = command.to_ascii_lowercase();
    if lower.contains("typecheck")
        || lower.contains("type-check")
        || lower.contains("tsc")
        || lower.contains("cargo check")
    {
        "typecheck".to_string()
    } else if lower.contains("lint")
        || lower.contains("eslint")
        || lower.contains("biome")
        || lower.contains("ruff")
        || lower.contains("flake8")
        || lower.contains("clippy")
        || lower.contains("fmt --check")
    {
        "lint".to_string()
    } else if lower.contains("build")
        || lower.contains("vite build")
        || lower.contains("webpack")
        || lower.contains("rollup")
        || lower.contains("tsup")
        || lower.contains("cargo build")
    {
        "build".to_string()
    } else {
        "unknown".to_string()
    }
}

fn extract_validation_commands_from_text(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let prefixed = strip_command_prefix(trimmed);
        if is_command_like(prefixed) {
            commands.push(prefixed.to_string());
        }
        for inline in backtick_segments(trimmed) {
            if is_command_like(&inline) {
                commands.push(inline);
            }
        }
        for marker in ["验证命令:", "验证命令：", "运行:", "运行："] {
            if let Some((_, value)) = trimmed.split_once(marker) {
                let command = value.trim().trim_matches('`').to_string();
                if !command.is_empty() {
                    commands.push(command);
                }
            }
        }
    }
    unique_strings(commands)
}

fn strip_command_prefix(value: &str) -> &str {
    let trimmed = value.trim_start();
    if let Some(rest) = trimmed.strip_prefix('$') {
        rest.trim_start()
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        rest.trim_start()
    } else if let Some(rest) = trimmed.strip_prefix('-') {
        rest.trim_start()
    } else if let Some(rest) = trimmed.strip_prefix('*') {
        rest.trim_start()
    } else {
        trimmed
    }
}

fn backtick_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        segments.push(after_start[..end].trim().to_string());
        rest = &after_start[end + 1..];
    }
    segments
}

fn is_command_like(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "npm ", "pnpm ", "yarn ", "bun ", "npx ", "node ", "tsc", "mvn ", "gradle ", "dotnet ",
        "cargo ", "go ", "python ", "pytest",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn dedupe_validation_commands(
    commands: Vec<ValidationCommandCandidate>,
) -> Vec<ValidationCommandCandidate> {
    let mut seen: BTreeMap<String, ValidationCommandCandidate> = BTreeMap::new();
    for command in commands {
        let key = command.command.to_ascii_lowercase();
        if let Some(existing) = seen.get_mut(&key) {
            existing.source_path = unique_strings(
                existing
                    .source_path
                    .split(", ")
                    .map(str::to_string)
                    .chain(std::iter::once(command.source_path))
                    .collect(),
            )
            .join(", ");
            existing.related_files =
                unique_strings([existing.related_files.clone(), command.related_files].concat());
        } else {
            seen.insert(key, command);
        }
    }
    let mut values = seen.into_values().collect::<Vec<_>>();
    values.sort_by(|a, b| {
        validation_sort_weight(&a.kind)
            .cmp(&validation_sort_weight(&b.kind))
            .then_with(|| a.command.cmp(&b.command))
    });
    values
}

fn validation_command_to_value(command: &ValidationCommandCandidate) -> Value {
    json!({
        "id": format!(
            "vcmd_{}",
            stable_hex(&format!("{}\n{}", command.command, command.source_path), 10)
        ),
        "command": command.command,
        "type": command.kind,
        "source": command.source,
        "sourcePath": command.source_path,
        "relatedFiles": command.related_files,
        "runCount": 0,
        "failureCount": 0,
        "failureRate": 0
    })
}

fn validation_sort_weight(kind: &str) -> usize {
    match kind {
        "typecheck" => 0,
        "lint" => 1,
        "build" => 2,
        _ => 3,
    }
}

fn infer_validation_related_files(project_dir: &Path, source_path: &str) -> Vec<String> {
    let mut files = ["package.json", "tsconfig.json", "Cargo.toml", "README.md"]
        .into_iter()
        .filter(|file| project_dir.join(file).exists())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if source_path != "package.json" {
        files.push(source_path.to_string());
    }
    unique_strings(files)
}

fn find_validation_files(root: &Path, filename: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.exists() {
        collect_validation_files(root, filename, max_depth, 0, &mut out);
    }
    out
}

fn collect_validation_files(
    dir: &Path,
    filename: &str,
    max_depth: usize,
    depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_validation_dir(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_validation_files(&path, filename, max_depth, depth + 1, out);
        } else if filename == "*" || name == filename {
            out.push(path);
        }
    }
}

fn should_skip_validation_dir(name: &str) -> bool {
    matches!(
        name,
        ".cache"
            | ".git"
            | ".hg"
            | ".svn"
            | ".xuanyu"
            | "AppData"
            | "Application Data"
            | "dist"
            | "Library"
            | "node_modules"
            | "out"
            | "target"
    ) || name.ends_with(".tmp")
        || name.ends_with(".temp")
}

fn diagnose_validation_failure(command: &str, output: &str, kind: Option<&str>) -> Value {
    let (file_paths, line_numbers) = extract_failure_locations(output);
    let stack = output
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("at ") || line.starts_with("Traceback") || line.starts_with("File \"")
        })
        .take(12)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let error_type = infer_validation_error_type(command, output, kind);
    let likely_causes = infer_validation_likely_causes(output, &error_type, &file_paths);
    let fingerprint_input = format!(
        "{command}\n{error_type}\n{}\n{}",
        file_paths
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("|"),
        normalize_numbers(&excerpt(output, 500))
    );
    json!({
        "errorType": error_type,
        "filePaths": file_paths.into_iter().take(20).collect::<Vec<_>>(),
        "lineNumbers": line_numbers.into_iter().take(20).collect::<Vec<_>>(),
        "stack": stack,
        "likelyCauses": likely_causes,
        "fingerprint": stable_hex(&fingerprint_input, 16),
        "occurrences": 1,
        "rawExcerpt": excerpt(output, 1800)
    })
}

async fn ingest_validation_failure_memory(
    memory: &MemoryService,
    call: &ToolCall,
    command: &str,
    diagnosis: &Value,
) -> Result<Value, String> {
    let error_type = diagnosis
        .get("errorType")
        .and_then(Value::as_str)
        .unwrap_or("validation")
        .to_string();
    let fingerprint = diagnosis
        .get("fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let file_paths = diagnosis_string_array(diagnosis, "filePaths");
    let line_numbers = diagnosis_u64_array(diagnosis, "lineNumbers");
    let likely_causes = diagnosis_string_array(diagnosis, "likelyCauses");
    let raw_excerpt = diagnosis
        .get("rawExcerpt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = [
        Some(format!("Error pattern: {error_type} {fingerprint}")),
        Some(format!("Command: {command}")),
        optional_joined_line("Files", &file_paths, ", "),
        optional_joined_line(
            "Lines",
            &line_numbers.iter().map(u64::to_string).collect::<Vec<_>>(),
            ", ",
        ),
        optional_joined_line("Likely causes", &likely_causes, "; "),
        optional_text_line("Excerpt", &excerpt(raw_excerpt, 600)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let session_id = string_arg_any(
        &call.args,
        &["sessionId", "session_id", "_sessionId", "_session_id"],
    )
    .map(str::to_string);
    let task_id = string_arg_any(&call.args, &["taskId", "task_id"]).map(str::to_string);
    let run_id = string_arg_any(&call.args, &["runId", "run_id"]).map(str::to_string);
    let trace_id = string_arg_any(&call.args, &["traceId", "trace_id"]).map(str::to_string);
    let tags = with_memory_session_tag(
        [
            "validation.failures".to_string(),
            "error_pattern".to_string(),
            error_type.clone(),
        ]
        .into_iter()
        .chain(file_paths.iter().take(4).cloned())
        .collect(),
        session_id.as_deref(),
    );
    let result = memory
        .ingest_memory_candidate(MemoryIngestInput {
            entry: NewMemoryEntry {
                kind: kind_for_category("validation.failures"),
                category: "validation.failures".to_string(),
                content,
                tags,
                context: session_id.clone(),
                importance: 0.7,
            },
            source: "validation.failures".to_string(),
            confidence: Some(0.78),
            reason: "Validation failure diagnosis captured for future recall.".to_string(),
            session_id,
            task_id,
            run_id,
            trace_id,
        })
        .await;
    Ok(json!({
        "status": result.status,
        "memoryId": result.memory.as_ref().map(|memory| memory.id.clone()),
        "pendingId": result.pending.as_ref().map(|pending| pending.id.clone()),
        "conflictCount": result.conflicts.len()
    }))
}

fn diagnosis_string_array(diagnosis: &Value, key: &str) -> Vec<String> {
    diagnosis
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn diagnosis_u64_array(diagnosis: &Value, key: &str) -> Vec<u64> {
    diagnosis
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn optional_joined_line(label: &str, values: &[String], separator: &str) -> Option<String> {
    (!values.is_empty()).then(|| format!("{label}: {}", values.join(separator)))
}

fn optional_text_line(label: &str, value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| format!("{label}: {}", value.trim()))
}

fn extract_failure_locations(output: &str) -> (Vec<String>, Vec<u64>) {
    let mut files = Vec::new();
    let mut lines = Vec::new();
    for line in output.lines() {
        for token in line.split_whitespace() {
            if let Some((file, line)) = extract_file_line_from_token(token) {
                push_unique(&mut files, file);
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
    }
    (files, lines)
}

fn extract_file_line_from_token(token: &str) -> Option<(String, u64)> {
    const EXTENSIONS: &[&str] = &[
        ".tsx", ".ts", ".jsx", ".js", ".json", ".scss", ".css", ".yaml", ".yml", ".md", ".py",
        ".java", ".go", ".rs", ".cpp", ".cs", ".xml", ".c", ".h",
    ];
    let cleaned = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
        )
    });
    let normalized = cleaned.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    for extension in EXTENSIONS {
        let Some(pos) = lower.find(extension) else {
            continue;
        };
        let end = pos + extension.len();
        let path = normalized[..end].trim_matches(':').to_string();
        let rest = normalized[end..].trim_start_matches([':', '(']);
        let digits = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        let line = digits.parse::<u64>().ok()?;
        if line > 0 && !path.is_empty() {
            return Some((path, line));
        }
    }
    None
}

fn infer_validation_error_type(command: &str, output: &str, kind: Option<&str>) -> String {
    let lower = format!("{command}\n{output}").to_ascii_lowercase();
    if matches!(kind, Some("typecheck"))
        || output.contains("TS")
        || lower.contains("tsc")
        || lower.contains("type error")
        || lower.contains("typeerror:")
    {
        "typecheck".to_string()
    } else if matches!(kind, Some("lint")) || lower.contains("eslint") || lower.contains("lint") {
        "lint".to_string()
    } else if matches!(kind, Some("build"))
        || lower.contains("build")
        || lower.contains("vite")
        || lower.contains("webpack")
        || lower.contains("rollup")
        || lower.contains("cannot find module")
        || lower.contains("module not found")
    {
        "build".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("exception")
        || lower.contains("error")
        || lower.contains("stack")
        || lower.contains("exit code")
    {
        "runtime".to_string()
    } else {
        "unknown".to_string()
    }
}

fn infer_validation_likely_causes(
    output: &str,
    error_type: &str,
    file_paths: &[String],
) -> Vec<String> {
    let lower = output.to_ascii_lowercase();
    let mut causes = Vec::new();
    if lower.contains("cannot find module") || lower.contains("module not found") {
        causes.push("Missing import, dependency, or build path alias.".to_string());
    }
    if output.contains("TS") {
        causes.push("TypeScript type mismatch or missing declaration.".to_string());
    }
    if lower.contains("undefined")
        || lower.contains("null")
        || lower.contains("cannot read properties")
    {
        causes
            .push("Runtime null/undefined access or incomplete state initialization.".to_string());
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        causes.push(
            "Command exceeded timeout or a background service did not become ready.".to_string(),
        );
    }
    if lower.contains("permission") || output.contains("EACCES") || output.contains("EPERM") {
        causes.push("Filesystem or process permission issue.".to_string());
    }
    if !file_paths.is_empty() {
        causes.push(
            "Failure points at specific files; prioritize those paths in the next fix.".to_string(),
        );
    }
    if causes.is_empty() {
        causes.push(format!(
            "{error_type} failure requires inspecting the captured excerpt and stack."
        ));
    }
    causes.into_iter().take(8).collect()
}

fn format_validation_diagnosis(diagnosis: &Value) -> String {
    let mut lines = vec![format!(
        "Error type: {}",
        diagnosis
            .get("errorType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    )];
    if let Some(files) = diagnosis.get("filePaths").and_then(Value::as_array) {
        if !files.is_empty() {
            lines.push(format!(
                "Files:\n{}",
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|file| format!("- {file}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
    if let Some(numbers) = diagnosis.get("lineNumbers").and_then(Value::as_array) {
        if !numbers.is_empty() {
            lines.push(format!(
                "Lines: {}",
                numbers
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if let Some(causes) = diagnosis.get("likelyCauses").and_then(Value::as_array) {
        if !causes.is_empty() {
            lines.push(format!(
                "Likely causes:\n{}",
                causes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|cause| format!("- {cause}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
    lines.push(format!(
        "Fingerprint: {} ({} occurrence{})",
        diagnosis
            .get("fingerprint")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        diagnosis
            .get("occurrences")
            .and_then(Value::as_u64)
            .unwrap_or(1),
        if diagnosis
            .get("occurrences")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            == 1
        {
            ""
        } else {
            "s"
        }
    ));
    lines.join("\n")
}

fn build_acceptance_report(
    criteria: Vec<String>,
    modified_files: Vec<String>,
    manual_evidence: Vec<String>,
) -> Value {
    let items = criteria
        .into_iter()
        .map(|criterion| {
            let mut evidence = Vec::new();
            if !modified_files.is_empty() {
                evidence.push(format!(
                    "diff: {}",
                    modified_files
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            evidence.extend(manual_evidence.iter().cloned());
            let has_evidence = !evidence.is_empty();
            json!({
                "criterion": criterion,
                "status": if has_evidence { "passed" } else { "missing" },
                "evidence": evidence,
                "missingEvidence": if has_evidence {
                    Vec::<String>::new()
                } else {
                    vec!["No diff, command, static check, or manual evidence is bound to this criterion.".to_string()]
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "passed": items.iter().all(|item| item.get("status").and_then(Value::as_str) == Some("passed")),
        "items": items
    })
}

fn format_acceptance_report(report: &Value) -> String {
    let Some(items) = report.get("items").and_then(Value::as_array) else {
        return "No acceptance criteria provided.".to_string();
    };
    if items.is_empty() {
        return "No acceptance criteria provided.".to_string();
    }
    items
        .iter()
        .map(|item| {
            let mut lines = vec![format!(
                "[{}] {}",
                item.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("missing"),
                item.get("criterion")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )];
            if let Some(evidence) = item.get("evidence").and_then(Value::as_array) {
                lines.extend(
                    evidence
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|entry| format!("  evidence: {entry}")),
                );
            }
            if let Some(missing) = item.get("missingEvidence").and_then(Value::as_array) {
                lines.extend(
                    missing
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|entry| format!("  missing: {entry}")),
                );
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_self_review_report(goal: &str, modified_files: Vec<String>) -> Value {
    let mut findings = Vec::new();
    if goal.trim().is_empty() {
        findings.push(json!({
            "severity": "blocking",
            "category": "goal",
            "message": "User goal is empty or was not preserved for review.",
            "evidence": []
        }));
    }

    let generated = modified_files
        .iter()
        .filter(|file| is_generated_or_dependency_path(file))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    if !generated.is_empty() {
        findings.push(json!({
            "severity": "warning",
            "category": "scope",
            "message": "Generated or dependency paths appear in the change set.",
            "evidence": generated
        }));
    }

    let large = modified_files
        .iter()
        .filter(|file| is_large_or_binary_path(file))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    if !large.is_empty() {
        findings.push(json!({
            "severity": "warning",
            "category": "large_file",
            "message": "Large or binary files are present in the change set.",
            "evidence": large
        }));
    }

    let sensitive = modified_files
        .iter()
        .filter(|file| is_sensitive_path(file))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    if !sensitive.is_empty() {
        findings.push(json!({
            "severity": "blocking",
            "category": "sensitive_file",
            "message": "Sensitive-looking files are present in the change set.",
            "evidence": sensitive
        }));
    }

    let passed = !findings
        .iter()
        .any(|finding| finding.get("severity").and_then(Value::as_str) == Some("blocking"));
    json!({ "passed": passed, "findings": findings })
}

fn format_self_review_report(report: &Value) -> String {
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if findings.is_empty() {
        return "Self review passed with no findings.".to_string();
    }
    findings
        .iter()
        .map(|finding| {
            let mut line = format!(
                "[{}/{}] {}",
                finding
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("info"),
                finding
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("goal"),
                finding
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            if let Some(evidence) = finding.get("evidence").and_then(Value::as_array) {
                let evidence = evidence
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                if !evidence.is_empty() {
                    line.push_str("\n  ");
                    line.push_str(&evidence.join("\n  "));
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_generated_or_dependency_path(path: &str) -> bool {
    let normalized = normalize_path_text(path);
    path_has_segment(&normalized, "node_modules")
        || path_has_segment(&normalized, "dist")
        || path_has_segment(&normalized, "coverage")
}

fn is_large_or_binary_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".zip", ".7z", ".tar", ".gz", ".png", ".jpg", ".jpeg", ".gif", ".mp4", ".mov", ".pdf",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn is_sensitive_path(path: &str) -> bool {
    let normalized = normalize_path_text(path);
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    name == ".env"
        || name == "id_rsa"
        || name.contains("secret")
        || name.contains("credential")
        || name.contains("token")
}

fn path_has_segment(path: &str, segment: &str) -> bool {
    path.split('/').any(|part| part == segment)
}

fn normalize_codebase_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn normalize_path_text(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn excerpt(output: &str, max_length: usize) -> String {
    let clean = output.replace("\r\n", "\n").trim().to_string();
    let chars = clean.chars().collect::<Vec<_>>();
    if chars.len() <= max_length {
        return clean;
    }
    let head = (max_length as f32 * 0.65) as usize;
    let tail = (max_length as f32 * 0.35) as usize;
    format!(
        "{}\n...\n{}",
        chars.iter().take(head).collect::<String>(),
        chars
            .iter()
            .skip(chars.len().saturating_sub(tail))
            .collect::<String>()
    )
}

fn normalize_numbers(value: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            if digits.len() >= 2 {
                out.push('#');
            } else {
                out.push_str(&digits);
            }
            digits.clear();
        }
        out.push(ch);
    }
    if !digits.is_empty() {
        if digits.len() >= 2 {
            out.push('#');
        } else {
            out.push_str(&digits);
        }
    }
    out
}

fn stable_hex(value: &str, length: usize) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}").chars().take(length).collect()
}

fn normalize_rel(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        text
    }
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).to_string()
}

fn unique_strings(items: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !value.is_empty() && !items.contains(&value) {
        items.push(value);
    }
}

struct MemoryProfileDefinition {
    kind: String,
    scope: String,
    path: String,
    relative_path: String,
    title: String,
    category: String,
    importance: f32,
    context: String,
}

async fn read_memory_profile(
    memory: &MemoryService,
    project_dir: &Path,
    kind: &str,
) -> Result<Value, String> {
    let definition = memory_profile_definition(project_dir, kind)?;
    let entry = find_memory_profile_entry(memory, &definition).await;
    Ok(memory_profile_value(&definition, entry.as_ref()))
}

async fn write_memory_profile(
    memory: &MemoryService,
    project_dir: &Path,
    kind: &str,
    content: &str,
) -> Result<Value, String> {
    let definition = memory_profile_definition(project_dir, kind)?;
    if let Some(existing) = find_memory_profile_entry(memory, &definition).await {
        let _ = memory
            .update_memory(
                &existing.id,
                MemoryUpdate {
                    kind: Some(kind_for_category(&definition.category)),
                    category: Some(definition.category.clone()),
                    content: Some(content.to_string()),
                    tags: Some(vec![
                        "profile".to_string(),
                        definition.kind.clone(),
                        definition.scope.clone(),
                    ]),
                    importance: Some(definition.importance),
                },
            )
            .await;
    } else {
        let _ = memory
            .add_memory(NewMemoryEntry {
                kind: kind_for_category(&definition.category),
                category: definition.category.clone(),
                content: content.to_string(),
                tags: vec![
                    "profile".to_string(),
                    definition.kind.clone(),
                    definition.scope.clone(),
                ],
                context: Some(definition.context.clone()),
                importance: definition.importance,
            })
            .await;
    }
    read_memory_profile(memory, project_dir, kind).await
}

async fn find_memory_profile_entry(
    memory: &MemoryService,
    definition: &MemoryProfileDefinition,
) -> Option<xuanyu_memory::MemoryEntry> {
    memory
        .recent(10_000)
        .await
        .into_iter()
        .filter(|entry| {
            entry.category == definition.category
                && entry.context.as_deref() == Some(definition.context.as_str())
                && entry.tags.iter().any(|tag| tag == "profile")
                && entry.tags.iter().any(|tag| tag == &definition.kind)
                && entry.tags.iter().any(|tag| tag == &definition.scope)
        })
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
}

fn memory_profile_definition(
    project_dir: &Path,
    kind: &str,
) -> Result<MemoryProfileDefinition, String> {
    let project_name = project_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("project");
    let (scope, title, category, importance) = match kind {
        "identity" => ("global", "XuanYu Identity", "identity", 1.0),
        "user" => ("global", "User Profile", "user_profile", 1.0),
        "project" => ("project", "Project Memory", "project_fact", 0.9),
        "agents" => ("project", "Agent Team", "workflow_rule", 0.8),
        "workflow" => ("project", "Workflow Rules", "workflow_rule", 0.8),
        "lessons" => ("project", "Lessons Learned", "task_lesson", 0.8),
        _ => return Err(format!("Unknown memory profile: {kind}")),
    };
    let context = if scope == "global" {
        "global".to_string()
    } else {
        format!("project:{}", path_to_string(project_dir))
    };
    let path = if scope == "global" {
        format!("memory://global/{kind}")
    } else {
        format!("memory://project/{project_name}/{kind}")
    };
    let relative_path = if scope == "global" {
        format!("db://memory/global/{kind}")
    } else {
        format!("db://memory/project/{project_name}/{kind}")
    };
    Ok(MemoryProfileDefinition {
        kind: kind.to_string(),
        scope: scope.to_string(),
        path,
        relative_path,
        title: title.to_string(),
        category: category.to_string(),
        importance,
        context,
    })
}

fn memory_profile_value(
    definition: &MemoryProfileDefinition,
    entry: Option<&xuanyu_memory::MemoryEntry>,
) -> Value {
    json!({
        "kind": definition.kind,
        "scope": definition.scope,
        "path": definition.path,
        "relativePath": definition.relative_path,
        "title": definition.title,
        "content": entry.map(|entry| entry.content.clone()).unwrap_or_default(),
        "exists": entry.is_some(),
        "updatedAt": entry.map(|entry| entry.updated_at.timestamp_millis()),
        "memoryId": entry.map(|entry| entry.id.clone())
    })
}

async fn command_cwd(call: &ToolCall) -> Result<PathBuf, String> {
    let cwd = string_arg(&call.args, "cwd")
        .map(|raw| {
            if Path::new(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                call.cwd.join(raw)
            }
        })
        .unwrap_or_else(|| call.cwd.clone());
    tokio::fs::canonicalize(&cwd)
        .await
        .map_err(|err| format!("cwd does not exist: {} ({err})", cwd.display()))
}

fn auto_for_risky_commands_enabled() -> bool {
    if std::env::var("XUANYU_AUTO_FOR_RISKY")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
    {
        return true;
    }
    for path in nearby_config_roots(None) {
        for file in ["config.json", "xuanyu.json"] {
            let candidate = path.join(file);
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if let Some(flag) = value
                        .pointer("/sandbox/autoForRiskyCommands")
                        .or_else(|| value.pointer("/sandbox/auto_for_risky_commands"))
                        .and_then(Value::as_bool)
                    {
                        return flag;
                    }
                }
            }
        }
    }
    false
}

fn load_nearby_app_config(project_dir: &Path) -> Value {
    for path in nearby_config_roots(Some(project_dir)) {
        for file in ["config.json", "xuanyu.json"] {
            let candidate = path.join(file);
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    return value;
                }
            }
        }
    }
    json!({})
}

fn nearby_config_roots(project_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(project_dir) = project_dir {
        roots.push(project_dir.to_path_buf());
        roots.push(project_data_root(project_dir));
    }
    roots.push(runtime_data_root());
    roots
}

fn looks_like_risky_exec(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    [
        "rm -rf",
        "rm -r ",
        "del /s",
        "format ",
        "mkfs",
        "dd if=",
        "shutdown",
        "reboot",
        "drop database",
        "truncate table",
        "git push --force",
        "git reset --hard",
        "npm publish",
        "cargo publish",
        "kubectl delete",
        "terraform destroy",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn spawn_output_reader<R>(mut reader: R, process: Arc<Mutex<BackgroundProcess>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = [0_u8; 8192];
        while let Ok(count) = reader.read(&mut buf) {
            if count == 0 {
                break;
            }
            let chunk = &buf[..count];
            let log_path = {
                let mut proc = process.lock().expect("process lock");
                proc.output.extend_from_slice(chunk);
                if proc.output.len() > 1024 * 1024 {
                    let overflow = proc.output.len() - 1024 * 1024;
                    proc.output.drain(0..overflow);
                }
                proc.log_path.clone()
            };
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                let _ = std::io::Write::write_all(&mut file, chunk);
            }
        }
    });
}

fn spawn_waiter(process: Arc<Mutex<BackgroundProcess>>) {
    thread::spawn(move || {
        let child = {
            let proc = process.lock().expect("process lock");
            proc.child.clone()
        };
        let status = child.lock().expect("child lock").wait();
        let mut proc = process.lock().expect("process lock");
        proc.completed_at = Some(now_ms());
        match status {
            Ok(status) => {
                proc.exit_code = status.code();
                if proc.stop_requested {
                    proc.status = "stopped".to_string();
                } else if status.success() {
                    proc.status = "completed".to_string();
                } else {
                    proc.status = "failed".to_string();
                }
            }
            Err(err) => {
                proc.status = "failed".to_string();
                proc.output
                    .extend_from_slice(format!("\n[Process wait error] {err}\n").as_bytes());
            }
        }
    });
}

fn terminate_child(proc: &BackgroundProcess) {
    if let Some(pid) = proc.pid {
        if cfg!(windows) {
            let _ = Command::new("taskkill")
                .args(["/pid", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            return;
        }
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        return;
    }
    let _ = proc.child.lock().expect("child lock").kill();
}

fn snapshot(process: &Arc<Mutex<BackgroundProcess>>) -> BackgroundProcessSnapshot {
    let guard = process.lock().expect("process lock");
    snapshot_from_guard(&guard)
}

fn snapshot_from_guard(proc: &BackgroundProcess) -> BackgroundProcessSnapshot {
    BackgroundProcessSnapshot {
        id: proc.id.clone(),
        pid: proc.pid,
        name: proc.name.clone(),
        command: proc.command.clone(),
        cwd: path_to_string(&proc.cwd),
        log_path: path_to_string(&proc.log_path),
        status: proc.status.clone(),
        started_at: proc.started_at,
        completed_at: proc.completed_at,
        exit_code: proc.exit_code,
        signal: proc.signal.clone(),
        output_bytes: output_len(proc),
    }
}

fn read_process_log(proc: &BackgroundProcess, offset: usize, max_bytes: usize) -> (String, usize) {
    if let Ok(mut file) = std::fs::File::open(&proc.log_path) {
        let size = file.metadata().map(|meta| meta.len() as usize).unwrap_or(0);
        let start = offset.min(size);
        if size > start && file.seek(SeekFrom::Start(start as u64)).is_ok() {
            let mut buf = vec![0_u8; max_bytes.min(size.saturating_sub(start))];
            if file.read_exact(&mut buf).is_ok() {
                return (String::from_utf8_lossy(&buf).to_string(), start + buf.len());
            }
        }
    }

    let start = offset.min(proc.output.len());
    let end = (start + max_bytes).min(proc.output.len());
    (
        String::from_utf8_lossy(&proc.output[start..end]).to_string(),
        end,
    )
}

fn output_len(proc: &BackgroundProcess) -> usize {
    let file_len = std::fs::metadata(&proc.log_path)
        .map(|metadata| metadata.len() as usize)
        .unwrap_or(0);
    file_len.max(proc.output.len())
}

fn truncate_text(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!(
            "{}\n\n[truncated] Output capped at {max} chars. Re-run with a narrower command, or read the process log with offset if available.",
            value.chars().take(max).collect::<String>()
        )
    }
}

fn empty_process_output(content: String, exit_code: Option<i32>) -> String {
    if content == "(no new output)" || content.trim().is_empty() {
        format!("Command exited with code {exit_code:?}")
    } else {
        content
    }
}

fn path_to_string(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = text.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = text.strip_prefix("//?/") {
        rest.to_string()
    } else {
        text
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

async fn atomic_write_file(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("xy")
    ));
    async_fs::write(&tmp, content).await?;
    async_fs::rename(tmp, path).await
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(path) = current {
        if path.exists() {
            return Some(path);
        }
        current = path.parent();
    }
    None
}

fn display_relative(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn count_line_changes(before: &str, after: &str) -> (usize, usize) {
    let before = normalize_newlines(before);
    let after = normalize_newlines(after);
    if before == after {
        return (0, 0);
    }
    let diff = similar::TextDiff::from_lines(&before, &after);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

fn create_unified_diff(path: &Path, before: &str, after: &str, cwd: &Path) -> String {
    let relative = display_relative(path, cwd);
    let before = normalize_newlines(before);
    let after = normalize_newlines(after);
    let old_header = if before.is_empty() {
        "/dev/null".to_string()
    } else {
        format!("a/{relative}")
    };
    let new_header = if after.is_empty() {
        "/dev/null".to_string()
    } else {
        format!("b/{relative}")
    };
    let diff = similar::TextDiff::from_lines(&before, &after);
    let body = diff
        .unified_diff()
        .context_radius(3)
        .header(&old_header, &new_header)
        .to_string();
    // Keep a git-style header so UI parsePatch can match file paths reliably.
    let mut out = format!("diff --git a/{relative} b/{relative}\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn args_touch_secret(args: &BTreeMap<String, serde_json::Value>) -> bool {
    args.values().any(|value| {
        let text = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        let lower = text.to_ascii_lowercase();
        [
            ".env",
            "id_rsa",
            "id_ed25519",
            "api_key",
            "secret",
            "token",
            "password",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    })
}

fn affected_files_for_ledger(output: &ToolOutput, fallback: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut files = output
        .data
        .as_ref()
        .and_then(|data| data.get("affectedFiles"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.extend(fallback.iter().cloned());
    files
        .into_iter()
        .filter(|file| !file.trim().is_empty())
        .filter(|file| seen.insert(file.clone()))
        .collect()
}

fn append_turn_ledger_warning(output: &mut ToolOutput, err: anyhow::Error) {
    let warning = format!("Turn ledger warning: {err}");
    if output.content.is_empty() {
        output.content = warning.clone();
    } else {
        output.content.push_str("\n\n");
        output.content.push_str(&warning);
    }
    match output.data.as_mut() {
        Some(Value::Object(object)) => {
            object.insert(
                "turnLedgerError".to_string(),
                Value::String(err.to_string()),
            );
        }
        _ => {
            output.data = Some(json!({ "turnLedgerError": err.to_string() }));
        }
    }
}

fn infer_tool_source(name: &str, fallback: ToolSource) -> ToolSource {
    if name.starts_with("mcp__") {
        ToolSource::Mcp
    } else {
        fallback
    }
}

fn normalize_path(path: &Path) -> String {
    // Strip Windows `\\?\` / `//?/` so tool output stays human-readable.
    path_to_string(path)
}

async fn write_scope_relative_path(cwd: &Path, path: &Path) -> String {
    let base = async_fs::canonicalize(cwd)
        .await
        .unwrap_or_else(|_| cwd.to_path_buf());
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    if let Ok(relative) = candidate.strip_prefix(cwd) {
        return normalize_scope_path(&relative.to_string_lossy());
    }
    if let Ok(relative) = candidate.strip_prefix(&base) {
        return normalize_scope_path(&relative.to_string_lossy());
    }

    let base_text = normalize_scope_path(&base.to_string_lossy());
    let candidate_text = normalize_scope_path(&candidate.to_string_lossy());
    let base_prefix = format!("{base_text}/");
    candidate_text
        .strip_prefix(&base_prefix)
        .map(str::to_string)
        .unwrap_or(candidate_text)
}

fn glob_scope_matches(pattern: &str, relative: &str) -> bool {
    let pattern = normalize_scope_path(pattern);
    let relative = normalize_scope_path(relative);
    if pattern.is_empty() {
        return relative.is_empty();
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == relative;
    }
    let pattern_segments = pattern.split('/').collect::<Vec<_>>();
    let relative_segments = if relative.is_empty() {
        Vec::new()
    } else {
        relative.split('/').collect::<Vec<_>>()
    };
    glob_scope_segments_match(&pattern_segments, &relative_segments)
}

fn glob_scope_segments_match(pattern: &[&str], relative: &[&str]) -> bool {
    if pattern.is_empty() {
        return relative.is_empty();
    }
    if pattern[0] == "**" {
        return glob_scope_segments_match(&pattern[1..], relative)
            || (!relative.is_empty() && glob_scope_segments_match(pattern, &relative[1..]));
    }
    !relative.is_empty()
        && wildcard_match(pattern[0], relative[0])
        && glob_scope_segments_match(&pattern[1..], &relative[1..])
}

fn normalize_scope_path(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        parts.push(part);
    }
    parts.join("/")
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_bytes(pattern.as_bytes(), text.as_bytes())
}

fn wildcard_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard_match_bytes(&pattern[1..], text)
                || (!text.is_empty() && wildcard_match_bytes(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && wildcard_match_bytes(&pattern[1..], &text[1..]),
        byte => {
            !text.is_empty() && byte == text[0] && wildcard_match_bytes(&pattern[1..], &text[1..])
        }
    }
}
