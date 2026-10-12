use anyhow::{anyhow, bail, Context};
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        Mutex as StdMutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

mod agent;
mod budget;
mod context_assembler;
mod context_compact;
mod context_surface;
mod main_turn;
mod main_turn_dispatch;
mod main_turn_loop;
mod message_sequence;
mod prepare_turn_context;
mod session_context;
mod stream_write_preview;
mod token_counter;
mod tool_call_loop;
mod tool_loop;
mod turn_memory_cache;
mod turn_state;

pub use main_turn::{
    bootstrap_main_turn, iteration_input, max_iterations_from_config,
    MainTurnBootstrap, MainTurnLoopState,
};
pub use main_turn_dispatch::{
    finalize_main_turn_tool_dispatch, patch_retrieved_memory_context,
    FinalizeMainTurnToolDispatchInput,
};
pub use main_turn_loop::{
    run_main_turn_loop, MainTurnLoopConfig, MainTurnLoopHost, MainTurnLoopResult,
    MainTurnModelResponse, MainTurnRuntimeToolCall, MainTurnToolDispatchResult,
    MainTurnToolResultMessage,
};

pub use context_assembler::{
    active_context_sections, context_available_sections, context_load_mode_from_config,
    context_load_policy, context_section_activation_reason, context_section_injection_gate,
    context_surface_snapshot, mark_lazy_sections_for_refresh, merge_lazy_context_override,
    policy_auto_sections, policy_bool, policy_refresh_sections, prepare_turn,
    should_auto_retrieve_memory_for_policy, should_inject_context_section,
    should_inject_key_facts,     should_refresh_memory_profile, strip_refresh_sections, memory_auto_inject_enabled,
    memory_auto_inject_include_cross_session, memory_auto_inject_limit, memory_auto_inject_min_score,
    LazyContextOverride, PrepareTurnInput, PrepareTurnPlan,
};
pub use context_surface::{
    is_context_section_name, section_label, ContextLoadResult, ContextSectionRejected,
    ContextSurface, CONTEXT_SECTION_NAMES, sections_need_memory_cache_invalidate,
};
pub use context_compact::{
    snip_message, should_compact, select_compact_keep_index, select_compact_keep_index_by_roles,
    CompactStrategyKind, MAX_COMPACTION_ATTEMPTS, MIN_MESSAGES_FOR_COMPACT, PRESERVE_RECENT_COUNT,
    PRESERVE_RECENT_USER_ROUNDS,
};
pub use prepare_turn_context::{PrepareTurnContextInput, PrepareTurnContextState};
pub use tool_call_loop::ToolCallLoop;
pub use turn_memory_cache::TurnMemoryCache;
pub use tool_loop::{
    arg_shape, text_similarity, LoopIntervention, ToolLoopGuard, TurnFingerprint,
};
pub use stream_write_preview::{
    is_mid_turn_write_tool, is_streamable_write_tool, preview_stream_write_args, StreamWritePreview,
};
pub use turn_state::TurnPhase;

pub use budget::{
    section_token_allocation, BudgetSectionInput, ContextBudgetAllocation, ContextBudgetAllocator,
    ContextBudgetSection,
};
pub use message_sequence::{
    protocol_message_from_record, sanitize_messages_for_tool_protocol, ProtocolMessage,
    ProtocolToolCallSpec, SanitizedMessageSequence,
};
pub use session_context::{
    pack_session_context, recent_dialogue_token_budget, PackSessionContextInput,
    PackedSessionContext, SessionHandoffSummary, COMPACT_RECENT_MAX_TOKENS,
    PROTECT_RECENT_TOOL_RESULT_TOKENS,
};
pub use token_counter::{
    count_message_roles, estimate_tokens, estimate_tool_schema_tokens, max_input_tokens,
    set_token_counter_runtime_config, token_counter_backend_from_value, MessageRoleKind,
    TokenCountBreakdown, TokenCounterBackend, TokenCounterConfig,
};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const MAX_QUEUE_DEPTH: usize = 100;
const RATE_LIMIT_FALLBACK_DEGRADE_MS: u64 = 30_000;
const CIRCUIT_BACKOFF_STEPS_MS: [u64; 4] = [30_000, 60_000, 180_000, 300_000];
const DEFAULT_SYSTEM_PROMPT: &str = concat!(
    "You are XuanYu (玄语), the main agent.\n\n",
    "## Language\n",
    "- Match the user's language for all user-visible text (status lines between tools, finals, questions).\n",
    "- Chinese user → Simplified Chinese for the whole turn; do not switch mid-turn.\n",
    "- Keep code, paths, commands, and quoted logs as written.\n",
    "\n",
    "## Core Behavior\n",
    "- Be concise and concrete. Avoid emoji and performative closings unless asked.\n",
    "- Use native tool calls only; never fake tool calls in plain text.\n",
    "- Core coding tools (file_read/file_write/file_edit/exec/search/glob/…) are already active — call them directly. Other tools: pick a name from the Tool catalog, then load_tools([\"name\"]) before calling. Prefer targeted search/list before whole files; summarize long command output.\n",
    "- Paths: tool paths are relative to cwd. list_files shows `.` as the workspace root — never prefix paths with the workspace folder name (wrong: book/backend/x.py when cwd is already .../book).\n",
    "- If list_files / Get-ChildItem returns empty or \"(empty directory)\", treat the folder as empty and proceed — do not re-run the same listing tools in a loop.\n",
    "- After a successful file_write / file_edit, trust the result — do not re-read the same path only to verify unless the tool errored or you need a different slice.\n",
    "- Prefer conversation/context you already have over rediscovering the workspace from scratch.\n",
    "- file_read also reads local images (vision) and PDFs (text extract). Prefer it over asking the user to describe screenshots on disk.\n",
    "- When blocked by missing preference or a discrete choice, call ask_user (if enabled) instead of guessing.\n",
    "- Durable memory: search_memory → load_memories (sections include memory_profile). Update profiles with memory tools only — never file_write to memory files.\n",
    "- If a tool fails, state why and continue with the best answer you can give.\n",
    "- Work tracking: one board. Use todo_write for multi-step checklists. If the user pinned an objective with /goal, it appears in context — keep working toward it and call update_goal only when verified complete. Prefer submit_task/agent for specialist/parallel work. enter_plan_mode is read-only planning until exit_plan_mode after approval.\n",
    "- Do not invent a second goal system. Objective status is in context / Goal bar — do not probe with tools.\n",
    "\n",
    "## Workspace focus\n",
    "- The working directory may contain unrelated sibling projects. After one quick look (list/glob), ignore them.\n",
    "- When creating a new app here: make one dedicated project folder, then keep EVERY file path and every build/test exec cwd under that folder only.\n",
    "- Do not hunt \"workspace roots\", sibling package.json/pom.xml, or foreign dependencies. Verify with the command for THIS project using cwd=that folder.\n",
    "- Match the host OS for shell commands (Windows PowerShell vs POSIX).\n",
    "- On Windows prefer PowerShell (Get-ChildItem / Get-Location). Avoid POSIX-only chains like `pwd && ls -la`.\n",
    "- Prefer `;` not `&&` for chaining on PowerShell. Avoid Format-Table/Format-List when you need captured stdout — use Select-Object | Out-String instead.\n",
    "\n",
    "## Git\n",
    "- Prefer structured git_* (load_tools then call): git_status, git_diff, git_log, git_blame, git_branch (list/create only), git_commit, git_patch_create/apply.\n",
    "- For switch/checkout/fetch/pull/push use exec. Destructive git is blocked (reset/clean/rebase/force-push) — same policy for git_* and exec.\n",
    "- Remote PRs/MRs: ask the user first, then use exec with gh/glab — never invent a dedicated PR-open tool.\n",
    "\n",
    "## Delegation\n",
    "Handle small single-scope work yourself; use submit_task (alias: agent) for multi-step, parallel, or specialist work.\n",
    "Assign by agentId:\n",
    "- developer: code, builds, bug fixes\n",
    "- explorer: read-only investigation\n",
    "- architect: design docs and technical plans\n",
    "- reviewer: review and risk reports\n",
    "- doc-writer: user docs and changelogs\n",
    "Do not nest further delegation from specialists.\n",
    "\n",
    "## Tools\n",
    "Already active each turn: file_read, file_write, file_edit, list_files, glob, search, exec, submit_task — call these directly; do not load_tools first.\n",
    "- Deferred tools: the Tool catalog lists name + short summary. Call load_tools([\"exact_name\"]) then invoke on the next iteration.\n",
    "- Memory/context: search_memory → load_memories (sections: memory, memory_profile, project_instructions).\n",
    "- Skills: activate_skill(id) only — do not use load_memories for skills.\n",
    "\n",
    "Be concise."
);
const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_RETRY_DELAY_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTaskKind {
    Main,
    Title,
    Summary,
    Memory,
    Intent,
    KeyFacts,
    Planning,
    Implementation,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub provider_type: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model_path: Option<String>,
    pub protocol: Option<String>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub retry: Option<u8>,
    pub extra_body: BTreeMap<String, serde_json::Value>,
}

impl ProviderConfig {
    pub fn openai_compatible(model: impl Into<String>) -> Self {
        Self {
            provider_type: "openai-compatible".to_string(),
            api_key: None,
            base_url: None,
            model_path: None,
            protocol: None,
            model: model.into(),
            max_tokens: Some(16384),
            temperature: Some(0.3),
            retry: Some(2),
            extra_body: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelSettings {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model_path: Option<String>,
    pub protocol: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub retry: Option<u8>,
    pub fallback: Option<Box<AgentModelSettings>>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRateLimitSettings {
    pub rpm: Option<u32>,
    pub concurrency: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRoutingConfig {
    pub auxiliary_models: BTreeMap<ModelTaskKind, AgentModelSettings>,
    pub fallback: Option<AgentModelSettings>,
    pub rate_limits: BTreeMap<ModelTaskKind, ModelRateLimitSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConfig {
    pub provider: ProviderConfig,
    pub model_routing: ModelRoutingConfig,
}

impl ExecutionConfig {
    pub fn from_app_config_value(value: &serde_json::Value) -> Self {
        let provider = provider_config_from_json(value.get("provider"));
        let mut model_routing = ModelRoutingConfig::default();
        if let Some(fallback) = value
            .get("provider")
            .and_then(|provider| provider.get("fallback"))
            .and_then(settings_from_json)
            .or_else(|| {
                value
                    .get("modelRouting")
                    .and_then(|routing| routing.get("fallback"))
                    .and_then(settings_from_json)
            })
        {
            model_routing.fallback = Some(fallback);
        }
        if let Some(auxiliary) = value
            .get("modelRouting")
            .and_then(|routing| routing.get("auxiliaryModels"))
            .and_then(serde_json::Value::as_object)
        {
            for (task, settings) in auxiliary {
                if let (Some(kind), Some(settings)) =
                    (task_kind_from_key(task), settings_from_json(settings))
                {
                    model_routing.auxiliary_models.insert(kind, settings);
                }
            }
        }
        if let Some(rate_limits) = value
            .get("modelRouting")
            .and_then(|routing| routing.get("rateLimits"))
            .and_then(serde_json::Value::as_object)
        {
            for (task, rate_limit) in rate_limits {
                if let Some(kind) = task_kind_from_key(task) {
                    model_routing
                        .rate_limits
                        .insert(kind, rate_limit_from_json(rate_limit));
                }
            }
        }
        Self {
            provider,
            model_routing,
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            provider: ProviderConfig::openai_compatible("default"),
            model_routing: ModelRoutingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub json_mode: bool,
    pub thinking_disable: bool,
    pub tool_calling: bool,
    pub vision: bool,
    pub rate_limit: ModelRateLimitSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCheck {
    pub compatible: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedModelRoute {
    pub task: ModelTaskKind,
    pub provider: ProviderConfig,
    pub capabilities: ModelCapabilities,
    pub compatibility: CapabilityCheck,
    pub inherited: bool,
    pub fallback: Option<Box<ResolvedModelRoute>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouterEvent {
    pub event_type: String,
    pub task: ModelTaskKind,
    pub provider: String,
    pub model: String,
    pub queued: usize,
    pub active: usize,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelRouter {
    config: ExecutionConfig,
}

#[derive(Debug, Clone, Default)]
struct CircuitState {
    failures: u32,
    degraded_until_ms: u64,
}

#[derive(Debug, Default)]
struct RouteQueueState {
    active: usize,
    last_start: Option<Instant>,
    waiting: usize,
}

#[derive(Debug, Default)]
struct RouterRuntimeState {
    circuits: HashMap<String, CircuitState>,
    queues: HashMap<String, RouteQueueState>,
}

impl ModelRouter {
    pub fn new(config: ExecutionConfig) -> Self {
        Self { config }
    }

    pub fn update_config(&mut self, config: ExecutionConfig) {
        self.config = config;
    }

    pub fn resolve(&self, task: ModelTaskKind) -> ResolvedModelRoute {
        let main = &self.config.provider;
        let override_settings = if task == ModelTaskKind::Main {
            None
        } else {
            self.config.model_routing.auxiliary_models.get(&task)
        };
        let provider = provider_from_settings(main, override_settings);
        let tuned = tune_provider_for_task(provider, task);
        let capabilities = resolve_model_capabilities(&tuned);
        let fallback = override_settings
            .and_then(|settings| settings.fallback.as_deref())
            .or(self.config.model_routing.fallback.as_ref())
            .and_then(|settings| self.resolve_fallback(task, settings, &tuned));
        ResolvedModelRoute {
            task,
            provider: tuned,
            compatibility: check_task_compatibility(task, &capabilities),
            capabilities,
            inherited: override_settings
                .map(|settings| settings.provider.is_none() && settings.model.is_none())
                .unwrap_or(true),
            fallback: fallback.map(Box::new),
        }
    }

    fn resolve_fallback(
        &self,
        task: ModelTaskKind,
        settings: &AgentModelSettings,
        primary: &ProviderConfig,
    ) -> Option<ResolvedModelRoute> {
        let provider = provider_from_settings(primary, Some(settings));
        if same_provider_route(&provider, primary) {
            return None;
        }
        let tuned = tune_provider_for_task(provider, task);
        let capabilities = resolve_model_capabilities(&tuned);
        Some(ResolvedModelRoute {
            task,
            provider: tuned,
            compatibility: check_task_compatibility(task, &capabilities),
            capabilities,
            inherited: false,
            fallback: None,
        })
    }

    pub fn resolve_rate_limit(
        &self,
        task: ModelTaskKind,
        route: &ResolvedModelRoute,
    ) -> ModelRateLimitSettings {
        self.config
            .model_routing
            .rate_limits
            .get(&task)
            .copied()
            .or(route.capabilities.rate_limit.into())
            .unwrap_or_else(|| {
                if task == ModelTaskKind::Main {
                    ModelRateLimitSettings {
                        rpm: None,
                        concurrency: Some(1),
                    }
                } else {
                    ModelRateLimitSettings {
                        rpm: Some(12),
                        concurrency: Some(2),
                    }
                }
            })
    }
}

pub struct ExecutionRuntime {
    router: RwLock<ModelRouter>,
    http: reqwest::Client,
    route_state: StdMutex<RouterRuntimeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Default)]
pub struct GenerationOptions {
    pub deep_thinking: bool,
    pub on_delta: Option<Arc<dyn Fn(String) + Send + Sync>>,
    /// Fired when a streamed tool call is sealed (next index starts or stream ends).
    pub on_tool_call_ready: Option<Arc<dyn Fn(ProviderToolCall) + Send + Sync>>,
    /// Fired on streaming tool-call argument deltas (partial JSON).
    pub on_tool_call_delta: Option<Arc<dyn Fn(ProviderToolCall) + Send + Sync>>,
    pub tools: Option<Vec<ProviderToolDefinition>>,
    /// Prior packed session turns (user/assistant/tool), excluding the current user turn.
    pub history: Vec<ProtocolMessage>,
}

/// Bundled completion inputs shared by the private route/completion helpers.
struct ChatCompletionRequest<'a> {
    user_input: &'a str,
    system_context: Option<&'a str>,
    history: &'a [ProtocolMessage],
    on_delta: Option<Arc<dyn Fn(String) + Send + Sync>>,
    on_tool_call_ready: Option<Arc<dyn Fn(ProviderToolCall) + Send + Sync>>,
    on_tool_call_delta: Option<Arc<dyn Fn(ProviderToolCall) + Send + Sync>>,
    tools: Option<&'a [ProviderToolDefinition]>,
}

impl ExecutionRuntime {
    pub fn new(config: ExecutionConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            router: RwLock::new(ModelRouter::new(config)),
            http,
            route_state: StdMutex::new(RouterRuntimeState::default()),
        }
    }

    pub async fn resolve_route(&self, task: ModelTaskKind) -> ResolvedModelRoute {
        self.router.read().await.resolve(task)
    }

    pub async fn resolve_rate_limit(
        &self,
        task: ModelTaskKind,
        route: &ResolvedModelRoute,
    ) -> ModelRateLimitSettings {
        self.router.read().await.resolve_rate_limit(task, route)
    }

    pub async fn update_config(&self, config: ExecutionConfig) {
        self.router.write().await.update_config(config);
    }

    pub async fn generate_reply(&self, user_input: &str) -> ModelResponse {
        self.generate_reply_with_context(user_input, None).await
    }

    pub async fn generate_reply_with_context(
        &self,
        user_input: &str,
        system_context: Option<&str>,
    ) -> ModelResponse {
        self.generate_reply_with_context_and_options(
            user_input,
            system_context,
            GenerationOptions::default(),
        )
        .await
    }

    pub async fn generate_reply_with_context_and_options(
        &self,
        user_input: &str,
        system_context: Option<&str>,
        options: GenerationOptions,
    ) -> ModelResponse {
        self.generate_for_task_with_options(
            ModelTaskKind::Main,
            user_input,
            system_context,
            options,
        )
        .await
    }

    pub async fn generate_for_task(
        &self,
        task: ModelTaskKind,
        user_input: &str,
        system_context: Option<&str>,
    ) -> ModelResponse {
        self.generate_for_task_with_options(
            task,
            user_input,
            system_context,
            GenerationOptions::default(),
        )
        .await
    }

    pub async fn generate_for_task_with_options(
        &self,
        task: ModelTaskKind,
        user_input: &str,
        system_context: Option<&str>,
        options: GenerationOptions,
    ) -> ModelResponse {
        let route = apply_generation_options_to_route(self.resolve_route(task).await, options.clone());
        let streamed_flag = Arc::new(AtomicBool::new(false));
        let on_delta = options.on_delta.as_ref().map(|callback| {
            let callback = callback.clone();
            let streamed_flag = streamed_flag.clone();
            Arc::new(move |content: String| {
                if !content.is_empty() {
                    streamed_flag.store(true, Ordering::Relaxed);
                    callback(content);
                }
            }) as Arc<dyn Fn(String) + Send + Sync>
        });
        let tools = options.tools.clone();
        let history = options.history.clone();
        let on_tool_call_ready = options.on_tool_call_ready.clone();
        let on_tool_call_delta = options.on_tool_call_delta.clone();
        // Stream text deltas even when tools are present; tool_call chunks are accumulated.
        let rate_limit = self.resolve_rate_limit(task, &route).await;
        let (provider, result) = self
            .run_with_route_controls(
                &route,
                &rate_limit,
                ChatCompletionRequest {
                    user_input,
                    system_context,
                    history: &history,
                    on_delta,
                    on_tool_call_ready,
                    on_tool_call_delta,
                    tools: tools.as_deref(),
                },
            )
            .await;
        let (text, reasoning, tool_calls, finish_reason, error, usage) = match result {
            Ok(completion) => (
                completion.content,
                completion.reasoning,
                completion.tool_calls,
                completion.finish_reason,
                None,
                completion.usage,
            ),
            Err(error) => {
                let error = error.to_string();
                (
                    model_error_reply(&provider, user_input, &error),
                    None,
                    Vec::new(),
                    None,
                    Some(error),
                    None,
                )
            }
        };
        ModelResponse {
            task,
            provider: provider.provider_type,
            model: provider.model,
            text,
            reasoning,
            tool_calls,
            finish_reason,
            error,
            streamed: streamed_flag.load(Ordering::Relaxed),
            usage,
        }
    }

    async fn run_with_route_controls(
        &self,
        route: &ResolvedModelRoute,
        rate_limit: &ModelRateLimitSettings,
        request: ChatCompletionRequest<'_>,
    ) -> (ProviderConfig, anyhow::Result<ChatCompletion>) {
        let queue_key = format!(
            "{}:{}/{}",
            format!("{:?}", route.task).to_ascii_lowercase(),
            route.provider.provider_type,
            route.provider.model
        );
        if let Err(err) = self.acquire_route_slot(&queue_key, rate_limit).await {
            return (route.provider.clone(), Err(err));
        }
        let result = self
            .complete_route_with_fallback(route, request)
            .await;
        self.release_route_slot(&queue_key);
        result
    }

    async fn acquire_route_slot(
        &self,
        key: &str,
        rate_limit: &ModelRateLimitSettings,
    ) -> anyhow::Result<()> {
        let concurrency = rate_limit.concurrency.unwrap_or(1).max(1) as usize;
        let rpm = rate_limit.rpm.unwrap_or(0);
        loop {
            let wait_ms = {
                let mut state = self
                    .route_state
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                let queue = state.queues.entry(key.to_string()).or_default();
                if queue.active >= concurrency {
                    if queue.waiting >= MAX_QUEUE_DEPTH {
                        bail!("Model router queue depth exceeded ({MAX_QUEUE_DEPTH}) for {key}");
                    }
                    queue.waiting += 1;
                    Some(25u64)
                } else if rpm > 0 {
                    if let Some(last_start) = queue.last_start {
                        let min_gap = Duration::from_millis((60_000 / rpm.max(1)) as u64);
                        let elapsed = last_start.elapsed();
                        if elapsed < min_gap {
                            if queue.waiting >= MAX_QUEUE_DEPTH {
                                bail!(
                                    "Model router queue depth exceeded ({MAX_QUEUE_DEPTH}) for {key}"
                                );
                            }
                            queue.waiting += 1;
                            Some((min_gap - elapsed).as_millis() as u64)
                        } else {
                            queue.active += 1;
                            queue.last_start = Some(Instant::now());
                            None
                        }
                    } else {
                        queue.active += 1;
                        queue.last_start = Some(Instant::now());
                        None
                    }
                } else {
                    queue.active += 1;
                    queue.last_start = Some(Instant::now());
                    None
                }
            };
            match wait_ms {
                Some(ms) => {
                    tokio::time::sleep(Duration::from_millis(ms.max(1))).await;
                    let mut state = self
                        .route_state
                        .lock()
                        .unwrap_or_else(|err| err.into_inner());
                    if let Some(queue) = state.queues.get_mut(key) {
                        queue.waiting = queue.waiting.saturating_sub(1);
                    }
                }
                None => return Ok(()),
            }
        }
    }

    fn release_route_slot(&self, key: &str) {
        let mut state = self
            .route_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(queue) = state.queues.get_mut(key) {
            queue.active = queue.active.saturating_sub(1);
        }
    }

    async fn complete_route_with_fallback(
        &self,
        route: &ResolvedModelRoute,
        request: ChatCompletionRequest<'_>,
    ) -> (ProviderConfig, anyhow::Result<ChatCompletion>) {
        let ChatCompletionRequest {
            user_input,
            system_context,
            history,
            on_delta,
            on_tool_call_ready,
            on_tool_call_delta,
            tools,
        } = request;
        let selected = self.select_route_for_circuit(route);
        let primary = selected.provider.clone();
        let emitted = Arc::new(AtomicBool::new(false));
        let on_delta = on_delta.map(|callback| {
            let emitted = emitted.clone();
            Arc::new(move |content: String| {
                if !content.is_empty() {
                    emitted.store(true, Ordering::Relaxed);
                }
                callback(content);
            }) as Arc<dyn Fn(String) + Send + Sync>
        });
        match self
            .complete_provider(
                &primary,
                ChatCompletionRequest {
                    user_input,
                    system_context,
                    history,
                    on_delta: on_delta.clone(),
                    on_tool_call_ready: on_tool_call_ready.clone(),
                    on_tool_call_delta: on_tool_call_delta.clone(),
                    tools,
                },
            )
            .await
        {
            Ok(completion) => {
                self.note_route_success(&primary);
                (primary, Ok(completion))
            }
            Err(primary_error) => {
                let kind = classify_provider_error(&primary_error);
                self.note_route_failure(&primary, kind, &primary_error);
                if kind == ProviderErrorKind::ClientError || emitted.load(Ordering::Relaxed) {
                    return (primary, Err(primary_error));
                }
                let Some(fallback) = selected.fallback.as_deref().or(route.fallback.as_deref())
                else {
                    return (primary, Err(primary_error));
                };
                if !self.route_is_available(&fallback.provider) {
                    return (primary, Err(primary_error));
                }
                let fallback_provider = fallback.provider.clone();
                match self
                    .complete_provider(
                        &fallback_provider,
                        ChatCompletionRequest {
                            user_input,
                            system_context,
                            history,
                            on_delta,
                            on_tool_call_ready,
                            on_tool_call_delta,
                            tools,
                        },
                    )
                    .await
                {
                    Ok(completion) => {
                        self.note_route_success(&fallback_provider);
                        (fallback_provider, Ok(completion))
                    }
                    Err(fallback_error) => {
                        self.note_route_failure(
                            &fallback_provider,
                            classify_provider_error(&fallback_error),
                            &fallback_error,
                        );
                        (
                            primary,
                            Err(anyhow!(
                                "primary route failed: {}; fallback route {}/{} failed: {}",
                                primary_error,
                                fallback_provider.provider_type,
                                fallback_provider.model,
                                fallback_error
                            )),
                        )
                    }
                }
            }
        }
    }

    fn select_route_for_circuit<'a>(&self, route: &'a ResolvedModelRoute) -> &'a ResolvedModelRoute {
        if self.route_is_available(&route.provider) {
            return route;
        }
        if let Some(fallback) = route.fallback.as_deref() {
            if self.route_is_available(&fallback.provider) {
                return fallback;
            }
        }
        route
    }

    fn route_key(provider: &ProviderConfig) -> String {
        format!("{}/{}", provider.provider_type, provider.model)
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn route_is_available(&self, provider: &ProviderConfig) -> bool {
        let key = Self::route_key(provider);
        let state = self
            .route_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        !matches!(
            state.circuits.get(&key),
            Some(circuit) if circuit.degraded_until_ms > Self::now_ms()
        )
    }

    fn note_route_success(&self, provider: &ProviderConfig) {
        let key = Self::route_key(provider);
        let mut state = self
            .route_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(circuit) = state.circuits.get_mut(&key) {
            if circuit.failures > 0 {
                circuit.failures -= 1;
            }
            if circuit.failures == 0 {
                state.circuits.remove(&key);
            }
        }
    }

    fn note_route_failure(
        &self,
        provider: &ProviderConfig,
        kind: ProviderErrorKind,
        error: &anyhow::Error,
    ) {
        if kind == ProviderErrorKind::ClientError {
            return;
        }
        let key = Self::route_key(provider);
        let now = Self::now_ms();
        let mut state = self
            .route_state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let circuit = state.circuits.entry(key).or_default();
        if kind == ProviderErrorKind::RateLimit {
            let retry_ms = retry_after_from_error_message(&error.to_string())
                .unwrap_or(RATE_LIMIT_FALLBACK_DEGRADE_MS);
            circuit.degraded_until_ms = now + retry_ms;
            return;
        }
        circuit.failures = circuit.failures.saturating_add(1);
        let step = CIRCUIT_BACKOFF_STEPS_MS
            [(circuit.failures.saturating_sub(1) as usize).min(CIRCUIT_BACKOFF_STEPS_MS.len() - 1)];
        circuit.degraded_until_ms = now + step;
    }

    async fn complete_provider(
        &self,
        provider: &ProviderConfig,
        request: ChatCompletionRequest<'_>,
    ) -> anyhow::Result<ChatCompletion> {
        let ChatCompletionRequest {
            user_input,
            system_context,
            history,
            on_delta,
            on_tool_call_ready,
            on_tool_call_delta,
            tools,
        } = request;
        if provider_uses_llama_cpp(provider) {
            bail!(
                "llama.cpp provider is not supported in the desktop build; use an OpenAI-compatible provider"
            );
        } else if on_delta.is_some() && !provider_uses_anthropic_protocol(provider) {
            match self
                .complete_openai_compatible_streaming(
                    provider,
                    ChatCompletionRequest {
                        user_input,
                        system_context,
                        history,
                        on_delta,
                        on_tool_call_ready,
                        on_tool_call_delta,
                        tools,
                    },
                )
                .await
            {
                Ok(completion) => Ok(completion),
                Err(stream_error) => {
                    tracing::warn!("openai streaming failed, falling back to non-streaming: {stream_error:#}");
                    self.complete_openai_compatible(
                        provider,
                        user_input,
                        system_context,
                        history,
                        tools,
                    )
                    .await
                }
            }
        } else if provider_uses_anthropic_protocol(provider) {
            let completion = self
                .complete_anthropic(provider, user_input, system_context, history, tools)
                .await?;
            if let Some(on_delta) = on_delta {
                if !completion.content.is_empty() {
                    on_delta(completion.content.clone());
                }
            }
            if let Some(on_tool_call_ready) = on_tool_call_ready {
                for call in &completion.tool_calls {
                    on_tool_call_ready(call.clone());
                }
            }
            Ok(completion)
        } else {
            let completion = self
                .complete_openai_compatible(provider, user_input, system_context, history, tools)
                .await?;
            if let Some(on_tool_call_ready) = on_tool_call_ready {
                for call in &completion.tool_calls {
                    on_tool_call_ready(call.clone());
                }
            }
            Ok(completion)
        }
    }

    async fn complete_openai_compatible(
        &self,
        provider: &ProviderConfig,
        user_input: &str,
        system_context: Option<&str>,
        history: &[ProtocolMessage],
        tools: Option<&[ProviderToolDefinition]>,
    ) -> anyhow::Result<ChatCompletion> {
        let api_key = provider
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "no API key is configured for {}/{}",
                    provider.provider_type,
                    provider.model
                )
            })?;
        let url = chat_completions_url(provider)?;
        let body = build_chat_completion_body(provider, user_input, system_context, tools, history);
        let headers = provider_headers(provider, api_key)?;
        let payload = self
            .post_provider_json(provider, &url, headers, &body, api_key)
            .await?;
        parse_chat_completion_response(&payload)
    }

    async fn complete_openai_compatible_streaming(
        &self,
        provider: &ProviderConfig,
        request: ChatCompletionRequest<'_>,
    ) -> anyhow::Result<ChatCompletion> {
        let ChatCompletionRequest {
            user_input,
            system_context,
            history,
            on_delta,
            on_tool_call_ready,
            on_tool_call_delta,
            tools,
        } = request;
        let on_delta = on_delta.expect("checked above");
        let api_key = provider
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "no API key is configured for {}/{}",
                    provider.provider_type,
                    provider.model
                )
            })?;
        let url = chat_completions_url(provider)?;
        let mut body =
            build_chat_completion_body(provider, user_input, system_context, tools, history);
        if let Value::Object(ref mut map) = body {
            map.insert("stream".to_string(), Value::Bool(true));
            map.insert(
                "stream_options".to_string(),
                json!({ "include_usage": true }),
            );
        }
        let headers = provider_headers(provider, api_key)?;
        let mut response = self
            .http
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("failed to start streaming provider request")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "API error ({status}): {}",
                sanitize_provider_error(&body, api_key)
            );
        }

        let mut buffer = String::new();
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason = None;
        let mut usage = None;
        let mut tool_acc = StreamingToolCallAccumulator::default();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("failed to read streaming provider chunk")?
        {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find("\n\n") {
                let block = buffer[..pos].to_string();
                buffer.drain(..pos + 2);
                for line in block.lines() {
                    let line = line.trim();
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line
                        .strip_prefix("data:")
                        .map(str::trim)
                        .unwrap_or_default();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    let payload: Value = serde_json::from_str(data)
                        .with_context(|| format!("invalid streaming payload: {data}"))?;
                    if let Some(error) = payload.get("error") {
                        bail!("provider returned error: {}", compact_json(error));
                    }
                    if let Some(parsed_usage) = parse_openai_usage(&payload) {
                        usage = Some(parsed_usage);
                    }
                    if let Some(reason) = payload
                        .pointer("/choices/0/finish_reason")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                    {
                        finish_reason = Some(reason.to_string());
                    }
                    let delta = payload
                        .pointer("/choices/0/delta")
                        .or_else(|| payload.pointer("/choices/0/message"));
                    if let Some(delta) = delta {
                        if let Some(piece) = delta.get("content") {
                            let piece = message_content_text(Some(piece));
                            if !piece.is_empty() {
                                content.push_str(&piece);
                                on_delta(piece);
                            }
                        }
                        if let Some(piece) = delta
                            .get("reasoning_content")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            reasoning.push_str(piece);
                        }
                        tool_acc.apply_delta(delta.get("tool_calls"));
                        if let Some(on_delta_tool) = &on_tool_call_delta {
                            for call in tool_acc.current_partial_calls() {
                                on_delta_tool(call);
                            }
                        }
                        if let Some(on_ready) = &on_tool_call_ready {
                            for sealed in tool_acc.drain_newly_sealed() {
                                on_ready(sealed);
                            }
                        }
                    }
                }
            }
        }
        tool_acc.seal_all_remaining();
        if let Some(on_ready) = &on_tool_call_ready {
            for sealed in tool_acc.drain_newly_sealed() {
                on_ready(sealed);
            }
        }
        Ok(ChatCompletion {
            content,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            tool_calls: tool_acc.into_tool_calls(),
            finish_reason,
            usage,
        })
    }

    async fn complete_anthropic(
        &self,
        provider: &ProviderConfig,
        user_input: &str,
        system_context: Option<&str>,
        history: &[ProtocolMessage],
        tools: Option<&[ProviderToolDefinition]>,
    ) -> anyhow::Result<ChatCompletion> {
        let api_key = provider
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "no API key is configured for {}/{}",
                    provider.provider_type,
                    provider.model
                )
            })?;
        let url = anthropic_messages_url(provider)?;
        let body =
            build_anthropic_messages_body(provider, user_input, system_context, tools, history);
        let headers = anthropic_headers(api_key)?;
        let payload = self
            .post_provider_json(provider, &url, headers, &body, api_key)
            .await?;
        parse_anthropic_response(&payload)
    }

    async fn post_provider_json(
        &self,
        provider: &ProviderConfig,
        url: &str,
        headers: HeaderMap,
        body: &Value,
        api_key: &str,
    ) -> anyhow::Result<Value> {
        let attempts = retry_attempts(provider.retry);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..attempts {
            let response = self
                .http
                .post(url)
                .headers(headers.clone())
                .json(body)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let payload = response
                        .json::<Value>()
                        .await
                        .context("failed to decode provider JSON response")?;
                    return Ok(payload);
                }
                Ok(response) => {
                    let status = response.status();
                    let retry_after = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| retry_after_ms(value).ok());
                    let body = response.text().await.unwrap_or_default();
                    let message = format!(
                        "API error ({status}): {}",
                        sanitize_provider_error(&body, api_key)
                    );
                    if is_retriable_status(status.as_u16()) && attempt + 1 < attempts {
                        last_error = Some(anyhow!(message));
                        sleep_before_retry(attempt, retry_after).await;
                        continue;
                    }
                    bail!(message);
                }
                Err(error) => {
                    let message = format!("provider request failed: {error}");
                    if attempt + 1 < attempts {
                        last_error = Some(anyhow!(message));
                        sleep_before_retry(attempt, None).await;
                        continue;
                    }
                    bail!(message);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("provider request failed without a response")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub task: ModelTaskKind,
    pub provider: String,
    pub model: String,
    pub text: String,
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    pub error: Option<String>,
    pub streamed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Provider-reported token usage for one model call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_tokens: Option<u64>,
    /// Sum of prompt tokens billed across model rounds in the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billed_prompt_tokens: Option<u64>,
    /// Completion tokens from the latest model round (not a streaming-inflated sum).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completion_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.total_tokens == 0
            && self.cache_tokens.unwrap_or(0) == 0
            && self.billed_prompt_tokens.unwrap_or(0) == 0
    }

    /// Merge provider usage into a running turn total without double-counting
    /// streaming snapshots that re-report the same call.
    pub fn merge_turn_usage(current: Option<&Self>, next: &Self) -> Option<Self> {
        if next.is_empty() {
            return current.cloned();
        }
        let Some(cur) = current else {
            let last_out = next
                .last_completion_tokens
                .unwrap_or(next.completion_tokens);
            let billed = next.billed_prompt_tokens.unwrap_or(next.prompt_tokens);
            return Some(Self {
                prompt_tokens: next.prompt_tokens,
                completion_tokens: next.completion_tokens,
                total_tokens: if next.total_tokens > 0 {
                    next.total_tokens
                } else {
                    next.prompt_tokens + next.completion_tokens
                },
                cache_tokens: next.cache_tokens.filter(|v| *v > 0),
                billed_prompt_tokens: (billed > 0).then_some(billed),
                last_completion_tokens: (last_out > 0).then_some(last_out),
            });
        };
        if next.prompt_tokens == cur.prompt_tokens
            && next.completion_tokens == cur.completion_tokens
            && next.total_tokens == cur.total_tokens
        {
            return Some(cur.clone());
        }
        let cur_last = cur.last_completion_tokens.unwrap_or(0);
        let cur_billed = cur.billed_prompt_tokens.unwrap_or(cur.prompt_tokens);
        let prompt_unchanged = cur.prompt_tokens > 0
            && next.prompt_tokens >= cur.prompt_tokens
            && next.prompt_tokens <= ((cur.prompt_tokens as f64 * 1.02).ceil() as u64);
        if prompt_unchanged && next.completion_tokens >= cur_last {
            let last_out = next.completion_tokens;
            let prior_sum = cur.completion_tokens.saturating_sub(cur_last);
            let completion_tokens = prior_sum + last_out;
            return Some(Self {
                prompt_tokens: next.prompt_tokens,
                completion_tokens,
                total_tokens: next.prompt_tokens + completion_tokens,
                cache_tokens: Some(
                    cur.cache_tokens
                        .unwrap_or(0)
                        .max(next.cache_tokens.unwrap_or(0)),
                )
                .filter(|v| *v > 0),
                billed_prompt_tokens: (cur_billed > 0)
                    .then_some(cur_billed)
                    .or((next.prompt_tokens > 0).then_some(next.prompt_tokens)),
                last_completion_tokens: (last_out > 0).then_some(last_out),
            });
        }
        let last_out = next.completion_tokens;
        let billed = cur_billed + next.prompt_tokens;
        let completion_tokens = cur.completion_tokens + last_out;
        Some(Self {
            prompt_tokens: next.prompt_tokens,
            completion_tokens,
            total_tokens: next.prompt_tokens + completion_tokens,
            cache_tokens: Some(
                cur.cache_tokens
                    .unwrap_or(0)
                    .max(next.cache_tokens.unwrap_or(0)),
            )
            .filter(|v| *v > 0),
            billed_prompt_tokens: Some(billed).filter(|v| *v > 0),
            last_completion_tokens: (last_out > 0).then_some(last_out),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatCompletion {
    content: String,
    reasoning: Option<String>,
    tool_calls: Vec<ProviderToolCall>,
    finish_reason: Option<String>,
    usage: Option<TokenUsage>,
}

fn apply_generation_options_to_route(
    mut route: ResolvedModelRoute,
    options: GenerationOptions,
) -> ResolvedModelRoute {
    if route.task == ModelTaskKind::Main && !options.deep_thinking {
        route.provider = apply_thinking_disabled_if_supported(route.provider);
        route.capabilities = resolve_model_capabilities(&route.provider);
    }
    if let Some(fallback) = route.fallback.take() {
        route.fallback = Some(Box::new(apply_generation_options_to_route(
            *fallback, options,
        )));
    }
    route
}

fn apply_thinking_disabled_if_supported(mut provider: ProviderConfig) -> ProviderConfig {
    if resolve_model_capabilities(&provider).thinking_disable {
        provider.extra_body.insert(
            "thinking".to_string(),
            serde_json::json!({ "type": "disabled" }),
        );
        provider
            .extra_body
            .insert("enable_thinking".to_string(), serde_json::json!(false));
    }
    provider
}

pub fn tune_provider_for_task(mut provider: ProviderConfig, task: ModelTaskKind) -> ProviderConfig {
    if matches!(
        task,
        ModelTaskKind::Title
            | ModelTaskKind::Summary
            | ModelTaskKind::Memory
            | ModelTaskKind::Intent
            | ModelTaskKind::KeyFacts
            | ModelTaskKind::Planning
    ) && resolve_model_capabilities(&provider).thinking_disable
    {
        provider = apply_thinking_disabled_if_supported(provider);
    }
    if matches!(
        task,
        ModelTaskKind::Title
            | ModelTaskKind::Memory
            | ModelTaskKind::Intent
            | ModelTaskKind::KeyFacts
            | ModelTaskKind::Planning
    ) && resolve_model_capabilities(&provider).json_mode
    {
        provider.extra_body.insert(
            "response_format".to_string(),
            serde_json::json!({ "type": "json_object" }),
        );
    }
    provider.max_tokens = clamp_task_max_tokens(provider.max_tokens, task);
    provider.temperature = Some(task_temperature(provider.temperature, task));
    provider
}

pub fn resolve_model_capabilities(provider: &ProviderConfig) -> ModelCapabilities {
    let model = provider.model.to_ascii_lowercase();
    let provider_type = provider.provider_type.to_ascii_lowercase();
    ModelCapabilities {
        json_mode: !model.contains("legacy"),
        thinking_disable: model.contains("deepseek")
            || model.contains("qwen")
            || model.contains("kimi")
            || model.contains("glm")
            || provider_type.contains("mimo"),
        tool_calling: !provider_uses_llama_cpp(provider)
            && !model.contains("embedding")
            && !model.contains("audio"),
        vision: model.contains("vision") || model.contains("vl"),
        rate_limit: ModelRateLimitSettings::default(),
    }
}

pub fn check_task_compatibility(
    task: ModelTaskKind,
    capabilities: &ModelCapabilities,
) -> CapabilityCheck {
    if matches!(
        task,
        ModelTaskKind::Implementation | ModelTaskKind::Review | ModelTaskKind::Main
    ) && !capabilities.tool_calling
    {
        return CapabilityCheck {
            compatible: false,
            reasons: vec!["task expects tool-capable chat model".to_string()],
        };
    }
    CapabilityCheck {
        compatible: true,
        reasons: Vec::new(),
    }
}

fn provider_from_settings(
    base: &ProviderConfig,
    settings: Option<&AgentModelSettings>,
) -> ProviderConfig {
    let Some(settings) = settings else {
        return base.clone();
    };
    ProviderConfig {
        provider_type: settings
            .provider
            .clone()
            .unwrap_or_else(|| base.provider_type.clone()),
        api_key: settings.api_key.clone().or_else(|| base.api_key.clone()),
        base_url: settings.base_url.clone().or_else(|| base.base_url.clone()),
        model_path: settings
            .model_path
            .clone()
            .or_else(|| base.model_path.clone()),
        protocol: settings.protocol.clone().or_else(|| base.protocol.clone()),
        model: settings.model.clone().unwrap_or_else(|| base.model.clone()),
        max_tokens: settings.max_tokens.or(base.max_tokens),
        temperature: settings.temperature.or(base.temperature),
        retry: settings.retry.or(base.retry),
        extra_body: base.extra_body.clone(),
    }
}

fn provider_config_from_json(value: Option<&serde_json::Value>) -> ProviderConfig {
    let Some(value) = value else {
        return ProviderConfig::openai_compatible("default");
    };
    let provider_type = string_field(value, "type")
        .or_else(|| string_field(value, "provider"))
        .unwrap_or_else(|| "openai-compatible".to_string());
    let model = string_field(value, "model").unwrap_or_else(|| "default".to_string());
    ProviderConfig {
        provider_type,
        api_key: string_field(value, "apiKey").filter(|item| !item.trim().is_empty()),
        base_url: string_field(value, "baseUrl").filter(|item| !item.trim().is_empty()),
        model_path: string_field(value, "modelPath")
            .or_else(|| string_field(value, "path"))
            .filter(|item| !item.trim().is_empty()),
        protocol: string_field(value, "protocol")
            .or_else(|| string_field(value, "apiProtocol"))
            .filter(|item| !item.trim().is_empty()),
        model,
        max_tokens: u32_field(value, "maxTokens").or(Some(16384)),
        temperature: f32_field(value, "temperature").or(Some(0.3)),
        retry: retry_field(value),
        extra_body: BTreeMap::new(),
    }
}

fn settings_from_json(value: &serde_json::Value) -> Option<AgentModelSettings> {
    if !value.is_object() {
        return None;
    }
    Some(AgentModelSettings {
        provider: string_field(value, "provider").or_else(|| string_field(value, "type")),
        api_key: string_field(value, "apiKey").filter(|item| !item.trim().is_empty()),
        base_url: string_field(value, "baseUrl").filter(|item| !item.trim().is_empty()),
        model_path: string_field(value, "modelPath")
            .or_else(|| string_field(value, "path"))
            .filter(|item| !item.trim().is_empty()),
        protocol: string_field(value, "protocol").or_else(|| string_field(value, "apiProtocol")),
        model: string_field(value, "model").filter(|item| !item.trim().is_empty()),
        max_tokens: u32_field(value, "maxTokens"),
        temperature: f32_field(value, "temperature"),
        retry: retry_field(value),
        fallback: value
            .get("fallback")
            .and_then(settings_from_json)
            .map(Box::new),
    })
}

fn rate_limit_from_json(value: &serde_json::Value) -> ModelRateLimitSettings {
    ModelRateLimitSettings {
        rpm: u32_field(value, "rpm"),
        concurrency: u32_field(value, "concurrency"),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn u32_field(value: &serde_json::Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|item| u32::try_from(item).ok())
}

fn f32_field(value: &serde_json::Value, key: &str) -> Option<f32> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|item| item as f32)
}

fn retry_field(value: &serde_json::Value) -> Option<u8> {
    value
        .get("retry")
        .and_then(|retry| {
            retry
                .as_u64()
                .or_else(|| retry.get("maxAttempts").and_then(serde_json::Value::as_u64))
        })
        .and_then(|item| u8::try_from(item).ok())
}

fn task_kind_from_key(key: &str) -> Option<ModelTaskKind> {
    match key {
        "main" => Some(ModelTaskKind::Main),
        "title" => Some(ModelTaskKind::Title),
        "summary" => Some(ModelTaskKind::Summary),
        "memory" => Some(ModelTaskKind::Memory),
        "intent" => Some(ModelTaskKind::Intent),
        "keyfacts" | "key_facts" | "keyFacts" => Some(ModelTaskKind::KeyFacts),
        "planning" => Some(ModelTaskKind::Planning),
        "implementation" => Some(ModelTaskKind::Implementation),
        "review" => Some(ModelTaskKind::Review),
        _ => None,
    }
}

fn clamp_task_max_tokens(max_tokens: Option<u32>, task: ModelTaskKind) -> Option<u32> {
    // Main coding turns need headroom for narration + native tool_calls; 4k truncates mid-turn.
    const MAIN_DEFAULT_MAX_TOKENS: u32 = 16_384;
    const MAIN_MAX_MAX_TOKENS: u32 = 65_536;
    match task {
        ModelTaskKind::Main | ModelTaskKind::Implementation => {
            let value = max_tokens.unwrap_or(MAIN_DEFAULT_MAX_TOKENS);
            Some(value.clamp(1_024, MAIN_MAX_MAX_TOKENS))
        }
        ModelTaskKind::Title => Some(max_tokens.unwrap_or(150).clamp(32, 150)),
        ModelTaskKind::Summary | ModelTaskKind::Memory | ModelTaskKind::Intent => {
            Some(max_tokens.unwrap_or(512).clamp(32, 512))
        }
        ModelTaskKind::KeyFacts => Some(max_tokens.unwrap_or(800).clamp(32, 800)),
        ModelTaskKind::Planning => Some(max_tokens.unwrap_or(1200).clamp(32, 1200)),
        ModelTaskKind::Review => Some(max_tokens.unwrap_or(1600).clamp(32, 1600)),
    }
}

fn task_temperature(temperature: Option<f32>, task: ModelTaskKind) -> f32 {
    match task {
        ModelTaskKind::Title => 0.1,
        ModelTaskKind::Memory | ModelTaskKind::Planning | ModelTaskKind::Intent => 0.2,
        ModelTaskKind::KeyFacts => 0.15,
        ModelTaskKind::Review => 0.25,
        _ => temperature.unwrap_or(0.3),
    }
}

fn same_provider_route(a: &ProviderConfig, b: &ProviderConfig) -> bool {
    a.provider_type == b.provider_type
        && a.model == b.model
        && a.base_url == b.base_url
        && a.model_path == b.model_path
        && a.protocol == b.protocol
}

fn provider_uses_llama_cpp(provider: &ProviderConfig) -> bool {
    let provider_type = provider
        .provider_type
        .replace('_', "-")
        .to_ascii_lowercase();
    let protocol = provider
        .protocol
        .as_deref()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_lowercase();
    provider_type == "llama-cpp"
        || provider_type == "llama.cpp"
        || provider_type == "llamacpp"
        || protocol == "llama-cpp"
        || protocol == "llama.cpp"
        || protocol == "llamacpp"
}

fn provider_uses_anthropic_protocol(provider: &ProviderConfig) -> bool {
    if let Some(protocol) = provider.protocol.as_deref().map(str::trim) {
        if !protocol.is_empty() {
            return protocol.eq_ignore_ascii_case("anthropic");
        }
    }
    if provider.provider_type.eq_ignore_ascii_case("anthropic") {
        return true;
    }
    provider
        .base_url
        .as_deref()
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("/anthropic") || value.contains("api.anthropic.com")
        })
        .unwrap_or(false)
}

fn chat_completions_url(provider: &ProviderConfig) -> anyhow::Result<String> {
    let base = provider_base_url(provider)?;
    let lower = base.to_ascii_lowercase();
    if lower.contains("/chat/completions") {
        return Ok(base);
    }
    if let Some((path, query)) = base.split_once('?') {
        return Ok(format!(
            "{}/chat/completions?{query}",
            path.trim_end_matches('/')
        ));
    }
    Ok(format!("{}/chat/completions", base.trim_end_matches('/')))
}

fn anthropic_messages_url(provider: &ProviderConfig) -> anyhow::Result<String> {
    let base = provider_base_url(provider)?;
    let lower = base.to_ascii_lowercase();
    if lower.contains("/v1/messages") {
        return Ok(base);
    }
    if let Some((path, query)) = base.split_once('?') {
        return Ok(format!(
            "{}?{query}",
            anthropic_messages_path(path.trim_end_matches('/'))
        ));
    }
    Ok(anthropic_messages_path(base.trim_end_matches('/')))
}

fn anthropic_messages_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with("/v1/messages") {
        path.to_string()
    } else if lower.ends_with("/v1") {
        format!("{path}/messages")
    } else {
        format!("{path}/v1/messages")
    }
}

fn provider_base_url(provider: &ProviderConfig) -> anyhow::Result<String> {
    if let Some(base_url) = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(normalize_base_url(base_url));
    }

    let provider_type = provider.provider_type.to_ascii_lowercase();
    if requires_explicit_base_url(&provider_type) {
        bail!(
            "provider \"{}\" requires an explicit baseUrl in configuration",
            provider.provider_type
        );
    }

    Ok(match provider_type.as_str() {
        "openai" | "openai-compatible" | "custom" => DEFAULT_OPENAI_BASE_URL.to_string(),
        "llama-cpp" | "llama.cpp" | "llamacpp" => {
            bail!("llama.cpp provider is handled by the native inference worker")
        }
        "anthropic" => DEFAULT_ANTHROPIC_BASE_URL.to_string(),
        "deepseek" => "https://api.deepseek.com/v1".to_string(),
        "mistral" => "https://api.mistral.ai/v1".to_string(),
        "zhipu" => "https://open.bigmodel.cn/api/paas/v4".to_string(),
        "moonshot" | "kimi" => "https://api.moonshot.cn/v1".to_string(),
        "minimax" => "https://api.minimax.chat/v1".to_string(),
        "qianfan" => "https://qianfan.baidubce.com/v2".to_string(),
        "tencent" => "https://api.hunyuan.cloud.tencent.com/v1".to_string(),
        "volcano" => "https://ark.cn-beijing.volces.com/api/v3".to_string(),
        "siliconflow" => "https://api.siliconflow.cn/v1".to_string(),
        "modelscope" => "https://api-inference.modelscope.cn/v1".to_string(),
        "longcat" => "https://api.longcat.chat/v1".to_string(),
        "ollama" => "http://localhost:11434/v1".to_string(),
        "xai" => "https://api.x.ai/v1".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        "google" => "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
        _ => DEFAULT_OPENAI_BASE_URL.to_string(),
    })
}

fn requires_explicit_base_url(provider_type: &str) -> bool {
    matches!(
        provider_type,
        "amazon" | "azure" | "byteplus" | "github" | "huawei" | "mimo" | "opencode"
    )
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if let Some((path, query)) = trimmed.split_once('?') {
        format!("{}?{query}", path.trim_end_matches('/'))
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn provider_headers(provider: &ProviderConfig, api_key: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let provider_type = provider.provider_type.to_ascii_lowercase();
    if provider_type == "azure" {
        headers.insert(
            "api-key",
            HeaderValue::from_str(api_key).context("invalid Azure api-key header")?,
        );
    } else {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}"))
                .context("invalid Authorization header")?,
        );
    }
    if provider_type == "openrouter" {
        headers.insert(
            "HTTP-Referer",
            HeaderValue::from_static("https://github.com/xuanyu-agent"),
        );
        headers.insert("X-Title", HeaderValue::from_static("XuanYu Agent"));
    }
    if provider_type == "google" {
        headers.insert(
            "x-goog-api-client",
            HeaderValue::from_static("xuanyu-agent"),
        );
    }
    Ok(headers)
}

fn anthropic_headers(api_key: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).context("invalid Anthropic x-api-key header")?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    Ok(headers)
}

fn build_chat_completion_body(
    provider: &ProviderConfig,
    user_input: &str,
    system_context: Option<&str>,
    tools: Option<&[ProviderToolDefinition]>,
    history: &[ProtocolMessage],
) -> Value {
    let mut body = Map::new();
    for (key, value) in &provider.extra_body {
        body.insert(key.clone(), value.clone());
    }
    let system_content = match system_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(context) => format!("{DEFAULT_SYSTEM_PROMPT}\n\n{context}"),
        None => DEFAULT_SYSTEM_PROMPT.to_string(),
    };
    body.insert("model".to_string(), Value::String(provider.model.clone()));
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(serde_json::json!({ "role": "system", "content": system_content }));
    messages.extend(history.iter().filter_map(protocol_message_to_openai_json));
    if !user_input.trim().is_empty() {
        messages.push(serde_json::json!({ "role": "user", "content": user_input }));
    }
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert(
        "max_tokens".to_string(),
        serde_json::json!(provider.max_tokens.unwrap_or(16384)),
    );
    body.insert(
        "temperature".to_string(),
        serde_json::json!(provider.temperature.unwrap_or(0.7)),
    );
    if let Some(tools) = tools.filter(|items| !items.is_empty()) {
        body.insert(
            "tools".to_string(),
            Value::Array(
                tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.parameters
                            }
                        })
                    })
                    .collect(),
            ),
        );
        body.insert("tool_choice".to_string(), json!("auto"));
    }
    Value::Object(body)
}

fn build_anthropic_messages_body(
    provider: &ProviderConfig,
    user_input: &str,
    system_context: Option<&str>,
    tools: Option<&[ProviderToolDefinition]>,
    history: &[ProtocolMessage],
) -> Value {
    let mut body = Map::new();
    for (key, value) in &provider.extra_body {
        body.insert(key.clone(), value.clone());
    }
    let system_content = match system_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(context) => format!("{DEFAULT_SYSTEM_PROMPT}\n\n{context}"),
        None => DEFAULT_SYSTEM_PROMPT.to_string(),
    };
    body.insert("model".to_string(), Value::String(provider.model.clone()));
    body.insert(
        "max_tokens".to_string(),
        serde_json::json!(provider.max_tokens.unwrap_or(16384)),
    );
    body.insert("system".to_string(), Value::String(system_content));
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.extend(history.iter().filter_map(protocol_message_to_anthropic_json));
    if !user_input.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_input
        }));
    }
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert(
        "temperature".to_string(),
        serde_json::json!(provider.temperature.unwrap_or(0.7)),
    );
    if let Some(tools) = tools.filter(|items| !items.is_empty()) {
        body.insert(
            "tools".to_string(),
            Value::Array(
                tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.parameters
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(body)
}

fn protocol_message_to_openai_json(message: &ProtocolMessage) -> Option<Value> {
    let role = message.role.trim().to_ascii_lowercase();
    match role.as_str() {
        "system" | "user" => {
            if message.content.trim().is_empty() {
                return None;
            }
            Some(json!({
                "role": role,
                "content": message.content,
            }))
        }
        "assistant" => {
            let has_tools = !message.tool_calls.is_empty();
            if message.content.trim().is_empty() && !has_tools {
                return None;
            }
            let mut obj = serde_json::Map::new();
            obj.insert("role".to_string(), Value::String("assistant".to_string()));
            // OpenAI accepts null content when tool_calls are present.
            if message.content.trim().is_empty() && has_tools {
                obj.insert("content".to_string(), Value::Null);
            } else {
                obj.insert(
                    "content".to_string(),
                    Value::String(message.content.clone()),
                );
            }
            if has_tools {
                obj.insert(
                    "tool_calls".to_string(),
                    Value::Array(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.name,
                                        "arguments": call.arguments
                                    }
                                })
                            })
                            .collect(),
                    ),
                );
            }
            Some(Value::Object(obj))
        }
        "tool" => {
            let tool_call_id = message
                .tool_call_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .unwrap_or("tool_call");
            Some(json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": message.content,
            }))
        }
        _ => None,
    }
}

fn protocol_message_to_anthropic_json(message: &ProtocolMessage) -> Option<Value> {
    let role = message.role.trim().to_ascii_lowercase();
    // Anthropic Messages API: only user/assistant in the messages array.
    // Fold tool results into user turns so history still reaches the model.
    match role.as_str() {
        "user" => {
            if message.content.trim().is_empty() {
                return None;
            }
            Some(json!({ "role": "user", "content": message.content }))
        }
        "assistant" => {
            if message.content.trim().is_empty() {
                return None;
            }
            Some(json!({ "role": "assistant", "content": message.content }))
        }
        "tool" => {
            let label = message
                .tool_call_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .unwrap_or("tool");
            Some(json!({
                "role": "user",
                "content": format!("Tool result ({label}):\n{}", message.content),
            }))
        }
        _ => None,
    }
}

fn parse_chat_completion_response(payload: &Value) -> anyhow::Result<ChatCompletion> {
    if let Some(error) = payload.get("error") {
        bail!("provider returned error: {}", compact_json(error));
    }
    let choice = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow!("provider response did not include choices[0]"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow!("provider response did not include choices[0].message"))?;
    let content = message_content_text(message.get("content"));
    let reasoning = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    let tool_calls = parse_openai_tool_calls(message.get("tool_calls"));
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ChatCompletion {
        content,
        reasoning,
        tool_calls,
        finish_reason,
        usage: parse_openai_usage(payload),
    })
}

fn parse_openai_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    let prompt_tokens = usage_number(usage, "prompt_tokens")
        .or_else(|| usage_number(usage, "input_tokens"))
        .unwrap_or(0);
    let completion_tokens = usage_number(usage, "completion_tokens")
        .or_else(|| usage_number(usage, "output_tokens"))
        .unwrap_or(0);
    let total_tokens = usage_number(usage, "total_tokens")
        .unwrap_or(prompt_tokens.saturating_add(completion_tokens));
    let cache_tokens = read_openai_cache_tokens(usage);
    let parsed = TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_tokens,
        ..Default::default()
    };
    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

fn read_openai_cache_tokens(usage: &Value) -> Option<u64> {
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            usage
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
        })
        .or_else(|| usage_number(usage, "prompt_cache_hit_tokens"))
        .or_else(|| usage_number(usage, "input_cache_hit_tokens"))
        .or_else(|| usage_number(usage, "cache_hit_tokens"))
        .or_else(|| usage_number(usage, "cached_tokens"))
        .or_else(|| usage_number(usage, "cache_read_input_tokens"))
        .unwrap_or(0);
    (cached > 0).then_some(cached)
}

fn usage_number(source: &Value, key: &str) -> Option<u64> {
    source.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
    })
}

fn parse_openai_tool_calls(value: Option<&Value>) -> Vec<ProviderToolCall> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let function = object.get("function").and_then(Value::as_object)?;
            let name = function.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(name)
                .to_string();
            let arguments = function
                .get("arguments")
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => compact_json(other),
                })
                .unwrap_or_else(|| "{}".to_string());
            Some(ProviderToolCall {
                id,
                name: name.to_string(),
                arguments,
            })
        })
        .collect()
}

#[derive(Default)]
struct StreamingToolCallAccumulator {
    slots: Vec<StreamingToolCallSlot>,
    pending_ready: Vec<ProviderToolCall>,
}

#[derive(Default)]
struct StreamingToolCallSlot {
    id: String,
    name: String,
    arguments: String,
    sealed: bool,
}

impl StreamingToolCallSlot {
    fn to_call(&self) -> ProviderToolCall {
        ProviderToolCall {
            id: if self.id.is_empty() {
                self.name.clone()
            } else {
                self.id.clone()
            },
            name: self.name.clone(),
            arguments: if self.arguments.is_empty() {
                "{}".to_string()
            } else {
                self.arguments.clone()
            },
        }
    }
}

impl StreamingToolCallAccumulator {
    fn apply_delta(&mut self, value: Option<&Value>) {
        let Some(Value::Array(items)) = value else {
            return;
        };
        let mut touched_indexes = Vec::new();
        for item in items {
            let Some(object) = item.as_object() else {
                continue;
            };
            let index = object
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(self.slots.len() as u64) as usize;
            while self.slots.len() <= index {
                self.slots.push(StreamingToolCallSlot::default());
            }
            let slot = &mut self.slots[index];
            if let Some(id) = object
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                slot.id = id.to_string();
            }
            if let Some(function) = object.get("function").and_then(Value::as_object) {
                if let Some(name) = function
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    slot.name = name.to_string();
                }
                if let Some(arguments) = function.get("arguments") {
                    match arguments {
                        Value::String(text) => slot.arguments.push_str(text),
                        other => slot.arguments.push_str(&compact_json(other)),
                    }
                }
            }
            touched_indexes.push(index);
        }
        // A new higher index implies previous tool calls are sealed.
        if let Some(max_index) = touched_indexes.iter().copied().max() {
            for index in 0..max_index {
                self.seal_index(index);
            }
        }
    }

    fn seal_index(&mut self, index: usize) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if slot.sealed || slot.name.trim().is_empty() {
            return;
        }
        slot.sealed = true;
        self.pending_ready.push(slot.to_call());
    }

    fn seal_all_remaining(&mut self) {
        for index in 0..self.slots.len() {
            self.seal_index(index);
        }
    }

    fn drain_newly_sealed(&mut self) -> Vec<ProviderToolCall> {
        std::mem::take(&mut self.pending_ready)
    }

    fn current_partial_calls(&self) -> Vec<ProviderToolCall> {
        self.slots
            .iter()
            .filter(|slot| !slot.sealed && !slot.name.trim().is_empty())
            .map(StreamingToolCallSlot::to_call)
            .collect()
    }

    fn into_tool_calls(self) -> Vec<ProviderToolCall> {
        self.slots
            .into_iter()
            .filter(|slot| !slot.name.trim().is_empty())
            .map(|slot| slot.to_call())
            .collect()
    }
}

fn parse_anthropic_response(payload: &Value) -> anyhow::Result<ChatCompletion> {
    if let Some(error) = payload.get("error") {
        bail!("provider returned error: {}", compact_json(error));
    }
    let content = payload
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("provider response did not include content blocks"))?;
    let text = content
        .iter()
        .filter_map(|block| {
            let object = block.as_object()?;
            match object.get("type").and_then(Value::as_str) {
                Some("text") => object.get("text").and_then(Value::as_str),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let reasoning = content
        .iter()
        .filter_map(|block| {
            let object = block.as_object()?;
            match object.get("type").and_then(Value::as_str) {
                Some("thinking") => object
                    .get("thinking")
                    .or_else(|| object.get("text"))
                    .and_then(Value::as_str),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let tool_calls: Vec<ProviderToolCall> = content
        .iter()
        .filter_map(|block| {
            let object = block.as_object()?;
            if object.get("type").and_then(Value::as_str)? != "tool_use" {
                return None;
            }
            let name = object.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(name)
                .to_string();
            let arguments = object
                .get("input")
                .map(compact_json)
                .unwrap_or_else(|| "{}".to_string());
            Some(ProviderToolCall {
                id,
                name: name.to_string(),
                arguments,
            })
        })
        .collect();
    let finish_reason = payload
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(|reason| {
            if tool_calls.is_empty() {
                reason.to_string()
            } else {
                "tool_calls".to_string()
            }
        });
    Ok(ChatCompletion {
        content: text,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls,
        finish_reason,
        usage: parse_anthropic_usage(payload),
    })
}

fn parse_anthropic_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    let prompt_tokens = usage_number(usage, "input_tokens").unwrap_or(0);
    let completion_tokens = usage_number(usage, "output_tokens").unwrap_or(0);
    let cache_tokens = usage_number(usage, "cache_read_input_tokens")
        .or_else(|| usage_number(usage, "cache_read_tokens"))
        .filter(|value| *value > 0);
    let total_tokens = prompt_tokens.saturating_add(completion_tokens);
    let parsed = TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_tokens,
        ..Default::default()
    };
    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

fn message_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.clone()),
                Value::Object(object) => object
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn retry_attempts(retry: Option<u8>) -> usize {
    usize::from(retry.unwrap_or(2)).clamp(1, 5)
}

async fn sleep_before_retry(attempt: usize, retry_after: Option<u64>) {
    let backoff = DEFAULT_RETRY_DELAY_MS.saturating_mul(1_u64 << attempt.min(5));
    let delay = retry_after.unwrap_or(backoff).max(backoff);
    tokio::time::sleep(Duration::from_millis(delay)).await;
}

fn retry_after_ms(value: &HeaderValue) -> anyhow::Result<u64> {
    let text = value.to_str()?;
    let seconds = text.parse::<f64>()?;
    Ok((seconds.max(0.0) * 1000.0).round() as u64)
}

fn is_retriable_status(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderErrorKind {
    ClientError,
    RateLimit,
    Transient,
}

fn classify_provider_error(error: &anyhow::Error) -> ProviderErrorKind {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("api error (429)") || text.contains("rate limit") || text.contains("too many requests")
    {
        return ProviderErrorKind::RateLimit;
    }
    if text.contains("api error (400)")
        || text.contains("api error (401)")
        || text.contains("api error (403)")
        || text.contains("api error (404)")
        || text.contains("incompatible")
        || text.contains("no api key")
    {
        return ProviderErrorKind::ClientError;
    }
    ProviderErrorKind::Transient
}

fn retry_after_from_error_message(message: &str) -> Option<u64> {
    let lower = message.to_ascii_lowercase();
    let marker = "retry-after";
    let idx = lower.find(marker)?;
    let rest = message[idx + marker.len()..].trim_start_matches(|ch: char| !ch.is_ascii_digit());
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse::<u64>().ok().map(|seconds| seconds * 1000)
}

fn sanitize_provider_error(body: &str, api_key: &str) -> String {
    let mut text = body.trim().replace(api_key, "[redacted]");
    if text.is_empty() {
        text = "empty error response".to_string();
    }
    if text.len() > 1200 {
        text.truncate(1200);
        text.push_str("...");
    }
    text
}

fn model_error_reply(
    provider: &ProviderConfig,
    user_input: &str,
    error: &dyn std::fmt::Display,
) -> String {
    format!(
        "Rust execution route is configured for {}/{}, but the model request could not be completed: {}. Received: {}",
        provider.provider_type, provider.model, error, user_input
    )
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}
