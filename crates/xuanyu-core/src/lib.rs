use anyhow::Context;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{oneshot, RwLock, Semaphore};

mod channel_manager;
mod channel_delivery;
mod config_transport;
mod config_schema;
mod marketplace;
mod marketplace_remote;
mod session_goal;
mod channel_runtime_api;
mod session_api;
mod marketplace_api;
mod memory_api;
mod speculative_tools;

pub use marketplace_remote::warm_remote_marketplace_cache;
pub use marketplace_api::MarketplaceSearchInput;

use config_schema::validate_app_config;
use config_transport::{preserve_configured_secrets, redact_config_for_transport};

use xuanyu_agent_bridge::{
    AgentPolicyBudget, AgentPolicyDecision, AgentPolicyMemoryCandidate, AgentPolicyMessage,
    AgentPolicyRequest, AgentWorkerBridge, AgentWorkerState, AgentWorkerStatus,
    SubAgentPlanRequest,
    SubAgentToolRequest,
};
use xuanyu_agents::{
    build_subagent_system_prompt, subagent_requires_artifact, AgentDefinition, AgentInput,
    AgentRuntime, SubAgentTaskLedger, SubAgentTaskResult, WatchdogSettings, WatchdogToolCall,
};
use xuanyu_channels::{
    ChannelHub,
};
use xuanyu_eventbus::{DeadLetterRecord, EventBus, EventFilter, EventReplayBatch};
use xuanyu_execution::{
    active_context_sections, context_load_mode_from_config, context_load_policy,
    context_section_activation_reason,
    context_surface_snapshot, estimate_tokens, estimate_tool_schema_tokens, mark_lazy_sections_for_refresh,
    memory_auto_inject_enabled, memory_auto_inject_include_cross_session,
    memory_auto_inject_limit, memory_auto_inject_min_score, policy_auto_sections, policy_bool,
    protocol_message_from_record, pack_session_context, PackSessionContextInput,
    SessionHandoffSummary, ProtocolMessage, ProtocolToolCallSpec,
    bootstrap_main_turn, iteration_input, max_iterations_from_config,
    run_main_turn_loop, MainTurnLoopConfig, MainTurnLoopHost,
    MainTurnLoopState, MainTurnModelResponse, MainTurnRuntimeToolCall, MainTurnToolDispatchResult,
    MainTurnToolResultMessage,
    sanitize_messages_for_tool_protocol, set_token_counter_runtime_config,
    should_inject_context_section, should_inject_key_facts, should_refresh_memory_profile,
    strip_refresh_sections, token_counter_backend_from_value,
    is_context_section_name, sections_need_memory_cache_invalidate, ContextSurface,
    ToolLoopGuard,
    snip_message, should_compact, select_compact_keep_index,
    select_compact_keep_index_by_roles, CompactStrategyKind,
    BudgetSectionInput, ContextBudgetAllocation, ContextBudgetAllocator, ContextBudgetSection,
    ExecutionConfig, ExecutionRuntime, GenerationOptions, LazyContextOverride, ModelRateLimitSettings,
    ModelTaskKind, ProviderToolCall, ProviderToolDefinition, ResolvedModelRoute,
    TurnMemoryCache,
    TokenCounterConfig,
    is_mid_turn_write_tool, is_streamable_write_tool, preview_stream_write_args,
};
use xuanyu_session::{SessionBridge, SessionManager};
use xuanyu_extensions::{
    format_skills_for_prompt, load_extensions, load_plugins, parse_plugin_manifest,
    run_plugin_before_turn_hooks, ExtensionLoadConfig, PluginBeforeTurnInput,
};
use xuanyu_governance::{
    format_sensitive_scan_warnings, scan_sensitive_files, GovernanceDecision, GovernanceEngine,
    PermissionPolicy, SensitiveFinding,
};
use xuanyu_memory::{
    classify_memory_write, extract_memories_from_messages,
    kind_for_category, with_memory_session_tag, ExtractionMessage,
    ExtractionRole, MemoryConflictResolution,
    MemoryEntry, MemoryIngestInput, MemoryIngress,
    MemorySearchQuery, MemoryService, MemorySnapshotInfo,
    MemoryWriteDecision, NewMemoryEntry, PendingMemory,
    SessionSummary, postgres_memory_config_from_value, tantivy_memory_config_from_value,
    PostgresMemoryConfig,
};
use xuanyu_eventbus::{AlertSeverity, HealthStatus, Observability, ObservabilityKind};
use xuanyu_protocol::{
    AppError, AppEvent, AppStatus, ChatAttachment, ChatMessage, ErrorCode, EventId, MessageRole,
    RuntimeModuleStatus, SessionId, SessionInfo, TurnId, WorkerHealth, WorkerStatus,
    PROTOCOL_VERSION,
};
use xuanyu_session::{
    AttemptComparison, CreateRunInput, EvidencePack, RunEvent, RunRecord, RunStore, RunTimeline,
    StartStepInput, StepRecord, UserFeedbackRecord,
};
use xuanyu_storage::{
    embedded_postgres_status, install_embedded_postgres, project_data_root,
    start_embedded_postgres, ConversationMessageRecord, ConversationSnapshotRecord,
    EmbeddedPostgresInstallOptions, EmbeddedPostgresPaths, StorageConfig, StorageService,
};
use xuanyu_tools::{
    format_deferred_tool_catalog_for_prompt, initial_active_tool_names,
    materialize_sandbox_files_with_auto_retry, project_instructions_context_text,
    todos_prompt_block, AGENT_CORE_PRELOAD_TOOLS, DeferredCatalogOptions, META_TOOL_NAMES,
    SandboxMaterializeOptions, SandboxMaterializeResult, ToolCall, ToolRuntime,
};
pub use xuanyu_tools::{ToolCatalogEntry, ToolDefinition, ToolOutput};
use xuanyu_governance::{RejectFileMutationInput, TurnLedgerStore};

const CONTEXT_COMPACT_SNAPSHOT_PREFIX: &str = "context_compact:";
const DATA_BACKUP_ROOTS: &[&str] = &[
    "config",
    "conversations",
    "memory",
    "agents",
    "runs",
    "events",
    "logs",
    "blobs",
    "delivery",
    "legacy-imports",
];
const STARTUP_SUBAGENT_RECOVERY_LIMIT: usize = 500;
/// 同时运行的子 Agent 上限。
const SUBAGENT_MAX_PARALLEL: usize = 6;

pub struct AppCore {
    workspace_root: PathBuf,
    eventbus: EventBus,
    storage: Arc<StorageService>,
    sessions: Arc<SessionManager>,
    execution: Arc<ExecutionRuntime>,
    governance: Arc<GovernanceEngine>,
    tools: Arc<ToolRuntime>,
    memory: Arc<MemoryService>,
    agents: Arc<AgentRuntime>,
    agent_bridge: Arc<AgentWorkerBridge>,
    runs: Arc<RunStore>,
    turn_ledger: Arc<TurnLedgerStore>,
    pending_tool_approvals: Arc<RwLock<BTreeMap<String, PendingToolApproval>>>,
    pending_questions: Arc<RwLock<BTreeMap<String, PendingQuestion>>>,
    subagent_startup_recovery_done: Arc<RwLock<bool>>,
    /// 限制同时执行的子 Agent 数量。
    subagent_exec_permits: Arc<Semaphore>,
    title_generation: Arc<RwLock<BTreeMap<String, Arc<tokio::sync::Notify>>>>,
    lazy_context_overrides: Arc<RwLock<BTreeMap<String, LazyContextOverride>>>,
    worker_health_cache: Arc<std::sync::RwLock<Vec<WorkerHealth>>>,
    channel_hub: Arc<RwLock<Option<Arc<ChannelHub>>>>,
    channel_runtime: Arc<RwLock<channel_manager::ChannelRuntimeStore>>,
    session_workspaces: Arc<RwLock<BTreeMap<String, PathBuf>>>,
    observability: Arc<Observability>,
    /// 嵌入式 PostgreSQL 自动安装/启动只触发一次（防止重复下载/初始化）。
    embedded_setup_started: Arc<std::sync::atomic::AtomicBool>,
    started_at: DateTime<Utc>,
}

#[derive(Clone)]
struct MainTurnDispatch {
    session_id: SessionId,
    turn_id: TurnId,
    attachments: Option<Vec<ChatAttachment>>,
    model_input: String,
    deep_thinking: bool,
    proactive: bool,
    run: RunRecord,
    model_step: StepRecord,
}

const AUX_TITLE_SYSTEM_PROMPT: &str = "Generate a concise, descriptive title (max 40 chars) for this conversation. Return ONLY the title, no quotes, no markdown.";
const AUX_KEY_FACTS_SYSTEM_PROMPT: &str = r#"Extract locked key facts from the conversation for an coding agent.
Return JSON only: {"facts":[{"kind":"constraint|decision|failure|file_path|command|api","text":"..."}]}
Rules: concrete reusable facts only; match conversation language; no invention; omit empty."#;

impl Clone for AppCore {
    fn clone(&self) -> Self {
        Self {
            workspace_root: self.workspace_root.clone(),
            eventbus: self.eventbus.clone(),
            storage: self.storage.clone(),
            sessions: self.sessions.clone(),
            execution: self.execution.clone(),
            governance: self.governance.clone(),
            tools: self.tools.clone(),
            memory: self.memory.clone(),
            agents: self.agents.clone(),
            agent_bridge: self.agent_bridge.clone(),
            runs: self.runs.clone(),
            turn_ledger: self.turn_ledger.clone(),
            pending_tool_approvals: self.pending_tool_approvals.clone(),
            pending_questions: self.pending_questions.clone(),
            subagent_startup_recovery_done: self.subagent_startup_recovery_done.clone(),
            subagent_exec_permits: self.subagent_exec_permits.clone(),
            title_generation: self.title_generation.clone(),
            lazy_context_overrides: self.lazy_context_overrides.clone(),
            worker_health_cache: self.worker_health_cache.clone(),
            channel_hub: self.channel_hub.clone(),
            channel_runtime: self.channel_runtime.clone(),
            session_workspaces: self.session_workspaces.clone(),
            observability: self.observability.clone(),
            embedded_setup_started: self.embedded_setup_started.clone(),
            started_at: self.started_at,
        }
    }
}

/// Bundled inputs for spawning a background memory-ingest batch.
struct MemoryIngestBatch {
    entries: Vec<NewMemoryEntry>,
    source: String,
    session_id: Option<String>,
    task_id: Option<String>,
    run_id: Option<String>,
    reason: String,
    confidence: Option<f32>,
}

/// Bundled health inputs for the doctor status builder.
struct DoctorStatusInput<'a> {
    config_info: &'a ConfigInfoDto,
    config_error: Option<String>,
    config: &'a Value,
    conversation: &'a ConversationStoreHealthDto,
    memory: &'a MemoryStoreHealthDto,
    gateway: &'a GatewayStatusDto,
    channels: &'a ChannelSummaryDto,
    workers: &'a [WorkerHealth],
    security: &'a SecurityStatusDto,
}

/// Bundled inputs for dispatching main-brain tool calls.
struct MainBrainToolDispatchInput<'a> {
    session_id: &'a SessionId,
    turn_id: &'a TurnId,
    run_id: &'a str,
    step_id: &'a str,
    cwd: &'a Path,
    tool_calls: Vec<MainBrainRuntimeToolCall>,
    iteration: usize,
    source: &'a str,
    speculative: Option<Arc<speculative_tools::SpeculativeToolBatch>>,
}

/// Bundled inputs for applying a main-brain tool dispatch result.
struct MainBrainToolApplyInput<'a> {
    context_surface: &'a mut ContextSurface,
    turn_memory_cache: &'a mut TurnMemoryCache,
    memory_context: &'a mut Option<String>,
    runtime_context: &'a mut String,
    tool_transcript: &'a mut Vec<String>,
    active_tool_names: &'a mut BTreeSet<String>,
    loop_guard: &'a mut ToolLoopGuard,
    guard_inputs: &'a [(&'a str, &'a Value)],
    model_output: &'a str,
    context_budget_allocation: &'a ContextBudgetAllocation,
    session_id: &'a SessionId,
    model_input: &'a str,
    config: &'a Value,
    dispatch_result: MainBrainToolDispatchResult,
}

impl AppCore {
    pub fn new(storage_config: StorageConfig) -> Self {
        let workspace_root =
            std::env::current_dir().unwrap_or_else(|_| storage_config.data_dir.clone());
        Self::new_with_workspace(storage_config, workspace_root)
    }

    fn new_with_workspace(storage_config: StorageConfig, workspace_root: PathBuf) -> Self {
        let storage = Arc::new(StorageService::new(storage_config.clone()));
        let journal = storage.event_journal();
        let eventbus = EventBus::new(1024).with_journal_sink(Arc::new({
            let journal = journal.clone();
            move |stored| {
                let record = xuanyu_storage::JournalEventRecord {
                    sequence: stored.sequence,
                    timestamp: stored.timestamp,
                    kind: stored.kind.clone(),
                    session_id: stored.session_id.clone(),
                    turn_id: stored.turn_id.clone(),
                    lane: stored.lane.clone(),
                    payload: stored.payload.clone(),
                };
                if let Err(err) = journal.append(&record) {
                    tracing::warn!("event journal append failed: {err:#}");
                }
            }
        }));
        let sessions = Arc::new(SessionManager::new(
            eventbus.clone(),
            storage.conversation_store(),
        ));
        let execution = Arc::new(ExecutionRuntime::new(ExecutionConfig::default()));
        let governance = Arc::new(GovernanceEngine::default());
        let memory = Arc::new(MemoryService::persistent(storage.memory_store_path()));
        let turn_ledger = Arc::new(TurnLedgerStore::new(workspace_root.clone()));
        let agents = AgentRuntime::new(
            storage.config().data_dir.clone(),
            Some(storage.control_plane()),
        );
        let tools = Arc::new(
            ToolRuntime::with_builtins(
                governance.clone(),
                vec![workspace_root.clone(), storage.config().data_dir.clone()],
            )
            .with_memory(memory.clone())
            .with_turn_ledger(turn_ledger.clone())
            .with_agents(agents.clone()),
        );
        let agent_bridge = Arc::new(AgentWorkerBridge::discover());
        let runs = RunStore::new(
            storage.config().data_dir.clone(),
            storage.config().project_name.clone(),
        );
        let pending_tool_approvals = Arc::new(RwLock::new(BTreeMap::new()));
        let pending_questions = Arc::new(RwLock::new(BTreeMap::new()));
        let subagent_startup_recovery_done = Arc::new(RwLock::new(false));
        let subagent_exec_permits = Arc::new(Semaphore::new(SUBAGENT_MAX_PARALLEL));
        let title_generation = Arc::new(RwLock::new(BTreeMap::new()));
        let lazy_context_overrides = Arc::new(RwLock::new(BTreeMap::new()));
        let worker_health_cache = Arc::new(std::sync::RwLock::new(Vec::new()));
        let channel_hub = Arc::new(RwLock::new(None));
        let channel_runtime = Arc::new(RwLock::new(channel_manager::ChannelRuntimeStore::default()));
        let observability = Arc::new(Observability::new(2048));
        let embedded_setup_started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_at = Utc::now();
        Self {
            workspace_root,
            eventbus,
            storage,
            sessions,
            execution,
            governance,
            tools,
            memory,
            agents,
            agent_bridge,
            runs,
            turn_ledger,
            pending_tool_approvals,
            pending_questions,
            subagent_startup_recovery_done,
            subagent_exec_permits,
            title_generation,
            lazy_context_overrides,
            worker_health_cache,
            channel_hub,
            channel_runtime,
            session_workspaces: Arc::new(RwLock::new(BTreeMap::new())),
            observability,
            embedded_setup_started,
            started_at,
        }
    }

    pub async fn initialize(&self) -> anyhow::Result<AppStatus> {
        self.storage.initialize().await?;
        self.agents.initialize().await?;
        self.runs.initialize().await?;
        self.memory.initialize().await?;
        if let Err(err) = xuanyu_tools::prune_stale_sandboxes(&self.workspace_root, None) {
            tracing::warn!("sandbox prune skipped: {err}");
        }
        let audit_path = self
            .storage
            .config()
            .data_dir
            .join("observability")
            .join("governance-audit.json");
        self.governance.set_audit_persist_path(audit_path).await;
        let config = self
            .storage
            .load_app_config()
            .await?
            .unwrap_or_else(|| json!({}));
        // 桌面端开箱即用：内置 PG 未就绪时由引擎自行拉起（已装则同步启动，未装则后台安装），
        // 不阻塞引擎 ready；确实连不上内置实例时降级上报并由后台自愈，绝不因此让引擎退出。
        match self.resolve_postgres_for_startup(&config).await? {
            Some((pg_conn, pg_schema, pg_prefix)) => {
                let attach = async {
                    self.storage
                        .configure_conversation_store(&pg_conn, &pg_schema, &pg_prefix)
                        .await
                        .context("configure conversation postgres store")?;
                    let mut pg_config =
                        postgres_memory_config_from_value(&config).unwrap_or_else(|| {
                            PostgresMemoryConfig {
                                connection_string: pg_conn.clone(),
                                schema: pg_schema.clone(),
                                table_prefix: "xuanyu".to_string(),
                                vector_dim: 1536,
                                project_name: self.storage.config().project_name.clone(),
                            }
                        });
                    if pg_config.project_name == "default" {
                        pg_config.project_name = self.storage.config().project_name.clone();
                    }
                    if pg_config.connection_string.trim().is_empty() {
                        pg_config.connection_string = pg_conn.clone();
                    }
                    self.memory
                        .configure_postgres(pg_config)
                        .await
                        .context("configure postgres memory backend")
                };
                if let Err(err) = attach.await {
                    let embedded_target =
                        postgres_connection_targets_embedded(&pg_conn, &self.embedded_postgres_paths());
                    if embedded_target {
                        // 内置实例未就绪（例如安装结束时按设计停掉了 PG）：引擎照常 ready，
                        // 后台重启实例并重新挂载，避免整个桌面端因数据库没起来而瘫痪。
                        self.observability
                            .health(
                                "storage",
                                HealthStatus::Degraded,
                                Some(format!("embedded PostgreSQL not reachable: {err:#}")),
                            )
                            .await;
                        self.observability
                            .health(
                                "memory",
                                HealthStatus::Degraded,
                                Some(
                                    "Postgres memory backend pending embedded PostgreSQL restart"
                                        .to_string(),
                                ),
                            )
                            .await;
                        self.eventbus.publish(AppEvent::LogEntry {
                            level: "warn".to_string(),
                            message: format!(
                                "内置数据库未就绪，正在后台重新启动（{err:#}），完成后会话与长期记忆自动恢复"
                            ),
                        });
                        self.spawn_embedded_postgres_auto_setup();
                    } else {
                        self.observability
                            .health(
                                "memory",
                                HealthStatus::Down,
                                Some(format!("postgres memory backend failed: {err:#}")),
                            )
                            .await;
                        anyhow::bail!("PostgreSQL/pgvector memory backend unavailable: {err:#}");
                    }
                } else {
                    self.observability
                        .log("info", "Postgres/pgvector memory backend configured")
                        .await;
                }
            }
            None => {
                // 嵌入式 PG 正在后台安装/启动；会话与长期记忆暂不可用，
                // 引擎仍正常 ready，存储状态由 health / engine.status 上报。
                self.observability
                    .health(
                        "storage",
                        HealthStatus::Degraded,
                        Some(
                            "Embedded PostgreSQL is being installed in background; first launch needs network to download database components."
                                .to_string(),
                        ),
                    )
                    .await;
                self.observability
                    .health(
                        "memory",
                        HealthStatus::Degraded,
                        Some(
                            "Postgres memory backend pending embedded PostgreSQL install".to_string(),
                        ),
                    )
                    .await;
                self.eventbus.publish(AppEvent::LogEntry {
                    level: "info".to_string(),
                    message: "首次使用：正在后台自动安装嵌入式 PostgreSQL（需联网下载数据库组件），完成后会话与长期记忆自动可用"
                        .to_string(),
                });
            }
        }
        if let Some(tv_config) =
            tantivy_memory_config_from_value(&config, &self.storage.config().data_dir)
        {
            if let Err(err) = self.memory.configure_tantivy(tv_config).await {
                tracing::warn!("tantivy memory backend unavailable: {err:#}");
            } else {
                self.observability
                    .log("info", "Tantivy local memory index configured")
                    .await;
            }
        }
        self.apply_execution_config(&config).await;
        let _project_dir = project_dir_from_config(&config)
            .unwrap_or_else(|| self.workspace_root.clone());
        let recovered_subagent_tasks = self.recover_subagent_execution_queue().await;
        self.observability
            .health(
                "storage",
                HealthStatus::Ok,
                Some("local data directories ready".to_string()),
            )
            .await;
        self.observability
            .health(
                "execution",
                HealthStatus::Ok,
                Some(self.execution_status_detail().await),
            )
            .await;
        self.observability
            .health(
                "tools",
                HealthStatus::Ok,
                Some("readonly builtin tools registered".to_string()),
            )
            .await;
        self.observability
            .health(
                "agents",
                HealthStatus::Ok,
                Some(if recovered_subagent_tasks > 0 {
                    format!(
                        "sub-agent definitions and task ledger ready; recovered {recovered_subagent_tasks} queued/running task(s)"
                    )
                } else {
                    "sub-agent definitions and task ledger ready".to_string()
                }),
            )
            .await;
        if recovered_subagent_tasks > 0 {
            self.observability
                .log(
                    "info",
                    &format!(
                        "Recovered {recovered_subagent_tasks} persisted sub-agent task(s) for local execution"
                    ),
                )
                .await;
        }
        self.warm_workers().await;
        let _ = self.refresh_workers_with_alerts().await;
        self.spawn_subagent_watchdog_loop();
        // Partial ASR while holding the mic competes with final transcription latency.
        // Person mode uses push-to-talk + finish_utterance for ASR instead.
        if let Ok(max_sequence) = self.storage.event_journal().max_sequence() {
            self.eventbus.restore_sequence(max_sequence);
        }
        let _ = self.start_delivery_channels(&config).await;
        self.observability
            .log("info", "XuanYu Rust core initialized")
            .await;
        Ok(self.status().await)
    }

    /// 内置 PostgreSQL 的安装路径（随数据目录走）。
    fn embedded_postgres_paths(&self) -> EmbeddedPostgresPaths {
        EmbeddedPostgresPaths::from_data_dir(&self.storage.config().data_dir)
    }

    /// 启动期 PostgreSQL 解析：
    /// - 配置指向外部数据库 → 直接使用（由调用方按现状失败处理，引擎不管理外部实例）；
    /// - 配置指向内置实例（安装器写入的本地连接串）→ 确保实例在跑：已在跑直接用，
    ///   已安装则同步拉起，未安装则后台安装；
    /// - 无配置 → 内置实例已在跑则直接用，否则后台自动安装/启动（幂等，不阻塞 ready）。
    ///
    /// 返回 None 表示内置实例尚未就绪，调用方应降级而非失败。
    async fn resolve_postgres_for_startup(
        &self,
        config: &Value,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let configured = configured_postgres(config);
        let paths = self.embedded_postgres_paths();
        if let Some(conn) = configured.connection_string {
            if !postgres_connection_targets_embedded(&conn, &paths) {
                return Ok(Some((conn, configured.schema, configured.table_prefix)));
            }
            let status = embedded_postgres_status(&paths);
            if status.running {
                return Ok(Some((conn, configured.schema, configured.table_prefix)));
            }
            if status.installed {
                let start_paths = paths.clone();
                match tokio::task::spawn_blocking(move || start_embedded_postgres(&start_paths))
                    .await
                {
                    Ok(Ok((connection_string, _logs))) => {
                        self.observability
                            .log("info", "Embedded PostgreSQL started for local connection")
                            .await;
                        return Ok(Some((
                            connection_string,
                            configured.schema,
                            configured.table_prefix,
                        )));
                    }
                    Ok(Err(err)) => {
                        tracing::warn!("start embedded postgres failed: {err}");
                    }
                    Err(err) => {
                        tracing::warn!("start embedded postgres task failed: {err}");
                    }
                }
            }
            self.spawn_embedded_postgres_auto_setup();
            return Ok(None);
        }
        let status = embedded_postgres_status(&paths);
        if status.running {
            return Ok(Some((
                paths.connection_string(),
                configured.schema,
                configured.table_prefix,
            )));
        }
        self.spawn_embedded_postgres_auto_setup();
        Ok(None)
    }

    /// 后台自动安装/启动嵌入式 PostgreSQL（仅触发一次；安装期间不阻塞引擎）。
    /// 成功 → 配置会话存储与长期记忆并上报 ready；失败 → 上报错误与降级健康。
    fn spawn_embedded_postgres_auto_setup(&self) {
        use std::sync::atomic::Ordering;
        if self
            .embedded_setup_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let core = self.clone();
        tokio::spawn(async move {
            // 仅当配置指向外部数据库时才跳过：内置实例（本地连接串）的安装/启动
            // 由引擎自己负责，安装器写入的本地连接串不代表"用户自备数据库"。
            let paths = core.embedded_postgres_paths();
            let skip_external = core
                .storage
                .load_app_config()
                .await
                .unwrap_or(None)
                .and_then(|cfg| configured_postgres(&cfg).connection_string)
                .map(|conn| !postgres_connection_targets_embedded(&conn, &paths))
                .unwrap_or(false);
            if skip_external {
                tracing::info!(
                    "external postgres configured; skipping embedded postgres auto setup"
                );
                return;
            }
            let status = embedded_postgres_status(&paths);
            let should_start_only = status.installed;
            let eventbus = core.eventbus.clone();
            let publish = |kind: &str,
                           phase: Option<&str>,
                           percent: Option<u8>,
                           message: String,
                           extra: Option<Value>| {
                let mut event = json!({ "type": kind, "message": message });
                if let Some(phase) = phase {
                    event["phase"] = json!(phase);
                }
                if let Some(percent) = percent {
                    event["percent"] = json!(percent);
                }
                if let Some(extra) = extra {
                    if let Some(obj) = extra.as_object() {
                        for (key, value) in obj {
                            event[key] = value.clone();
                        }
                    }
                }
                eventbus.publish(AppEvent::StorageProgress { event });
            };
            let result = if should_start_only {
                publish(
                    "progress",
                    Some("start"),
                    Some(75),
                    "Starting embedded PostgreSQL".to_string(),
                    None,
                );
                tokio::task::spawn_blocking(move || start_embedded_postgres(&paths)).await
            } else {
                let install_paths = paths.clone();
                let install_eventbus = eventbus.clone();
                tokio::task::spawn_blocking(move || {
                    install_embedded_postgres(
                        &install_paths,
                        EmbeddedPostgresInstallOptions {
                            skip_download: false,
                            install_pgvector: true,
                        },
                        move |progress| {
                            let event = json!({
                                "type": "progress",
                                "message": progress.message,
                                "phase": progress.phase,
                                "percent": progress.percent,
                                "downloaded": progress.downloaded,
                                "total": progress.total,
                            });
                            install_eventbus.publish(AppEvent::StorageProgress { event });
                        },
                    )
                })
                .await
            };
            match result {
                Ok(Ok((connection_string, logs))) => {
                    let config = core.storage.load_app_config().await.unwrap_or(None);
                    let config = config.unwrap_or_else(|| json!({}));
                    let configured = configured_postgres(&config);
                    let mut attached = true;
                    if let Err(err) = core
                        .storage
                        .configure_conversation_store(
                            &connection_string,
                            &configured.schema,
                            &configured.table_prefix,
                        )
                        .await
                    {
                        attached = false;
                        tracing::warn!("auto-configure conversation store failed: {err:#}");
                    }
                    let mut pg_config = postgres_memory_config_from_value(&config).unwrap_or_else(
                        || PostgresMemoryConfig {
                            connection_string: connection_string.clone(),
                            schema: configured.schema.clone(),
                            table_prefix: "xuanyu".to_string(),
                            vector_dim: 1536,
                            project_name: core.storage.config().project_name.clone(),
                        },
                    );
                    if pg_config.project_name == "default" {
                        pg_config.project_name = core.storage.config().project_name.clone();
                    }
                    if pg_config.connection_string.trim().is_empty() {
                        pg_config.connection_string = connection_string.clone();
                    }
                    if let Err(err) = core.memory.configure_postgres(pg_config).await {
                        attached = false;
                        tracing::warn!("auto-configure postgres memory failed: {err:#}");
                    }
                    if attached {
                        core.observability
                            .health(
                                "storage",
                                HealthStatus::Ok,
                                Some("embedded postgresql ready".to_string()),
                            )
                            .await;
                        core.observability
                            .health(
                                "memory",
                                HealthStatus::Ok,
                                Some("postgres/pgvector memory backend configured".to_string()),
                            )
                            .await;
                        core.observability
                            .log(
                                "info",
                                "Embedded PostgreSQL installed and attached automatically",
                            )
                            .await;
                        publish(
                            "done",
                            None,
                            Some(100),
                            "Embedded PostgreSQL is ready".to_string(),
                            Some(json!({
                                "ok": true,
                                "connectionString": connection_string,
                                "pgvector": true,
                                "logs": logs,
                            })),
                        );
                    } else {
                        publish(
                            "done",
                            None,
                            Some(60),
                            "Embedded PostgreSQL installed but store attach failed".to_string(),
                            Some(json!({
                                "ok": false,
                                "connectionString": connection_string,
                                "logs": logs,
                                "error": "Embedded PostgreSQL is ready but conversation/memory store attach failed; restart the app or set memory.postgres.connectionString.".to_string(),
                            })),
                        );
                    }
                }
                Ok(Err(error)) => {
                    core.observability
                        .health(
                            "storage",
                            HealthStatus::Down,
                            Some(format!("embedded postgresql setup failed: {error}")),
                        )
                        .await;
                    let message = format!(
                        "嵌入式 PostgreSQL 自动安装失败：{error}（首次使用需联网下载数据库组件，请检查网络后重试）"
                    );
                    core.observability.log("error", &message).await;
                    publish(
                        "done",
                        None,
                        Some(60),
                        message.clone(),
                        Some(json!({ "ok": false, "logs": [], "error": message })),
                    );
                }
                Err(err) => {
                    core.observability
                        .health(
                            "storage",
                            HealthStatus::Down,
                            Some(format!("embedded postgresql setup task failed: {err}")),
                        )
                        .await;
                    let message = format!(
                        "嵌入式 PostgreSQL 自动安装任务失败：{err}（首次使用需联网下载数据库组件，请检查网络后重试）"
                    );
                    core.observability.log("error", &message).await;
                    publish(
                        "done",
                        None,
                        Some(60),
                        message.clone(),
                        Some(json!({ "ok": false, "logs": [], "error": message })),
                    );
                }
            }
        });
    }

    fn spawn_subagent_watchdog_loop(&self) {
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let config = core.config_get().await.unwrap_or_else(|_| json!({}));
                let settings = watchdog_settings_from_config(&config);
                let issues = core.agents.poll_watchdog().await;
                for (task_id, task_issues) in issues {
                    for issue in task_issues {
                        core.eventbus.publish(AppEvent::LogEntry {
                            level: "warn".to_string(),
                            message: format!(
                                "Sub-agent watchdog [{task_id}] {}: {}",
                                issue.kind, issue.message
                            ),
                        });
                        if let Some(plan) = core
                            .agents
                            .apply_watchdog_recovery(&task_id, &issue, &settings)
                            .await
                        {
                            match plan.action {
                                xuanyu_agents::WatchdogRecoveryAction::Retry => {
                                    let spawn_id = plan
                                        .new_task_id
                                        .clone()
                                        .unwrap_or_else(|| task_id.clone());
                                    core.eventbus.publish(AppEvent::LogEntry {
                                        level: "info".to_string(),
                                        message: format!(
                                            "Sub-agent watchdog auto-recovery spawned {spawn_id} (from {task_id}): {}",
                                            plan.reason
                                        ),
                                    });
                                    core.spawn_subagent_execution(spawn_id);
                                }
                                xuanyu_agents::WatchdogRecoveryAction::Block => {
                                    if let Some(task) =
                                        core.agents.task_status(task_id.clone()).await
                                    {
                                        core.publish_team_agent_event("failed", &task).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    async fn warm_workers(&self) {
        let mut workers = Vec::new();
        if self.agent_bridge.is_configured() {
            let bridge = self.agent_bridge.clone();
            match tokio::task::spawn_blocking(move || bridge.warm()).await {
                Ok(Ok(result)) => {
                    let status = result
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    self.observability
                        .log("info", &format!("Python agent worker warm ping: {status}"))
                        .await;
                }
                Ok(Err(err)) => {
                    self.observability
                        .log(
                            "warn",
                            &format!("Python agent worker warm ping failed: {err:#}"),
                        )
                        .await;
                }
                Err(err) => {
                    self.observability
                        .log(
                            "warn",
                            &format!("Python agent worker warm task failed: {err}"),
                        )
                        .await;
                }
            }
            workers.push(worker_health_from_agent_status(
                self.agent_bridge.status_snapshot(),
            ));
        }
        if workers.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.worker_health_cache.write() {
            *guard = workers.clone();
        }
        for worker in workers {
            self.eventbus.publish(AppEvent::WorkerHealth { worker });
        }
    }

    pub fn refresh_workers_and_publish(&self) -> Vec<WorkerHealth> {
        let workers = vec![worker_health_from_agent_status(self.agent_bridge.status())];
        if let Ok(mut guard) = self.worker_health_cache.write() {
            *guard = workers.clone();
        }
        for worker in &workers {
            self.eventbus
                .publish(AppEvent::WorkerHealth { worker: worker.clone() });
        }
        workers
    }

    pub async fn refresh_workers_with_alerts(&self) -> Vec<WorkerHealth> {
        let previous = self
            .worker_health_cache
            .read()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let workers = tokio::task::spawn_blocking({
            let core = self.clone();
            move || core.refresh_workers_and_publish()
        })
        .await
        .unwrap_or_default();
        for worker in &workers {
            let previous_status = previous
                .iter()
                .find(|item| item.name == worker.name)
                .map(|item| item.status);
            if previous_status == Some(worker.status) {
                continue;
            }
            match worker.status {
                WorkerStatus::Degraded => {
                    self.observability
                        .alert(
                            AlertSeverity::Warning,
                            format!("Worker {} degraded", worker.name),
                            worker
                                .detail
                                .as_ref()
                                .map(|detail| json!({ "detail": detail })),
                            None,
                        )
                        .await;
                }
                WorkerStatus::Unavailable => {
                    self.observability
                        .alert(
                            AlertSeverity::Critical,
                            format!("Worker {} unavailable", worker.name),
                            worker
                                .detail
                                .as_ref()
                                .map(|detail| json!({ "detail": detail })),
                            None,
                        )
                        .await;
                }
                WorkerStatus::Healthy
                    if matches!(
                        previous_status,
                        Some(WorkerStatus::Degraded | WorkerStatus::Unavailable)
                    ) =>
                {
                    self.observability
                        .alert(
                            AlertSeverity::Info,
                            format!("Worker {} recovered", worker.name),
                            worker
                                .detail
                                .as_ref()
                                .map(|detail| json!({ "detail": detail })),
                            None,
                        )
                        .await;
                }
                _ => {}
            }
        }
        workers
    }

    pub async fn audit_list(&self, limit: Option<usize>) -> Value {
        let limit = limit.unwrap_or(200).clamp(1, 2_000);
        let events = self.governance.audit_events().await;
        let events = events
            .into_iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>();
        let rollbacks = self.governance.rollback_actions().await;
        json!({ "events": events, "rollbackActions": rollbacks })
    }

    pub async fn rollback_list(&self) -> Value {
        json!({
            "actions": self.governance.rollback_actions().await
        })
    }

    pub async fn rollback_run(&self, rollback_id: String) -> Value {
        let Some(action) = self
            .governance
            .rollback_registry()
            .find(rollback_id.trim())
            .await
        else {
            return json!({
                "ok": false,
                "error": "Rollback action not found"
            });
        };
        let result = match action.kind.as_str() {
            "memory_snapshot" | "changeset" => json!({
                "ok": false,
                "error": "Rollback kind is no longer supported"
            }),
            "context" => {
                let session_id = action
                    .payload
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if session_id.is_empty() {
                    json!({
                        "ok": false,
                        "error": "context rollback requires sessionId"
                    })
                } else {
                    self.context_rollback(SessionId(session_id.to_string())).await
                }
            }
            other => json!({
                "ok": false,
                "error": format!("Unsupported rollback kind: {other}")
            }),
        };
        if result.get("ok").and_then(Value::as_bool) == Some(true)
            || result.get("result")
                .and_then(|value| value.get("ok"))
                .and_then(Value::as_bool)
                == Some(true)
        {
            let _ = self
                .governance
                .rollback_registry()
                .remove(&action.id)
                .await;
        }
        json!({
            "ok": result.get("ok").and_then(Value::as_bool).unwrap_or(false)
                || result.get("result").and_then(|value| value.get("ok")).and_then(Value::as_bool).unwrap_or(false),
            "action": action,
            "result": result,
        })
    }

    pub async fn status(&self) -> AppStatus {
        let memory_stats = self.memory.stats().await;
        let tool_count = self.tools.catalog().await.len();
        let agent_count = self.agents.list_agents().await.len();
        let health_entries = self.observability.health_summary().await.len();
        let execution_status = self.execution_module_status().await;
        AppStatus {
            protocol_version: PROTOCOL_VERSION.to_string(),
            initialized: self.storage.is_initialized().await,
            storage_backend: "sqlite".to_string(),
            active_sessions: self.sessions.list_sessions().await.len(),
            modules: vec![
                execution_status,
                RuntimeModuleStatus {
                    name: "tools".to_string(),
                    status: WorkerStatus::Healthy,
                    detail: Some(format!("{tool_count} builtin readonly tools")),
                },
                RuntimeModuleStatus {
                    name: "governance".to_string(),
                    status: WorkerStatus::Healthy,
                    detail: Some(
                        "permission, risk, budget, quota, audit decisions ready".to_string(),
                    ),
                },
                RuntimeModuleStatus {
                    name: "memory".to_string(),
                    status: WorkerStatus::Healthy,
                    detail: Some(format!(
                        "{} local memory entries",
                        memory_stats.total_entries
                    )),
                },
                RuntimeModuleStatus {
                    name: "agents".to_string(),
                    status: WorkerStatus::Healthy,
                    detail: Some(format!("{agent_count} sub-agent definitions")),
                },
                RuntimeModuleStatus {
                    name: "observability".to_string(),
                    status: WorkerStatus::Healthy,
                    detail: Some(format!("{health_entries} health checks")),
                },
            ],
            workers: self.runtime_workers(),
        }
    }

    fn runtime_workers(&self) -> Vec<WorkerHealth> {
        if let Ok(guard) = self.worker_health_cache.read() {
            if !guard.is_empty() {
                return guard.clone();
            }
        }
        vec![worker_health_from_agent_status(self.agent_bridge.status())]
    }

    async fn execution_module_status(&self) -> RuntimeModuleStatus {
        let route = self.execution.resolve_route(ModelTaskKind::Main).await;
        RuntimeModuleStatus {
            name: "execution".to_string(),
            status: if route.compatibility.compatible {
                WorkerStatus::Healthy
            } else {
                WorkerStatus::Degraded
            },
            detail: Some(model_route_status_detail(&route)),
        }
    }

    async fn execution_status_detail(&self) -> String {
        let route = self.execution.resolve_route(ModelTaskKind::Main).await;
        model_route_status_detail(&route)
    }

    /// Refresh aggregated component health (storage / execution / tools / memory / agents)
    /// with short timeouts so doctor / system_status stay responsive.
    async fn refresh_health_registry(&self) {
        let timeout = std::time::Duration::from_millis(1500);

        let storage = tokio::time::timeout(timeout, async {
            self.storage.conversation_store().health().await
        })
        .await;
        match storage {
            Ok(health) if health.ok => {
                self.observability
                    .health(
                        "storage",
                        HealthStatus::Ok,
                        Some(format!(
                            "{} @ {}",
                            health.backend,
                            summarize_db_location(&health.location)
                        )),
                    )
                    .await;
            }
            Ok(health) => {
                self.observability
                    .health(
                        "storage",
                        HealthStatus::Degraded,
                        health.error.or_else(|| Some(health.backend.clone())),
                    )
                    .await;
            }
            Err(_) => {
                self.observability
                    .health(
                        "storage",
                        HealthStatus::Degraded,
                        Some("storage health probe timed out".to_string()),
                    )
                    .await;
            }
        }

        let execution = tokio::time::timeout(timeout, self.execution_status_detail()).await;
        match execution {
            Ok(detail) => {
                self.observability
                    .health("execution", HealthStatus::Ok, Some(detail))
                    .await;
            }
            Err(_) => {
                self.observability
                    .health(
                        "execution",
                        HealthStatus::Degraded,
                        Some("execution health probe timed out".to_string()),
                    )
                    .await;
            }
        }

        let tools = tokio::time::timeout(timeout, self.tools.catalog()).await;
        match tools {
            Ok(catalog) => {
                self.observability
                    .health(
                        "tools",
                        HealthStatus::Ok,
                        Some(format!("{} tools registered", catalog.len())),
                    )
                    .await;
            }
            Err(_) => {
                self.observability
                    .health(
                        "tools",
                        HealthStatus::Degraded,
                        Some("tools catalog probe timed out".to_string()),
                    )
                    .await;
            }
        }

        let memory = tokio::time::timeout(timeout, self.memory.stats()).await;
        match memory {
            Ok(stats) => {
                self.observability
                    .health(
                        "memory",
                        HealthStatus::Ok,
                        Some(format!("entries={}", stats.total_entries)),
                    )
                    .await;
            }
            Err(_) => {
                self.observability
                    .health(
                        "memory",
                        HealthStatus::Degraded,
                        Some("memory stats probe timed out".to_string()),
                    )
                    .await;
            }
        }

        let agents = tokio::time::timeout(timeout, self.agents.list_agents()).await;
        match agents {
            Ok(list) => {
                self.observability
                    .health(
                        "agents",
                        HealthStatus::Ok,
                        Some(format!("{} agent definition(s)", list.len())),
                    )
                    .await;
            }
            Err(_) => {
                self.observability
                    .health(
                        "agents",
                        HealthStatus::Degraded,
                        Some("agents probe timed out".to_string()),
                    )
                    .await;
            }
        }
    }

    pub fn eventbus(&self) -> EventBus {
        self.eventbus.clone()
    }

    pub async fn event_replay(&self, last_sequence: u64) -> EventReplayBatch {
        self.event_replay_filtered(last_sequence, EventFilter::default())
            .await
    }

    pub async fn event_replay_filtered(
        &self,
        last_sequence: u64,
        filter: EventFilter,
    ) -> EventReplayBatch {
        let mut batch = self
            .eventbus
            .replay_since_filtered(last_sequence, filter.clone())
            .await;
        batch.events = self
            .merge_journal_replay(last_sequence, &filter, batch.events)
            .await;
        if !batch.events.is_empty() {
            batch.overflow = false;
        }
        batch
    }

    async fn merge_journal_replay(
        &self,
        last_sequence: u64,
        filter: &EventFilter,
        ring_events: Vec<xuanyu_eventbus::StoredEvent>,
    ) -> Vec<xuanyu_eventbus::StoredEvent> {
        let journal = self.storage.event_journal();
        let session_id = filter.session_id.clone();
        let turn_id = filter.turn_id.clone();
        let lane = filter.lane.clone();
        let records = match tokio::task::spawn_blocking(move || {
            journal.replay_since(
                last_sequence,
                session_id.as_deref(),
                turn_id.as_deref(),
                lane.as_deref(),
                4096,
            )
        })
        .await
        {
            Ok(Ok(records)) => records,
            _ => Vec::new(),
        };
        let mut merged = std::collections::BTreeMap::new();
        for event in ring_events {
            merged.insert(event.sequence, event);
        }
        for record in records {
            let stored = xuanyu_eventbus::StoredEvent {
                sequence: record.sequence,
                timestamp: record.timestamp,
                kind: record.kind.clone(),
                session_id: record.session_id.clone(),
                turn_id: record.turn_id.clone(),
                lane: record.lane.clone(),
                payload: record.payload.clone(),
            };
            if filter.matches_stored(&stored) {
                merged.entry(stored.sequence).or_insert(stored);
            }
        }
        let mut events = merged.into_values().collect::<Vec<_>>();
        events.sort_by_key(|event| event.sequence);
        events
    }

    pub async fn control_plane_list_tasks(
        &self,
        session_id: Option<String>,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let store = self.storage.control_plane();
        let session = session_id;
        tokio::task::spawn_blocking(move || store.list_tasks(session.as_deref(), limit))
            .await?
    }

    pub async fn event_dead_letters(&self, limit: usize) -> Vec<DeadLetterRecord> {
        self.eventbus.dead_letters(limit).await
    }

    pub async fn create_main_session(&self) -> anyhow::Result<SessionInfo> {
        self.sessions.create_main_session().await
    }

    pub async fn create_session(
        &self,
        mode: Option<String>,
        title: Option<String>,
        project_dir: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        let mode = mode
            .as_deref()
            .map(str::trim)
            .unwrap_or("main")
            .to_ascii_lowercase();
        match mode.as_str() {
            "recent" => {
                self.sessions
                    .create_branch_session(title, None)
                    .await
            }
            "branch" => {
                self.sessions
                    .create_branch_session(title, project_dir)
                    .await
            }
            _ => self.sessions.create_main_session().await,
        }
    }

    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.list_sessions().await
    }

    pub async fn query_sessions(
        &self,
        include_archived: bool,
        search: Option<String>,
        limit: usize,
    ) -> Vec<SessionInfo> {
        self.sessions
            .query_sessions(include_archived, search.as_deref(), limit)
            .await
    }

    pub async fn conversation_search(&self, query: String, limit: usize) -> Vec<SessionInfo> {
        self.sessions.search_conversations(&query, limit).await
    }

    pub async fn send_message(
        &self,
        session_id: SessionId,
        content: String,
    ) -> anyhow::Result<TurnId> {
        self.send_message_with_attachments(session_id, content, None, false)
            .await
    }

    pub async fn send_message_with_attachments(
        &self,
        session_id: SessionId,
        content: String,
        attachments: Option<Vec<ChatAttachment>>,
        deep_thinking: bool,
    ) -> anyhow::Result<TurnId> {
        let attachments = attachments.filter(|items| !items.is_empty());
        let turn_id = self
            .sessions
            .append_user_message_with_attachments(&session_id, content.clone(), attachments.clone())
            .await?;

        // Align with reference: kick off title generation in parallel with the main reply
        // when the session still has a placeholder name.
        if let Some(session) = self
            .list_sessions()
            .await
            .into_iter()
            .find(|item| item.id == session_id)
        {
            if should_auto_title(&session.name, session.title_generated_by.as_deref())
                && !content.trim().is_empty()
            {
                let core = self.clone();
                let title_session = session_id.clone();
                let title_seed = content.clone();
                tokio::spawn(async move {
                    core.maybe_auto_title_session(title_session, title_seed)
                        .await;
                });
            }
        }

        if self.sessions.is_processing(&session_id).await {
            self.sessions
                .queue_message(
                    &session_id,
                    &turn_id,
                    content.clone(),
                    attachments.clone(),
                    deep_thinking,
                )
                .await;
            self.eventbus.publish(AppEvent::LogEntry {
                level: "info".to_string(),
                message: format!(
                    "Message queued for session {} (turn {})",
                    session_id.0, turn_id.0
                ),
            });
            return Ok(turn_id);
        }

        self.spawn_main_turn(
            session_id,
            turn_id.clone(),
            content,
            attachments,
            deep_thinking,
            false,
        )
        .await?;
        Ok(turn_id)
    }

    async fn spawn_main_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        content: String,
        attachments: Option<Vec<ChatAttachment>>,
        deep_thinking: bool,
        proactive: bool,
    ) -> anyhow::Result<()> {
        let model_input = model_input_with_attachments(&content, attachments.as_deref());
        let run = self
            .runs
            .create_run(CreateRunInput {
                session_id: session_id.0.clone(),
                goal: if content.trim().is_empty() {
                    "(attachment message)".to_string()
                } else {
                    content.clone()
                },
                metadata: BTreeMap::from([
                    ("source".to_string(), json!("chat")),
                    ("kind".to_string(), json!("mainbrain")),
                    (
                        "attachmentCount".to_string(),
                        json!(attachments.as_ref().map(Vec::len).unwrap_or_default()),
                    ),
                    ("deepThinking".to_string(), json!(deep_thinking)),
                ]),
            })
            .await?;
        let model_step = self
            .runs
            .start_step(StartStepInput {
                run_id: run.id.clone(),
                step_type: "model".to_string(),
                name: "Main brain response".to_string(),
                metadata: BTreeMap::new(),
            })
            .await?;
        if let Err(err) = self
            .turn_ledger
            .capture_workspace_checkpoint(&session_id.0, &turn_id.0)
        {
            self.observability
                .log(
                    "warn",
                    format!("Failed to capture workspace checkpoint: {err}"),
                )
                .await;
        }
        let dispatch = MainTurnDispatch {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            attachments,
            model_input,
            deep_thinking,
            proactive,
            run,
            model_step,
        };
        self.sessions.set_processing(&session_id, true).await;
        self.run_main_turn_dispatch(session_id, dispatch).await
    }

    async fn run_main_turn_dispatch(
        &self,
        session_id: SessionId,
        dispatch: MainTurnDispatch,
    ) -> anyhow::Result<()> {
        let core_bg = self.clone();
        let drain_session_id = session_id.clone();
        let fail_session_id = dispatch.session_id.clone();
        let fail_turn_id = dispatch.turn_id.clone();
        tokio::spawn(async move {
            let result = core_bg
                .complete_main_turn_after_user_message_inner(dispatch)
                .await;
            core_bg
                .sessions
                .set_processing(&fail_session_id, false)
                .await;
            if result.is_ok() {
                core_bg.schedule_next_queued_turn(drain_session_id);
            }
            if let Err(err) = result {
                tracing::error!("main turn failed: {err:#}");
                core_bg.eventbus.publish(AppEvent::TurnFailed {
                    session_id: fail_session_id.clone(),
                    turn_id: fail_turn_id.clone(),
                    error: AppError {
                        code: ErrorCode::InternalInvariantBroken,
                        message: err.to_string(),
                        recoverable: true,
                        suggestion: Some(
                            "Retry the message or check model configuration.".to_string(),
                        ),
                        context: BTreeMap::new(),
                    },
                });
                let _ = core_bg.sessions.abort_session(&fail_session_id).await;
            }
        });
        Ok(())
    }

    fn schedule_next_queued_turn(&self, session_id: SessionId) {
        let core = self.clone();
        tokio::spawn(async move {
            let Some(queued) = core.sessions.dequeue_if_idle(&session_id).await else {
                return;
            };
            let _ = core
                .spawn_main_turn(
                    session_id,
                    TurnId(queued.turn_id),
                    queued.content,
                    queued.attachments,
                    queued.deep_thinking,
                    false,
                )
                .await;
        });
    }

    async fn complete_main_turn_after_user_message_inner(
        &self,
        dispatch: MainTurnDispatch,
    ) -> anyhow::Result<()> {
        let MainTurnDispatch {
            session_id,
            turn_id,
            attachments,
            model_input,
            deep_thinking,
            proactive,
            run,
            model_step,
        } = dispatch;
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let turn_history = self.sessions.history(&session_id).await.unwrap_or_default();
        let main_brain_intent = infer_mainbrain_intent_from_messages(&turn_history);
        if !proactive {
            self.publish_initial_run_plan(
                &session_id,
                &run.id,
                &run.goal,
                main_brain_intent,
            )
            .await?;
        }
        let plugin_hook_notes = if proactive {
            Vec::new()
        } else {
            let project_dir = project_dir_from_config(&config).unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
            });
            let extension_config = ExtensionLoadConfig::from_app_config(project_dir, &config);
            let plugins = load_plugins(&extension_config);
            run_plugin_before_turn_hooks(
                &extension_config,
                &plugins,
                &PluginBeforeTurnInput {
                    session_id: session_id.0.clone(),
                    run_id: run.id.clone(),
                    user_message: model_input.clone(),
                    intent: main_brain_intent.to_string(),
                },
            )
        };
        let lazy_override = self.take_lazy_context_override(&session_id).await;
        let bootstrap = bootstrap_main_turn(
            &config,
            &model_input,
            main_brain_intent,
            lazy_override.clone(),
        );
        let turn_plan = bootstrap.turn_plan;
        let context_policy = turn_plan.context_policy.clone();
        let mut agent = bootstrap.agent;
        let context_surface = std::mem::take(agent.context_surface_mut());
        let turn_memory_cache = std::mem::take(agent.memory_cache_mut());
        let memory_context = if turn_plan.auto_retrieve_memory {
            self.retrieved_memory_context(&session_id, &model_input, &config)
                .await
        } else {
            None
        };
        let key_facts_context = if turn_plan.inject_key_facts {
            self.build_key_facts_context_for_turn(&turn_history, &model_input)
                .await
        } else {
            None
        };
        let intent_guidance = format_intent_guidance(main_brain_intent);
        let active_context_sections_text = self
            .build_turn_active_context_sections(
                &session_id.0,
                &context_policy,
                &model_input,
                &config,
            )
            .await;
        let active_context_sections_text = match active_context_sections_text {
            Some(text) => Some(format!("{intent_guidance}\n\n{text}")),
            None => Some(intent_guidance),
        };
        let policy_decision = self
            .plan_main_turn_policy(
                &session_id,
                &model_input,
                memory_context.as_deref(),
                &config,
            )
            .await;
        let policy_tool_report = match &policy_decision {
            Ok(decision) => {
                self.execute_mainbrain_policy_tool_plan(
                    &session_id,
                    &run.id,
                    &model_step.id,
                    &decision.tool_plan,
                )
                .await?
            }
            Err(_) => MainBrainToolPlanReport::default(),
        };
        let tool_catalog = self.tools.catalog().await;
        let tool_definitions = self.tools.definitions().await;
        let goal_preload = self.active_goal_preload_tools(&session_id).await;
        let mut active_tool_names = reset_lazy_tool_surface(&goal_preload);
        if let Some(override_state) = lazy_override.as_ref() {
            for name in &override_state.active_tool_names {
                active_tool_names.insert(name.clone());
            }
        }
        let tool_schema_tokens =
            estimate_active_tool_schema_tokens(&tool_definitions, &active_tool_names);
        let deferred_catalog = mainbrain_deferred_tool_catalog(&tool_catalog);
        let runtime_context_parts = MainBrainRuntimeContextParts {
            system: runtime_agent_prompt(),
            memory: memory_context.clone(),
            proactive: if proactive {
                Some(format!(
                    "Proactive initiative:\n{model_input}\n\nYou are speaking proactively in Person mode. Be brief, warm, and natural."
                ))
            } else {
                None
            },
            key_facts: key_facts_context.clone(),
            context_sections: active_context_sections_text.clone(),
            plugin_notes: plugin_hook_notes.clone(),
            policy_tools: policy_tool_report.to_prompt_context(),
            policy_guidance: policy_decision
                .as_ref()
                .ok()
                .map(|decision| decision.system_prompt_patch.clone())
                .unwrap_or_default(),
            tool_bridge: deferred_catalog,
        };
        let (runtime_context, context_budget_allocation) = build_budgeted_mainbrain_runtime_context(
            runtime_context_parts,
            &config,
            &model_input,
            tool_schema_tokens,
        );
        let session_handoff = self
            .memory
            .get_session_summary(&session_id.0)
            .await
            .map(|summary| SessionHandoffSummary {
                summary: summary.summary,
                key_decisions: summary.key_decisions,
                files_modified: summary.files_modified,
                tools_used: summary.tools_used,
            })
            .filter(|handoff| !handoff.is_empty());
        let protocol_history = self.conversation_protocol_messages(&session_id).await;
        let packed_session = pack_session_context(PackSessionContextInput {
            messages: &protocol_history,
            handoff: session_handoff.as_ref(),
            current_user_input: &model_input,
            max_input_tokens: context_budget_allocation.max_input_tokens,
            force_compacted_layout: session_handoff.is_some(),
        });
        let packed_history = packed_session.messages;
        match &policy_decision {
            Ok(decision) => {
                self.runs
                    .record_event(
                        &run.id,
                        "agent.policy.plan",
                        Some(json!({
                            "intent": decision.intent,
                            "responseStyle": decision.response_style,
                            "modelPreference": decision.model_preference,
                            "contextHints": decision.context_hints,
                            "toolPlan": decision.tool_plan,
                            "toolPlanReport": policy_tool_report.summary_payload(),
                            "memoryWriteCandidates": decision.memory_write_candidates,
                            "safetyNotes": decision.safety_notes,
                            "evidence": decision.evidence
                        })),
                        Some(model_step.id.clone()),
                    )
                    .await?;
                self.enqueue_policy_memory_candidates(
                    &session_id,
                    &run.id,
                    &decision.memory_write_candidates,
                );
            }
            Err(error) => {
                self.runs
                    .record_event(
                        &run.id,
                        "agent.policy.fallback",
                        Some(json!({ "warning": error })),
                        Some(model_step.id.clone()),
                    )
                    .await?;
            }
        }
        self.runs
            .record_event(
                &run.id,
                "context.policy.plan",
                Some(json!({
                    "intent": main_brain_intent,
                    "loadMode": turn_plan.load_mode,
                    "policy": context_policy,
                    "surface": context_surface_snapshot(&context_policy),
                    "activeSections": active_context_sections(&context_policy),
                    "activeToolNames": active_tool_names.iter().cloned().collect::<Vec<_>>(),
                    "keyFacts": key_facts_from_messages(&turn_history, 24),
                    "contextBudget": context_budget_allocation
                })),
                Some(model_step.id.clone()),
            )
            .await?;
        let max_iterations = max_iterations_from_config(&config);
        let cwd = self
            .resolve_workspace_dir(Some(&session_id.0))
            .await;
        // 轮间工具回传不能只用静态 section 的 7%（tool_results），否则长任务早期结果被裁掉、模型反复探目录。
        let max_transcript_tokens = ((context_budget_allocation.max_input_tokens as f64) * 0.40)
            .floor()
            .max(8_192.0) as u64;
        let turn_started_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let loop_host = MainTurnLoopHostAdapter {
            core: self.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            run_id: run.id.clone(),
            model_step_id: model_step.id.clone(),
            cwd,
            turn_started_ms,
            main_brain_intent: main_brain_intent.to_string(),
            context_policy: context_policy.clone(),
            context_budget_allocation: context_budget_allocation.clone(),
            tool_catalog,
            tool_definitions,
            deep_thinking,
            attachments: attachments.clone(),
            config: config.clone(),
            model_input: model_input.clone(),
            policy_tool_report: policy_tool_report.clone(),
            policy_decision: policy_decision.clone(),
            key_facts_context: key_facts_context.clone(),
            packed_history,
            mutable: tokio::sync::Mutex::new(MainTurnLoopHostMut {
                context_surface,
                turn_memory_cache,
                memory_context,
                runtime_context,
                active_tool_names,
                writes_since_mid_validation: 0,
                turn_usage: None,
                mid_turn_messages: Vec::new(),
            }),
            speculative: tokio::sync::Mutex::new(None),
        };
        let mut loop_state = MainTurnLoopState::default();
        let loop_result = run_main_turn_loop(
            &loop_host,
            MainTurnLoopConfig {
                max_iterations,
                model_input: model_input.clone(),
                max_transcript_tokens,
                deadline_ms: turn_deadline_ms_from_config(&config, turn_started_ms),
            },
            &mut loop_state,
        )
        .await?;
        if loop_result.cancelled {
            return Ok(());
        }
        let final_text = loop_result.final_text;
        let final_provider = loop_result.final_provider;
        let final_model = loop_result.final_model;
        let final_error = loop_result.final_error;
        let completed_iterations = loop_result.completed_iterations;
        let runtime_tool_count = loop_result.runtime_tool_count;
        let response_streamed = loop_result.response_streamed;
        if !response_streamed {
            self.publish_turn_text_deltas(&session_id, &turn_id, &final_text)
                .await;
        }
        // 本轮合并后的用量随最终 assistant 消息落库，重开会话后仍能展示 token 用量。
        // turn_usage 累计在回合适配器的 mutable 上（on_model_response 里 merge），不是 AppCore 字段。
        let turn_usage = loop_host
            .mutable
            .lock()
            .await
            .turn_usage
            .clone()
            .filter(|usage| !usage.is_empty());
        let mut assistant_metadata = if proactive {
            BTreeMap::from([("origin".to_string(), json!("proactive"))])
        } else {
            BTreeMap::new()
        };
        if let Some(usage) = turn_usage.as_ref() {
            if let Ok(value) = serde_json::to_value(usage) {
                assistant_metadata.insert("usage".to_string(), value);
            }
        }
        self.sessions
            .append_assistant_message_with_metadata(
                &session_id,
                &turn_id,
                final_text.clone(),
                assistant_metadata,
                true,
            )
            .await?;
        let run_payload = BTreeMap::from([
            ("turnId".to_string(), json!(turn_id.0.clone())),
            ("provider".to_string(), json!(final_provider)),
            ("model".to_string(), json!(final_model)),
            ("iterations".to_string(), json!(completed_iterations)),
            (
                "toolCallCount".to_string(),
                json!(policy_tool_report.executed + runtime_tool_count),
            ),
        ]);
        let turn_failed = final_error.is_some();
        if let Some(error) = final_error {
            self.runs.fail_step(&model_step.id, error.clone()).await?;
            self.runs.fail_run(&run.id, error).await?;
        } else {
            self.runs
                .complete_step(&model_step.id, run_payload.clone())
                .await?;
            self.runs.complete_run(&run.id, run_payload).await?;
            // Turn done: replay typecheck/lint/test already executed this turn.
            self.run_turn_end_auto_validations(
                &session_id,
                &turn_id,
                &run.id,
                &loop_host.cwd,
                turn_started_ms,
                turn_usage.as_ref(),
            )
            .await;
        }
        if !turn_failed && !proactive && !model_input.trim().is_empty() {
            let core = self.clone();
            let title_session = session_id.clone();
            let title_seed = model_input.clone();
            tokio::spawn(async move {
                core.maybe_auto_title_session(title_session, title_seed)
                    .await;
            });
        }
        let host_state = loop_host.mutable.lock().await;
        self.persist_turn_lazy_context(
            &session_id,
            &turn_id,
            &strip_refresh_sections(&context_policy),
            &host_state.active_tool_names,
            main_brain_intent,
            &host_state.context_surface,
        )
        .await;
        Ok(())
    }

    async fn apply_mainbrain_tool_dispatch(
        &self,
        input: MainBrainToolApplyInput<'_>,
    ) -> anyhow::Result<Option<String>> {
        let MainBrainToolApplyInput {
            context_surface,
            turn_memory_cache,
            memory_context,
            runtime_context,
            tool_transcript,
            active_tool_names,
            loop_guard,
            guard_inputs,
            model_output,
            context_budget_allocation,
            session_id,
            model_input,
            config,
            dispatch_result,
        } = input;
        let load_result = context_surface.load(&dispatch_result.loaded_context_sections);
        turn_memory_cache.refresh_after_context_sections_loaded(&load_result.loaded);
        if sections_need_memory_cache_invalidate(&load_result.loaded) {
            *memory_context = self
                .retrieved_memory_context(session_id, model_input, config)
                .await;
            if let Some(memory) = memory_context.as_ref() {
                xuanyu_execution::patch_retrieved_memory_context(runtime_context, memory);
            }
        }
        let mapped = MainTurnToolDispatchResult {
            transcript_entries: dispatch_result.transcript_entries,
            tool_results: dispatch_result.tool_results,
            loaded_tool_names: dispatch_result.loaded_tool_names,
            context_patches: dispatch_result.context_patches,
            loaded_context_sections: dispatch_result.loaded_context_sections,
        };
        Ok(xuanyu_execution::finalize_main_turn_tool_dispatch(
            xuanyu_execution::FinalizeMainTurnToolDispatchInput {
                runtime_context,
                tool_transcript,
                active_tool_names,
                loop_guard,
                guard_inputs,
                model_output,
                context_budget_allocation,
                dispatch_result: mapped,
            },
        ))
    }

    async fn dispatch_mainbrain_tool_calls(
        &self,
        input: MainBrainToolDispatchInput<'_>,
    ) -> anyhow::Result<MainBrainToolDispatchResult> {
        let MainBrainToolDispatchInput {
            session_id,
            turn_id,
            run_id,
            step_id,
            cwd,
            tool_calls,
            iteration,
            source,
            speculative,
        } = input;
        if tool_calls.is_empty() {
            return Ok(MainBrainToolDispatchResult::default());
        }
        let mut transcript_entries = Vec::with_capacity(tool_calls.len());
        let mut tool_results = Vec::with_capacity(tool_calls.len());
        let mut loaded_tool_names = Vec::new();
        let mut context_patches = Vec::new();
        let mut loaded_context_sections = Vec::new();
        let mut write_tool_names = Vec::new();
        let mut affected_files = Vec::new();
        for tool_call in tool_calls {
            let mut args = tool_call.args.clone();
            args.entry("_sessionId".to_string())
                .or_insert_with(|| json!(session_id.0.clone()));
            // One assistant per turn; tool calls append to turn_segments.
            if let Err(error) = self
                .sessions
                .record_tool_call_segment(
                    session_id,
                    turn_id,
                    &tool_call.id,
                    &tool_call.name,
                    &json!(args),
                )
                .await
            {
                tracing::warn!(
                    session_id = %session_id.0,
                    tool_call_id = %tool_call.id,
                    error = %error,
                    "record_tool_call_segment failed"
                );
            }
            self.runs
                .record_event(
                    run_id,
                    "agent.tool.requested",
                    Some(json!({
                        "source": source,
                        "iteration": iteration,
                        "toolCallId": tool_call.id,
                        "tool": tool_call.name,
                        "args": args
                    })),
                    Some(step_id.to_string()),
                )
                .await?;
            self.eventbus.publish(AppEvent::AgentToolRequested {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                args: json!(args),
            });
            let call = ToolCall {
                name: tool_call.name.clone(),
                args: args.clone(),
                cwd: cwd.to_path_buf(),
            };
            let speculative = speculative.clone();
            let mut output = if call.name == "ask_user" {
                self.execute_ask_user(
                    Some(session_id.0.clone()),
                    Some(run_id.to_string()),
                    &call,
                )
                .await
            } else if let Some(batch) = speculative.as_ref() {
                if let Some(prefetched) = batch.take(&tool_call.id).await {
                    prefetched
                } else {
                    let mut output = self.tools.execute(call.clone()).await;
                    output = self
                        .capture_pending_tool_approval(
                            Some(session_id.0.clone()),
                            call.clone(),
                            output,
                        )
                        .await;
                    output
                }
            } else {
                let mut output = self.tools.execute(call.clone()).await;
                output = self
                    .capture_pending_tool_approval(Some(session_id.0.clone()), call.clone(), output)
                    .await;
                output
            };
            // Speculative path still needs approval capture when result asks for it.
            if call.name != "ask_user" {
                output = self
                    .capture_pending_tool_approval(Some(session_id.0.clone()), call.clone(), output)
                    .await;
            }
            self.spawn_subagent_tasks_from_tool_output(&call.name, &output);
            if call.name == "load_tools" {
                loaded_tool_names.extend(loaded_tool_names_from_output(&output));
            }
            if call.name == "load_memories" {
                if let Some(patch) = context_patch_from_load_memories_output(&output) {
                    context_patches.push(patch);
                }
                loaded_context_sections.extend(sections_from_load_memories_output(&output));
            }
            if output.is_error
                && call.name != "ask_user"
                && approval_id_from_tool_output(&output).is_none()
                && should_retry_mainbrain_tool_result(&output.content)
            {
                self.runs
                    .record_event(
                        run_id,
                        "agent.tool.retry",
                        Some(json!({
                            "source": source,
                            "iteration": iteration,
                            "toolCallId": tool_call.id,
                            "tool": call.name
                        })),
                        Some(step_id.to_string()),
                    )
                    .await?;
                let retry_output = self.tools.execute(call.clone()).await;
                let retry_output = self
                    .capture_pending_tool_approval(
                        Some(session_id.0.clone()),
                        call.clone(),
                        retry_output,
                    )
                    .await;
                if retry_output.is_error {
                    output.content.push_str("\n\nRetry failed: ");
                    output.content.push_str(&retry_output.content);
                } else {
                    output = ToolOutput {
                        content: format!("[Recovered after retry]\n{}", retry_output.content),
                        is_error: false,
                        data: retry_output.data,
                    };
                }
            }
            self.runs
                .record_event(
                    run_id,
                    "agent.tool.result",
                    Some(json!({
                        "source": source,
                        "iteration": iteration,
                        "toolCallId": tool_call.id,
                        "tool": call.name,
                        "isError": output.is_error,
                        "content": output.content,
                        "data": output.data
                    })),
                    Some(step_id.to_string()),
                )
                .await?;
            self.eventbus.publish(AppEvent::AgentToolResult {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                tool_call_id: tool_call.id.clone(),
                content: output.content.clone(),
                is_error: output.is_error,
            });
            if let Err(error) = self
                .sessions
                .complete_tool_call_segment(
                    session_id,
                    turn_id,
                    &tool_call.id,
                    &tool_call.name,
                    &output.content,
                    output.is_error,
                )
                .await
            {
                tracing::warn!(
                    session_id = %session_id.0,
                    tool_call_id = %tool_call.id,
                    error = %error,
                    "complete_tool_call_segment failed"
                );
            }
            self.sessions
                .append_tool_message(
                    session_id,
                    turn_id,
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                    output.content.clone(),
                    BTreeMap::from([
                        ("toolName".to_string(), json!(tool_call.name)),
                        ("isError".to_string(), json!(output.is_error)),
                        (
                            "data".to_string(),
                            output.data.clone().unwrap_or(Value::Null),
                        ),
                    ]),
                )
                .await?;
            transcript_entries.push(mainbrain_tool_transcript_entry(
                &tool_call, &output, iteration,
            ));
            tool_results.push(MainTurnToolResultMessage {
                call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                arguments_json: serde_json::to_string(&tool_call.args)
                    .unwrap_or_else(|_| "{}".to_string()),
                content: output.content.clone(),
            });
            if !output.is_error && is_mid_turn_write_tool(&call.name) {
                write_tool_names.push(call.name.clone());
                affected_files.extend(affected_files_from_tool_output(&output));
            }
        }
        Ok(MainBrainToolDispatchResult {
            transcript_entries,
            tool_results,
            loaded_tool_names,
            context_patches,
            loaded_context_sections,
            write_tool_names,
            affected_files,
        })
    }

    async fn run_mid_turn_write_validation(
        &self,
        cwd: &Path,
        turn_started_ms: u64,
        tool_names: &[String],
        affected_files: &[String],
        run_id: &str,
    ) -> Option<String> {
        if tool_names.is_empty() && affected_files.is_empty() {
            return None;
        }
        let mut args = BTreeMap::new();
        args.insert("sinceMs".to_string(), json!(turn_started_ms));
        args.insert("maxCommands".to_string(), json!(2));
        args.insert("timeoutMs".to_string(), json!(90_000));
        args.insert("stopOnFailure".to_string(), json!(true));
        args.insert("includeBuild".to_string(), json!(false));
        let output = self
            .tools
            .execute(ToolCall {
                name: "validation_replay".to_string(),
                args,
                cwd: cwd.to_path_buf(),
            })
            .await;
        let failed = output
            .data
            .as_ref()
            .and_then(|data| data.get("failed"))
            .and_then(Value::as_bool)
            .unwrap_or(output.is_error);
        let ran = output
            .data
            .as_ref()
            .and_then(|data| data.get("ran"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !ran || !failed {
            return None;
        }
        let _ = self
            .runs
            .record_event(
                run_id,
                "trace.validation.failure",
                Some(json!({
                    "failed": true,
                    "midTurn": true,
                    "toolNames": tool_names,
                    "affectedFiles": affected_files.iter().take(20).cloned().collect::<Vec<_>>(),
                    "summary": output.content.chars().take(1000).collect::<String>(),
                })),
                None,
            )
            .await;
        let files = if affected_files.is_empty() {
            "(unlisted)".to_string()
        } else {
            affected_files
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some(format!(
            "Mid-turn validation failed after recent writes. Fix the failures before more edits.\nChanged files: {files}\n\n{}",
            output.content
        ))
    }

    async fn run_turn_end_auto_validations(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        run_id: &str,
        cwd: &Path,
        turn_started_ms: u64,
        turn_usage: Option<&xuanyu_execution::TokenUsage>,
    ) {
        let mut args = BTreeMap::new();
        args.insert("sinceMs".to_string(), json!(turn_started_ms));
        args.insert("maxCommands".to_string(), json!(4));
        args.insert("timeoutMs".to_string(), json!(90_000));
        args.insert("stopOnFailure".to_string(), json!(true));
        args.insert("includeBuild".to_string(), json!(false));
        let output = self
            .tools
            .execute(ToolCall {
                name: "validation_replay".to_string(),
                args,
                cwd: cwd.to_path_buf(),
            })
            .await;
        let ran = output
            .data
            .as_ref()
            .and_then(|data| data.get("ran"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !ran {
            return;
        }
        let failed = output
            .data
            .as_ref()
            .and_then(|data| data.get("failed"))
            .and_then(Value::as_bool)
            .unwrap_or(output.is_error);
        let _ = self
            .runs
            .record_event(
                run_id,
                "trace.validation.auto",
                Some(json!({
                    "failed": failed,
                    "summary": output.content.chars().take(2000).collect::<String>(),
                    "data": output.data.clone(),
                })),
                None,
            )
            .await;
        if failed {
            let mut metadata =
                BTreeMap::from([("origin".to_string(), json!("auto_validation"))]);
            if let Some(usage) = turn_usage {
                if let Ok(value) = serde_json::to_value(usage) {
                    metadata.insert("usage".to_string(), value);
                }
            }
            let _ = self
                .sessions
                .append_assistant_message_with_metadata(
                    session_id,
                    turn_id,
                    format!("Auto-validation after turn:\n{}", output.content),
                    metadata,
                    false,
                )
                .await;
        }
    }

    async fn publish_team_agent_event(
        &self,
        kind: &str,
        ledger: &SubAgentTaskLedger,
    ) {
        publish_team_agent_event(
            &self.eventbus,
            self.agents.as_ref(),
            kind,
            ledger,
        )
        .await;
    }

    async fn take_lazy_context_override(&self, session_id: &SessionId) -> Option<LazyContextOverride> {
        self.lazy_context_overrides
            .write()
            .await
            .remove(&session_id.0)
    }

    async fn set_lazy_context_override(
        &self,
        session_id: &SessionId,
        override_state: LazyContextOverride,
    ) {
        self.lazy_context_overrides
            .write()
            .await
            .insert(session_id.0.clone(), override_state);
    }

    async fn persist_turn_lazy_context(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        policy: &Value,
        active_tool_names: &BTreeSet<String>,
        intent: &str,
        context_surface: &ContextSurface,
    ) {
        let store = self.storage.conversation_store();
        let snapshots = store
            .list_snapshots(&session_id.0)
            .await
            .unwrap_or_default();
        let snapshot_id = format!("turn_cp_{}", turn_id.0);
        let Some(mut snapshot) = snapshots.into_iter().find(|item| item.id == snapshot_id) else {
            return;
        };
        let lazy_context = json!({
            "contextLoadPolicy": policy,
            "activeSections": context_surface.get_active_sections(),
            "activeToolNames": active_tool_names.iter().cloned().collect::<Vec<_>>(),
            "mainBrainIntent": intent,
        });
        if let Some(object) = snapshot.conversation.as_object_mut() {
            object.insert("lazyContext".to_string(), lazy_context);
        } else {
            snapshot.conversation = json!({ "lazyContext": lazy_context });
        }
        let _ = store.add_snapshot(snapshot).await;
    }

    async fn maybe_auto_title_session(&self, session_id: SessionId, seed: String) {
        let Some(session) = self
            .list_sessions()
            .await
            .into_iter()
            .find(|item| item.id == session_id)
        else {
            return;
        };
        if !should_auto_title(&session.name, session.title_generated_by.as_deref()) {
            return;
        }
        let seed = seed.trim().to_string();
        if seed.is_empty() {
            return;
        }
        {
            let in_flight = self.title_generation.read().await;
            if in_flight.contains_key(&session_id.0) {
                return;
            }
        }
        let gate = Arc::new(tokio::sync::Notify::new());
        self.title_generation
            .write()
            .await
            .insert(session_id.0.clone(), gate.clone());
        self.publish_session_title_status(
            &session_id,
            "queued",
            None,
            None,
            None,
            None,
        )
        .await;
        self.publish_session_title_status(
            &session_id,
            "running",
            None,
            None,
            None,
            None,
        )
        .await;
        let title_result = self.generate_session_title(&seed).await;
        match title_result {
            Ok(result) if should_auto_title(&session.name, session.title_generated_by.as_deref()) => {
                if let Some(title) = result.title.filter(|value| value != &session.name) {
                    let renamed = self
                        .sessions
                        .rename_session(&session_id, title.clone())
                        .await
                        .ok()
                        .flatten();
                    let _ = self
                        .storage
                        .conversation_store()
                        .update_session(
                            &session_id.0,
                            xuanyu_storage::ConversationSessionPatch {
                                metadata: Some(BTreeMap::from([
                                    (
                                        "titleStatus".to_string(),
                                        json!("succeeded"),
                                    ),
                                    (
                                        "titleGeneratedBy".to_string(),
                                        json!(result.source),
                                    ),
                                    (
                                        "titleGeneratedAt".to_string(),
                                        json!(Utc::now().timestamp_millis()),
                                    ),
                                ])),
                                ..Default::default()
                            },
                        )
                        .await;
                    // Reference broadcasts session.renamed after auto-title; SessionUpdated
                    // already fired from rename_session — re-publish with latest metadata fields.
                    if let Some(mut info) = renamed {
                        info.title_status = Some("succeeded".to_string());
                        info.title_generated_by = Some(result.source.clone());
                        info.title_provider = Some(result.provider.clone());
                        info.title_model = Some(result.model.clone());
                        self.eventbus.publish(AppEvent::SessionUpdated { session: info });
                    }
                    self.publish_session_title_status(
                        &session_id,
                        "succeeded",
                        Some(result.source),
                        result.error,
                        Some(result.provider),
                        Some(result.model),
                    )
                    .await;
                } else {
                    self.publish_session_title_status(
                        &session_id,
                        "failed",
                        Some(result.source),
                        result
                            .error
                            .or_else(|| Some("title generation returned empty text".to_string())),
                        Some(result.provider),
                        Some(result.model),
                    )
                    .await;
                }
            }
            Ok(result) => {
                self.publish_session_title_status(
                    &session_id,
                    "failed",
                    Some(result.source),
                    result.error,
                    Some(result.provider),
                    Some(result.model),
                )
                .await;
            }
            Err(error) => {
                self.publish_session_title_status(
                    &session_id,
                    "failed",
                    Some("fallback".to_string()),
                    Some(error.to_string()),
                    None,
                    None,
                )
                .await;
            }
        }
        self.title_generation.write().await.remove(&session_id.0);
    }

    async fn publish_session_title_status(
        &self,
        session_id: &SessionId,
        status: &str,
        generated_by: Option<String>,
        error: Option<String>,
        provider: Option<String>,
        model: Option<String>,
    ) {
        let _ = self
            .storage
            .conversation_store()
            .update_session(
                &session_id.0,
                xuanyu_storage::ConversationSessionPatch {
                    metadata: Some(BTreeMap::from([
                        ("titleStatus".to_string(), json!(status)),
                        (
                            "titleGeneratedBy".to_string(),
                            json!(generated_by.clone().unwrap_or_default()),
                        ),
                        (
                            "titleGenerationError".to_string(),
                            json!(error.clone().unwrap_or_default()),
                        ),
                        (
                            "titleProvider".to_string(),
                            json!(provider.clone().unwrap_or_default()),
                        ),
                        (
                            "titleModel".to_string(),
                            json!(model.clone().unwrap_or_default()),
                        ),
                    ])),
                    ..Default::default()
                },
            )
            .await;
        self.eventbus.publish(AppEvent::SessionTitleStatus {
            session_id: session_id.clone(),
            status: status.to_string(),
            generated_by,
            error,
            provider,
            model,
        });
    }

    async fn generate_session_title(&self, seed: &str) -> anyhow::Result<GeneratedSessionTitle> {
        let route = self.execution.resolve_route(ModelTaskKind::Title).await;
        let provider = route.provider.provider_type.clone();
        let model = route.provider.model.clone();
        let response = self
            .execution
            .generate_for_task(
                ModelTaskKind::Title,
                seed,
                Some(AUX_TITLE_SYSTEM_PROMPT),
            )
            .await;
        if let Some(error) = response.error {
            let fallback = generate_fallback_session_title(seed);
            return Ok(GeneratedSessionTitle {
                title: fallback,
                source: "fallback".to_string(),
                error: Some(error),
                provider,
                model,
            });
        }
        let mut title = sanitize_model_session_title(&response.text);
        if title.is_none() {
            title = response
                .reasoning
                .as_deref()
                .and_then(extract_title_from_reasoning);
        }
        if title.is_none() {
            title = generate_fallback_session_title(seed);
        }
        Ok(GeneratedSessionTitle {
            title,
            source: if response.text.trim().is_empty() {
                "fallback".to_string()
            } else {
                "model".to_string()
            },
            error: if response.text.trim().is_empty() {
                Some("stream returned empty title text".to_string())
            } else {
                None
            },
            provider,
            model,
        })
    }

    async fn turn_has_finished_assistant(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> bool {
        // 工具开始就会 ensure 空 assistant；不能仅凭「存在 assistant 行」判定回合结束。
        if self
            .sessions
            .turn_persist_active(&session_id.0, &turn_id.0)
            .await
        {
            return false;
        }
        let history = self.sessions.history(session_id).await.unwrap_or_default();
        history.iter().any(|message| {
            message.role == MessageRole::Assistant && message.turn_id.as_ref() == Some(turn_id)
        })
    }

    async fn wait_for_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        if self.turn_has_finished_assistant(session_id, turn_id).await {
            return Ok(());
        }
        let mut rx = self.eventbus.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow::anyhow!(
                    "timed out waiting for turn {}",
                    turn_id.0
                ));
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(envelope)) => match envelope.payload {
                    AppEvent::TurnFinished {
                        session_id: sid,
                        turn_id: tid,
                    } if sid == *session_id && tid == *turn_id => return Ok(()),
                    AppEvent::TurnFailed {
                        session_id: sid,
                        turn_id: tid,
                        error,
                    } if sid == *session_id && tid == *turn_id => {
                        return Err(anyhow::anyhow!("{}", error.message));
                    }
                    _ => {}
                },
                Ok(Err(_)) => continue,
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "timed out waiting for turn {}",
                        turn_id.0
                    ));
                }
            }
        }
    }

    async fn publish_turn_text_deltas(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        text: &str,
    ) {
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let chunk_size = if text.is_ascii() { 24 } else { 6 };
        for chunk in chars.chunks(chunk_size) {
            let content: String = chunk.iter().collect();
            self.eventbus.publish(AppEvent::TurnDelta {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                content,
            });
            tokio::task::yield_now().await;
        }
    }

    async fn plan_main_turn_policy(
        &self,
        session_id: &SessionId,
        model_input: &str,
        memory_context: Option<&str>,
        config: &Value,
    ) -> Result<AgentPolicyDecision, String> {
        let history = self.sessions.history(session_id).await.unwrap_or_default();
        let mut recent_messages = history.iter().rev().take(8).collect::<Vec<_>>();
        recent_messages.reverse();
        let recent_messages = recent_messages
            .into_iter()
            .map(|message| AgentPolicyMessage {
                role: policy_message_role(message.role).to_string(),
                content: compact_context_line(&message.content, 1_200),
            })
            .collect::<Vec<_>>();
        let (input_tokens, output_tokens) = split_token_estimate(&history);
        let max_context_tokens = config.get("maxContextTokens").and_then(Value::as_u64);
        let request = AgentPolicyRequest {
            session_id: Some(session_id.0.clone()),
            mode: Some("agent".to_string()),
            user_input: model_input.to_string(),
            recent_messages,
            available_tools: self
                .tools
                .catalog()
                .await
                .into_iter()
                .map(|tool| tool.name)
                .collect(),
            memory_hits: policy_memory_hits_from_context(memory_context),
            constraints: policy_constraints_from_config(config),
            budget: Some(AgentPolicyBudget {
                max_context_tokens,
                remaining_tokens: max_context_tokens
                    .map(|max| max.saturating_sub(input_tokens + output_tokens)),
                max_turns: policy_max_turns_from_config(config),
            }),
        };
        self.agent_bridge
            .plan_turn(request)
            .map_err(|err| err.to_string())
    }

    async fn execute_mainbrain_policy_tool_plan(
        &self,
        session_id: &SessionId,
        run_id: &str,
        step_id: &str,
        requests: &[SubAgentToolRequest],
    ) -> anyhow::Result<MainBrainToolPlanReport> {
        let mut report = MainBrainToolPlanReport::default();
        if requests.is_empty() {
            return Ok(report);
        }
        let cwd = self
            .resolve_workspace_dir(Some(&session_id.0))
            .await;
        let catalog = self.tools.catalog().await;
        let mut seen = BTreeMap::<String, ()>::new();
        for request in requests.iter().take(8) {
            let requested_name = request.name.trim();
            if requested_name.is_empty() {
                continue;
            }
            let mapped_name = canonical_mainbrain_policy_tool_name(requested_name);
            let Some(entry) = tool_catalog_entry(&catalog, mapped_name) else {
                report.skipped += 1;
                report.risks.push(format!(
                    "Skipped unknown main-brain policy tool requested by worker: {requested_name}"
                ));
                self.runs
                    .record_event(
                        run_id,
                        "agent.tool.skipped",
                        Some(json!({
                            "source": "mainbrain.policy",
                            "tool": requested_name,
                            "reason": "unknown_tool"
                        })),
                        Some(step_id.to_string()),
                    )
                    .await?;
                continue;
            };
            let canonical_tool_name = entry.name.clone();
            if seen
                .insert(
                    format!("{}:{}", canonical_tool_name, json!(request.args)),
                    (),
                )
                .is_some()
            {
                continue;
            }
            if !is_mainbrain_policy_premodel_tool(entry) {
                report.skipped += 1;
                report.risks.push(format!(
                    "Skipped main-brain policy tool {}; pre-model policy plans only execute read/context tools.",
                    canonical_tool_name
                ));
                self.runs
                    .record_event(
                        run_id,
                        "agent.tool.skipped",
                        Some(json!({
                            "source": "mainbrain.policy",
                            "tool": canonical_tool_name,
                            "reason": "not_premodel_read_tool"
                        })),
                        Some(step_id.to_string()),
                    )
                    .await?;
                continue;
            }
            let mut args =
                sanitize_mainbrain_policy_tool_args(&canonical_tool_name, request.args.clone());
            args.entry("_sessionId".to_string())
                .or_insert_with(|| json!(session_id.0.clone()));
            self.runs
                .record_event(
                    run_id,
                    "agent.tool.requested",
                    Some(json!({
                        "source": "mainbrain.policy",
                        "tool": canonical_tool_name,
                        "args": args,
                        "reason": request.reason
                    })),
                    Some(step_id.to_string()),
                )
                .await?;
            let call = ToolCall {
                name: canonical_tool_name.clone(),
                args,
                cwd: cwd.clone(),
            };
            let output = self.tools.execute(call.clone()).await;
            let output = self
                .capture_pending_tool_approval(Some(session_id.0.clone()), call, output)
                .await;
            self.spawn_subagent_tasks_from_tool_output(&canonical_tool_name, &output);
            if output.is_error {
                report.failed += 1;
                report.risks.push(format!(
                    "{} failed: {}",
                    canonical_tool_name,
                    compact_context_line(&output.content, 220)
                ));
            } else {
                report.executed += 1;
                report.evidence.push(format!(
                    "{}: {}",
                    canonical_tool_name,
                    compact_context_line(&output.content, 220)
                ));
                report.context_lines.push(format!(
                    "[{}]\n{}",
                    canonical_tool_name,
                    compact_context_line(&output.content, 900)
                ));
            }
            if let Some(approval_id) = approval_id_from_tool_output(&output) {
                report.approval_requests.push(approval_id);
            }
            self.runs
                .record_event(
                    run_id,
                    "agent.tool.result",
                    Some(json!({
                        "source": "mainbrain.policy",
                        "tool": canonical_tool_name,
                        "isError": output.is_error,
                        "content": output.content,
                        "data": output.data
                    })),
                    Some(step_id.to_string()),
                )
                .await?;
        }
        self.runs
            .record_event(
                run_id,
                "agent.policy.tools",
                Some(report.summary_payload()),
                Some(step_id.to_string()),
            )
            .await?;
        Ok(report)
    }

    pub fn execution(&self) -> Arc<ExecutionRuntime> {
        self.execution.clone()
    }

    pub fn governance(&self) -> Arc<GovernanceEngine> {
        self.governance.clone()
    }

    pub fn tools(&self) -> Arc<ToolRuntime> {
        self.tools.clone()
    }

    pub async fn tool_catalog(&self) -> Vec<ToolCatalogEntry> {
        self.tools.catalog().await
    }

    pub async fn tool_execute(&self, input: ToolExecuteInput) -> ToolOutput {
        let cwd = input.cwd.unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let mut args = input.args;
        if let Some(session_id) = &input.session_id {
            args.entry("_sessionId".to_string())
                .or_insert_with(|| json!(session_id));
        }
        let call = ToolCall {
            name: input.name,
            args,
            cwd,
        };
        let output = self.tools.execute(call.clone()).await;
        let output = self
            .capture_pending_tool_approval(input.session_id, call.clone(), output)
            .await;
        self.spawn_subagent_tasks_from_tool_output(&call.name, &output);
        output
    }

    pub async fn permission_list(&self, session_id: Option<String>) -> Value {
        if session_id.is_none() {
            return self.sessions.list_pending_permissions(self).await;
        }
        let pending = self.pending_tool_approvals.read().await;
        let permissions = pending
            .values()
            .filter(|item| {
                session_id
                    .as_ref()
                    .map(|session_id| item.session_id.as_deref() == Some(session_id.as_str()))
                    .unwrap_or(true)
            })
            .map(PendingToolApproval::to_dto)
            .collect::<Vec<_>>();
        json!({ "permissions": permissions })
    }

    pub async fn permission_respond(
        &self,
        request_id: String,
        allowed: bool,
        remember: bool,
    ) -> Value {
        self.sessions
            .permission_respond(request_id, allowed, remember, self)
            .await
    }

    pub async fn permission_respond_with_scope(
        &self,
        request_id: String,
        allowed: bool,
        scope: Option<String>,
    ) -> Value {
        self.sessions
            .permission_respond_with_scope(request_id, allowed, scope, self)
            .await
    }

    async fn permission_respond_with_scope_inner(
        &self,
        request_id: String,
        allowed: bool,
        scope: Option<String>,
    ) -> Value {
        let Some(pending) = self
            .pending_tool_approvals
            .write()
            .await
            .remove(&request_id)
        else {
            return json!({
                "ok": false,
                "requestId": request_id,
                "allowed": false,
                "remember": false,
                "error": "permission request not found"
            });
        };
        if !allowed {
            return json!({
                "ok": true,
                "requestId": request_id,
                "sessionId": pending.session_id,
                "toolName": pending.tool_name,
                "allowed": false,
                "remember": false
            });
        }
        let response_scope = normalize_permission_response_scope(scope.as_deref());
        let mut remember_error: Option<String> = None;
        if response_scope == "always" {
            if let Err(err) = self.allow_tool_always(&pending.tool_name).await {
                remember_error = Some(err.to_string());
            }
        }
        if response_scope == "always" || response_scope == "session" {
            if let Some(session_id) = &pending.session_id {
                self.governance
                    .allow_tool_for_session(session_id, &pending.tool_name)
                    .await;
            }
        }
        let response_session_id = pending.session_id.clone();
        let response_tool_name = pending.tool_name.clone();
        let output = self
            .tools
            .execute_approved(
                ToolCall {
                    name: pending.tool_name,
                    args: pending.args,
                    cwd: pending.cwd,
                },
                request_id.clone(),
            )
            .await;
        json!({
            "ok": !output.is_error,
            "requestId": request_id,
            "sessionId": response_session_id,
            "toolName": response_tool_name,
            "allowed": true,
            "remember": response_scope != "once",
            "scope": response_scope,
            "rememberError": remember_error,
            "output": output
        })
    }

    async fn allow_tool_always(&self, tool_name: &str) -> anyhow::Result<Value> {
        let tool_name = tool_name.trim();
        if tool_name.is_empty() {
            anyhow::bail!("tool name is empty");
        }
        let mut current = self.config_get().await?;
        let root = ensure_json_object(&mut current);
        let permissions = root.entry("permissions").or_insert_with(|| json!({}));
        let permissions = ensure_json_object(permissions);
        let whitelist = permissions.entry("whitelist").or_insert_with(|| json!([]));
        if !whitelist.is_array() {
            *whitelist = json!([]);
        }
        let items = whitelist.as_array_mut().expect("whitelist array");
        if !items.iter().any(|item| item.as_str() == Some(tool_name)) {
            items.push(json!(tool_name));
        }
        let saved = self.storage.save_app_config(current).await?;
        self.apply_execution_config(&saved).await;
        self.observability
            .log(
                "info",
                format!("Permission whitelist updated for tool: {tool_name}"),
            )
            .await;
        Ok(saved)
    }

    async fn capture_pending_tool_approval(
        &self,
        session_id: Option<String>,
        call: ToolCall,
        mut output: ToolOutput,
    ) -> ToolOutput {
        let Some(request_id) = approval_id_from_tool_output(&output) else {
            return output;
        };
        let reason = output
            .content
            .strip_prefix("Approval required: ")
            .unwrap_or(&output.content)
            .to_string();
        let risk = output
            .data
            .as_ref()
            .and_then(|data| data.get("risk").cloned());
        let pending = PendingToolApproval {
            request_id: request_id.clone(),
            session_id,
            tool_name: call.name,
            args: call.args,
            cwd: call.cwd,
            reason,
            risk,
            created_at: Utc::now().timestamp_millis(),
        };
        let dto = pending.to_dto();
        self.pending_tool_approvals
            .write()
            .await
            .insert(request_id, pending);
        output.data = Some(merge_tool_output_data(
            output.data.take(),
            json!({ "pendingPermission": dto }),
        ));
        output
    }

    pub async fn extensions_list(&self) -> Value {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let project_dir = project_dir_from_config(&config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let mut payload =
            load_extensions(&ExtensionLoadConfig::from_app_config(project_dir, &config));
        payload.totals.tools += self.tools.catalog().await.len();
        json!(payload)
    }

    pub async fn plugin_set_enabled(&self, id: String, enabled: bool) -> anyhow::Result<Value> {
        let mut config = self.config_get().await?;
        let root = ensure_json_object(&mut config);
        let plugins = root.entry("plugins").or_insert_with(|| json!({}));
        let plugins = ensure_json_object(plugins);
        let enabled_by_id = plugins.entry("enabledById").or_insert_with(|| json!({}));
        let enabled_by_id = ensure_json_object(enabled_by_id);
        enabled_by_id.insert(id.clone(), Value::Bool(enabled));

        let saved = self.storage.save_app_config(config).await?;
        self.apply_execution_config(&saved).await;
        self.observability
            .log(
                "info",
                &format!(
                    "Plugin {} {}",
                    id,
                    if enabled { "enabled" } else { "disabled" }
                ),
            )
            .await;
        Ok(json!({
            "ok": true,
            "id": id,
            "enabled": enabled,
            "extensions": self.extensions_list().await
        }))
    }

    pub async fn plugin_install(&self, payload: Value) -> Value {
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("plugin")
            .to_string();
        let install_type = plugin_install_type(&payload).to_string();
        if is_local_plugin_install_type(&install_type) {
            let config = self.config_get().await.unwrap_or_else(|_| json!({}));
            match self
                .install_local_plugin(&payload, &id, &install_type, &config)
                .await
            {
                Ok(result) => return result,
                Err(err) => {
                    self.observability
                        .log(
                            "warn",
                            &format!("Local plugin install failed for {id}: {err:#}"),
                        )
                        .await;
                    return json!({
                        "ok": false,
                        "id": id,
                        "installType": install_type,
                        "error": err.to_string()
                    });
                }
            }
        }
        if install_type.eq_ignore_ascii_case("mcp") {
            match self.install_mcp_plugin(&payload, &id).await {
                Ok(result) => return result,
                Err(err) => {
                    self.observability
                        .log("warn", &format!("MCP plugin install failed for {id}: {err:#}"))
                        .await;
                    return json!({
                        "ok": false,
                        "id": id,
                        "installType": install_type,
                        "error": err.to_string()
                    });
                }
            }
        }

        let manual = plugin_manual_install_hint(&payload);
        self.observability
            .log(
                "info",
                &format!("Plugin install for {id} requires manual {install_type} setup"),
            )
            .await;
        json!({
            "ok": false,
            "id": id,
            "installType": install_type,
            "requiresManualInstall": true,
            "manualCommand": manual.command_line,
            "manualArgs": manual.args,
            "error": manual.message
        })
    }

    pub async fn question_respond(&self, request_id: String, answers: Value) -> Value {
        let pending = {
            let mut pending = self.pending_questions.write().await;
            pending.remove(&request_id)
        };
        let Some(pending) = pending else {
            return json!({
                "ok": false,
                "requestId": request_id,
                "error": format!("Unknown question request: {request_id}")
            });
        };
        let session_id = pending.session_id.clone();
        if let Some(tx) = pending.resolve {
            let _ = tx.send(Ok(answers.clone()));
        }
        self.eventbus.publish(AppEvent::QuestionAnswered {
            session_id: SessionId(session_id.clone()),
            request_id: request_id.clone(),
            answers: answers.clone(),
        });
        json!({
            "ok": true,
            "requestId": request_id,
            "sessionId": session_id,
            "answers": answers
        })
    }

    async fn execute_ask_user(
        &self,
        session_id: Option<String>,
        run_id: Option<String>,
        call: &ToolCall,
    ) -> ToolOutput {
        let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
            return ToolOutput {
                content: "ask_user unavailable: no interactive user channel in this runtime. Ask in your final reply instead.".to_string(),
                is_error: true,
                data: None,
            };
        };
        let questions = match parse_ask_user_questions(&call.args) {
            Ok(questions) => questions,
            Err(err) => {
                return ToolOutput {
                    content: err,
                    is_error: true,
                    data: None,
                };
            }
        };
        let request_id = format!("question_{}", EventId::new().0);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_questions.write().await;
            pending.insert(
                request_id.clone(),
                PendingQuestion {
                    session_id: session_id.clone(),
                    resolve: Some(tx),
                },
            );
        }
        if let Some(run_id) = run_id.as_deref() {
            let _ = self
                .runs
                .record_event(
                    run_id,
                    "trace.permission.decision",
                    Some(json!({
                        "toolName": "ask_user",
                        "args": { "questions": questions },
                        "allowed": true,
                        "reason": "awaiting_user_answers",
                        "needsConfirmation": true,
                        "requestId": request_id
                    })),
                    None,
                )
                .await;
        }
        self.eventbus.publish(AppEvent::QuestionRequested {
            session_id: SessionId(session_id.clone()),
            request_id: request_id.clone(),
            questions: questions.clone(),
            run_id: run_id.clone(),
        });
        let answered = tokio::time::timeout(Duration::from_secs(30 * 60), rx).await;
        self.pending_questions.write().await.remove(&request_id);
        match answered {
            Ok(Ok(Ok(answers))) => format_ask_user_answers(&questions, &answers),
            Ok(Ok(Err(err))) => ToolOutput {
                content: format!("ask_user cancelled or failed: {err}"),
                is_error: true,
                data: Some(json!({ "requestId": request_id })),
            },
            Ok(Err(_)) => ToolOutput {
                content: "ask_user cancelled or failed: question channel closed".to_string(),
                is_error: true,
                data: Some(json!({ "requestId": request_id })),
            },
            Err(_) => ToolOutput {
                content: "ask_user cancelled or failed: timed out waiting for user answers"
                    .to_string(),
                is_error: true,
                data: Some(json!({ "requestId": request_id })),
            },
        }
    }

    async fn reject_pending_questions_for_session(&self, session_id: &str, reason: &str) {
        let mut pending = self.pending_questions.write().await;
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, item)| item.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(item) = pending.remove(&id) {
                if let Some(tx) = item.resolve {
                    let _ = tx.send(Err(reason.to_string()));
                }
            }
        }
    }

    async fn install_local_plugin(
        &self,
        payload: &Value,
        requested_id: &str,
        install_type: &str,
        config: &Value,
    ) -> anyhow::Result<Value> {
        let project_dir = project_dir_from_config(config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let extension_config = ExtensionLoadConfig::from_app_config(project_dir.clone(), config);
        let source_path = local_plugin_install_path(payload)
            .map(|path| resolve_path_from(&project_dir, &path))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Local plugin install requires install.path, install.directory, or install.manifestPath."
                )
            })?;
        let (source_dir, manifest_path) = local_plugin_manifest_paths(&source_path)?;
        let manifest = parse_plugin_manifest(&manifest_path, "local").ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid plugin manifest: {}",
                path_to_string(&manifest_path)
            )
        })?;
        let requested_slug = slugify_identifier(requested_id);
        if requested_slug != "plugin" && requested_slug != manifest.id {
            anyhow::bail!(
                "Local plugin manifest id '{}' does not match requested plugin id '{}'.",
                manifest.id,
                requested_id
            );
        }

        let source_dir = fs::canonicalize(&source_dir).map_err(|err| {
            anyhow::anyhow!(
                "Cannot resolve local plugin source '{}': {err}",
                path_to_string(&source_dir)
            )
        })?;
        if plugin_source_is_configured(&source_dir, &extension_config.plugin_directories) {
            self.observability
                .log(
                    "info",
                    &format!(
                        "Local plugin {} is already in a configured plugin directory",
                        manifest.id
                    ),
                )
                .await;
            return Ok(json!({
                "ok": true,
                "id": manifest.id.clone(),
                "installType": install_type,
                "alreadyInstalled": true,
                "installedDir": path_to_string(&source_dir),
                "manifest": manifest,
                "extensions": self.extensions_list().await
            }));
        }

        let target_root = extension_config
            .plugin_directories
            .first()
            .cloned()
            .unwrap_or_else(|| project_data_root(&project_dir).join("plugins"));
        fs::create_dir_all(&target_root).map_err(|err| {
            anyhow::anyhow!(
                "Cannot create plugin directory '{}': {err}",
                path_to_string(&target_root)
            )
        })?;
        let target_root = fs::canonicalize(&target_root).map_err(|err| {
            anyhow::anyhow!(
                "Cannot resolve plugin directory '{}': {err}",
                path_to_string(&target_root)
            )
        })?;
        let target_dir = target_root.join(&manifest.id);
        if target_dir.exists() {
            let target_manifest_path = target_dir.join("plugin.json");
            let installed_manifest = parse_plugin_manifest(&target_manifest_path, "local")
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Plugin target '{}' already exists but does not contain a valid plugin.json.",
                        path_to_string(&target_dir)
                    )
                })?;
            self.observability
                .log(
                    "info",
                    &format!(
                        "Local plugin {} was already installed",
                        installed_manifest.id
                    ),
                )
                .await;
            return Ok(json!({
                "ok": true,
                "id": installed_manifest.id.clone(),
                "installType": install_type,
                "alreadyInstalled": true,
                "installedDir": path_to_string(&target_dir),
                "manifest": installed_manifest,
                "extensions": self.extensions_list().await
            }));
        }
        if target_dir.starts_with(&source_dir) {
            anyhow::bail!(
                "Refusing to copy plugin '{}' into its own source tree.",
                manifest.id
            );
        }

        copy_plugin_directory(&source_dir, &target_dir)?;
        let installed_manifest = parse_plugin_manifest(&target_dir.join("plugin.json"), "local")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Copied plugin '{}' but destination manifest could not be loaded.",
                    manifest.id
                )
            })?;
        self.observability
            .log(
                "info",
                &format!(
                    "Installed local plugin {} into {}",
                    installed_manifest.id,
                    path_to_string(&target_dir)
                ),
            )
            .await;
        Ok(json!({
            "ok": true,
            "id": installed_manifest.id.clone(),
            "installType": install_type,
            "installedDir": path_to_string(&target_dir),
            "manifest": installed_manifest,
            "extensions": self.extensions_list().await
        }))
    }

    async fn install_mcp_plugin(&self, payload: &Value, server_id: &str) -> anyhow::Result<Value> {
        let install = payload
            .get("install")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("MCP plugin install requires an install object."))?;
        let remote_url = string_field_in_object(Some(install), &["url"])
            .filter(|value| !value.trim().is_empty());
        let command = string_field_in_object(Some(install), &["command"])
            .filter(|value| !value.trim().is_empty());
        if remote_url.is_none() && command.is_none() {
            anyhow::bail!("MCP plugin install requires command or url.");
        }
        let args = install
            .get("args")
            .map(json_string_array)
            .unwrap_or_default();
        let env = install
            .get("env")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        value
                            .as_str()
                            .map(|text| (key.clone(), text.to_string()))
                    })
                    .collect::<BTreeMap<String, String>>()
            })
            .unwrap_or_default();
        let cwd = string_field_in_object(Some(install), &["cwd", "workingDirectory"]);
        let timeout_ms = install
            .get("timeoutMs")
            .or_else(|| install.get("timeout_ms"))
            .and_then(Value::as_u64);
        let transport = string_field_in_object(Some(install), &["transport", "type"])
            .filter(|value| !value.trim().is_empty());

        let config = self.config_get().await?;
        let project_dir = project_dir_from_config(&config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let xuanyu_dir = project_data_root(&project_dir);
        tokio::fs::create_dir_all(&xuanyu_dir).await?;
        let mcp_path = xuanyu_dir.join("mcp.json");
        let mut root = if tokio::fs::metadata(&mcp_path).await.is_ok() {
            let raw = tokio::fs::read_to_string(&mcp_path).await?;
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "mcpServers": {} }))
        } else {
            json!({ "mcpServers": {} })
        };
        let servers = root
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP config root must be a JSON object."))?
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        let server_entry = servers
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP mcpServers must be a JSON object."))?;
        let config_value = if let Some(url) = remote_url {
            let mut value = json!({
                "type": transport.unwrap_or_else(|| "http".to_string()),
                "url": url,
            });
            if !env.is_empty() {
                value["env"] = json!(env);
                value["headers"] = json!(env);
            }
            if let Some(timeout_ms) = timeout_ms {
                value["timeoutMs"] = json!(timeout_ms);
            }
            value
        } else {
            let mut value = json!({
                "command": command.expect("command checked above"),
                "args": args,
            });
            if !env.is_empty() {
                value["env"] = json!(env);
            }
            if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
                value["cwd"] = json!(cwd);
            }
            if let Some(timeout_ms) = timeout_ms {
                value["timeoutMs"] = json!(timeout_ms);
            }
            value
        };
        server_entry.insert(server_id.to_string(), config_value);
        tokio::fs::write(
            &mcp_path,
            serde_json::to_string_pretty(&root)? + "\n",
        )
        .await?;
        let summary = self.tools.refresh_mcp_tools(&project_dir).await?;
        self.observability
            .log(
                "info",
                &format!(
                    "Installed MCP server {server_id} into {} ({} tool(s) from {} server(s))",
                    path_to_string(&mcp_path),
                    summary.tools,
                    summary.servers
                ),
            )
            .await;
        Ok(json!({
            "ok": true,
            "id": server_id,
            "installType": "mcp",
            "path": path_to_string(&mcp_path),
            "mcp": {
                "servers": summary.servers,
                "tools": summary.tools,
                "errors": summary.errors,
            },
            "extensions": self.extensions_list().await
        }))
    }

    pub async fn config_get(&self) -> anyhow::Result<Value> {
        self.config_get_raw().await
    }

    async fn config_get_raw(&self) -> anyhow::Result<Value> {
        Ok(self
            .storage
            .load_app_config()
            .await?
            .unwrap_or_else(|| json!({})))
    }

    /// IPC / UI 下发用：密钥已清空，并带 `credentialHints`。
    pub async fn config_get_public(&self) -> anyhow::Result<Value> {
        Ok(redact_config_for_transport(self.config_get_raw().await?))
    }

    pub async fn config_info(&self) -> ConfigInfoDto {
        let path = self.storage.app_config_path();
        ConfigInfoDto {
            path: path_to_string(&path),
            exists: tokio::fs::metadata(&path).await.is_ok(),
            data_dir: path_to_string(&self.storage.config().data_dir),
            project_name: self.storage.config().project_name.clone(),
        }
    }

    pub async fn config_update(&self, partial: Value) -> anyhow::Result<Value> {
        let current = self.config_get_raw().await?;
        let safe_partial = preserve_configured_secrets(partial, &current);
        let mut merged = current;
        merge_json(&mut merged, safe_partial);
        if let Err(errors) = validate_app_config(&merged) {
            anyhow::bail!("invalid configuration:\n- {}", errors.join("\n- "));
        }
        let saved = self.storage.save_app_config(merged).await?;
        self.apply_execution_config(&saved).await;
        self.observability
            .log("info", "Configuration saved to Rust storage")
            .await;
        // 本机返回完整配置，便于桌面 UI 继续编辑密钥字段。
        Ok(saved)
    }

    pub async fn config_capabilities(&self) -> Value {
        let mut models = Vec::new();
        for task in model_capability_tasks() {
            let route = self.execution.resolve_route(task).await;
            let rate_limit = self.execution.resolve_rate_limit(task, &route).await;
            models.push(model_capability_route_payload(&route, rate_limit));
        }
        json!({ "models": models })
    }

    pub async fn init_models_fetch(&self, input: InitModelsFetchInput) -> InitModelsFetchResult {
        let mut input = input;
        let needs_key = input
            .api_key
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty();
        let needs_base = input
            .base_url
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty();
        if needs_key || needs_base {
            if let Ok(config) = self.config_get_raw().await {
                let resolved =
                    resolve_stored_model_fetch_credentials(&config, &input.provider_type);
                if needs_key {
                    if let Some(api_key) = resolved.api_key {
                        input.api_key = Some(api_key);
                    }
                }
                if needs_base {
                    if let Some(base_url) = resolved.base_url {
                        input.base_url = Some(base_url);
                    }
                }
            }
        }

        match fetch_models_for_provider(input).await {
            Ok(models) => InitModelsFetchResult {
                models,
                error: None,
            },
            Err(error) => InitModelsFetchResult {
                models: Vec::new(),
                error: Some(error.to_string()),
            },
        }
    }

    pub async fn init_postgres_status(&self, connection_string: Option<String>) -> Value {
        if let Some(connection_string) = connection_string.as_deref().filter(|value| !value.trim().is_empty()) {
            if init_is_sqlite_connection(connection_string) {
                let database_path = init_sqlite_path_from_connection(connection_string)
                    .unwrap_or_else(|| self.storage.config().database_path.clone());
                return json!({
                    "phase": "ready",
                    "installed": true,
                    "pgvectorFiles": true,
                    "running": true,
                    "pgvector": true,
                    "port": 0,
                    "database": database_path.file_name().and_then(|name| name.to_str()).unwrap_or("xuanyu.sqlite"),
                    "username": "local",
                    "connectionString": connection_string,
                    "embedded": {
                        "backend": "sqlite",
                        "databasePath": path_to_string(&database_path),
                        "vectorIndex": "local-hash"
                    }
                });
            }
            if init_is_postgres_connection(connection_string) {
                let test = self
                    .init_postgres_test_connection(connection_string.to_string())
                    .await;
                let phase = if test.get("connected").and_then(Value::as_bool) == Some(true) {
                    if test.get("pgvector").and_then(Value::as_bool) == Some(true) {
                        "ready"
                    } else {
                        "needs_pgvector"
                    }
                } else {
                    "not_running"
                };
                return json!({
                    "phase": phase,
                    "installed": test.get("connected").and_then(Value::as_bool).unwrap_or(false),
                    "pgvectorFiles": test.get("pgvector").and_then(Value::as_bool).unwrap_or(false),
                    "running": test.get("connected").and_then(Value::as_bool).unwrap_or(false),
                    "pgvector": test.get("pgvector").and_then(Value::as_bool).unwrap_or(false),
                    "connectionString": connection_string,
                    "error": test.get("error").cloned().unwrap_or(Value::Null),
                });
            }
        }

        let paths = EmbeddedPostgresPaths::from_data_dir(&self.storage.config().data_dir);
        let embedded = embedded_postgres_status(&paths);
        if embedded.running && embedded.pgvector {
            return serde_json::to_value(embedded).unwrap_or_else(|_| json!({}));
        }

        let storage = self.storage.config();
        let database_path = storage.database_path.clone();
        let data_dir = database_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| storage.data_dir.join("db"));
        let mut status = serde_json::to_value(&embedded).unwrap_or_else(|_| json!({}));
        if status
            .get("connectionString")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            status["connectionString"] = json!(paths.connection_string());
        }
        if !embedded.installed {
            let sqlite_ready = self.storage.is_initialized().await
                && fs::metadata(&database_path)
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false);
            status["sqliteFallback"] = json!({
                "phase": if sqlite_ready { "ready" } else { "needs_install" },
                "connectionString": init_sqlite_connection_string(&database_path),
                "embedded": {
                    "backend": "sqlite",
                    "databasePath": path_to_string(&database_path),
                    "vectorIndex": "local-hash"
                },
                "dataDir": path_to_string(&data_dir),
            });
            if sqlite_ready {
                status["phase"] = json!("ready");
                status["installed"] = json!(true);
                status["running"] = json!(true);
                status["pgvector"] = json!(true);
                status["pgvectorFiles"] = json!(true);
                status["database"] = json!(database_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("xuanyu.sqlite"));
                status["connectionString"] = json!(init_sqlite_connection_string(&database_path));
                status["embedded"] = json!({
                    "backend": "sqlite",
                    "databasePath": path_to_string(&database_path),
                    "vectorIndex": "local-hash"
                });
            }
        }
        status
    }

    pub async fn init_postgres_install(
        &self,
        payload: Value,
        progress_tx: Option<std::sync::mpsc::Sender<Value>>,
    ) -> Value {
        let paths = EmbeddedPostgresPaths::from_data_dir(&self.storage.config().data_dir);
        let options = EmbeddedPostgresInstallOptions {
            skip_download: payload
                .get("skipDownload")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            install_pgvector: payload
                .get("installPgvector")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        };
        let install_paths = paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            install_embedded_postgres(&install_paths, options, |progress| {
                if let Some(tx) = progress_tx.as_ref() {
                    let mut event = json!({
                        "type": "progress",
                        "phase": progress.phase,
                        "percent": progress.percent,
                        "message": progress.message,
                    });
                    if let Some(downloaded) = progress.downloaded {
                        event["downloaded"] = json!(downloaded);
                    }
                    if let Some(total) = progress.total {
                        event["total"] = json!(total);
                    }
                    let _ = tx.send(event);
                }
            })
        })
        .await;
        match result {
            Ok(Ok((connection_string, logs))) => {
                let status = embedded_postgres_status(&paths);
                json!({
                    "ok": true,
                    "connectionString": connection_string,
                    "config": status.embedded,
                    "pgvector": status.pgvector,
                    "logs": logs,
                })
            }
            Ok(Err(error)) => json!({
                "ok": false,
                "error": error,
                "logs": [],
            }),
            Err(error) => json!({
                "ok": false,
                "error": error.to_string(),
                "logs": [],
            }),
        }
    }

    pub async fn init_postgres_test_connection(&self, connection_string: String) -> Value {
        let started = std::time::Instant::now();
        let trimmed = connection_string.trim();
        if trimmed.is_empty() {
            return json!({
                "ok": false,
                "connected": false,
                "verified": true,
                "pgvector": false,
                "error": "Connection string is required."
            });
        }

        if init_is_sqlite_connection(trimmed) {
            let database_path = init_sqlite_path_from_connection(trimmed)
                .unwrap_or_else(|| self.storage.config().database_path.clone());
            return json!({
                "ok": true,
                "connected": true,
                "verified": true,
                "latencyMs": started.elapsed().as_millis() as u64,
                "version": "Local desktop store (sqlite config backend)",
                "database": database_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("xuanyu.sqlite"),
                "pgvector": true
            });
        }

        if !init_is_postgres_connection(trimmed) {
            return json!({
                "ok": false,
                "connected": false,
                "verified": true,
                "pgvector": false,
                "error": "Use a sqlite://, postgres://, or postgresql:// connection string."
            });
        }

        let config = LegacyPostgresConfig {
            connection_string: trimmed.to_string(),
            psql_path: init_postgres_psql_path(),
        };
        match legacy_postgres_query_scalar(&config, "SELECT version()") {
            Ok(version) => {
                let pgvector = legacy_postgres_query_scalar(
                    &config,
                    "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector')",
                )
                .map(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "t" | "true" | "1"
                    )
                })
                .unwrap_or(false);
                json!({
                    "ok": true,
                    "connected": true,
                    "verified": true,
                    "latencyMs": started.elapsed().as_millis() as u64,
                    "version": version,
                    "database": init_postgres_database_name(trimmed),
                    "pgvector": pgvector,
                    "pgvectorError": if pgvector {
                        Value::Null
                    } else {
                        Value::String("pgvector extension is not enabled in this database.".to_string())
                    }
                })
            }
            Err(err) => json!({
                "ok": false,
                "connected": false,
                "verified": true,
                "pgvector": false,
                "error": err.to_string()
            }),
        }
    }

    pub async fn init_postgres_start(&self) -> anyhow::Result<Value> {
        let data_dir = self.storage.config().data_dir.clone();
        let paths = EmbeddedPostgresPaths::from_data_dir(&data_dir);
        let embedded = embedded_postgres_status(&paths);
        if embedded.installed {
            let start_paths = paths.clone();
            let start_result =
                tokio::task::spawn_blocking(move || start_embedded_postgres(&start_paths)).await;
            match start_result {
                Ok(Ok((connection_string, logs))) => {
                    let status = embedded_postgres_status(&paths);
                    return Ok(json!({
                        "ok": true,
                        "connectionString": connection_string,
                        "config": status.embedded,
                        "pgvector": status.pgvector,
                        "logs": logs,
                    }));
                }
                Ok(Err(error)) => {
                    return Ok(json!({
                        "ok": false,
                        "error": error,
                        "logs": [],
                    }));
                }
                Err(error) => {
                    return Ok(json!({
                        "ok": false,
                        "error": error.to_string(),
                        "logs": [],
                    }));
                }
            }
        }

        self.storage.initialize().await?;
        let status = self.init_postgres_status(None).await;
        Ok(json!({
            "ok": true,
            "connectionString": status.get("connectionString").cloned().unwrap_or(Value::Null),
            "config": status.get("embedded").cloned().unwrap_or(Value::Null),
            "pgvector": true,
            "logs": ["Local desktop store is ready."]
        }))
    }

    /// `storage.get_setup`：返回外部数据库是否已配置 + 嵌入式 PG 的当前阶段。
    pub async fn storage_get_setup(&self) -> Value {
        let external_configured = self
            .storage
            .load_app_config()
            .await
            .unwrap_or(None)
            .map(|cfg| {
                cfg.get("memory")
                    .and_then(|memory| memory.get("postgres"))
                    .and_then(|pg| {
                        pg.get("connectionString")
                            .or_else(|| pg.get("connection_string"))
                    })
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some()
            })
            .unwrap_or(false);
        let paths = EmbeddedPostgresPaths::from_data_dir(&self.storage.config().data_dir);
        let status = embedded_postgres_status(&paths);
        let embedded_phase = serde_json::to_value(status.phase)
            .and_then(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| serde_json::Error::io(std::io::Error::other("not a string")))
            })
            .unwrap_or_else(|_| "unknown".to_string());
        json!({
            "externalConfigured": external_configured,
            "embeddedInstalled": status.installed,
            "embeddedPhase": embedded_phase,
        })
    }

    /// `storage.test_external_connection`：只测试连接 + `select version()`，不写配置。
    pub async fn storage_test_external_connection(&self, connection_string: String) -> Value {
        let trimmed = connection_string.trim().to_string();
        if trimmed.is_empty() {
            return json!({ "ok": false, "message": "请填写连接串 / connection string is required." });
        }
        match self
            .storage
            .test_external_postgres_connection(&trimmed)
            .await
        {
            Ok(version) => json!({ "ok": true, "message": version }),
            Err(err) => json!({ "ok": false, "message": err.to_string() }),
        }
    }

    /// `storage.save_external`：用连接串验证可建 xy_ 表（权限检查），成功后写入
    /// 应用配置文件 `memory.postgres.connectionString`（保留其余字段原样）。
    pub async fn storage_save_external(&self, connection_string: String) -> Value {
        let trimmed = connection_string.trim().to_string();
        if trimmed.is_empty() {
            return json!({ "ok": false, "message": "请填写连接串 / connection string is required." });
        }
        if let Err(err) = self
            .storage
            .configure_conversation_store(&trimmed, "public", "xy")
            .await
        {
            return json!({ "ok": false, "message": format!("数据库接入失败（需可创建 xy_ 表权限）: {err:#}") });
        }
        let existing = self
            .storage
            .load_app_config()
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| json!({}));
        let mut next = existing;
        if !next.is_object() {
            next = json!({});
        }
        let object = next.as_object_mut().expect("object");
        let memory = object
            .entry("memory".to_string())
            .or_insert_with(|| json!({}));
        if !memory.is_object() {
            *memory = json!({});
        }
        let pg = memory
            .as_object_mut()
            .expect("object")
            .entry("postgres".to_string())
            .or_insert_with(|| json!({}));
        if !pg.is_object() {
            *pg = json!({});
        }
        pg["connectionString"] = json!(trimmed.clone());
        pg["connection_string"] = json!(trimmed.clone());
        match self.storage.save_app_config(next).await {
            Ok(config) => {
                let configured = config
                    .get("memory")
                    .and_then(|memory| memory.get("postgres"))
                    .and_then(|pg| pg.get("connectionString"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some();
                json!({
                    "ok": configured,
                    "message": if configured {
                        "已写入外部数据库配置 / external database configured.".to_string()
                    } else {
                        "配置写入后未检测到连接串".to_string()
                    },
                })
            }
            Err(err) => {
                json!({ "ok": false, "message": format!("写入配置文件失败: {err:#}") })
            }
        }
    }

    async fn apply_execution_config(&self, config: &Value) {
        let token_config = token_counter_config_from_app_config(config);
        set_token_counter_runtime_config(token_config);
        self.execution
            .update_config(ExecutionConfig::from_app_config_value(config))
            .await;
        self.governance
            .update_policy(PermissionPolicy::from_app_config_value(config))
            .await;
        let project_dir = project_dir_from_config(config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        match self.tools.refresh_mcp_tools(&project_dir).await {
            Ok(summary) => {
                let level = if summary.errors.is_empty() {
                    "info"
                } else {
                    "warn"
                };
                self.observability
                    .log(
                        level,
                        &format!(
                            "MCP refresh loaded {} tool(s) from {} server(s)",
                            summary.tools, summary.servers
                        ),
                    )
                    .await;
            }
            Err(err) => {
                self.observability
                    .log("warn", &format!("MCP refresh failed: {err}"))
                    .await;
            }
        }
    }

    async fn retrieved_memory_context(
        &self,
        session_id: &SessionId,
        user_input: &str,
        config: &Value,
    ) -> Option<String> {
        if !memory_auto_inject_enabled(config) || user_input.trim().is_empty() {
            return None;
        }
        let history = self.sessions.history(session_id).await.unwrap_or_default();
        let retrieval_query = build_memory_retrieval_query(user_input, &history)?;
        let min_score = memory_auto_inject_min_score(config);
        let limit = memory_auto_inject_limit(config);
        let include_cross_session = memory_auto_inject_include_cross_session(config);
        let embedding_provider = self.current_memory_embedding_provider().await;
        let results = self
            .memory
            .search_with_embedding_provider(
                MemorySearchQuery {
                    text: Some(retrieval_query.clone()),
                    session_id: Some(session_id.0.clone()),
                    include_cross_session,
                    limit: limit.saturating_mul(2).max(limit),
                    ..MemorySearchQuery::default()
                },
                &embedding_provider,
            )
            .await
            .into_iter()
            .filter(|result| result.score >= min_score)
            .collect::<Vec<_>>();
        let results = self.rerank_memory_results(&retrieval_query, results, limit);

        if results.is_empty() {
            return None;
        }

        let mut lines = vec![
            "Retrieved memories relevant to this turn. Use them as private context.".to_string(),
        ];
        for result in results {
            lines.push(format!(
                "- [{} score {:.2}] {}",
                result.entry.category,
                result.score,
                compact_context_line(&result.entry.content, 360)
            ));
        }
        Some(lines.join("\n"))
    }

    pub async fn system_status(&self) -> SystemStatusDto {
        let (config, config_error) = match self.config_get().await {
            Ok(config) => (config, None),
            Err(error) => (json!({}), Some(error.to_string())),
        };
        let config_info = self.config_info().await;
        let conversation_health = self.storage.conversation_store().health().await;
        let cwd = self
            .resolve_workspace_dir(None)
            .await;
        let now = Utc::now();
        let gateway = config.get("gateway").and_then(Value::as_object);
        let memory_cfg = config.get("memory").and_then(Value::as_object);
        let sensitive_scan = scan_sensitive_files(&cwd);
        let security = SecurityStatusDto {
            warnings: format_sensitive_scan_warnings(&sensitive_scan),
            sensitive_findings: sensitive_scan.findings,
        };
        let conversation = ConversationStoreHealthDto {
            backend: conversation_health.backend,
            ok: conversation_health.ok,
            location: conversation_health.location,
            pgvector: conversation_health.pgvector,
            error: conversation_health.error,
        };
        let memory_backend = memory_cfg
            .and_then(|item| item.get("backend"))
            .and_then(Value::as_str)
            .unwrap_or("sqlite")
            .to_string();
        let memory_conn = memory_cfg
            .and_then(|item| item.get("postgres"))
            .and_then(Value::as_object)
            .and_then(|postgres| postgres.get("connectionString").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let postgres_configured = memory_conn.is_some();
        let postgres_active = self.memory.has_postgres().await;
        let memory_backend_lc = memory_backend.trim().to_ascii_lowercase();
        let memory_ok = match memory_backend_lc.as_str() {
            "postgres" | "postgresql" => postgres_active,
            _ => true,
        };
        let memory = MemoryStoreHealthDto {
            backend: memory_backend,
            postgres_configured,
            ok: memory_ok,
            location: memory_conn.clone(),
            pgvector: if postgres_active {
                Some(true)
            } else if postgres_configured && matches!(memory_backend_lc.as_str(), "postgres" | "postgresql")
            {
                Some(false)
            } else {
                None
            },
        };
        let gateway = GatewayStatusDto {
            enabled: gateway
                .and_then(|item| item.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            port: gateway
                .and_then(|item| item.get("port"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            bind: gateway
                .and_then(|item| item.get("bind"))
                .and_then(Value::as_str)
                .unwrap_or("desktop-ipc")
                .to_string(),
            ws_clients: Some(1),
        };
        let channels = channel_summary_from_config(&config);
        let runtime = RuntimeStatusDto {
            pid: std::process::id(),
            uptime_sec: (now - self.started_at).num_seconds().max(0) as u64,
            node: "electron".to_string(),
            platform: std::env::consts::OS.to_string(),
            cwd: path_to_display_string(&cwd),
            started_at: self.started_at.timestamp_millis(),
        };
        let workers = self.runtime_workers();
        self.refresh_health_registry().await;
        let doctor = self
            .build_doctor_status(DoctorStatusInput {
                config_info: &config_info,
                config_error,
                config: &config,
                conversation: &conversation,
                memory: &memory,
                gateway: &gateway,
                channels: &channels,
                workers: &workers,
                security: &security,
            })
            .await;
        SystemStatusDto {
            project_name: self.storage.config().project_name.clone(),
            project_dir: path_to_display_string(&cwd),
            conversation,
            memory,
            gateway,
            channels,
            runtime,
            workers,
            security,
            doctor,
        }
    }

    async fn build_doctor_status(&self, input: DoctorStatusInput<'_>) -> DoctorStatusDto {
        let DoctorStatusInput {
            config_info,
            config_error,
            config,
            conversation,
            memory,
            gateway,
            channels,
            workers,
            security,
        } = input;
        let mut checks = Vec::new();
        if let Some(error) = config_error {
            checks.push(doctor_check(
                "Config load",
                DoctorCheckStatus::Fail,
                error,
                Some("Open the init/config screen and save a valid configuration."),
            ));
        } else {
            checks.push(doctor_check(
                "Config file",
                if config_info.exists {
                    DoctorCheckStatus::Ok
                } else {
                    DoctorCheckStatus::Warn
                },
                format!(
                    "global={} path={}",
                    if config_info.exists { "yes" } else { "no" },
                    config_info.path
                ),
                (!config_info.exists)
                    .then_some("Open the init/config screen to create app-config.json."),
            ));
        }

        checks.push(provider_config_doctor_check(config));
        checks.push(memory_backend_doctor_check(memory));
        checks.push(conversation_store_doctor_check(conversation));
        checks.push(gateway_doctor_check(gateway));
        checks.push(channel_summary_doctor_check(channels));

        for task in model_capability_tasks() {
            let route = self.execution.resolve_route(task).await;
            let status = if route.compatibility.compatible {
                DoctorCheckStatus::Ok
            } else {
                DoctorCheckStatus::Fail
            };
            checks.push(doctor_check(
                format!("Model capability: {}", model_task_key(task)),
                status,
                model_route_status_detail(&route),
                (status == DoctorCheckStatus::Fail)
                    .then_some("Adjust model routing or choose a tool-capable chat model."),
            ));
        }

        for worker in workers {
            checks.push(worker_doctor_check(worker));
        }

        checks.push(security_doctor_check(security));
        DoctorStatusDto::from_checks(checks)
    }

    pub async fn gateway_logs(&self, limit: usize) -> Vec<GatewayLogEntryDto> {
        self.observability
            .recent_events(limit.max(1))
            .await
            .into_iter()
            .filter_map(gateway_log_from_event)
            .collect()
    }

    pub async fn diagnostics_export(&self, limit: usize) -> anyhow::Result<Value> {
        let limit = limit.clamp(1, 500);
        let now = Utc::now();
        let dest_dir = self.storage.config().data_dir.join("diagnostics");
        fs::create_dir_all(&dest_dir)?;
        let stem = format!("diagnostics-{}", now.format("%Y%m%dT%H%M%SZ"));
        let json_path = dest_dir.join(format!("{stem}.json"));
        let zip_path = dest_dir.join(format!("{stem}.zip"));
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let payload = json!({
            "kind": "xuanyu.diagnostics",
            "version": 1,
            "exportedAt": now.timestamp_millis(),
            "protocolVersion": PROTOCOL_VERSION,
            "path": path_to_string(&zip_path),
            "jsonPath": path_to_string(&json_path),
            "archiveFormat": "zip",
            "system": self.system_status().await,
            "configRedacted": redact_json_clone(&config),
            "recentEvents": self.observability.recent_events(limit).await,
            "metrics": self.observability.metrics().await,
            "health": self.observability.health_summary().await,
            "alerts": self.observability.alerts(limit).await,
            "profileSpans": self.observability.profile_spans(limit).await,
            "release": release_readiness_snapshot(&self.workspace_root),
            "runs": self.runs.list_runs(limit.min(100)).await,
        });
        let json_bytes = serde_json::to_vec_pretty(&payload)?;
        fs::write(&json_path, &json_bytes)?;
        write_single_file_zip(&zip_path, "diagnostics.json", &json_bytes)?;
        self.observability
            .log(
                "info",
                &format!("Diagnostics exported to {}", path_to_string(&zip_path)),
            )
            .await;
        Ok(payload)
    }

    pub async fn data_backup_export(&self) -> anyhow::Result<Value> {
        let now = Utc::now();
        let data_dir = &self.storage.config().data_dir;
        let backup_dir = data_dir
            .join("backups")
            .join(format!("backup-{}", now.format("%Y%m%dT%H%M%S%.3fZ")));
        let files_root = backup_dir.join("files");
        fs::create_dir_all(&files_root)?;

        let mut entries = Vec::new();
        let mut included_roots = Vec::new();
        let mut skipped_roots = Vec::new();
        for root_name in DATA_BACKUP_ROOTS {
            let source = data_dir.join(root_name);
            if !source.exists() {
                skipped_roots.push((*root_name).to_string());
                continue;
            }
            included_roots.push((*root_name).to_string());
            backup_copy_path(
                &source,
                &files_root.join(root_name),
                Path::new(root_name),
                &mut entries,
            )?;
        }

        entries.sort_by(|a, b| {
            a.get("path")
                .and_then(Value::as_str)
                .cmp(&b.get("path").and_then(Value::as_str))
        });
        let total_bytes = entries
            .iter()
            .filter_map(|entry| entry.get("sizeBytes").and_then(Value::as_u64))
            .sum::<u64>();
        let manifest_path = backup_dir.join("manifest.json");
        let manifest = json!({
            "kind": "xuanyu.dataBackup",
            "version": 1,
            "exportedAt": now.timestamp_millis(),
            "protocolVersion": PROTOCOL_VERSION,
            "projectName": self.storage.config().project_name,
            "dataDir": path_to_string(data_dir),
            "path": path_to_string(&backup_dir),
            "filesRoot": path_to_string(&files_root),
            "includedRoots": included_roots,
            "skippedRoots": skipped_roots,
            "fileCount": entries.len(),
            "totalBytes": total_bytes,
            "files": entries,
        });
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        self.observability
            .log(
                "info",
                &format!("Data backup exported to {}", path_to_string(&backup_dir)),
            )
            .await;
        Ok(json!({
            "ok": true,
            "path": path_to_string(&backup_dir),
            "manifestPath": path_to_string(&manifest_path),
            "filesRoot": path_to_string(&files_root),
            "fileCount": manifest["fileCount"],
            "totalBytes": manifest["totalBytes"],
            "includedRoots": manifest["includedRoots"],
            "skippedRoots": manifest["skippedRoots"],
            "restartRecommended": false
        }))
    }

    pub async fn data_backup_restore(&self, backup_path: PathBuf) -> anyhow::Result<Value> {
        let backup_dir = if backup_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("manifest.json"))
        {
            backup_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow::anyhow!("Backup manifest has no parent directory"))?
        } else {
            backup_path
        };
        let manifest_path = backup_dir.join("manifest.json");
        let manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|err| {
                anyhow::anyhow!(
                    "Cannot read backup manifest '{}': {err}",
                    path_to_string(&manifest_path)
                )
            })?)?;
        if manifest.get("kind").and_then(Value::as_str) != Some("xuanyu.dataBackup") {
            anyhow::bail!("Backup manifest kind is not xuanyu.dataBackup");
        }
        let files_root = backup_dir.join("files");
        if !files_root.is_dir() {
            anyhow::bail!(
                "Backup files directory is missing: {}",
                path_to_string(&files_root)
            );
        }

        let mut restored_files = Vec::new();
        let mut total_bytes = 0u64;
        backup_restore_path(
            &files_root,
            &self.storage.config().data_dir,
            &files_root,
            &mut restored_files,
            &mut total_bytes,
        )?;
        restored_files.sort();
        let restored_file_count = restored_files.len();
        self.observability
            .log(
                "warn",
                &format!(
                    "Data backup restored from {}; restart recommended",
                    path_to_string(&backup_dir)
                ),
            )
            .await;
        Ok(json!({
            "ok": true,
            "path": path_to_string(&backup_dir),
            "manifestPath": path_to_string(&manifest_path),
            "restoredFiles": restored_files,
            "restoredFileCount": restored_file_count,
            "totalBytes": total_bytes,
            "restartRecommended": true
        }))
    }

    pub async fn stats_snapshot(&self) -> Value {
        let sessions = self.sessions.list_sessions().await;
        let mut session_estimates = BTreeMap::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut total_messages = 0usize;

        for session in sessions {
            let history = self.sessions.history(&session.id).await.unwrap_or_default();
            let (input_tokens, output_tokens) = split_token_estimate(&history);
            let total_tokens = input_tokens + output_tokens;
            total_input += input_tokens;
            total_output += output_tokens;
            total_messages += history.len();
            session_estimates.insert(
                session.id.0,
                SessionAnalyticsEstimate {
                    label: session.name,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    message_count: history.len(),
                },
            );
        }

        let mut timelines = Vec::new();
        for run in self.runs.list_runs(500).await {
            if let Some(timeline) = self.runs.timeline(&run.id).await {
                timelines.push(timeline);
            }
        }
        let session_activity = session_activity_from_timelines(&timelines);
        let session_rows = session_estimates
            .iter()
            .map(|(session_id, estimate)| {
                let activity = session_activity
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "sessionId": session_id,
                    "name": estimate.label,
                    "totalTokens": estimate.total_tokens,
                    "inputTokens": estimate.input_tokens,
                    "outputTokens": estimate.output_tokens,
                    "messageCount": estimate.message_count,
                    "toolCallCount": activity.tool_call_count,
                    "compactCount": activity.compact_count,
                    "tokenUsageEstimated": true,
                })
            })
            .collect::<Vec<_>>();

        let analytics = build_analytics_snapshot(
            &timelines,
            &session_estimates,
            total_input,
            total_output,
            total_messages,
        );
        json!({
            "sessions": session_rows,
            "analytics": analytics
        })
    }

    pub async fn context_status(&self, session_id: SessionId) -> Value {
        self.sessions.get_context_status(&session_id, self).await
    }

    async fn build_context_status_inner(&self, session_id: &SessionId) -> Value {
        let history = self.sessions.history(session_id).await.unwrap_or_default();
        let protocol_messages = self.conversation_protocol_messages(session_id).await;
        let sanitized = sanitize_messages_for_tool_protocol(&protocol_messages);
        let (input_tokens, output_tokens) = split_token_estimate(&history);
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let max_tokens = config
            .get("maxContextTokens")
            .and_then(Value::as_u64)
            .unwrap_or(120_000);
        let max_output_tokens = config
            .get("provider")
            .and_then(Value::as_object)
            .and_then(|provider| provider.get("maxTokens"))
            .and_then(Value::as_u64)
            .unwrap_or(4096);
        let tool_schema_tokens = self.tools.catalog().await.len() as u64 * 80;
        let load_mode = context_load_mode_from_config(&config);
        let threshold = config
            .get("contextManagement")
            .and_then(Value::as_object)
            .and_then(|context| context.get("autoCompactThreshold"))
            .and_then(Value::as_f64)
            .unwrap_or(0.82);
        let main_brain_intent = infer_mainbrain_intent_from_messages(&history);
        let policy = context_load_policy(&config, load_mode, main_brain_intent);
        let memory_stats = self.memory.stats().await;
        let key_facts = key_facts_from_messages(&history, 40);
        let key_fact_tokens = if should_inject_key_facts(&policy) {
            key_facts_token_estimate(&key_facts)
        } else {
            0
        };
        let context_section_tokens = context_policy_used_tokens(
            &policy,
            memory_stats.total_entries as u64,
            key_fact_tokens,
            key_facts.len(),
        );
        let total_tokens =
            input_tokens + output_tokens + tool_schema_tokens + context_section_tokens;
        let breakdown = token_breakdown(
            input_tokens,
            output_tokens,
            tool_schema_tokens,
            context_section_tokens,
            max_tokens,
            max_output_tokens,
        );
        let allocation = context_allocation(ContextAllocationInput {
            messages: &history,
            max_tokens,
            tool_schema_tokens,
            used_tokens: total_tokens,
            policy: &policy,
            memory_entry_count: memory_stats.total_entries as u64,
            key_fact_tokens,
            key_fact_count: key_facts.len(),
        });
        let compact_snapshots = self.context_compact_snapshots(&session_id.0).await;
        json!({
            "messageCount": history.len(),
            "estimatedTokens": total_tokens,
            "maxTokens": max_tokens,
            "usageRatio": if max_tokens == 0 { 0.0 } else { total_tokens as f64 / max_tokens as f64 },
            "autoCompactEnabled": true,
            "autoCompactThreshold": threshold,
            "compactCount": compact_snapshots.len(),
            "canRollback": !compact_snapshots.is_empty(),
            "latestSnapshot": compact_snapshots.first(),
            "breakdown": breakdown,
            "allocation": allocation,
            "contextLoadMode": load_mode,
            "mainBrainIntent": main_brain_intent,
            "activeSections": active_context_sections(&policy),
            "contextSurface": context_surface_snapshot(&policy),
            "contextLoadPolicy": policy,
            "keyFacts": key_facts,
            "toolProtocolSanitize": {
                "droppedCount": sanitized.dropped_count,
                "strippedToolCallCount": sanitized.stripped_tool_call_count,
                "protocolSafeMessageCount": sanitized.messages.len()
            }
        })
    }

    async fn conversation_protocol_messages(
        &self,
        session_id: &SessionId,
    ) -> Vec<xuanyu_execution::ProtocolMessage> {
        let store = self.storage.conversation_store();
        let records = store
            .list_messages(&session_id.0)
            .await
            .unwrap_or_default();
        records
            .into_iter()
            .map(|record| {
                protocol_message_from_record(
                    &record.role,
                    &record.content,
                    &record.metadata,
                    record.tool_call_id.as_deref(),
                )
            })
            .collect()
    }

    pub async fn context_compact(&self, session_id: SessionId, strategy: Option<String>) -> Value {
        self.sessions
            .compact_context(&session_id, strategy, self)
            .await
    }

    async fn compact_context_inner(
        &self,
        session_id: &SessionId,
        strategy: Option<String>,
    ) -> Value {
        let initial_status = self.build_context_status_inner(session_id).await;
        let before = initial_status
            .get("estimatedTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let strategy = strategy.unwrap_or_else(|| "autocompact".to_string());
        let strategy_kind = CompactStrategyKind::parse(&strategy);
        let usage_ratio = initial_status
            .get("usageRatio")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let message_count = initial_status
            .get("messageCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let mut tokens_after = before;
        if should_compact(usage_ratio, message_count, strategy_kind) {
            let store = self.storage.conversation_store();
            if let Ok(mut messages) = store.list_messages(&session_id.0).await {
                let roles = messages
                    .iter()
                    .map(|message| message.role.as_str())
                    .collect::<Vec<_>>();
                let keep_from = if roles.iter().any(|role| role.eq_ignore_ascii_case("user")) {
                    select_compact_keep_index_by_roles(&roles, strategy_kind)
                } else {
                    select_compact_keep_index(messages.len(), strategy_kind)
                };
                match strategy_kind {
                    CompactStrategyKind::Snip => {
                        for message in messages.iter_mut().take(keep_from) {
                            message.content = snip_message(&message.content, 400);
                        }
                    }
                    _ => {
                        messages = messages.into_iter().skip(keep_from).collect();
                    }
                }
                if self
                    .sessions
                    .replace_history_records(session_id, messages)
                    .await
                    .is_ok()
                {
                    let refreshed = self.build_context_status_inner(session_id).await;
                    tokens_after = refreshed
                        .get("estimatedTokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(before);
                }
            }
        }
        let history = self
            .session_history(session_id.clone())
            .await
            .unwrap_or_default();
        let extraction_messages = history
            .iter()
            .map(extraction_message_from_chat)
            .collect::<Vec<_>>();
        let extraction = extract_memories_from_messages(&extraction_messages);
        let now_ms = Utc::now().timestamp_millis();
        let started_at = history
            .first()
            .map(|message| message.timestamp.timestamp_millis())
            .unwrap_or(now_ms);
        self.memory
            .add_session_summary(SessionSummary {
                session_id: session_id.0.clone(),
                started_at,
                ended_at: Some(now_ms),
                key_decisions: extraction.key_decisions.clone(),
                files_modified: extraction.files_modified.clone(),
                tools_used: extraction.tools_used.clone(),
                summary: Some({
                    let text = extraction.summary.trim();
                    if !text.is_empty() {
                        text.to_string()
                    } else if !extraction.files_modified.is_empty() {
                        format!(
                            "Earlier turns compacted. Files touched: {}.",
                            extraction.files_modified.join(", ")
                        )
                    } else {
                        "Earlier turns were compacted. Continue from the remaining recent messages and this handoff."
                            .to_string()
                    }
                }),
                token_count: Some(before.min(u32::MAX as u64) as u32),
            })
            .await;
        let mut compact_memories = extraction.memories;
        for memory in &mut compact_memories {
            memory.tags = with_memory_session_tag(memory.tags.clone(), Some(&session_id.0));
            if memory.context.is_none() {
                memory.context = Some(session_id.0.clone());
            }
        }
        let snapshot = if self
            .context_compact_should_snapshot(&compact_memories)
            .await
        {
            self.memory
                .save_snapshot(context_compact_snapshot_reason(&session_id.0, &strategy))
                .await
                .ok()
        } else {
            None
        };
        let queued = compact_memories.len();
        if queued > 0 {
            self.spawn_memory_ingest_batch(MemoryIngestBatch {
                entries: compact_memories,
                source: "context_compact".to_string(),
                session_id: Some(session_id.0.clone()),
                task_id: None,
                run_id: None,
                reason: "Automatically extracted memory requires confirmation before entering long-term memory."
                    .to_string(),
                confidence: Some(0.7),
            });
        }
        let stored = 0usize;
        let pending = queued;
        let denied = 0usize;
        let summary = if extraction.summary.trim().is_empty() {
            if tokens_after < before {
                format!(
                    "Context compacted ({strategy}): older turns dropped; resume uses the latest handoff summary plus recent messages."
                )
            } else {
                "Rust context estimator found no compaction candidate yet.".to_string()
            }
        } else if queued > 0 {
            format!(
                "{}. Compact handoff saved for resume (summary + recent turns). {} memories were queued for background ingestion.",
                extraction.summary, queued
            )
        } else {
            format!(
                "{}. Compact handoff saved for resume (summary + recent turns).",
                extraction.summary
            )
        };
        let can_rollback = snapshot.is_some() && queued > 0;
        if let Some(snapshot_info) = snapshot.as_ref() {
            let _ = self
                .governance
                .rollback_registry()
                .register(
                    format!(
                        "Rollback context compact for session {}",
                        session_id.0
                    ),
                    "context",
                    json!({
                        "sessionId": session_id.0,
                        "snapshotVersion": snapshot_info.version,
                        "strategy": strategy,
                    }),
                )
                .await;
            let _ = self
                .governance
                .record_audit_event(
                    "system",
                    "context.compact.snapshot",
                    Some(format!("session:{}", session_id.0)),
                    GovernanceDecision::Allow,
                    format!(
                        "Context compact created memory snapshot v{} ({strategy})",
                        snapshot_info.version
                    ),
                    None,
                )
                .await;
        }
        let snapshot = snapshot.map(memory_snapshot_from_info);
        let snapshot_version = snapshot.as_ref().map(|snapshot| snapshot.version);
        let result = json!({
            "success": queued > 0 || tokens_after < before,
            "strategy": strategy,
            "tokensBefore": before,
            "tokensAfter": tokens_after,
            "memoriesExtracted": stored,
            "memoriesQueued": queued,
            "pendingMemories": pending,
            "deniedMemories": denied,
            "summary": summary,
            "canRollback": can_rollback,
            "snapshotVersion": snapshot_version,
            "snapshot": snapshot
        });
        let mut status = self.build_context_status_inner(session_id).await;
        if let Some(status) = status.as_object_mut() {
            let compact_snapshots = self.context_compact_snapshots(&session_id.0).await;
            status.insert("compactCount".to_string(), json!(compact_snapshots.len()));
            status.insert(
                "canRollback".to_string(),
                json!(!compact_snapshots.is_empty()),
            );
            status.insert("lastCompact".to_string(), result.clone());
        }
        json!({
            "sessionId": session_id.0,
            "result": result,
            "status": status
        })
    }

    pub async fn context_rollback(&self, session_id: SessionId) -> Value {
        self.sessions.rollback_context(&session_id, self).await
    }

    async fn rollback_context_inner(&self, session_id: &SessionId) -> Value {
        let session_id_value = session_id.0.clone();
        let Some(snapshot) = self
            .latest_context_compact_snapshot(&session_id_value)
            .await
        else {
            return json!({
                "sessionId": session_id_value,
                "ok": false,
                "reason": "No context compaction snapshot is available for this session.",
                "status": self.build_context_status_inner(session_id).await
            });
        };
        match self.memory.rollback_snapshot(snapshot.version).await {
            Ok(Some(restored)) => {
                let restored = memory_snapshot_from_info(restored);
                let snapshot_version = restored.version;
                let restored_count = restored.entry_count;
                json!({
                    "sessionId": session_id_value,
                    "ok": true,
                    "snapshotVersion": snapshot_version,
                    "snapshot": restored,
                    "restoredCount": restored_count,
                    "status": self.build_context_status_inner(session_id).await
                })
            }
            Ok(None) => json!({
                "sessionId": session_id_value,
                "ok": false,
                "reason": format!("Context compaction snapshot {} was not found.", snapshot.version),
                "status": self.build_context_status_inner(session_id).await
            }),
            Err(err) => json!({
                "sessionId": session_id_value,
                "ok": false,
                "reason": err.to_string(),
                "status": self.build_context_status_inner(session_id).await
            }),
        }
    }

    pub async fn run_list(&self, limit: usize) -> Value {
        json!({ "runs": self.runs.list_runs(limit).await })
    }

    pub async fn run_timeline(&self, run_id: String) -> Value {
        json!({ "timeline": self.runs.timeline(&run_id).await })
    }

    pub async fn trace_timeline(&self, id: String) -> Value {
        json!({ "trace": self.runs.trace_timeline(&id).await })
    }

    pub async fn run_cancel(&self, run_id: String, reason: Option<String>) -> Value {
        match self.runs.cancel_run(&run_id, reason).await {
            Ok(run) => json!({ "ok": true, "run": run }),
            Err(err) => json!({ "ok": false, "runId": run_id, "error": err.to_string() }),
        }
    }

    pub async fn run_resume(&self, run_id: String) -> Value {
        match self.runs.resume_run(&run_id).await {
            Ok(run) => json!({ "ok": true, "run": run }),
            Err(err) => json!({ "ok": false, "runId": run_id, "error": err.to_string() }),
        }
    }

    pub async fn run_replay(&self, run_id: String) -> Value {
        match self.runs.replay_run(&run_id).await {
            Ok(timeline) => json!({ "timeline": timeline }),
            Err(err) => json!({ "timeline": null, "runId": run_id, "error": err.to_string() }),
        }
    }

    pub async fn trace_evidence(&self, run_id: String, format: Option<String>) -> Value {
        let evidence: Option<EvidencePack> = self.runs.evidence_pack(&run_id).await;
        let markdown = if format.as_deref() == Some("markdown") {
            self.runs.evidence_markdown(&run_id).await
        } else {
            None
        };
        json!({
            "runId": run_id,
            "format": format.unwrap_or_else(|| "json".to_string()),
            "evidence": evidence,
            "markdown": markdown,
        })
    }

    pub async fn trace_compare(&self, base_run_id: String, target_run_id: String) -> Value {
        let comparison: Option<AttemptComparison> = self
            .runs
            .compare_attempts(&base_run_id, &target_run_id)
            .await;
        json!({ "comparison": comparison })
    }

    pub async fn feedback_submit(&self, payload: Value) -> Value {
        let run_id = payload
            .get("runId")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let session_id = payload
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("partial")
            .to_string();
        let note = payload
            .get("note")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let tags = payload
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let feedback: anyhow::Result<UserFeedbackRecord> = self
            .runs
            .submit_feedback(run_id, session_id, status, note, tags)
            .await;
        match feedback {
            Ok(feedback) => json!({ "feedback": feedback }),
            Err(err) => json!({ "error": err.to_string() }),
        }
    }

    pub async fn process_list(&self) -> Value {
        json!({ "processes": self.tools.list_processes() })
    }

    pub async fn process_read(&self, process_id: String, offset: u64) -> Value {
        let read = self.tools.read_process(
            &process_id,
            usize::try_from(offset).unwrap_or(0),
            256 * 1024,
        );
        match read {
            Some(read) => json!({
                "processId": read.process_id,
                "content": read.content,
                "offset": read.offset,
                "process": read.process
            }),
            None => json!({
                "processId": process_id,
                "content": format!("Unknown background process: {process_id}"),
                "offset": offset,
                "error": "not_found"
            }),
        }
    }

    pub async fn process_stop(&self, process_id: String) -> Value {
        match self.tools.stop_process(&process_id) {
            Ok(process) => json!({
                "processId": process_id,
                "content": format!("Stopped background process {}", process.id),
                "offset": process.output_bytes,
                "process": process
            }),
            Err(err) => json!({
                "processId": process_id,
                "content": err.to_string(),
                "offset": 0,
                "error": err.to_string()
            }),
        }
    }

    pub async fn open_workspace(
        &self,
        path: String,
        session_id: Option<String>,
    ) -> anyhow::Result<Value> {
        let resolved = validate_workspace_directory(&path)?;
        let resolved_str = path_to_display_string(&resolved);
        let project_name = project_name_from_dir(&resolved);
        let _ = ensure_workspace_meta(&self.storage.config().data_dir, &resolved);
        let preference = build_workspace_preference_update(
            &self.config_get().await.unwrap_or_else(|_| json!({})),
            &resolved_str,
        );
        let _ = self.config_update(preference).await?;
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            self.bind_session_workspace(&session_id, &resolved).await?;
        }
        Ok(json!({
            "projectDir": resolved_str,
            "projectName": project_name
        }))
    }

    pub async fn clear_session_workspace(&self, session_id: String) -> anyhow::Result<Value> {
        let session_id = session_id.trim().to_string();
        if session_id.is_empty() {
            anyhow::bail!("sessionId is required");
        }
        self.session_workspaces.write().await.remove(&session_id);
        let fallback = unscoped_workspace_root();
        let fallback_str = path_to_string(&fallback);
        self.sessions
            .kernel()
            .isolation()
            .set(
                &session_id,
                xuanyu_protocol::SessionIsolationScope {
                    workspace_root: fallback_str.clone(),
                    allowed_paths: Vec::new(),
                    readonly: false,
                },
            )
            .await;
        self.storage
            .conversation_store()
            .update_session(
                &session_id,
                xuanyu_storage::ConversationSessionPatch {
                    metadata: Some(BTreeMap::from([
                        ("projectDir".to_string(), Value::Null),
                        ("workspace".to_string(), Value::Null),
                    ])),
                    ..Default::default()
                },
            )
            .await?;
        Ok(json!({ "sessionId": session_id }))
    }

    pub async fn create_workspace(
        &self,
        parent_dir: String,
        name: String,
        session_id: Option<String>,
    ) -> anyhow::Result<Value> {
        let parent = validate_workspace_directory(&parent_dir)?;
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Parent directory and project name are required");
        }
        if name == "." || name == ".." || name.chars().any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*') || ch.is_control()) {
            anyhow::bail!("Invalid project name: {name}");
        }
        let target = parent.join(name);
        if target.exists() {
            anyhow::bail!("Path already exists: {}", path_to_string(&target));
        }
        std::fs::create_dir(&target)?;
        self.open_workspace(path_to_string(&target), session_id).await
    }

    pub async fn workspace_file_reject(
        &self,
        session_id: String,
        turn_id: String,
        path: String,
    ) -> Value {
        let project_root = self.resolve_workspace_dir(Some(session_id.as_str())).await;
        let store = TurnLedgerStore::new(project_root);
        match store.reject_file_mutation(RejectFileMutationInput {
            session_id: &session_id,
            turn_id: &turn_id,
            file_path: &path,
        }) {
            Ok(result) => json!({
                "ok": result.conflicts.is_empty(),
                "sessionId": session_id,
                "turnId": turn_id,
                "path": path,
                "restored": result.restored,
                "deleted": result.deleted,
                "conflicts": result.conflicts,
            }),
            Err(err) => json!({
                "ok": false,
                "sessionId": session_id,
                "turnId": turn_id,
                "path": path,
                "error": err.to_string(),
            }),
        }
    }

    pub fn init_channel_plugins_list(&self) -> Value {
        // Channel adapters are built into the desktop engine; expose them as already available.
        let plugins = channel_manager::builtin_channel_adapters()
            .into_iter()
            .map(|adapter| {
                json!({
                    "id": format!("xuanyu-channel-{}", adapter.channel_type),
                    "name": adapter.label,
                    "version": "builtin",
                    "description": adapter.description,
                    "kind": "channel",
                    "channelType": adapter.channel_type,
                    "summary": adapter.setup_hint,
                    "installed": true,
                    "builtin": true,
                    "installedPath": Value::Null,
                })
            })
            .collect::<Vec<_>>();
        json!({ "plugins": plugins })
    }

    pub async fn init_channel_plugin_install(&self, plugin_id: String) -> Value {
        let plugin_id = plugin_id.trim().to_string();
        if plugin_id.is_empty() {
            return json!({ "ok": false, "error": "pluginId is required" });
        }
        let plugin = self
            .init_channel_plugins_list()
            .get("plugins")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .find(|item| {
                item.get("id").and_then(Value::as_str) == Some(plugin_id.as_str())
                    || item
                        .get("channelType")
                        .and_then(Value::as_str)
                        .is_some_and(|channel_type| {
                            plugin_id == channel_type
                                || plugin_id == format!("xuanyu-channel-{channel_type}")
                        })
            });
        if let Some(plugin) = plugin {
            tracing::info!(
                plugin_id = %plugin_id,
                "channel adapters are builtin; install is a no-op"
            );
            return json!({ "ok": true, "builtin": true, "plugin": plugin });
        }
        json!({
            "ok": false,
            "error": format!("Unknown channel adapter: {plugin_id}")
        })
    }

    pub async fn git_status(&self, session_id: Option<String>) -> Value {
        let cwd = self.resolve_workspace_dir(session_id.as_deref()).await;
        json!({ "status": git_status_snapshot_at(&cwd) })
    }

    pub async fn git_branches(&self, session_id: Option<String>) -> Value {
        let cwd = self.resolve_workspace_dir(session_id.as_deref()).await;
        match list_git_branches_at(&cwd) {
            Ok(branches) => json!({ "branches": branches }),
            Err(error) => json!({ "branches": [], "error": error }),
        }
    }

    pub async fn git_checkout(&self, branch: String, session_id: Option<String>) -> Value {
        let cwd = self.resolve_workspace_dir(session_id.as_deref()).await;
        match git_checkout_at(&cwd, &branch) {
            Ok((name, status)) => json!({ "ok": true, "branch": name, "status": status }),
            Err(error) => json!({ "ok": false, "error": error }),
        }
    }

    async fn bind_session_workspace(
        &self,
        session_id: &str,
        resolved: &Path,
    ) -> anyhow::Result<()> {
        let resolved_str = path_to_string(resolved);
        self.session_workspaces
            .write()
            .await
            .insert(session_id.to_string(), resolved.to_path_buf());
        self.sessions
            .kernel()
            .isolation()
            .set(
                session_id,
                xuanyu_protocol::SessionIsolationScope {
                    workspace_root: resolved_str.clone(),
                    allowed_paths: Vec::new(),
                    readonly: false,
                },
            )
            .await;
        self.storage
            .conversation_store()
            .update_session(
                session_id,
                xuanyu_storage::ConversationSessionPatch {
                    metadata: Some(BTreeMap::from([
                        ("projectDir".to_string(), json!(resolved_str)),
                        ("workspace".to_string(), json!(true)),
                    ])),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    async fn resolve_workspace_dir(&self, session_id: Option<&str>) -> PathBuf {
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            if let Some(path) = self.session_workspaces.read().await.get(session_id).cloned() {
                return path;
            }
            if let Ok(Some(record)) = self.storage.conversation_store().get_session(session_id).await {
                if let Some(dir) = record
                    .metadata
                    .get("projectDir")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    return PathBuf::from(dir);
                }
            }
        }
        if let Ok(config) = self.config_get().await {
            if let Some(dir) = config
                .get("workspace")
                .and_then(|value| value.get("lastDir"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return PathBuf::from(dir);
            }
            if let Some(dir) = project_dir_from_config(&config) {
                return dir;
            }
        }
        self.workspace_root.clone()
    }

    async fn active_goal_preload_tools(&self, session_id: &SessionId) -> Vec<String> {
        let root = self.resolve_workspace_dir(Some(session_id.0.as_str())).await;
        let store = session_goal::SessionGoalStore::new(root);
        match store.load(&session_id.0) {
            Some(goal) if goal.status == session_goal::SessionGoalStatus::Active => {
                vec!["update_goal".to_string()]
            }
            _ => Vec::new(),
        }
    }

    pub async fn agent_list(&self) -> Value {
        json!({ "agents": self.agents.list_agents().await })
    }

    pub async fn agent_create(&self, payload: Value) -> Value {
        match serde_json::from_value::<AgentInput>(payload) {
            Ok(input) => json!(self.agents.create_agent(input).await),
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        }
    }

    pub async fn agent_update(&self, id: String, payload: Value) -> Value {
        match serde_json::from_value::<AgentInput>(payload) {
            Ok(input) => json!(self.agents.update_agent(id, input).await),
            Err(err) => json!({ "ok": false, "id": id, "error": err.to_string() }),
        }
    }

    pub async fn agent_delete(&self, id: String) -> Value {
        json!(self.agents.delete_agent(id).await)
    }

    pub async fn team_status(&self, session_id: Option<String>) -> Value {
        let team = self.agents.team_status(session_id.clone()).await;
        json!({ "sessionId": session_id, "team": team })
    }

    pub async fn subagent_timeline(&self, session_id: Option<String>, task_id: String) -> Value {
        let (ledger, timeline) = self
            .agents
            .subagent_timeline(session_id.clone(), task_id.clone())
            .await;
        json!({
            "sessionId": session_id,
            "taskId": task_id,
            "ledger": ledger,
            "timeline": timeline
        })
    }

    pub async fn task_list(&self, session_id: Option<String>, limit: usize) -> Value {
        json!({ "tasks": self.agents.list_tasks(session_id, limit).await })
    }

    pub async fn task_create(&self, session_id: Option<String>, payload: Value) -> Value {
        let run_id = payload
            .get("runId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let auto_execute = payload
            .get("autoExecute")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let input_payload =
            normalize_subagent_task_payload(payload, session_id.as_deref(), run_id.as_deref());
        match serde_json::from_value::<xuanyu_agents::SubAgentTaskInput>(input_payload) {
            Ok(input) => match self
                .agents
                .submit_task(input, session_id.clone(), run_id)
                .await
            {
                Ok(task) => {
                    self.publish_team_agent_event("progress", &task).await;
                    if auto_execute {
                        self.spawn_subagent_execution(task.task_id.clone());
                    }
                    json!({
                        "ok": true,
                        "sessionId": session_id,
                        "task": task,
                        "autoExecute": auto_execute
                    })
                }
                Err(err) => json!({
                    "ok": false,
                    "sessionId": session_id,
                    "error": err.to_string()
                }),
            },
            Err(err) => json!({
                "ok": false,
                "sessionId": session_id,
                "error": err.to_string()
            }),
        }
    }

    pub async fn subagent_delegate_many(&self, payload: Value) -> Value {
        let session_id = payload
            .get("sessionId")
            .and_then(Value::as_str)
            .map(|id| id.to_string());
        let run_id = payload
            .get("runId")
            .and_then(Value::as_str)
            .map(|id| id.to_string());
        let inputs = payload
            .get("tasks")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        serde_json::from_value::<xuanyu_agents::SubAgentTaskInput>(item.clone()).ok()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if inputs.is_empty() {
            return json!({
                "ok": false,
                "reason": "No valid subagent task inputs were provided."
            });
        }
        let auto_execute = payload
            .get("autoExecute")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        match self
            .agents
            .delegate_many(inputs, session_id.clone(), run_id.clone())
            .await
        {
            Ok(ledgers) => {
                if auto_execute {
                    for ledger in &ledgers {
                        self.spawn_subagent_execution(ledger.task_id.clone());
                    }
                }
                json!({
                    "ok": true,
                    "sessionId": session_id,
                    "runId": run_id,
                    "tasks": ledgers,
                    "autoExecute": auto_execute
                })
            }
            Err(error) => json!({
                "ok": false,
                "reason": error.to_string()
            }),
        }
    }

    pub async fn subagent_assess_merge(&self, payload: Value) -> Value {
        let results = payload
            .get("results")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        serde_json::from_value::<SubAgentTaskResult>(item.clone()).ok()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let assessment = self.agents.assess_merge_results(&results);
        json!({
            "ok": true,
            "assessment": assessment
        })
    }

    fn spawn_subagent_execution(&self, task_id: String) {
        let agents = self.agents.clone();
        let execution = self.execution.clone();
        let agent_bridge = self.agent_bridge.clone();
        let tools = self.tools.clone();
        let runs = self.runs.clone();
        let observability = self.observability.clone();
        let eventbus = self.eventbus.clone();
        let permits = self.subagent_exec_permits.clone();
        tokio::spawn(async move {
            let Ok(_permit) = permits.acquire().await else {
                observability
                    .log(
                        "warn",
                        &format!("Sub-agent task {task_id} skipped: concurrency limiter closed"),
                    )
                    .await;
                return;
            };
            if let Err(err) = execute_subagent_task(
                agents,
                execution,
                agent_bridge,
                tools,
                runs,
                eventbus,
                task_id.clone(),
            )
            .await
            {
                observability
                    .log(
                        "warn",
                        &format!("Sub-agent task {task_id} execution failed: {err}"),
                    )
                    .await;
            }
        });
    }

    async fn recover_subagent_execution_queue(&self) -> usize {
        let mut recovery_done = self.subagent_startup_recovery_done.write().await;
        if *recovery_done {
            return 0;
        }
        *recovery_done = true;
        drop(recovery_done);

        let tasks = self
            .agents
            .list_tasks(None, STARTUP_SUBAGENT_RECOVERY_LIMIT)
            .await;
        let mut recovered = 0usize;
        for task in tasks {
            match task.status.as_str() {
                "queued" => {
                    self.spawn_subagent_execution(task.task_id.clone());
                    recovered += 1;
                }
                "running" => {
                    let result = self
                        .agents
                        .control_task(
                            task.task_id.clone(),
                            "retry".to_string(),
                            Some(
                                "Recovered stale running SubAgent task after app restart"
                                    .to_string(),
                            ),
                        )
                        .await;
                    if result.get("ok").and_then(Value::as_bool) == Some(true)
                        && result
                            .get("task")
                            .and_then(|task| task.get("status"))
                            .and_then(Value::as_str)
                            == Some("queued")
                    {
                        self.spawn_subagent_execution(task.task_id.clone());
                        recovered += 1;
                    } else {
                        self.observability
                            .log(
                                "warn",
                                &format!(
                                    "Failed to recover stale sub-agent task {}: {}",
                                    task.task_id,
                                    result
                                        .get("reason")
                                        .and_then(Value::as_str)
                                        .unwrap_or("unknown recovery error")
                                ),
                            )
                            .await;
                    }
                }
                _ => {}
            }
        }
        recovered
    }

    fn spawn_subagent_tasks_from_tool_output(&self, tool_name: &str, output: &ToolOutput) {
        if tool_name != "submit_task" || output.is_error {
            return;
        }
        let task_ids = output
            .data
            .as_ref()
            .and_then(|data| data.get("taskIds"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        for task_id in task_ids {
            self.spawn_subagent_execution(task_id);
        }
    }

    pub async fn task_status(&self, task_id: String) -> Value {
        json!({ "task": self.agents.task_status(task_id).await })
    }

    pub async fn task_control(
        &self,
        task_id: String,
        action: String,
        reason: Option<String>,
    ) -> Value {
        let result = self
            .agents
            .control_task(task_id.clone(), action.clone(), reason)
            .await;
        if matches!(action.as_str(), "resume" | "retry")
            && result.get("ok").and_then(Value::as_bool) == Some(true)
            && result
                .get("task")
                .and_then(|task| task.get("status"))
                .and_then(Value::as_str)
                == Some("queued")
        {
            self.spawn_subagent_execution(task_id);
        }
        result
    }

    async fn cancel_subagent_tasks_after(&self, session_id: &str, cutoff_ms: i64) -> usize {
        let tasks = self
            .agents
            .list_tasks(Some(session_id.to_string()), 200)
            .await;
        let mut cancelled = 0usize;
        for task in tasks {
            if task.started_at < cutoff_ms {
                continue;
            }
            if matches!(task.status.as_str(), "completed" | "failed" | "cancelled") {
                continue;
            }
            let result = self
                .agents
                .control_task(
                    task.task_id.clone(),
                    "cancel".to_string(),
                    Some("Cancelled by session rewind".to_string()),
                )
                .await;
            if result.get("ok").and_then(Value::as_bool) == Some(true) {
                cancelled += 1;
            }
        }
        cancelled
    }

    pub fn memory(&self) -> Arc<MemoryService> {
        self.memory.clone()
    }

    fn spawn_memory_ingest_batch(&self, batch: MemoryIngestBatch) {
        let MemoryIngestBatch {
            entries,
            source,
            session_id,
            task_id,
            run_id,
            reason,
            confidence,
        } = batch;
        if entries.is_empty() {
            return;
        }
        let core = self.clone();
        let source = source.to_string();
        let inputs = entries
            .into_iter()
            .map(|entry| MemoryIngestInput {
                entry,
                source: source.clone(),
                confidence,
                reason: reason.clone(),
                session_id: session_id.clone(),
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                trace_id: None,
            })
            .collect::<Vec<_>>();
        tokio::spawn(async move {
            let embedding_provider = core.current_memory_embedding_provider().await;
            let core_for_stored = core.clone();
            let core_for_complete = core.clone();
            MemoryIngress::spawn_batch(
                core.memory.clone(),
                inputs,
                move |stored| {
                    core_for_stored.schedule_memory_embedding_refresh_with_provider(
                        stored,
                        embedding_provider.clone(),
                    );
                },
                move || {
                    let core = core_for_complete.clone();
                    tokio::spawn(async move {
                        let total = core.memory.stats().await.total_entries;
                        core.eventbus
                            .publish(AppEvent::MemoryUpdated { total_entries: total });
                    });
                },
            );
        });
    }

    fn enqueue_policy_memory_candidates(
        &self,
        session_id: &SessionId,
        run_id: &str,
        candidates: &[AgentPolicyMemoryCandidate],
    ) {
        if candidates.is_empty() {
            return;
        }
        let entries = candidates
            .iter()
            .filter_map(|candidate| {
                let content = candidate.content.trim();
                if content.is_empty() {
                    return None;
                }
                let category = candidate
                    .category
                    .clone()
                    .unwrap_or_else(|| "preference".to_string());
                let importance = candidate.importance.unwrap_or(5) as f32 / 10.0;
                Some(NewMemoryEntry {
                    kind: kind_for_category(&category),
                    category,
                    content: content.to_string(),
                    tags: vec!["agent_policy".to_string()],
                    context: Some(session_id.0.clone()),
                    importance,
                })
            })
            .collect::<Vec<_>>();
        self.spawn_memory_ingest_batch(MemoryIngestBatch {
            entries,
            source: "agent_policy".to_string(),
            session_id: Some(session_id.0.clone()),
            task_id: None,
            run_id: Some(run_id.to_string()),
            reason: "Suggested by Python agent policy.".to_string(),
            confidence: Some(0.75),
        });
    }

    async fn publish_initial_run_plan(
        &self,
        session_id: &SessionId,
        run_id: &str,
        goal: &str,
        intent: &str,
    ) -> anyhow::Result<()> {
        if should_maintain_run_plan_for_intent(intent) {
            let plan = create_initial_run_plan(goal, intent);
            self.runs.set_run_plan(run_id, plan.clone()).await?;
            self.runs
                .update_run_todo(run_id, "understand", "completed")
                .await?;
            self.runs
                .update_run_todo(run_id, "execute", "running")
                .await?;
            self.eventbus.publish(AppEvent::RunPlan {
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                plan: Some(plan),
            });
        } else {
            self.eventbus.publish(AppEvent::RunPlan {
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                plan: None,
            });
        }
        Ok(())
    }

    async fn build_key_facts_context_for_turn(
        &self,
        messages: &[ChatMessage],
        user_message: &str,
    ) -> Option<String> {
        let heuristic = key_facts_context_from_messages(messages, 24);
        let seed = {
            let mut parts = Vec::new();
            for message in messages.iter().rev().take(8).rev() {
                let role = match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::System => "system",
                    _ => "other",
                };
                parts.push(format!(
                    "{role}: {}",
                    compact_context_line(&message.content, 400)
                ));
            }
            if parts.is_empty() {
                parts.push(format!("user: {}", compact_context_line(user_message, 800)));
            }
            parts.join("\n")
        };
        let llm = tokio::time::timeout(
            Duration::from_secs(3),
            self.execution.generate_for_task(
                ModelTaskKind::KeyFacts,
                &seed,
                Some(AUX_KEY_FACTS_SYSTEM_PROMPT),
            ),
        )
        .await;
        let Ok(response) = llm else {
            return heuristic;
        };
        if response.error.is_some() || response.text.trim().is_empty() {
            return heuristic;
        }
        let text = response.text.trim();
        let json_text = text
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        let Ok(value) = serde_json::from_str::<Value>(json_text) else {
            return heuristic;
        };
        let Some(facts) = value.get("facts").and_then(Value::as_array) else {
            return heuristic;
        };
        let mut lines = vec![
            "[Locked key facts]".to_string(),
            "Preserve these facts across compaction and request trimming. If a new instruction conflicts with one of them, explicitly resolve the conflict with the user or the newer source.".to_string(),
        ];
        let mut count = 0usize;
        for fact in facts.iter().take(24) {
            let kind = fact
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("fact")
                .trim();
            let text = fact
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if kind.is_empty() || text.is_empty() {
                continue;
            }
            lines.push(format!("- {kind}: {text}"));
            count += 1;
        }
        if count == 0 {
            return heuristic;
        }
        lines.push("[End locked key facts]".to_string());
        Some(lines.join("\n"))
    }

    pub fn observability(&self) -> Arc<Observability> {
        self.observability.clone()
    }

    async fn build_turn_active_context_sections(
        &self,
        session_id: &str,
        policy: &Value,
        user_message: &str,
        config: &Value,
    ) -> Option<String> {
        let project_dir = project_dir_from_config(config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let cwd = self
            .tools
            .workspace_roots()
            .first()
            .cloned()
            .unwrap_or_else(|| project_dir.clone());
        let mut blocks = Vec::new();

        if should_inject_context_section(policy, "memory_profile") {
            match self.build_memory_profile_context_text().await {
                Some(text) => blocks.push(format!("[Memory profile]\n{text}")),
                None if should_refresh_memory_profile(policy) => {
                    blocks.push(
                        "Memory profile refresh: no explicit profile content is stored yet."
                            .to_string(),
                    );
                }
                None => {}
            }
        }

        if should_inject_context_section(policy, "project_instructions") {
            if let Some(text) = project_instructions_context_text(&cwd) {
                blocks.push(text);
            }
        }

        if should_inject_context_section(policy, "skills") {
            let extension_config = ExtensionLoadConfig::from_app_config(project_dir.clone(), config);
            let extensions = load_extensions(&extension_config);
            let skill_lines = format_skills_for_prompt(
                &extensions.skills,
                Some(user_message),
                12,
                false,
            );
            if !skill_lines.is_empty() {
                blocks.push(skill_lines.join("\n"));
            }
        }

        if let Some(todo_block) = todos_prompt_block(&project_dir, session_id) {
            blocks.push(todo_block);
        }

        if blocks.is_empty() {
            None
        } else {
            Some(blocks.join("\n\n"))
        }
    }

    async fn context_compact_snapshots(&self, session_id: &str) -> Vec<MemorySnapshotDto> {
        let mut snapshots = self
            .memory
            .list_snapshots()
            .await
            .into_iter()
            .filter(|snapshot| {
                context_compact_snapshot_matches_session(&snapshot.reason, session_id)
            })
            .map(memory_snapshot_from_info)
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|s| std::cmp::Reverse(s.version));
        snapshots
    }

    async fn latest_context_compact_snapshot(&self, session_id: &str) -> Option<MemorySnapshotDto> {
        if let Some(snapshot) = self
            .context_compact_snapshots(session_id)
            .await
            .into_iter()
            .next()
        {
            return Some(snapshot);
        }
        self.memory
            .list_snapshots()
            .await
            .into_iter()
            .filter(|snapshot| context_compact_snapshot_reason_unscoped(&snapshot.reason))
            .map(memory_snapshot_from_info)
            .max_by_key(|snapshot| snapshot.version)
    }

    async fn context_compact_should_snapshot(&self, memories: &[NewMemoryEntry]) -> bool {
        for memory in memories {
            match classify_memory_write(&memory.content, &memory.category, "context_compact") {
                MemoryWriteDecision::Deny | MemoryWriteDecision::Pending => continue,
                MemoryWriteDecision::Direct => {}
            }
            let conflicts = self.memory.detect_conflicts(memory).await;
            if conflicts
                .iter()
                .any(|conflict| conflict.resolution == MemoryConflictResolution::Pending)
            {
                continue;
            }
            if conflicts.first().is_some_and(|conflict| {
                conflict.resolution == MemoryConflictResolution::KeepExisting
            }) {
                continue;
            }
            return true;
        }
        false
    }

}

fn context_compact_snapshot_reason(session_id: &str, strategy: &str) -> String {
    format!(
        "{CONTEXT_COMPACT_SNAPSHOT_PREFIX}{}:{}",
        session_id.trim(),
        strategy.trim()
    )
}

fn context_compact_snapshot_matches_session(reason: &str, session_id: &str) -> bool {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return false;
    }
    let Some(rest) = reason.strip_prefix(CONTEXT_COMPACT_SNAPSHOT_PREFIX) else {
        return false;
    };
    rest == session_id || rest.starts_with(&format!("{session_id}:"))
}

fn context_compact_snapshot_reason_unscoped(reason: &str) -> bool {
    reason == "context_compact"
}

fn worker_health_from_agent_status(status: AgentWorkerStatus) -> WorkerHealth {
    let worker_status = match status.state {
        AgentWorkerState::Healthy => WorkerStatus::Healthy,
        AgentWorkerState::Degraded => WorkerStatus::Degraded,
        AgentWorkerState::Unavailable => WorkerStatus::Unavailable,
    };
    WorkerHealth {
        name: "agent-python".to_string(),
        status: worker_status,
        last_seen_ms: if status.state == AgentWorkerState::Healthy {
            Some(Utc::now().timestamp_millis())
        } else {
            None
        },
        detail: Some(status.detail),
    }
}

fn normalize_subagent_task_payload(
    mut payload: Value,
    session_id: Option<&str>,
    run_id: Option<&str>,
) -> Value {
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    let control_plane_task_id = object_string_any(
        object,
        &["controlPlaneTaskId", "control_plane_task_id", "taskId"],
    );
    let control_plane_run_id = object_string_any(
        object,
        &[
            "controlPlaneRunId",
            "control_plane_run_id",
            "runId",
            "run_id",
        ],
    )
    .or_else(|| run_id.map(str::to_string));
    let control_plane_session_id = object_string_any(
        object,
        &[
            "controlPlaneSessionId",
            "control_plane_session_id",
            "sessionId",
            "session_id",
        ],
    )
    .or_else(|| session_id.map(str::to_string));
    let metadata = object
        .entry("metadata".to_string())
        .or_insert_with(|| json!({}));
    if !metadata.is_object() {
        *metadata = json!({});
    }
    if let Some(metadata) = metadata.as_object_mut() {
        insert_metadata_if_missing(metadata, "controlPlaneTaskId", control_plane_task_id);
        insert_metadata_if_missing(metadata, "controlPlaneRunId", control_plane_run_id);
        insert_metadata_if_missing(metadata, "controlPlaneSessionId", control_plane_session_id);
    }
    payload
}

fn object_string_any(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn insert_metadata_if_missing(
    metadata: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<String>,
) {
    if metadata.contains_key(key) {
        return;
    }
    if let Some(value) = value {
        metadata.insert(key.to_string(), json!(value));
    }
}

fn doctor_check(
    name: impl Into<String>,
    status: DoctorCheckStatus,
    detail: impl Into<String>,
    suggestion: Option<&str>,
) -> DoctorCheckDto {
    DoctorCheckDto {
        name: name.into(),
        status,
        detail: detail.into(),
        suggestion: suggestion.map(str::to_string),
    }
}

fn provider_config_doctor_check(config: &Value) -> DoctorCheckDto {
    let provider = json_string_at(config, &["provider", "type"])
        .or_else(|| json_string_at(config, &["provider", "provider"]))
        .unwrap_or_else(|| "openai-compatible".to_string());
    let model = json_string_at(config, &["provider", "model"]).unwrap_or_else(|| "default".into());
    let needs_key = provider_needs_api_key(&provider, config);
    let has_key = json_string_at(config, &["provider", "apiKey"]).is_some()
        || provider_api_env_candidates(&provider)
            .iter()
            .any(|name| env_value_configured(name));
    let status = if needs_key && !has_key {
        DoctorCheckStatus::Warn
    } else {
        DoctorCheckStatus::Ok
    };
    doctor_check(
        "Provider config",
        status,
        format!(
            "{provider}/{model} {}",
            if has_key {
                "API key configured"
            } else if needs_key {
                "no API key in config/env"
            } else {
                "local/no-key provider"
            }
        ),
        (status == DoctorCheckStatus::Warn)
            .then_some("Set provider.apiKey or XY_API_KEY/provider-specific API key env."),
    )
}

fn memory_backend_doctor_check(memory: &MemoryStoreHealthDto) -> DoctorCheckDto {
    let backend = memory.backend.trim().to_ascii_lowercase();
    let (status, detail, suggestion) = match backend.as_str() {
        "sqlite" | "local" | "file" => (
            DoctorCheckStatus::Ok,
            format!("{} local desktop memory store", memory.backend),
            None,
        ),
        "postgres" | "postgresql" if memory.ok && memory.pgvector == Some(true) => (
            DoctorCheckStatus::Ok,
            format!(
                "PostgreSQL/pgvector memory active ({})",
                memory
                    .location
                    .as_deref()
                    .map(summarize_db_location)
                    .unwrap_or_else(|| "connected".to_string())
            ),
            None,
        ),
        "postgres" | "postgresql" if memory.postgres_configured => (
            DoctorCheckStatus::Warn,
            "PostgreSQL is configured but the live memory store is not attached.".to_string(),
            Some("Restart the local engine after Postgres/pgvector is ready."),
        ),
        "postgres" | "postgresql" => (
            DoctorCheckStatus::Warn,
            "memory.backend=postgres but no connectionString is configured.".to_string(),
            Some("Add memory.postgres.connectionString in storage settings."),
        ),
        _ => (
            DoctorCheckStatus::Warn,
            format!("Unknown memory backend '{}'.", memory.backend),
            Some("Open storage settings and choose postgres or a supported local backend."),
        ),
    };
    doctor_check("Memory backend", status, detail, suggestion)
}

fn conversation_store_doctor_check(conversation: &ConversationStoreHealthDto) -> DoctorCheckDto {
    doctor_check(
        "Conversation store",
        if conversation.ok {
            DoctorCheckStatus::Ok
        } else {
            DoctorCheckStatus::Fail
        },
        format!(
            "{} {}{}",
            conversation.backend,
            conversation.location,
            conversation
                .error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default()
        ),
        (!conversation.ok).then_some("Check storage configuration or rerun initialization."),
    )
}

fn channel_summary_from_config(config: &Value) -> ChannelSummaryDto {
    let channels = config
        .get("channels")
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .or_else(|| config.get("channels").and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(channel_status_from_config_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let enabled = channels.iter().filter(|channel| channel.enabled).count();
    let ready = channels
        .iter()
        .filter(|channel| channel.status == "ready")
        .count();
    let needs_config = channels
        .iter()
        .filter(|channel| channel.enabled && channel.status == "needs_config")
        .count();
    let unsupported = channels
        .iter()
        .filter(|channel| channel.enabled && channel.status == "unsupported")
        .count();

    ChannelSummaryDto {
        total: channels.len(),
        enabled,
        ready,
        needs_config,
        unsupported,
        channels,
    }
}

fn channel_status_from_config_value(value: &Value) -> Option<ChannelStatusDto> {
    let object = value.as_object()?;
    let channel_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("custom")
        .to_ascii_lowercase();
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| channel_type.clone());
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let token = object
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let app_id = object
        .get("appId")
        .or_else(|| object.get("app_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let token_required = channel_token_required(&channel_type);
    let app_id_required = matches!(channel_type.as_str(), "feishu" | "lark");
    let configured = if app_id_required {
        !app_id.is_empty()
            && !token.is_empty()
            && channel_token_valid(&channel_type, token)
    } else if token_required {
        channel_token_valid(&channel_type, token)
    } else {
        token.is_empty() || channel_token_valid(&channel_type, token)
    };
    let mut missing = Vec::new();
    if app_id_required && app_id.is_empty() {
        missing.push("appId".to_string());
    }
    if token_required && token.is_empty() {
        missing.push(if matches!(channel_type.as_str(), "feishu" | "lark") {
            "token (app_secret)".to_string()
        } else if channel_type == "discord" {
            "token (Discord Bot Token)".to_string()
        } else {
            "token".to_string()
        });
    } else if token_required && !token.is_empty() && !channel_token_valid(&channel_type, token) {
        missing.push("validToken".to_string());
    }
    let adapter_supported = channel_adapter_supported(&channel_type);
    let status = if !enabled {
        "disabled"
    } else if !configured {
        "needs_config"
    } else if !adapter_supported {
        "unsupported"
    } else {
        "ready"
    };
    let allowlist_count = object
        .get("allowlist")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let mut extra_keys = object
        .get("extra")
        .and_then(Value::as_object)
        .map(|extra| extra.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    extra_keys.sort();

    Some(ChannelStatusDto {
        channel_type: channel_type.clone(),
        name,
        label: channel_label(&channel_type).to_string(),
        enabled,
        configured,
        adapter_supported,
        status: status.to_string(),
        mention_only: object
            .get("mentionOnly")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        allowlist_count,
        missing,
        extra_keys,
    })
}

fn channel_label(channel_type: &str) -> &'static str {
    match channel_type {
        "discord" => "Discord",
        "telegram" => "Telegram",
        "slack" => "Slack",
        "wechat" => "WeChat",
        "dingtalk" => "DingTalk",
        "feishu" => "Feishu",
        "whatsapp" => "WhatsApp",
        "line" => "LINE",
        "signal" => "Signal",
        "custom" => "Custom Webhook",
        _ => "Custom Webhook",
    }
}

fn channel_adapter_supported(channel_type: &str) -> bool {
    matches!(
        channel_type,
        "discord"
            | "telegram"
            | "feishu"
            | "lark"
            | "wechat"
            | "qq"
            | "dingtalk"
            | "whatsapp"
            | "line"
    )
}

fn channel_token_required(channel_type: &str) -> bool {
    !matches!(channel_type, "signal" | "custom")
}

fn channel_token_valid(channel_type: &str, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    match channel_type {
        "telegram" => {
            let Some((bot_id, secret)) = token.split_once(':') else {
                return false;
            };
            bot_id.chars().all(|ch| ch.is_ascii_digit())
                && secret.len() >= 20
                && secret
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        }
        "discord" => {
            token.len() >= 40
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
        }
        "slack" => token.starts_with("xoxb-") || token.starts_with("xapp-"),
        "feishu" | "lark" => token.len() >= 8
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
        "dingtalk" | "wechat" => token.len() > 10,
        "whatsapp" => token.starts_with("EAA"),
        "line" => token.len() > 20,
        "signal" | "custom" => true,
        _ => token.len() > 8,
    }
}

fn gateway_doctor_check(gateway: &GatewayStatusDto) -> DoctorCheckDto {
    let ipc = gateway.bind.eq_ignore_ascii_case("electron-ipc")
        || gateway.bind.eq_ignore_ascii_case("tauri-ipc")
        || gateway.bind.eq_ignore_ascii_case("desktop-ipc");
    let status = if !gateway.enabled || ipc || gateway.port > 0 {
        DoctorCheckStatus::Ok
    } else {
        DoctorCheckStatus::Warn
    };
    doctor_check(
        "Gateway runtime",
        status,
        format!(
            "enabled={} bind={} port={} clients={}",
            gateway.enabled,
            gateway.bind,
            gateway.port,
            gateway.ws_clients.unwrap_or_default()
        ),
        (status == DoctorCheckStatus::Warn)
            .then_some("Use electron-ipc bind or configure a non-zero gateway port."),
    )
}

fn channel_summary_doctor_check(channels: &ChannelSummaryDto) -> DoctorCheckDto {
    let status = if channels.needs_config > 0 || channels.unsupported > 0 {
        DoctorCheckStatus::Warn
    } else {
        DoctorCheckStatus::Ok
    };
    let suggestion = if channels.needs_config > 0 {
        Some("Complete token/config fields for enabled delivery channels.")
    } else if channels.unsupported > 0 {
        Some("Only Discord, Telegram, and Feishu have native adapters in the current desktop runtime.")
    } else {
        None
    };
    doctor_check(
        "Delivery channels",
        status,
        format!(
            "total={} enabled={} ready={} needsConfig={} unsupported={}",
            channels.total,
            channels.enabled,
            channels.ready,
            channels.needs_config,
            channels.unsupported
        ),
        suggestion,
    )
}

fn worker_doctor_check(worker: &WorkerHealth) -> DoctorCheckDto {
    let status = match worker.status {
        WorkerStatus::Healthy => DoctorCheckStatus::Ok,
        WorkerStatus::Starting
        | WorkerStatus::Degraded
        | WorkerStatus::Stopped
        | WorkerStatus::Unavailable => DoctorCheckStatus::Warn,
    };
    doctor_check(
        format!("Worker: {}", worker.name),
        status,
        worker
            .detail
            .clone()
            .unwrap_or_else(|| format!("worker status {}", worker_status_text(worker.status))),
        (status == DoctorCheckStatus::Warn)
            .then_some("Configure this adapter in init/config if the feature is needed."),
    )
}

fn security_doctor_check(security: &SecurityStatusDto) -> DoctorCheckDto {
    let findings = security.sensitive_findings.len();
    let warnings = security.warnings.len();
    doctor_check(
        "Security scan",
        if findings > 0 || warnings > 0 {
            DoctorCheckStatus::Warn
        } else {
            DoctorCheckStatus::Ok
        },
        if findings > 0 || warnings > 0 {
            format!("{findings} sensitive finding(s), {warnings} warning(s)")
        } else {
            "No sensitive config findings.".to_string()
        },
        (findings > 0 || warnings > 0)
            .then_some("Move secrets into the config vault or environment variables."),
    )
}

fn provider_needs_api_key(provider: &str, config: &Value) -> bool {
    if provider_base_url_is_local(config) {
        return false;
    }
    !matches!(
        normalized_provider_type(provider).as_str(),
        "ollama"
            | "llama-cpp"
            | "llama.cpp"
            | "llamacpp"
            | "opencode"
            | "local"
            | "desktop-preview"
    )
}

fn provider_base_url_is_local(config: &Value) -> bool {
    json_string_at(config, &["provider", "baseUrl"])
        .map(|base_url| {
            let lower = base_url.to_ascii_lowercase();
            lower.contains("localhost")
                || lower.contains("127.0.0.1")
                || lower.contains("[::1]")
                || lower.starts_with("http://0.0.0.0")
        })
        .unwrap_or(false)
}

fn provider_api_env_candidates(provider: &str) -> Vec<&'static str> {
    let mut candidates = vec!["XY_API_KEY"];
    match normalized_provider_type(provider).as_str() {
        "anthropic" => candidates.push("ANTHROPIC_API_KEY"),
        "deepseek" => candidates.push("DEEPSEEK_API_KEY"),
        "google" | "gemini" => {
            candidates.push("GOOGLE_API_KEY");
            candidates.push("GEMINI_API_KEY");
        }
        "longcat" => candidates.push("LONGCAT_API_KEY"),
        "mimo" => candidates.push("MIMO_API_KEY"),
        "moonshot" | "kimi" => candidates.push("MOONSHOT_API_KEY"),
        "openrouter" => candidates.push("OPENROUTER_API_KEY"),
        "zhipu" | "zhipuai" => {
            candidates.push("ZHIPU_API_KEY");
            candidates.push("ZHIPUAI_API_KEY");
        }
        _ => candidates.push("OPENAI_API_KEY"),
    }
    candidates
}

fn env_value_configured(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn normalized_provider_type(provider: &str) -> String {
    provider.trim().replace('_', "-").to_ascii_lowercase()
}

fn worker_status_text(status: WorkerStatus) -> &'static str {
    match status {
        WorkerStatus::Starting => "starting",
        WorkerStatus::Healthy => "healthy",
        WorkerStatus::Degraded => "degraded",
        WorkerStatus::Stopped => "stopped",
        WorkerStatus::Unavailable => "unavailable",
    }
}

fn model_capability_tasks() -> [ModelTaskKind; 9] {
    [
        ModelTaskKind::Main,
        ModelTaskKind::Title,
        ModelTaskKind::Summary,
        ModelTaskKind::Memory,
        ModelTaskKind::Intent,
        ModelTaskKind::KeyFacts,
        ModelTaskKind::Planning,
        ModelTaskKind::Implementation,
        ModelTaskKind::Review,
    ]
}

fn model_task_key(task: ModelTaskKind) -> &'static str {
    match task {
        ModelTaskKind::Main => "main",
        ModelTaskKind::Title => "title",
        ModelTaskKind::Summary => "summary",
        ModelTaskKind::Memory => "memory",
        ModelTaskKind::Intent => "intent",
        ModelTaskKind::KeyFacts => "keyfacts",
        ModelTaskKind::Planning => "planning",
        ModelTaskKind::Implementation => "implementation",
        ModelTaskKind::Review => "review",
    }
}

fn model_capability_route_payload(
    route: &ResolvedModelRoute,
    rate_limit: ModelRateLimitSettings,
) -> Value {
    let mut warnings = Vec::new();
    if route.inherited && route.task != ModelTaskKind::Main {
        warnings.push("Route inherits the main model configuration.".to_string());
    }
    if let Some(fallback) = route.fallback.as_deref() {
        warnings.push(format!(
            "Fallback route configured: {}/{}.",
            fallback.provider.provider_type, fallback.provider.model
        ));
    }
    json!({
        "task": model_task_key(route.task),
        "provider": route.provider.provider_type,
        "model": route.provider.model,
        "capabilities": {
            "provider": route.provider.provider_type,
            "model": route.provider.model,
            "stream": true,
            "toolCall": route.capabilities.tool_calling,
            "reasoning": false,
            "thinkingDisable": route.capabilities.thinking_disable,
            "jsonMode": route.capabilities.json_mode,
            "usage": false,
            "vision": route.capabilities.vision,
            "maxOutput": route.provider.max_tokens,
            "rateLimit": {
                "rpm": rate_limit.rpm,
                "concurrency": rate_limit.concurrency
            },
            "cost": Value::Null,
            "source": ["rust-execution"],
            "warnings": warnings
        },
        "check": {
            "ok": route.compatibility.compatible,
            "warnings": warnings,
            "errors": route.compatibility.reasons
        }
    })
}

fn model_route_status_detail(route: &ResolvedModelRoute) -> String {
    let provider = &route.provider;
    let mut parts = vec![format!(
        "main route {}/{}",
        provider.provider_type, provider.model
    )];
    if let Some(protocol) = provider
        .protocol
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("protocol {protocol}"));
    }
    if provider
        .base_url
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_some()
    {
        parts.push("baseUrl configured".to_string());
    }
    let mut capabilities = Vec::new();
    if route.capabilities.tool_calling {
        capabilities.push("tools");
    }
    if route.capabilities.json_mode {
        capabilities.push("json");
    }
    if route.capabilities.vision {
        capabilities.push("vision");
    }
    if route.capabilities.thinking_disable {
        capabilities.push("thinking-toggle");
    }
    if capabilities.is_empty() {
        capabilities.push("chat");
    }
    parts.push(format!("capabilities {}", capabilities.join(", ")));
    if let Some(fallback) = route.fallback.as_deref() {
        parts.push(format!(
            "fallback {}/{}",
            fallback.provider.provider_type, fallback.provider.model
        ));
    }
    if route.compatibility.compatible {
        parts.push("compatible".to_string());
    } else if route.compatibility.reasons.is_empty() {
        parts.push("incompatible route".to_string());
    } else {
        parts.push(format!(
            "incompatible: {}",
            route.compatibility.reasons.join("; ")
        ));
    }
    parts.join("; ")
}

struct SubAgentControlStop {
    status: String,
    reason: String,
}

async fn subagent_control_stop_signal(
    agents: &AgentRuntime,
    task_id: &str,
) -> Option<SubAgentControlStop> {
    let task = agents.task_status(task_id.to_string()).await?;
    match task.status.as_str() {
        "cancelled" | "blocked" => {
            let reason = task
                .watchdog
                .blocked_reason
                .clone()
                .or_else(|| task.result.as_ref().and_then(|result| result.error.clone()))
                .unwrap_or_else(|| format!("Sub-agent task is {}.", task.status));
            Some(SubAgentControlStop {
                status: task.status,
                reason,
            })
        }
        _ => None,
    }
}

async fn finish_subagent_run_if_controlled(
    agents: &AgentRuntime,
    runs: &RunStore,
    task_id: &str,
    run_id: &str,
    step_id: &str,
) -> anyhow::Result<bool> {
    let Some(stop) = subagent_control_stop_signal(agents, task_id).await else {
        return Ok(false);
    };
    runs.record_event(
        run_id,
        "agent.execution.stopped",
        Some(json!({
            "taskId": task_id,
            "status": stop.status,
            "reason": stop.reason
        })),
        Some(step_id.to_string()),
    )
    .await?;
    runs.fail_step(step_id, stop.reason.clone()).await?;
    runs.cancel_run(run_id, Some(stop.reason)).await?;
    Ok(true)
}

fn default_subagent_execution_cwd(tools: &ToolRuntime) -> PathBuf {
    tools
        .workspace_roots()
        .first()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn subagent_sandbox_mode_enabled(definition: &AgentDefinition) -> bool {
    definition
        .default_sandbox_mode
        .as_deref()
        .unwrap_or_default()
        .contains("worktree")
}

struct SubagentSandboxCleanup {
    project_dir: PathBuf,
    sandbox_id: Option<String>,
}

impl Drop for SubagentSandboxCleanup {
    fn drop(&mut self) {
        let Some(sandbox_id) = self.sandbox_id.take() else {
            return;
        };
        if let Err(err) = xuanyu_tools::record_sandbox_diff(&self.project_dir, &sandbox_id) {
            tracing::warn!(
                sandbox_id = %sandbox_id,
                error = %err,
                "subagent sandbox diff failed"
            );
        }
        if let Err(err) = xuanyu_tools::dispose_sandbox(&self.project_dir, &sandbox_id) {
            tracing::warn!(
                sandbox_id = %sandbox_id,
                error = %err,
                "subagent sandbox dispose failed"
            );
        }
    }
}

fn integrate_subagent_sandbox(
    project_dir: &Path,
    sandbox_id: &str,
    modified_files: &[String],
    artifacts: &[String],
    write_scope: &[String],
) -> Option<SandboxMaterializeResult> {
    let files = normalize_relative_file_list(
        artifacts
            .iter()
            .chain(modified_files.iter())
            .cloned()
            .collect(),
    );
    match materialize_sandbox_files_with_auto_retry(
        project_dir,
        sandbox_id,
        SandboxMaterializeOptions {
            files: if files.is_empty() { None } else { Some(files) },
            write_scope: if write_scope.is_empty() {
                None
            } else {
                Some(write_scope.to_vec())
            },
            conflict_policy: None,
            force_files: None,
        },
    ) {
        Ok(result) => Some(result),
        Err(err) => {
            tracing::warn!(
                sandbox_id,
                error = %err,
                "subagent sandbox materialize rejected"
            );
            None
        }
    }
}

async fn ensure_subagent_sandbox(
    tools: &ToolRuntime,
    definition: &AgentDefinition,
    ledger: &SubAgentTaskLedger,
    runs: &RunStore,
    run_id: &str,
    step_id: &str,
    fallback_cwd: &Path,
) -> (Option<String>, PathBuf) {
    if !subagent_sandbox_mode_enabled(definition) {
        return (None, fallback_cwd.to_path_buf());
    }
    let name = format!(
        "{}-{}",
        definition.id,
        ledger
            .task_id
            .chars()
            .take(8)
            .collect::<String>()
    );
    let output = tools
        .execute(ToolCall {
            name: "sandbox_create".to_string(),
            args: BTreeMap::from([
                ("name".to_string(), json!(name)),
                ("baseRef".to_string(), json!("HEAD")),
            ]),
            cwd: fallback_cwd.to_path_buf(),
        })
        .await;
    if output.is_error {
        let _ = runs
            .record_event(
                run_id,
                "agent.sandbox.failed",
                Some(json!({
                    "taskId": ledger.task_id,
                    "agentId": definition.id,
                    "error": output.content
                })),
                Some(step_id.to_string()),
            )
            .await;
        return (None, fallback_cwd.to_path_buf());
    }
    let sandbox_id = output
        .data
        .as_ref()
        .and_then(|data| data.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let worktree = output
        .data
        .as_ref()
        .and_then(|data| data.get("worktreePath"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_cwd.to_path_buf());
    let _ = runs
        .record_event(
            run_id,
            "agent.sandbox.created",
            Some(json!({
                "taskId": ledger.task_id,
                "agentId": definition.id,
                "sandboxId": sandbox_id,
                "worktreePath": path_to_string(&worktree)
            })),
            Some(step_id.to_string()),
        )
        .await;
    (sandbox_id, worktree)
}

fn provider_tools_for_subagent(
    definition: &AgentDefinition,
    definitions: &[ToolDefinition],
    catalog: &[ToolCatalogEntry],
) -> Vec<ProviderToolDefinition> {
    definitions
        .iter()
        .filter(|tool| !is_subagent_nesting_tool(&tool.name))
        .filter(|tool| agent_allows_tool(definition, &tool.name, &tool.name, catalog))
        .map(|tool| ProviderToolDefinition {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        })
        .collect()
}

fn subagent_tool_request_from_provider(
    call: &ProviderToolCall,
    iteration: usize,
) -> Option<SubAgentToolRequest> {
    let name = call.name.trim();
    if name.is_empty() {
        return None;
    }
    Some(SubAgentToolRequest {
        name: name.to_string(),
        args: parse_mainbrain_tool_arguments(&call.arguments).unwrap_or_default(),
        reason: Some(format!("Model requested tool in iteration {iteration}")),
    })
}

fn subagent_tool_request_from_runtime(
    call: &MainBrainRuntimeToolCall,
) -> SubAgentToolRequest {
    SubAgentToolRequest {
        name: call.name.clone(),
        args: call.args.clone(),
        reason: Some(format!("Printed tool call ({})", call.source)),
    }
}

fn append_subagent_tool_transcript(report: &SubAgentToolReport, transcript: &mut Vec<String>) {
    for summary in &report.evidence {
        transcript.push(format!("Tool result: {summary}"));
    }
    for risk in &report.risks {
        if risk.starts_with("Skipped") {
            transcript.push(format!("System note: {risk}"));
        }
    }
}

async fn publish_subagent_timeline_progress(
    eventbus: &EventBus,
    agents: &AgentRuntime,
    task_id: &str,
) {
    if let Some(ledger) = agents.task_status(task_id.to_string()).await {
        publish_team_agent_event(eventbus, agents, "progress", &ledger).await;
    }
}

async fn execute_subagent_task(
    agents: Arc<AgentRuntime>,
    execution: Arc<ExecutionRuntime>,
    agent_bridge: Arc<AgentWorkerBridge>,
    tools: Arc<ToolRuntime>,
    runs: Arc<RunStore>,
    eventbus: EventBus,
    task_id: String,
) -> anyhow::Result<()> {
    let started = agents.start_task(&task_id).await?;
    publish_team_agent_event(&eventbus, agents.as_ref(), "started", &started).await;
    let run = runs
        .create_run(CreateRunInput {
            session_id: started
                .control_plane_session_id
                .clone()
                .unwrap_or_else(|| "subagents".to_string()),
            goal: started.task.task.clone(),
            metadata: BTreeMap::from([
                ("source".to_string(), json!("subagent")),
                ("taskId".to_string(), json!(started.task_id.clone())),
                ("agentId".to_string(), json!(started.agent_id.clone())),
                ("agentName".to_string(), json!(started.agent_name.clone())),
            ]),
        })
        .await?;
    let step = runs
        .start_step(StartStepInput {
            run_id: run.id.clone(),
            step_type: "agent".to_string(),
            name: format!("{} SubAgent execution", started.agent_name),
            metadata: BTreeMap::from([("taskId".to_string(), json!(started.task_id.clone()))]),
        })
        .await?;
    runs.record_event(
        &run.id,
        "task.created",
        Some(json!({
            "taskId": started.task_id.clone(),
            "agentId": started.agent_id.clone(),
            "agentName": started.agent_name.clone(),
            "status": started.status.clone()
        })),
        Some(step.id.clone()),
    )
    .await?;
    let definition = agents
        .agent_definition(&started.agent_id)
        .await
        .unwrap_or_else(|| fallback_agent_definition(&started));
    let subagent_system_prompt = build_subagent_system_prompt(&definition);
    let mut execution_cwd = default_subagent_execution_cwd(tools.as_ref());
    let mut pre_model_evidence = Vec::new();
    let mut tool_report = SubAgentToolReport::default();
    match agent_bridge.plan_subagent_task(SubAgentPlanRequest {
        task_id: started.task_id.clone(),
        agent_id: definition.id.clone(),
        role: definition.role.clone(),
        system_prompt: Some(subagent_system_prompt.clone()),
        task: started.task.task.clone(),
        context: started.task.context.clone(),
        write_scope: started.task.write_scope.clone(),
        tools: definition
            .tools
            .iter()
            .filter(|tool| !is_subagent_nesting_tool(tool))
            .cloned()
            .collect(),
        artifact_required: subagent_requires_artifact(&definition),
    }) {
        Ok(plan) => {
            let planned = agents
                .report_task_plan(&task_id, Some(plan.title.clone()), plan.steps.clone())
                .await?;
            publish_team_agent_event(&eventbus, agents.as_ref(), "progress", &planned).await;
            tool_report = execute_subagent_tool_requests(SubAgentToolDispatchInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition: &definition,
                ledger: &started,
                requests: &plan.tool_requests,
                runs: runs.clone(),
                run_id: &run.id,
                step_id: &step.id,
                execution_cwd: &execution_cwd,
            })
            .await?;
            let mut evidence = plan.evidence.clone();
            let tool_evidence = tool_report.evidence.clone();
            pre_model_evidence.extend(tool_evidence.clone());
            evidence.extend(tool_evidence);
            let mut risks = plan.risks.clone();
            risks.extend(tool_report.risks.clone());
            let progressed = agents
                .report_progress(
                    &task_id,
                    plan.summary.clone(),
                    evidence,
                    risks,
                    Some("Call the configured model route for this SubAgent task.".to_string()),
                )
                .await?;
            publish_team_agent_event(&eventbus, agents.as_ref(), "progress", &progressed).await;
            runs.record_event(
                &run.id,
                "agent.worker.plan",
                Some(json!({
                    "taskId": started.task_id.clone(),
                    "agentId": definition.id.clone(),
                    "title": plan.title,
                    "steps": plan.steps,
                    "evidence": plan.evidence,
                    "risks": plan.risks,
                    "toolRequests": plan.tool_requests
                })),
                Some(step.id.clone()),
            )
            .await?;
        }
        Err(err) => {
            let warning =
                format!("Python agent worker unavailable; using Rust fallback plan: {err}");
            let progressed = agents
                .report_progress(
                    &task_id,
                    "Loaded agent definition and task context for local execution.".to_string(),
                    vec![format!("agent: {}", definition.id)],
                    vec![warning.clone()],
                    Some("Call the configured model route for this SubAgent task.".to_string()),
                )
                .await?;
            publish_team_agent_event(&eventbus, agents.as_ref(), "progress", &progressed).await;
            runs.record_event(
                &run.id,
                "agent.worker.fallback",
                Some(json!({
                    "taskId": started.task_id.clone(),
                    "agentId": definition.id.clone(),
                    "warning": warning
                })),
                Some(step.id.clone()),
            )
            .await?;
        }
    }
    if finish_subagent_run_if_controlled(&agents, &runs, &task_id, &run.id, &step.id).await? {
        return Ok(());
    }

    let project_dir_for_sandbox = execution_cwd.clone();
    let (sandbox_id, sandbox_cwd) = ensure_subagent_sandbox(
        tools.as_ref(),
        &definition,
        &started,
        &runs,
        &run.id,
        &step.id,
        &execution_cwd,
    )
    .await;
    execution_cwd = sandbox_cwd;
    let _sandbox_cleanup = SubagentSandboxCleanup {
        project_dir: project_dir_for_sandbox.clone(),
        sandbox_id: sandbox_id.clone(),
    };

    let max_turns = subagent_max_turns(definition.max_turns);
    let task_kind = model_task_for_subagent(&definition);
    let system_context = subagent_system_context(
        &started,
        &definition,
        &subagent_system_prompt,
        &pre_model_evidence,
    );
    let catalog = tools.catalog().await;
    let tool_definitions = tools.definitions().await;
    let provider_tools = provider_tools_for_subagent(&definition, &tool_definitions, &catalog);
    let use_native_tools = !provider_tools.is_empty();
    let mut tool_transcript = Vec::<String>::new();
    let mut final_text = String::new();
    let mut final_provider = String::from("unknown");
    let mut final_model = String::from("unknown");
    let mut final_error: Option<String> = None;
    let mut completed_iterations = 0u32;

    for iteration in 1..=max_turns {
        completed_iterations = iteration as u32;
        if finish_subagent_run_if_controlled(&agents, &runs, &task_id, &run.id, &step.id).await? {
            return Ok(());
        }
        let iteration_input = if tool_transcript.is_empty() {
            started.task.task.clone()
        } else {
            iteration_input(&started.task.task, &tool_transcript, Some(48_000))
        };
        runs.record_event(
            &run.id,
            "trace.model.request",
            Some(json!({
                "task": format!("{task_kind:?}"),
                "agentId": definition.id.clone(),
                "taskId": task_id.clone(),
                "iteration": iteration,
                "maxTurns": max_turns,
                "sandboxId": sandbox_id,
                "runtimeToolCount": tool_report.executed
            })),
            Some(step.id.clone()),
        )
        .await?;

        let agents_delta = agents.clone();
        let task_id_delta = task_id.clone();
        let agent_id_delta = definition.id.clone();
        let eventbus_delta = eventbus.clone();
        let on_delta = Some(Arc::new(move |content: String| {
            if content.is_empty() {
                return;
            }
            let agents = agents_delta.clone();
            let task_id = task_id_delta.clone();
            let agent_id = agent_id_delta.clone();
            let eventbus = eventbus_delta.clone();
            tokio::spawn(async move {
                agents.append_timeline_text(&task_id, &content).await;
                if agents
                    .flush_timeline_event(&task_id, "text", &agent_id)
                    .await
                    .is_some()
                {
                    publish_subagent_timeline_progress(&eventbus, &agents, &task_id).await;
                }
            });
        }) as Arc<dyn Fn(String) + Send + Sync>);

        let response = execution
            .generate_for_task_with_options(
                task_kind,
                &iteration_input,
                Some(&system_context),
                GenerationOptions {
                    deep_thinking: false,
                    tools: if use_native_tools {
                        Some(provider_tools.clone())
                    } else {
                        None
                    },
                    history: Vec::new(),
                    on_delta,
                    on_tool_call_ready: None,
                    on_tool_call_delta: None,
                },
            )
            .await;

        if let Some(reasoning) = response
            .reasoning
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            agents
                .append_timeline_reasoning(&task_id, reasoning)
                .await;
            if agents
                .flush_timeline_event(&task_id, "reasoning", &definition.id)
                .await
                .is_some()
            {
                publish_subagent_timeline_progress(&eventbus, agents.as_ref(), &task_id).await;
            }
        }
        if response.streamed
            && agents
                .flush_timeline_event(&task_id, "text", &definition.id)
                .await
                .is_some()
            {
                publish_subagent_timeline_progress(&eventbus, agents.as_ref(), &task_id).await;
            }

        final_provider = response.provider.clone();
        final_model = response.model.clone();
        runs.record_event(
            &run.id,
            "trace.model.response",
            Some(json!({
                "provider": response.provider.clone(),
                "model": response.model.clone(),
                "content": response.text.clone(),
                "error": response.error.clone(),
                "iteration": iteration,
                "toolCallCount": response.tool_calls.len()
            })),
            Some(step.id.clone()),
        )
        .await?;

        if finish_subagent_run_if_controlled(&agents, &runs, &task_id, &run.id, &step.id).await? {
            return Ok(());
        }
        if let Some(error) = response.error.clone() {
            final_text = response.text.clone();
            final_error = Some(error);
            break;
        }

        let native_requests = response
            .tool_calls
            .iter()
            .filter_map(|call| subagent_tool_request_from_provider(call, iteration))
            .collect::<Vec<_>>();
        if !native_requests.is_empty() {
            let dispatch = execute_subagent_tool_requests(SubAgentToolDispatchInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition: &definition,
                ledger: &started,
                requests: &native_requests,
                runs: runs.clone(),
                run_id: &run.id,
                step_id: &step.id,
                execution_cwd: &execution_cwd,
            })
            .await?;
            append_subagent_tool_transcript(&dispatch, &mut tool_transcript);
            tool_report.merge(dispatch);
            publish_subagent_timeline_progress(&eventbus, agents.as_ref(), &task_id).await;
            continue;
        }

        if let Some(tool_call) =
            parse_printed_mainbrain_tool_call(&response.text, &catalog, iteration)
        {
            let dispatch = execute_subagent_tool_requests(SubAgentToolDispatchInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition: &definition,
                ledger: &started,
                requests: &[subagent_tool_request_from_runtime(&tool_call)],
                runs: runs.clone(),
                run_id: &run.id,
                step_id: &step.id,
                execution_cwd: &execution_cwd,
            })
            .await?;
            append_subagent_tool_transcript(&dispatch, &mut tool_transcript);
            tool_report.merge(dispatch);
            publish_subagent_timeline_progress(&eventbus, agents.as_ref(), &task_id).await;
            continue;
        }

        if looks_like_printed_mainbrain_tool_call(&response.text) && iteration < max_turns {
            tool_transcript.push(
                "System correction: the previous assistant message printed invalid tool-call syntax. If a tool is needed, respond with exactly one JSON object: {\"tool\":\"tool_name\",\"args\":{...}}. Otherwise answer normally.".to_string(),
            );
            continue;
        }

        final_text = response.text.clone();
        break;
    }

    if final_text.trim().is_empty() && final_error.is_none() {
        final_text = format!(
            "Sub-agent reached the configured turn limit ({max_turns} iterations). Review tool evidence and continue if needed."
        );
    }

    let mut response_error = final_error;
    if finish_subagent_run_if_controlled(&agents, &runs, &task_id, &run.id, &step.id).await? {
        return Ok(());
    }
    if response_error.is_none()
        && subagent_requires_artifact(&definition)
        && tool_report.artifacts.is_empty()
    {
        let (corrected_text, correction_error) = run_subagent_artifact_correction(
            SubAgentArtifactCorrectionInput {
                agents: agents.clone(),
                execution: execution.clone(),
                tools: tools.clone(),
                eventbus: eventbus.clone(),
                definition: &definition,
                started: &started,
                tool_report: &mut tool_report,
                runs: runs.clone(),
                run_id: &run.id,
                step_id: &step.id,
                execution_cwd: &execution_cwd,
                task_id: &task_id,
                task_kind,
                system_context: &system_context,
                tool_definitions: &tool_definitions,
                catalog: &catalog,
                final_text,
                completed_iterations: &mut completed_iterations,
            },
        )
        .await;
        final_text = corrected_text;
        if correction_error.is_some() {
            response_error = correction_error;
        }
    }
    if response_error.is_none()
        && subagent_requires_artifact(&definition)
        && tool_report.artifacts.is_empty()
    {
        if let Some(artifact_report) = materialize_subagent_model_artifact(
            SubAgentArtifactMaterializeInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition: &definition,
                ledger: &started,
                content: &final_text,
                runs: runs.clone(),
                run_id: &run.id,
                step_id: &step.id,
                execution_cwd: &execution_cwd,
            },
        )
        .await?
        {
            tool_report.merge(artifact_report);
        }
    }
    if finish_subagent_run_if_controlled(&agents, &runs, &task_id, &run.id, &step.id).await? {
        return Ok(());
    }
    let artifact_error = if response_error.is_none()
        && subagent_requires_artifact(&definition)
        && tool_report.artifacts.is_empty()
        && tool_report.approval_requests.is_empty()
    {
        if is_reasoning_only_failure(&final_text) {
            Some(format!(
                "{} finished without a visible artifact deliverable.",
                definition.name
            ))
        } else {
            Some(format!(
                "artifact_missing: {} finished without a real write-tool artifact",
                definition.name
            ))
        }
    } else {
        None
    };
    let missing_artifact = artifact_error
        .as_deref()
        .is_some_and(|msg| msg.starts_with("artifact_missing:"));
    let visible_output_missing = artifact_error
        .as_deref()
        .is_some_and(|msg| msg.contains("without a visible artifact"));
    let should_materialize = sandbox_id.is_some()
        && !missing_artifact
        && !visible_output_missing
        && response_error.is_none();
    let materialized = if let (Some(sandbox_id), true) = (sandbox_id.as_deref(), should_materialize)
    {
        integrate_subagent_sandbox(
            &project_dir_for_sandbox,
            sandbox_id,
            &tool_report.modified_files,
            &tool_report.artifacts,
            &started.task.write_scope,
        )
    } else {
        None
    };
    let materialize_conflicts = materialized
        .as_ref()
        .map(|item| item.conflicts.clone())
        .unwrap_or_default();
    let materialize_missing = should_materialize && materialized.is_none();
    let materialize_empty = should_materialize
        && materialized.as_ref().is_some_and(|item| {
            !tool_report.modified_files.is_empty()
                && item.copied_files.is_empty()
                && item.deleted_files.is_empty()
                && item.conflicts.is_empty()
        });
    let mut result_evidence = vec![
        format!("model route: {}/{}", final_provider, final_model),
        format!("execution: rust-local-model ({completed_iterations} iteration(s))"),
    ];
    if let Some(sandbox_id) = sandbox_id.as_deref() {
        result_evidence.push(format!("sandbox: {sandbox_id}"));
    }
    result_evidence.extend(tool_report.evidence.clone());
    if let Some(materialized) = materialized.as_ref() {
        let landed = normalize_relative_file_list(
            materialized
                .copied_files
                .iter()
                .chain(materialized.deleted_files.iter())
                .cloned()
                .collect(),
        );
        if !landed.is_empty() {
            result_evidence.push(format!("sandbox_materialized: {}", landed.join(", ")));
        }
        if let Some(auto_retried) = materialized.auto_retried_files.as_ref() {
            if !auto_retried.is_empty() {
                result_evidence.push(format!("sandbox_auto_retry: {}", auto_retried.join(", ")));
            }
        }
        let residual = materialized
            .residual_conflicts
            .clone()
            .unwrap_or_else(|| materialized.conflicts.clone());
        if !residual.is_empty() {
            result_evidence.push(format!("sandbox_conflicts: {}", residual.join(", ")));
            if let Some(diffs) = materialized.conflict_diffs.as_ref() {
                for file in residual.iter().take(6) {
                    if let Some(diff) = diffs.get(file) {
                        let clipped: String = diff.chars().take(1200).collect();
                        result_evidence.push(format!("conflict_diff:{file}\n{clipped}"));
                    }
                }
            }
        }
    } else if materialize_missing {
        if let Some(sandbox_id) = sandbox_id.as_deref() {
            result_evidence.push(format!(
                "sandbox_materialize_failed: could not materialize {sandbox_id}"
            ));
        }
    }
    let mutated_files = normalize_relative_file_list(
        tool_report
            .modified_files
            .iter()
            .cloned()
            .chain(
                materialized
                    .as_ref()
                    .into_iter()
                    .flat_map(|item| {
                        item.copied_files
                            .iter()
                            .chain(item.deleted_files.iter())
                            .cloned()
                    }),
            )
            .collect(),
    );
    let result_artifacts = normalize_relative_file_list(
        tool_report
            .artifacts
            .iter()
            .cloned()
            .chain(
                materialized
                    .as_ref()
                    .into_iter()
                    .flat_map(|item| item.copied_files.iter().cloned())
                    .filter(|file| tool_report.artifacts.iter().any(|artifact| artifact == file)),
            )
            .collect(),
    );
    let failed = response_error.is_some()
        || artifact_error.is_some()
        || !materialize_conflicts.is_empty()
        || materialize_missing
        || materialize_empty;
    let conclusion = if failed {
        response_error
            .clone()
            .or_else(|| artifact_error.clone())
            .unwrap_or_else(|| {
                if materialize_missing {
                    format!(
                        "Sandbox {} could not be materialized to the project root.",
                        sandbox_id.as_deref().unwrap_or("unknown")
                    )
                } else if materialize_empty {
                    format!(
                        "Sandbox {} produced no project-root file changes within the declared output paths.",
                        sandbox_id.as_deref().unwrap_or("unknown")
                    )
                } else if !materialize_conflicts.is_empty() {
                    format!(
                        "Sandbox materialization conflict: {}",
                        materialize_conflicts.join(", ")
                    )
                } else {
                    "Sub-agent execution failed".to_string()
                }
            })
    } else {
        first_non_empty_line(&final_text)
            .unwrap_or_else(|| "Sub-agent task completed".to_string())
    };
    let next_steps = if response_error.is_some() {
        vec!["Configure a valid model provider/API key, then retry the task.".to_string()]
    } else if artifact_error.is_some() {
        vec![format!(
            "Retry {} with a valid artifact path or approve the generated file write.",
            definition.name
        )]
    } else if materialize_missing || materialize_empty || !materialize_conflicts.is_empty() {
        vec![
            "Review sandbox materialization conflicts or empty apply, then retry.".to_string(),
        ]
    } else if !tool_report.approval_requests.is_empty() {
        vec![
            "Review pending SubAgent tool approval requests in the permission UI.".to_string(),
            "Retry or continue the task after approvals are resolved.".to_string(),
        ]
    } else if mutated_files.is_empty() {
        vec!["Review the SubAgent output and delegate follow-up work if needed.".to_string()]
    } else {
        vec!["Review modified files and run focused validation before merging.".to_string()]
    };
    let mut risks = tool_report.risks.clone();
    if materialize_missing || materialize_empty {
        risks.push(
            "Sandbox changes were not visible in the project root after completion.".to_string(),
        );
    }
    if !materialize_conflicts.is_empty() {
        risks.push(
            "Sandbox changes were not copied to project root because target files changed outside the sandbox."
                .to_string(),
        );
    }
    let mut blockers = response_error
        .clone()
        .into_iter()
        .chain(artifact_error.clone())
        .collect::<Vec<_>>();
    if materialize_missing {
        blockers.push(format!(
            "Sandbox {} could not be materialized to the project root.",
            sandbox_id.as_deref().unwrap_or("unknown")
        ));
    }
    if materialize_empty {
        blockers.push(format!(
            "Sandbox {} produced no project-root file changes within the declared output paths.",
            sandbox_id.as_deref().unwrap_or("unknown")
        ));
    }
    blockers.extend(
        materialize_conflicts
            .iter()
            .map(|file| format!("Sandbox materialization conflict: {file}")),
    );
    let completed_at = Utc::now().timestamp_millis();
    let result = SubAgentTaskResult {
        task_id: started.task_id.clone(),
        agent_id: started.agent_id.clone(),
        agent_name: started.agent_name.clone(),
        status: if failed { "failed" } else { "completed" }.to_string(),
        conclusion,
        evidence: result_evidence,
        modified_files: mutated_files,
        risks,
        blockers,
        next_steps,
        output: final_text,
        started_at: started.started_at,
        completed_at,
        iterations: Some(completed_iterations),
        error: response_error.clone(),
        sandbox_id,
        provider: Some(final_provider.clone()),
        model: Some(final_model.clone()),
        role: definition.role.clone(),
        write_scope: started.task.write_scope.clone(),
        artifacts: result_artifacts,
    };
    let ledger = agents.complete_task(&task_id, result).await?;
    publish_team_agent_event(
        &eventbus,
        agents.as_ref(),
        if failed { "failed" } else { "completed" },
        &ledger,
    )
    .await;
    if failed {
        let error = response_error
            .or(artifact_error)
            .unwrap_or_else(|| "Sub-agent execution failed".to_string());
        runs.fail_step(&step.id, error.clone()).await?;
        runs.fail_run(&run.id, error).await?;
    } else {
        let payload = BTreeMap::from([
            ("taskId".to_string(), json!(task_id)),
            ("provider".to_string(), json!(final_provider)),
            ("model".to_string(), json!(final_model)),
            ("iterations".to_string(), json!(completed_iterations)),
        ]);
        runs.complete_step(&step.id, payload.clone()).await?;
        runs.complete_run(&run.id, payload).await?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone)]
struct MainBrainToolPlanReport {
    evidence: Vec<String>,
    risks: Vec<String>,
    context_lines: Vec<String>,
    approval_requests: Vec<String>,
    executed: usize,
    skipped: usize,
    failed: usize,
}

impl MainBrainToolPlanReport {
    fn to_prompt_context(&self) -> Option<String> {
        if self.context_lines.is_empty() {
            return None;
        }
        Some(format!(
            "Main-brain policy tool evidence:\n{}",
            self.context_lines.join("\n\n")
        ))
    }

    fn summary_payload(&self) -> Value {
        json!({
            "executed": self.executed,
            "skipped": self.skipped,
            "failed": self.failed,
            "evidence": self.evidence,
            "risks": self.risks,
            "approvalRequests": self.approval_requests
        })
    }
}

struct MainTurnLoopHostMut {
    context_surface: ContextSurface,
    turn_memory_cache: TurnMemoryCache,
    memory_context: Option<String>,
    runtime_context: String,
    active_tool_names: BTreeSet<String>,
    writes_since_mid_validation: usize,
    turn_usage: Option<xuanyu_execution::TokenUsage>,
    /// Mid-turn assistant/tool evidence as protocol messages (not English user scaffolds).
    mid_turn_messages: Vec<ProtocolMessage>,
}

struct MainTurnLoopHostAdapter {
    core: AppCore,
    session_id: SessionId,
    turn_id: TurnId,
    run_id: String,
    model_step_id: String,
    cwd: PathBuf,
    turn_started_ms: u64,
    main_brain_intent: String,
    context_policy: Value,
    context_budget_allocation: ContextBudgetAllocation,
    tool_catalog: Vec<ToolCatalogEntry>,
    tool_definitions: Vec<ToolDefinition>,
    deep_thinking: bool,
    attachments: Option<Vec<ChatAttachment>>,
    config: Value,
    model_input: String,
    policy_tool_report: MainBrainToolPlanReport,
    policy_decision: Result<AgentPolicyDecision, String>,
    key_facts_context: Option<String>,
    packed_history: Vec<ProtocolMessage>,
    mutable: tokio::sync::Mutex<MainTurnLoopHostMut>,
    speculative: tokio::sync::Mutex<Option<Arc<speculative_tools::SpeculativeToolBatch>>>,
}

#[async_trait::async_trait]
impl MainTurnLoopHost for MainTurnLoopHostAdapter {
    async fn on_iteration_start(
        &self,
        iteration: usize,
        iteration_input: &str,
        runtime_tool_count: usize,
    ) -> anyhow::Result<()> {
        let has_memory_context = self.mutable.lock().await.memory_context.is_some();
        self.core
            .runs
            .record_event(
                &self.run_id,
                "trace.model.request",
                Some(json!({
                    "task": "main",
                    "iteration": iteration,
                    "brainMode": "agent",
                    "mainBrainIntent": self.main_brain_intent,
                    "contextLoadPolicy": self.context_policy,
                    "contextSurface": context_surface_snapshot(&self.context_policy),
                    "activeContextSections": active_context_sections(&self.context_policy),
                    "hasMemoryContext": has_memory_context,
                    "hasKeyFacts": self.key_facts_context.is_some(),
                    "policyToolCount": self.policy_tool_report.executed,
                    "runtimeToolCount": runtime_tool_count,
                    "hasRuntimeContext": true,
                    "attachmentCount": self.attachments.as_ref().map(Vec::len).unwrap_or_default(),
                    "deepThinking": self.deep_thinking,
                    "packedHistoryMessages": self.packed_history.len(),
                    "policyIntent": self.policy_decision.as_ref().ok().map(|decision| decision.intent.clone())
                })),
                Some(self.model_step_id.clone()),
            )
            .await?;
        let _ = iteration_input;
        Ok(())
    }

    async fn generate_response(
        &self,
        _iteration: usize,
        iteration_input: &str,
    ) -> anyhow::Result<MainTurnModelResponse> {
        let (runtime_context, active_tool_names, mid_turn_messages) = {
            let mutable = self.mutable.lock().await;
            (
                mutable.runtime_context.clone(),
                mutable.active_tool_names.clone(),
                mutable.mid_turn_messages.clone(),
            )
        };
        let mut history = self.packed_history.clone();
        // First model call this turn: trailing user message is the request.
        // After tools: put the request once *before* the tool chain, then stop.
        // Re-appending the same user text after tool results makes the model treat
        // a successful edit as if the user just repeated the complaint.
        let user_content = if mid_turn_messages.is_empty() {
            iteration_input
        } else {
            let current = self.model_input.trim();
            let already_has_current = history.last().is_some_and(|message| {
                message.role.eq_ignore_ascii_case("user") && message.content.trim() == current
            });
            if !current.is_empty() && !already_has_current {
                history.push(ProtocolMessage {
                    role: "user".to_string(),
                    content: self.model_input.clone(),
                    tool_call_ids: Vec::new(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                });
            }
            history.extend(mid_turn_messages);
            ""
        };
        let eventbus = self.core.eventbus.clone();
        let session_id_delta = self.session_id.clone();
        let turn_id_delta = self.turn_id.clone();
        let route = self.core.execution.resolve_route(ModelTaskKind::Main).await;
        eventbus.publish(AppEvent::ModelRouteSelected {
            session_id: self.session_id.clone(),
            task: "main".to_string(),
            provider: route.provider.provider_type.clone(),
            model: route.provider.model.clone(),
        });
        let provider_tools = if route.capabilities.tool_calling {
            Some(provider_tools_for_active_surface(
                &self.tool_definitions,
                &active_tool_names,
            ))
        } else {
            None
        };
        let speculative = speculative_tools::SpeculativeToolBatch::new(
            self.core.tools(),
            self.cwd.clone(),
            self.session_id.0.clone(),
            self.tool_definitions.clone(),
        );
        *self.speculative.lock().await = Some(speculative.clone());
        let on_tool_call_ready = {
            let speculative = speculative.clone();
            Arc::new(move |call: xuanyu_execution::ProviderToolCall| {
                speculative.enqueue(call);
            }) as Arc<dyn Fn(xuanyu_execution::ProviderToolCall) + Send + Sync>
        };
        let write_before_cache: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let announced_write_ids: Arc<Mutex<BTreeSet<String>>> =
            Arc::new(Mutex::new(BTreeSet::new()));
        let last_progress: Arc<Mutex<HashMap<String, (usize, usize, u64)>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let on_tool_call_delta = {
            let eventbus = self.core.eventbus.clone();
            let session_id = self.session_id.clone();
            let turn_id = self.turn_id.clone();
            let cwd = self.cwd.clone();
            let write_before_cache = write_before_cache.clone();
            let announced_write_ids = announced_write_ids.clone();
            let last_progress = last_progress.clone();
            Arc::new(move |call: xuanyu_execution::ProviderToolCall| {
                if !is_streamable_write_tool(&call.name) {
                    return;
                }
                let path_hint = preview_stream_write_args(&call.name, &call.arguments, None)
                    .and_then(|preview| preview.path);
                let existing_before = if call.name == "file_write" {
                    path_hint.as_ref().map(|path| {
                        let key = cwd.join(path).to_string_lossy().to_string();
                        let mut cache = write_before_cache
                            .lock()
                            .unwrap_or_else(|err| err.into_inner());
                        if let Some(existing) = cache.get(&key) {
                            return existing.clone();
                        }
                        let content = fs::read_to_string(cwd.join(path)).unwrap_or_default();
                        cache.insert(key, content.clone());
                        content
                    })
                } else {
                    None
                };
                let Some(preview) = preview_stream_write_args(
                    &call.name,
                    &call.arguments,
                    existing_before.as_deref(),
                ) else {
                    return;
                };
                // Match reference: announce once we have a path or live +/- preview.
                // Do not wait for path alone — content often streams first; delaying hides +N flips.
                let path_missing = preview
                    .path
                    .as_ref()
                    .map(|p| p.trim().is_empty())
                    .unwrap_or(true);
                if path_missing && preview.add == 0 && preview.del == 0 {
                    return;
                }
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                {
                    let mut last = last_progress.lock().unwrap_or_else(|err| err.into_inner());
                    if let Some((add, del, at)) = last.get(&call.id) {
                        if *add == preview.add && *del == preview.del && now_ms.saturating_sub(*at) < 64
                        {
                            return;
                        }
                    }
                    last.insert(call.id.clone(), (preview.add, preview.del, now_ms));
                }
                let args = Value::Object(
                    preview
                        .args
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );
                {
                    let mut announced =
                        announced_write_ids.lock().unwrap_or_else(|err| err.into_inner());
                    if announced.insert(call.id.clone()) {
                        eventbus.publish(AppEvent::AgentToolRequested {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            tool_call_id: call.id.clone(),
                            tool_name: call.name.clone(),
                            args: args.clone(),
                        });
                    }
                }
                eventbus.publish(AppEvent::AgentToolProgress {
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    data: json!({
                        "streamWrite": true,
                        "args": args,
                        "rawArgs": call.arguments,
                        "add": preview.add,
                        "del": preview.del,
                        "existingBefore": preview.existing_before,
                    }),
                });
            }) as Arc<dyn Fn(xuanyu_execution::ProviderToolCall) + Send + Sync>
        };
        let response = self
            .core
            .execution
            .generate_reply_with_context_and_options(
                user_content,
                Some(&runtime_context),
                GenerationOptions {
                    deep_thinking: self.deep_thinking,
                    tools: provider_tools.clone(),
                    history,
                    on_delta: Some(Arc::new(move |content: String| {
                        eventbus.publish(AppEvent::TurnDelta {
                            session_id: session_id_delta.clone(),
                            turn_id: turn_id_delta.clone(),
                            content,
                        });
                    })),
                    on_tool_call_ready: Some(on_tool_call_ready),
                    on_tool_call_delta: Some(on_tool_call_delta),
                },
            )
            .await;
        if response.error.is_some() {
            speculative.discard("model stream failed; discard speculative tools");
        }
        Ok(MainTurnModelResponse {
            provider: response.provider,
            model: response.model,
            text: response.text,
            reasoning: response.reasoning,
            error: response.error,
            streamed: response.streamed,
            tool_calls: response.tool_calls,
            usage: response.usage,
            finish_reason: response.finish_reason,
        })
    }

    async fn on_model_response(
        &self,
        iteration: usize,
        response: &MainTurnModelResponse,
    ) -> anyhow::Result<()> {
        if let Some(usage) = response.usage.as_ref().filter(|usage| !usage.is_empty()) {
            let mut mutable = self.mutable.lock().await;
            let merged = xuanyu_execution::TokenUsage::merge_turn_usage(
                mutable.turn_usage.as_ref(),
                usage,
            );
            if let Some(merged) = merged {
                mutable.turn_usage = Some(merged.clone());
                self.core.eventbus.publish(AppEvent::TurnUsage {
                    session_id: self.session_id.clone(),
                    turn_id: self.turn_id.clone(),
                    prompt_tokens: merged.prompt_tokens,
                    completion_tokens: merged.completion_tokens,
                    total_tokens: merged.total_tokens,
                    cache_tokens: merged.cache_tokens,
                });
            }
        }
        self.core
            .runs
            .record_event(
                &self.run_id,
                "trace.model.response",
                Some(json!({
                    "provider": response.provider,
                    "model": response.model,
                    "content": response.text,
                    "error": response.error,
                    "iteration": iteration,
                    "usage": response.usage
                })),
                Some(self.model_step_id.clone()),
            )
            .await?;
        // 每轮模型正文/思考按序落库，保证重开会话后旁白仍在工具之间。
        let has_text = !response.text.trim().is_empty();
        let thinking = response
            .reasoning
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if response.error.is_none() && (has_text || thinking.is_some()) {
            if let Err(error) = self
                .core
                .sessions
                .record_assistant_text_segment(
                    &self.session_id,
                    &self.turn_id,
                    if has_text { &response.text } else { "" },
                    thinking,
                )
                .await
            {
                tracing::warn!(
                    session_id = %self.session_id.0,
                    turn_id = %self.turn_id.0,
                    iteration,
                    error = %error,
                    "record_assistant_text_segment failed"
                );
            }
        }
        Ok(())
    }

    async fn is_cancelled(&self) -> anyhow::Result<bool> {
        if let Some(deadline_ms) = turn_deadline_ms_from_config(&self.config, self.turn_started_ms) {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if now_ms >= deadline_ms {
                return Ok(true);
            }
        }
        Ok(self
            .core
            .runs
            .get_run(&self.run_id)
            .await
            .map(|run| run.status == "cancelled")
            .unwrap_or(false))
    }

    async fn on_cancelled(&self) -> anyhow::Result<()> {
        let deadline_hit = turn_deadline_ms_from_config(&self.config, self.turn_started_ms)
            .is_some_and(|deadline| {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                now_ms >= deadline
            });
        let reason = if deadline_hit {
            "Run deadline exceeded".to_string()
        } else {
            "Run cancelled by user".to_string()
        };
        let _ = self
            .core
            .runs
            .fail_step(&self.model_step_id, reason)
            .await;
        self.core.sessions.abort_session(&self.session_id).await?;
        Ok(())
    }

    async fn extract_native_tool_calls(
        &self,
        response: &MainTurnModelResponse,
        iteration: usize,
    ) -> Vec<MainTurnRuntimeToolCall> {
        response
            .tool_calls
            .iter()
            .filter_map(|call| runtime_tool_call_from_provider(call, &self.tool_catalog, iteration))
            .map(|call| MainTurnRuntimeToolCall {
                id: call.id,
                name: call.name,
                args: call.args,
                source: call.source,
            })
            .collect()
    }

    async fn extract_printed_tool_call(
        &self,
        response: &MainTurnModelResponse,
        iteration: usize,
    ) -> Option<MainTurnRuntimeToolCall> {
        parse_printed_mainbrain_tool_call(&response.text, &self.tool_catalog, iteration).map(|call| {
            MainTurnRuntimeToolCall {
                id: call.id,
                name: call.name,
                args: call.args,
                source: call.source,
            }
        })
    }

    fn looks_like_printed_tool_call(&self, text: &str) -> bool {
        looks_like_printed_mainbrain_tool_call(text)
    }

    async fn dispatch_tool_calls(
        &self,
        calls: Vec<MainTurnRuntimeToolCall>,
        iteration: usize,
        source: &str,
    ) -> anyhow::Result<MainTurnToolDispatchResult> {
        let active_tool_names = self.mutable.lock().await.active_tool_names.clone();
        let active_list = active_tool_names.iter().cloned().collect::<Vec<_>>();
        let mapped = calls
            .into_iter()
            .map(|mut call| {
                if call.name == "load_tools" {
                    call.args
                        .entry("_activeTools".to_string())
                        .or_insert_with(|| json!(active_list.clone()));
                }
                MainBrainRuntimeToolCall {
                    id: call.id,
                    name: call.name,
                    args: call.args,
                    source: call.source,
                }
            })
            .collect();
        let speculative = self.speculative.lock().await.clone();
        let result = self
            .core
            .dispatch_mainbrain_tool_calls(MainBrainToolDispatchInput {
                session_id: &self.session_id,
                turn_id: &self.turn_id,
                run_id: &self.run_id,
                step_id: &self.model_step_id,
                cwd: &self.cwd,
                tool_calls: mapped,
                iteration,
                source,
                speculative,
            })
            .await?;
        let mut transcript_entries = result.transcript_entries;
        if !result.write_tool_names.is_empty() {
            let mut mutable = self.mutable.lock().await;
            mutable.writes_since_mid_validation += result.write_tool_names.len();
            if mutable.writes_since_mid_validation >= 2 {
                mutable.writes_since_mid_validation = 0;
                drop(mutable);
                if let Some(feedback) = self
                    .core
                    .run_mid_turn_write_validation(
                        &self.cwd,
                        self.turn_started_ms,
                        &result.write_tool_names,
                        &result.affected_files,
                        &self.run_id,
                    )
                    .await
                {
                    transcript_entries.push(feedback);
                }
            }
        }
        Ok(MainTurnToolDispatchResult {
            transcript_entries,
            tool_results: result.tool_results,
            loaded_tool_names: result.loaded_tool_names,
            context_patches: result.context_patches,
            loaded_context_sections: result.loaded_context_sections,
        })
    }

    async fn apply_tool_dispatch(
        &self,
        dispatch: MainTurnToolDispatchResult,
        guard_inputs: Vec<(String, Value)>,
        model_output: &str,
        loop_state: &mut MainTurnLoopState,
    ) -> anyhow::Result<Option<String>> {
        let mut mutable = self.mutable.lock().await;
        // Real OpenAI-style mid-turn chain: assistant(+tool_calls) then role=tool results.
        if !dispatch.tool_results.is_empty() {
            let tool_calls: Vec<ProtocolToolCallSpec> = dispatch
                .tool_results
                .iter()
                .map(|item| ProtocolToolCallSpec {
                    id: item.call_id.clone(),
                    name: item.tool_name.clone(),
                    arguments: item.arguments_json.clone(),
                })
                .collect();
            let tool_call_ids = tool_calls.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
            mutable.mid_turn_messages.push(ProtocolMessage {
                role: "assistant".to_string(),
                content: model_output.trim().to_string(),
                tool_call_ids,
                tool_call_id: None,
                tool_calls,
            });
            for item in &dispatch.tool_results {
                mutable.mid_turn_messages.push(ProtocolMessage {
                    role: "tool".to_string(),
                    content: item.content.clone(),
                    tool_call_ids: Vec::new(),
                    tool_call_id: Some(item.call_id.clone()),
                    tool_calls: Vec::new(),
                });
            }
        } else if !model_output.trim().is_empty() {
            mutable.mid_turn_messages.push(ProtocolMessage {
                role: "assistant".to_string(),
                content: model_output.trim().to_string(),
                tool_call_ids: Vec::new(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            });
        }
        let transcript_len_before = loop_state.tool_transcript.len();
        let dispatch_result = MainBrainToolDispatchResult {
            transcript_entries: dispatch.transcript_entries,
            tool_results: dispatch.tool_results,
            loaded_tool_names: dispatch.loaded_tool_names,
            context_patches: dispatch.context_patches,
            loaded_context_sections: dispatch.loaded_context_sections,
            write_tool_names: Vec::new(),
            affected_files: Vec::new(),
        };
        let guard_refs = guard_inputs
            .iter()
            .map(|(name, args)| (name.as_str(), args))
            .collect::<Vec<_>>();
        let MainTurnLoopHostMut {
            context_surface,
            turn_memory_cache,
            memory_context,
            runtime_context,
            active_tool_names,
            writes_since_mid_validation: _,
            turn_usage: _,
            mid_turn_messages,
        } = &mut *mutable;
        let stop = self
            .core
            .apply_mainbrain_tool_dispatch(MainBrainToolApplyInput {
                context_surface,
                turn_memory_cache,
                memory_context,
                runtime_context,
                tool_transcript: &mut loop_state.tool_transcript,
                active_tool_names,
                loop_guard: &mut loop_state.loop_guard,
                guard_inputs: &guard_refs,
                model_output,
                context_budget_allocation: &self.context_budget_allocation,
                session_id: &self.session_id,
                model_input: &self.model_input,
                config: &self.config,
                dispatch_result,
            })
            .await?;
        // Loop-guard / system notes appended to transcript must also enter mid-turn history.
        for entry in loop_state.tool_transcript.iter().skip(transcript_len_before) {
            if entry.starts_with("[System]") || entry.starts_with("System:") {
                mid_turn_messages.push(ProtocolMessage {
                    role: "user".to_string(),
                    content: entry.clone(),
                    tool_call_ids: Vec::new(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                });
            }
        }
        Ok(stop)
    }

    async fn note_mid_turn_user_message(&self, content: &str) -> anyhow::Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }
        let mut mutable = self.mutable.lock().await;
        mutable.mid_turn_messages.push(ProtocolMessage {
            role: "user".to_string(),
            content: content.trim().to_string(),
            tool_call_ids: Vec::new(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        });
        Ok(())
    }
}

#[derive(Debug, Default)]
struct MainBrainToolDispatchResult {
    transcript_entries: Vec<String>,
    tool_results: Vec<MainTurnToolResultMessage>,
    loaded_tool_names: Vec<String>,
    context_patches: Vec<String>,
    loaded_context_sections: Vec<String>,
    write_tool_names: Vec<String>,
    affected_files: Vec<String>,
}

#[derive(Debug, Clone)]
struct MainBrainRuntimeToolCall {
    id: String,
    name: String,
    args: BTreeMap<String, Value>,
    source: String,
}

#[derive(Debug, Default)]
struct SubAgentToolReport {
    evidence: Vec<String>,
    risks: Vec<String>,
    modified_files: Vec<String>,
    artifacts: Vec<String>,
    approval_requests: Vec<String>,
    executed: usize,
    skipped: usize,
    failed: usize,
}

impl SubAgentToolReport {
    fn merge(&mut self, other: SubAgentToolReport) {
        self.evidence.extend(other.evidence);
        self.risks.extend(other.risks);
        self.modified_files.extend(other.modified_files);
        self.artifacts.extend(other.artifacts);
        self.approval_requests.extend(other.approval_requests);
        self.executed += other.executed;
        self.skipped += other.skipped;
        self.failed += other.failed;
        self.modified_files =
            normalize_relative_file_list(std::mem::take(&mut self.modified_files));
        self.artifacts = normalize_relative_file_list(std::mem::take(&mut self.artifacts));
    }
}

struct SubAgentToolDispatchInput<'a> {
    agents: Arc<AgentRuntime>,
    tools: Arc<ToolRuntime>,
    definition: &'a AgentDefinition,
    ledger: &'a SubAgentTaskLedger,
    requests: &'a [SubAgentToolRequest],
    runs: Arc<RunStore>,
    run_id: &'a str,
    step_id: &'a str,
    execution_cwd: &'a Path,
}

async fn execute_subagent_tool_requests(
    input: SubAgentToolDispatchInput<'_>,
) -> anyhow::Result<SubAgentToolReport> {
    let SubAgentToolDispatchInput {
        agents,
        tools,
        definition,
        ledger,
        requests,
        runs,
        run_id,
        step_id,
        execution_cwd,
    } = input;
    let mut report = SubAgentToolReport::default();
    let cwd = execution_cwd.to_path_buf();
    let catalog = tools.catalog().await;
    // 一批模型返回的工具请求应全部执行；max_turns 是轮次上限，不能拿来砍工具批。
    for request in requests.iter() {
        let tool_name = request.name.trim();
        if tool_name.is_empty() {
            continue;
        }
        let catalog_entry = tool_catalog_entry(&catalog, tool_name);
        let canonical_tool_name = catalog_entry
            .map(|entry| entry.name.as_str())
            .unwrap_or(tool_name);
        let tool_is_write = catalog_entry
            .map(|entry| entry.is_write)
            .unwrap_or_else(|| is_subagent_write_tool(tool_name));
        let tool_is_execute = catalog_entry
            .map(|entry| entry.is_execute)
            .unwrap_or_else(|| matches!(tool_name, "exec" | "start_process"));
        runs.record_event(
            run_id,
            "agent.tool.requested",
            Some(json!({
                "taskId": ledger.task_id,
                "agentId": definition.id,
                "tool": tool_name,
                "args": request.args,
                "reason": request.reason
            })),
            Some(step_id.to_string()),
        )
        .await?;
        if catalog_entry.is_none() {
            report.skipped += 1;
            report.risks.push(format!(
                "Skipped unknown SubAgent tool requested by worker: {tool_name}"
            ));
            continue;
        }
        if is_subagent_nesting_tool(canonical_tool_name) || is_subagent_nesting_tool(tool_name) {
            report.skipped += 1;
            report.risks.push(
                "Skipped nested delegation; sub-agents cannot spawn further agents. Solve the task yourself."
                    .to_string(),
            );
            continue;
        }
        if !agent_allows_tool(definition, canonical_tool_name, tool_name, &catalog) {
            report.skipped += 1;
            report.risks.push(format!(
                "Skipped tool {tool_name}; it is not enabled for agent {}",
                definition.id
            ));
            continue;
        }
        if let Err(err) = validate_subagent_artifact_targets(definition, tool_name, &request.args) {
            report.skipped += 1;
            report.risks.push(err);
            continue;
        }
        if tool_is_write
            && !definition.can_write
            && !subagent_artifact_write_allowed(definition, tool_name, &request.args)
        {
            report.skipped += 1;
            report.risks.push(format!(
                "Skipped write tool {tool_name}; agent {} is read-only",
                definition.id
            ));
            continue;
        }
        if let Err(err) =
            validate_subagent_write_scope(tool_name, &request.args, &ledger.task.write_scope)
        {
            report.skipped += 1;
            report.risks.push(err);
            continue;
        }
        let mut tool_args = sanitize_subagent_tool_args(tool_name, request.args.clone());
        tool_args
            .entry("_agentId".to_string())
            .or_insert_with(|| json!(definition.id.clone()));
        if let Some(role) = &definition.role {
            tool_args
                .entry("_agentRole".to_string())
                .or_insert_with(|| json!(role.clone()));
        }
        tool_args
            .entry("_taskId".to_string())
            .or_insert_with(|| json!(ledger.task_id.clone()));
        if let Some(session_id) = &ledger.control_plane_session_id {
            tool_args
                .entry("_sessionId".to_string())
                .or_insert_with(|| json!(session_id.clone()));
        }
        agents
            .track_tool_call(
                &ledger.task_id,
                &WatchdogToolCall {
                    name: canonical_tool_name.to_string(),
                    args: request.args.clone(),
                },
            )
            .await;
        let output = tools
            .execute(ToolCall {
                name: canonical_tool_name.to_string(),
                args: tool_args,
                cwd: cwd.clone(),
            })
            .await;
        agents
            .record_tool_result(&ledger.task_id, output.is_error)
            .await;
        report.executed += 1;
        let summary = summarize_tool_output(tool_name, request.reason.as_deref(), &output.content);
        let affected_files = affected_files_from_tool_output(&output);
        let approval_id = approval_id_from_tool_output(&output);
        runs.record_event(
            run_id,
            "agent.tool.result",
            Some(json!({
                "taskId": ledger.task_id,
                "agentId": definition.id,
                "tool": canonical_tool_name,
                "requestedTool": tool_name,
                "isError": output.is_error,
                "content": summary,
                "data": output.data,
                "affectedFiles": affected_files,
                "requiresApproval": approval_id.is_some(),
                "isWrite": tool_is_write,
                "isExecute": tool_is_execute
            })),
            Some(step_id.to_string()),
        )
        .await?;
        if let Some(approval_id) = approval_id {
            report.approval_requests.push(approval_id);
        }
        if output.is_error {
            report.failed += 1;
            report.risks.push(summary);
        } else {
            if tool_is_write && !affected_files.is_empty() {
                report.modified_files.extend(affected_files.clone());
                report.artifacts.extend(affected_files);
            }
            report.evidence.push(summary);
        }
    }
    report.modified_files = normalize_relative_file_list(report.modified_files);
    report.artifacts = normalize_relative_file_list(report.artifacts);
    Ok(report)
}

struct SubAgentArtifactMaterializeInput<'a> {
    agents: Arc<AgentRuntime>,
    tools: Arc<ToolRuntime>,
    definition: &'a AgentDefinition,
    ledger: &'a SubAgentTaskLedger,
    content: &'a str,
    runs: Arc<RunStore>,
    run_id: &'a str,
    step_id: &'a str,
    execution_cwd: &'a Path,
}

async fn materialize_subagent_model_artifact(
    input: SubAgentArtifactMaterializeInput<'_>,
) -> anyhow::Result<Option<SubAgentToolReport>> {
    let SubAgentArtifactMaterializeInput {
        agents,
        tools,
        definition,
        ledger,
        content,
        runs,
        run_id,
        step_id,
        execution_cwd,
    } = input;
    if !subagent_requires_artifact(definition) || content.trim().is_empty() {
        return Ok(None);
    }
    let Some(path) = preferred_subagent_artifact_path(ledger, definition) else {
        return Ok(None);
    };
    let request = SubAgentToolRequest {
        name: "file_write".to_string(),
        args: BTreeMap::from([
            ("path".to_string(), json!(path)),
            ("content".to_string(), json!(content.trim())),
        ]),
        reason: Some("Materialize SubAgent model output as the required artifact.".to_string()),
    };
    execute_subagent_tool_requests(SubAgentToolDispatchInput {
        agents,
        tools,
        definition,
        ledger,
        requests: &[request],
        runs,
        run_id,
        step_id,
        execution_cwd,
    })
    .await
    .map(Some)
}

fn tool_catalog_entry<'a>(
    catalog: &'a [ToolCatalogEntry],
    tool_name: &str,
) -> Option<&'a ToolCatalogEntry> {
    catalog.iter().find(|entry| {
        entry.name == tool_name || entry.aliases.iter().any(|alias| alias == tool_name)
    })
}

fn canonical_mainbrain_policy_tool_name(name: &str) -> &str {
    match name {
        "memory_search" => "search_memory",
        "tool_search" => "search_tools",
        _ => name,
    }
}

fn is_mainbrain_policy_premodel_tool(entry: &ToolCatalogEntry) -> bool {
    if entry.is_write || entry.is_execute || entry.name.starts_with("mcp__") {
        return false;
    }
    matches!(
        entry.name.as_str(),
        "list_files"
            | "search"
            | "glob"
            | "git_status"
            | "git_diff"
            | "git_log"
            | "search_tools"
            | "load_tools"
            | "search_memory"
            | "load_memories"
            | "validation_discover"
            | "codebase_index"
    )
}

fn agent_allows_tool(
    definition: &AgentDefinition,
    canonical_tool_name: &str,
    requested_tool_name: &str,
    catalog: &[ToolCatalogEntry],
) -> bool {
    definition.tools.iter().any(|allowed| {
        allowed == canonical_tool_name
            || allowed == requested_tool_name
            || catalog
                .iter()
                .find(|entry| entry.name == canonical_tool_name)
                .map(|entry| entry.aliases.iter().any(|alias| alias == allowed))
                .unwrap_or(false)
    })
}

/// 子 Agent 禁止再委派（嵌套深度固定为 1：仅主 Agent 可 spawn）。
fn is_subagent_nesting_tool(tool_name: &str) -> bool {
    matches!(tool_name, "submit_task" | "agent")
}

fn is_subagent_write_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_write" | "write_file" | "file_edit" | "apply_patch" | "git_patch_apply"
    )
}

fn validate_subagent_write_scope(
    tool_name: &str,
    args: &BTreeMap<String, Value>,
    write_scope: &[String],
) -> Result<(), String> {
    let targets = subagent_file_write_targets(tool_name, args);
    if targets.is_empty() || write_scope.is_empty() {
        return Ok(());
    }
    let blocked = targets
        .iter()
        .filter(|target| !path_matches_write_scope(target, write_scope))
        .cloned()
        .collect::<Vec<_>>();
    if blocked.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Skipped write tool {tool_name}; target outside write scope: {} (allowed: {})",
            blocked.join(", "),
            write_scope
                .iter()
                .map(|scope| normalize_scope_path(scope))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn subagent_artifact_write_allowed(
    definition: &AgentDefinition,
    tool_name: &str,
    args: &BTreeMap<String, Value>,
) -> bool {
    subagent_requires_artifact(definition)
        && !subagent_file_write_targets(tool_name, args).is_empty()
        && validate_subagent_artifact_targets(definition, tool_name, args).is_ok()
}

fn validate_subagent_artifact_targets(
    definition: &AgentDefinition,
    tool_name: &str,
    args: &BTreeMap<String, Value>,
) -> Result<(), String> {
    if !subagent_requires_artifact(definition) {
        return Ok(());
    }
    let targets = subagent_file_write_targets(tool_name, args);
    if targets.is_empty() {
        return Ok(());
    }
    let blocked = targets
        .iter()
        .filter(|target| !is_subagent_artifact_path(definition, target))
        .cloned()
        .collect::<Vec<_>>();
    if blocked.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Skipped write tool {tool_name}; {} may only write role artifact files: {}",
            architect_artifact_denial_message(definition),
            blocked.join(", ")
        ))
    }
}

fn architect_artifact_denial_message(definition: &AgentDefinition) -> String {
    let role = definition
        .role
        .as_deref()
        .unwrap_or(&definition.id)
        .to_ascii_lowercase();
    if role.contains("architect") {
        "Architect may only write design artifacts (.md, .mmd, .drawio)".to_string()
    } else {
        definition.name.clone()
    }
}

fn looks_like_benchmark_noise(output: &str) -> bool {
    output.contains("\"task_category\"")
        || output.contains("\"data_features\"")
        || output.contains("\"applicable_task_categories\"")
        || output.contains("\"estimated_total_samples\"")
        || output.contains("single_needle_retrieval")
        || output.contains("multi_needle_retrieval")
        || output.contains("comprehensive_qa")
}

fn is_reasoning_only_failure(output: &str) -> bool {
    output.contains("Model returned reasoning but no user-visible answer")
}

fn build_subagent_artifact_correction_prompt(
    definition: &AgentDefinition,
    ledger: &SubAgentTaskLedger,
    previous_output: &str,
    attempt: usize,
    max_attempts: usize,
) -> String {
    let artifact_path = preferred_subagent_artifact_path(ledger, definition)
        .unwrap_or_else(|| "docs/subagent-artifact.md".to_string());
    let noise_warning = if looks_like_benchmark_noise(previous_output) {
        "Your previous response was benchmark metadata / QA samples. That is invalid for this task."
    } else {
        "Your previous response did not create any artifact file."
    };
    let role = definition
        .role
        .as_deref()
        .unwrap_or(&definition.id)
        .to_ascii_lowercase();
    let role_content_requirement = if role.contains("architect") {
        "The file content must be a complete engineering architecture design document with concrete sections, tables, and API examples."
    } else {
        "The file content must be the complete requested deliverable, not a task log or progress summary."
    };
    format!(
        "[MainBrain correction]\nAttempt {attempt}/{max_attempts}.\n{noise_warning}\n\nThe recovery turn exposes only file write tools. Your next assistant action must be a native file_write/apply_patch/file_edit tool call.\nRequired artifact path: {artifact_path}\n\n{role_content_requirement}\n\nAllowed response shape:\n- Native file_write call with {{\"path\":\"{artifact_path}\",\"content\":\"complete file content\"}}\n- Or apply_patch only: {{\"patches\":[{{\"path\":\"{artifact_path}\",\"action\":\"add\",\"content\":\"complete file content\"}}]}}\n\nForbidden now:\n- Do not call report_task_plan or report_progress.\n- Do not output final summary before the write tool succeeds.\n- Do not output benchmark metadata or dataset analysis."
    )
}

fn artifact_recovery_provider_tools(
    definition: &AgentDefinition,
    definitions: &[ToolDefinition],
    catalog: &[ToolCatalogEntry],
) -> Vec<ProviderToolDefinition> {
    const WRITE_TOOLS: &[&str] = &["file_write", "apply_patch", "file_edit"];
    provider_tools_for_subagent(definition, definitions, catalog)
        .into_iter()
        .filter(|tool| WRITE_TOOLS.iter().any(|allowed| *allowed == tool.name))
        .collect()
}

struct SubAgentArtifactCorrectionInput<'a> {
    agents: Arc<AgentRuntime>,
    execution: Arc<ExecutionRuntime>,
    tools: Arc<ToolRuntime>,
    eventbus: EventBus,
    definition: &'a AgentDefinition,
    started: &'a SubAgentTaskLedger,
    tool_report: &'a mut SubAgentToolReport,
    runs: Arc<RunStore>,
    run_id: &'a str,
    step_id: &'a str,
    execution_cwd: &'a Path,
    task_id: &'a str,
    task_kind: ModelTaskKind,
    system_context: &'a str,
    tool_definitions: &'a [ToolDefinition],
    catalog: &'a [ToolCatalogEntry],
    final_text: String,
    completed_iterations: &'a mut u32,
}

async fn run_subagent_artifact_correction(
    input: SubAgentArtifactCorrectionInput<'_>,
) -> (String, Option<String>) {
    let SubAgentArtifactCorrectionInput {
        agents,
        execution,
        tools,
        eventbus,
        definition,
        started,
        tool_report,
        runs,
        run_id,
        step_id,
        execution_cwd,
        task_id,
        task_kind,
        system_context,
        tool_definitions,
        catalog,
        mut final_text,
        completed_iterations,
    } = input;
    if !subagent_requires_artifact(definition) || !tool_report.artifacts.is_empty() {
        return (final_text, None);
    }
    let max_attempts = if looks_like_benchmark_noise(&final_text) {
        3
    } else {
        2
    };
    let recovery_tools = artifact_recovery_provider_tools(definition, tool_definitions, catalog);
    if recovery_tools.is_empty() {
        return (final_text, None);
    }
    let mut final_error = None;
    for attempt in 1..=max_attempts {
        if !tool_report.artifacts.is_empty() {
            break;
        }
        *completed_iterations += 1;
        let correction_input =
            build_subagent_artifact_correction_prompt(definition, started, &final_text, attempt, max_attempts);
        runs.record_event(
            run_id,
            "trace.model.request",
            Some(json!({
                "task": format!("{task_kind:?}"),
                "agentId": definition.id.clone(),
                "taskId": task_id,
                "iteration": completed_iterations,
                "phase": "artifact_correction",
                "attempt": attempt
            })),
            Some(step_id.to_string()),
        )
        .await
        .ok();
        let response = execution
            .generate_for_task_with_options(
                task_kind,
                &correction_input,
                Some(system_context),
                GenerationOptions {
                    deep_thinking: false,
                    tools: Some(recovery_tools.clone()),
                    history: Vec::new(),
                    on_delta: None,
                    on_tool_call_ready: None,
                    on_tool_call_delta: None,
                },
            )
            .await;
        if let Some(reasoning) = response
            .reasoning
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            agents
                .append_timeline_reasoning(task_id, reasoning)
                .await;
            if agents
                .flush_timeline_event(task_id, "reasoning", &definition.id)
                .await
                .is_some()
            {
                publish_subagent_timeline_progress(&eventbus, agents.as_ref(), task_id).await;
            }
        }
        runs.record_event(
            run_id,
            "trace.model.response",
            Some(json!({
                "provider": response.provider.clone(),
                "model": response.model.clone(),
                "content": response.text.clone(),
                "error": response.error.clone(),
                "iteration": completed_iterations,
                "phase": "artifact_correction",
                "toolCallCount": response.tool_calls.len()
            })),
            Some(step_id.to_string()),
        )
        .await
        .ok();
        if let Some(error) = response.error.clone() {
            final_text = response.text.clone();
            final_error = Some(error);
            break;
        }
        let native_requests = response
            .tool_calls
            .iter()
            .filter_map(|call| subagent_tool_request_from_provider(call, *completed_iterations as usize))
            .collect::<Vec<_>>();
        if !native_requests.is_empty() {
            if let Ok(dispatch) = execute_subagent_tool_requests(SubAgentToolDispatchInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition,
                ledger: started,
                requests: &native_requests,
                runs: runs.clone(),
                run_id,
                step_id,
                execution_cwd,
            })
            .await
            {
                tool_report.merge(dispatch);
                publish_subagent_timeline_progress(&eventbus, agents.as_ref(), task_id).await;
            }
        } else if let Some(tool_call) =
            parse_printed_mainbrain_tool_call(&response.text, catalog, *completed_iterations as usize)
        {
            if let Ok(dispatch) = execute_subagent_tool_requests(SubAgentToolDispatchInput {
                agents: agents.clone(),
                tools: tools.clone(),
                definition,
                ledger: started,
                requests: &[subagent_tool_request_from_runtime(&tool_call)],
                runs: runs.clone(),
                run_id,
                step_id,
                execution_cwd,
            })
            .await
            {
                tool_report.merge(dispatch);
                publish_subagent_timeline_progress(&eventbus, agents.as_ref(), task_id).await;
            }
        }
        final_text = response.text.clone();
    }
    (final_text, final_error)
}

fn subagent_file_write_targets(tool_name: &str, args: &BTreeMap<String, Value>) -> Vec<String> {
    match tool_name {
        "file_write" | "write_file" | "file_edit" => args
            .get("path")
            .and_then(Value::as_str)
            .map(normalize_scope_path)
            .into_iter()
            .collect(),
        "apply_patch" => args
            .get("patches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|patch| patch.get("path").and_then(Value::as_str))
            .map(normalize_scope_path)
            .collect(),
        _ => Vec::new(),
    }
}

fn is_subagent_artifact_path(definition: &AgentDefinition, path: &str) -> bool {
    let normalized = normalize_scope_path(path).to_ascii_lowercase();
    let role = definition
        .role
        .as_deref()
        .unwrap_or(&definition.id)
        .to_ascii_lowercase();
    if role.contains("architect") {
        normalized.ends_with(".md")
            || normalized.ends_with(".mmd")
            || normalized.ends_with(".drawio")
    } else if role.contains("reviewer") || role.contains("writer") || role.contains("doc-writer") {
        normalized.ends_with(".md") || normalized.ends_with(".markdown")
    } else {
        true
    }
}

fn preferred_subagent_artifact_path(
    ledger: &SubAgentTaskLedger,
    definition: &AgentDefinition,
) -> Option<String> {
    ledger
        .task
        .write_scope
        .iter()
        .map(|path| normalize_scope_path(path))
        .find(|path| is_subagent_artifact_path(definition, path))
        .or_else(|| {
            let role = definition
                .role
                .as_deref()
                .unwrap_or(&definition.id)
                .to_ascii_lowercase();
            if role.contains("architect") {
                Some("docs/architecture-design.md".to_string())
            } else if role.contains("reviewer") {
                Some("docs/review-report.md".to_string())
            } else if role.contains("writer") || role.contains("doc-writer") {
                Some("docs/documentation.md".to_string())
            } else {
                None
            }
        })
}

fn path_matches_write_scope(path: &str, write_scope: &[String]) -> bool {
    let target = normalize_scope_path(path);
    write_scope.iter().any(|scope| {
        let scope = normalize_scope_path(scope);
        scope == "."
            || scope == "*"
            || scope == "**"
            || target == scope
            || target.starts_with(&format!("{scope}/"))
            || glob_scope_matches(&scope, &target)
    })
}

fn normalize_scope_path(value: &str) -> String {
    let mut path = value
        .trim()
        .trim_matches(&['"', '\'', '`'][..])
        .replace('\\', "/");
    while path.starts_with("./") {
        path = path[2..].to_string();
    }
    while path.ends_with('/') && path.len() > 1 {
        path.pop();
    }
    if path.is_empty() {
        ".".to_string()
    } else {
        path
    }
}

fn glob_scope_matches(pattern: &str, target: &str) -> bool {
    let pattern = normalize_scope_path(pattern);
    let target = normalize_scope_path(target);
    if pattern.is_empty() || pattern == "." {
        return target.is_empty() || target == ".";
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == target;
    }
    let pattern_segments = pattern.split('/').collect::<Vec<_>>();
    let target_segments = if target == "." {
        Vec::new()
    } else {
        target.split('/').collect::<Vec<_>>()
    };
    glob_scope_segments_match(&pattern_segments, &target_segments)
}

fn glob_scope_segments_match(pattern: &[&str], target: &[&str]) -> bool {
    if pattern.is_empty() {
        return target.is_empty();
    }
    if pattern[0] == "**" {
        return glob_scope_segments_match(&pattern[1..], target)
            || (!target.is_empty() && glob_scope_segments_match(pattern, &target[1..]));
    }
    !target.is_empty()
        && wildcard_scope_segment_matches(pattern[0], target[0])
        && glob_scope_segments_match(&pattern[1..], &target[1..])
}

fn wildcard_scope_segment_matches(pattern: &str, target: &str) -> bool {
    wildcard_scope_bytes_match(pattern.as_bytes(), target.as_bytes())
}

fn wildcard_scope_bytes_match(pattern: &[u8], target: &[u8]) -> bool {
    if pattern.is_empty() {
        return target.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard_scope_bytes_match(&pattern[1..], target)
                || (!target.is_empty() && wildcard_scope_bytes_match(pattern, &target[1..]))
        }
        b'?' => !target.is_empty() && wildcard_scope_bytes_match(&pattern[1..], &target[1..]),
        byte => {
            !target.is_empty()
                && byte == target[0]
                && wildcard_scope_bytes_match(&pattern[1..], &target[1..])
        }
    }
}

fn affected_files_from_tool_output(output: &ToolOutput) -> Vec<String> {
    output
        .data
        .as_ref()
        .and_then(|data| data.get("affectedFiles"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(normalize_scope_path)
        .collect()
}

fn approval_id_from_tool_output(output: &ToolOutput) -> Option<String> {
    output
        .data
        .as_ref()
        .and_then(|data| data.get("approvalId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_ask_user_questions(args: &BTreeMap<String, Value>) -> Result<Value, String> {
    let questions = args
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| "ask_user requires questions array".to_string())?;
    if questions.is_empty() || questions.len() > 4 {
        return Err("ask_user requires 1-4 questions".to_string());
    }
    let mut ids = BTreeSet::new();
    let mut normalized = Vec::with_capacity(questions.len());
    for question in questions {
        let object = question
            .as_object()
            .ok_or_else(|| "ask_user question must be an object".to_string())?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "ask_user question id is required".to_string())?;
        if !ids.insert(id.to_string()) {
            return Err(format!("Duplicate question id: {id}"));
        }
        let prompt = object
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("ask_user question prompt is required for {id}"))?;
        let mut item = json!({
            "id": id,
            "prompt": prompt,
        });
        if let Some(options) = object.get("options").and_then(Value::as_array) {
            if !(2..=8).contains(&options.len()) {
                return Err(format!("ask_user options for {id} must contain 2-8 items"));
            }
            let cleaned: Vec<String> = options
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            if cleaned.len() != options.len() {
                return Err(format!("ask_user options for {id} must be non-empty strings"));
            }
            item.as_object_mut()
                .expect("question object")
                .insert("options".to_string(), json!(cleaned));
        }
        let allow_multiple = object
            .get("allow_multiple")
            .or_else(|| object.get("allowMultiple"))
            .and_then(Value::as_bool);
        if let Some(allow_multiple) = allow_multiple {
            item.as_object_mut()
                .expect("question object")
                .insert("allow_multiple".to_string(), json!(allow_multiple));
        }
        normalized.push(item);
    }
    Ok(Value::Array(normalized))
}

fn format_ask_user_answers(questions: &Value, answers: &Value) -> ToolOutput {
    let answer_items = answers.as_array().cloned().unwrap_or_default();
    let mut lines = vec!["User answers:".to_string(), String::new()];
    for question in questions.as_array().into_iter().flatten() {
        let id = question
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("question");
        let prompt = question
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("");
        let answer = answer_items.iter().find(|item| {
            item.get("id").and_then(Value::as_str) == Some(id)
        });
        lines.push(format!("### {id}"));
        lines.push(prompt.to_string());
        match answer {
            None => lines.push("(no answer)".to_string()),
            Some(answer) => {
                let selected = answer
                    .get("selected")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                let text = answer
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("");
                if !selected.is_empty() {
                    lines.push(format!("Selected: {}", selected.join(", ")));
                }
                if !text.is_empty() {
                    lines.push(format!("Text: {text}"));
                }
                if selected.is_empty() && text.is_empty() {
                    lines.push("(empty)".to_string());
                }
            }
        }
        lines.push(String::new());
    }
    ToolOutput {
        content: lines.join("\n").trim().to_string(),
        is_error: false,
        data: Some(json!({
            "answers": answers,
            "questions": questions
        })),
    }
}

fn normalize_permission_response_scope(scope: Option<&str>) -> &'static str {
    match scope.unwrap_or("once").trim().to_ascii_lowercase().as_str() {
        "always" | "all" | "global" | "permanent" => "always",
        "session" | "remember" => "session",
        _ => "once",
    }
}

fn normalize_relative_file_list(files: Vec<String>) -> Vec<String> {
    let mut deduped = BTreeMap::new();
    for file in files {
        let normalized = normalize_scope_path(&file);
        if normalized != "." {
            deduped.insert(normalized, ());
        }
    }
    deduped.into_keys().collect()
}

fn sanitize_subagent_tool_args(
    tool_name: &str,
    mut args: BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    match tool_name {
        "file_read" => {
            args.entry("maxBytes".to_string())
                .or_insert(json!(16 * 1024));
        }
        "list_files" => {
            args.entry("limit".to_string()).or_insert(json!(80));
        }
        "search" | "glob" => {
            args.entry("limit".to_string()).or_insert(json!(40));
        }
        _ => {}
    }
    args
}

fn sanitize_mainbrain_policy_tool_args(
    tool_name: &str,
    mut args: BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    match tool_name {
        "list_files" => {
            args.entry("limit".to_string()).or_insert(json!(80));
        }
        "search" | "glob" => {
            args.entry("limit".to_string()).or_insert(json!(40));
        }
        "git_log" => {
            args.entry("limit".to_string()).or_insert(json!(20));
        }
        "codebase_index" => {
            args.entry("summary_limit".to_string()).or_insert(json!(20));
        }
        "search_tools" => {
            args.entry("limit".to_string()).or_insert(json!(8));
        }
        "search_memory" => {
            if !args.contains_key("query") {
                if let Some(value) = args.get("text").cloned() {
                    args.insert("query".to_string(), value);
                }
            }
            args.entry("limit".to_string()).or_insert(json!(8));
        }
        "load_memories" => {
            args.entry("sections".to_string())
                .or_insert_with(|| json!(["memory"]));
            args.entry("limit".to_string()).or_insert(json!(8));
        }
        _ => {}
    }
    args
}

fn mainbrain_deferred_tool_catalog(catalog: &[ToolCatalogEntry]) -> String {
    format_deferred_tool_catalog_for_prompt(
        catalog,
        DeferredCatalogOptions {
            core_active_names: AGENT_CORE_PRELOAD_TOOLS,
            enabled_meta_names: META_TOOL_NAMES,
            max_description_chars: None,
        },
    )
}

fn reset_lazy_tool_surface(extra_preload: &[String]) -> BTreeSet<String> {
    let extras = extra_preload
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    initial_active_tool_names(&extras)
}

fn provider_tools_for_active_surface(
    definitions: &[ToolDefinition],
    active_names: &BTreeSet<String>,
) -> Vec<ProviderToolDefinition> {
    definitions
        .iter()
        .filter(|tool| {
            active_names.contains(&tool.name)
                || tool
                    .aliases
                    .iter()
                    .any(|alias| active_names.contains(alias))
        })
        .map(|tool| ProviderToolDefinition {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        })
        .collect()
}

fn loaded_tool_names_from_output(output: &ToolOutput) -> Vec<String> {
    output
        .data
        .as_ref()
        .and_then(|data| data.get("loaded"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn publish_team_agent_event(
    eventbus: &EventBus,
    agents: &AgentRuntime,
    kind: &str,
    ledger: &SubAgentTaskLedger,
) {
    let session_id = ledger.control_plane_session_id.clone();
    let team = agents.team_status(session_id.clone()).await;
    let team_value = serde_json::to_value(&team).unwrap_or_else(|_| json!({}));
    let ledger_value = serde_json::to_value(ledger).unwrap_or_else(|_| json!({}));
    let event = match kind {
        "started" => AppEvent::TeamAgentStarted {
            session_id: session_id.clone(),
            team: team_value.clone(),
            ledger: ledger_value.clone(),
        },
        "progress" => AppEvent::TeamAgentProgress {
            session_id: session_id.clone(),
            team: team_value.clone(),
            ledger: ledger_value.clone(),
        },
        "completed" => AppEvent::TeamAgentCompleted {
            session_id: session_id.clone(),
            team: team_value.clone(),
            ledger: ledger_value.clone(),
        },
        "failed" => AppEvent::TeamAgentFailed {
            session_id: session_id.clone(),
            team: team_value.clone(),
            ledger: ledger_value.clone(),
        },
        _ => return,
    };
    eventbus.publish(event);
    eventbus.publish(AppEvent::TaskSnapshot {
        session_id: session_id.clone(),
        task: ledger_value.clone(),
    });
    if let (Some(control_session_id), Some(control_task_id)) = (
        ledger
            .control_plane_session_id
            .as_deref()
            .filter(|value| !value.is_empty()),
        ledger
            .control_plane_task_id
            .as_deref()
            .filter(|value| !value.is_empty()),
    ) {
        if let Some(task_event) = build_control_task_event(ledger, kind) {
            eventbus.publish(AppEvent::TaskEvent {
                session_id: control_session_id.to_string(),
                task_id: control_task_id.to_string(),
                event: task_event,
            });
        }
        if matches!(kind, "completed" | "failed") {
            if let Some(task_result) = build_control_task_result(ledger) {
                eventbus.publish(AppEvent::TaskResult {
                    session_id: control_session_id.to_string(),
                    task_id: control_task_id.to_string(),
                    result: task_result,
                });
            }
        }
    }
}

fn map_team_event_to_task_event_type(kind: &str) -> &'static str {
    match kind {
        "started" => "task.started",
        "completed" => "task.done",
        "failed" => "task.failed",
        _ => "task.progress",
    }
}

fn build_control_task_event(ledger: &SubAgentTaskLedger, kind: &str) -> Option<Value> {
    let task_id = ledger.control_plane_task_id.as_deref()?;
    let session_id = ledger.control_plane_session_id.as_deref()?;
    Some(json!({
        "eventId": format!("evt_subagent_{}_{}", task_id, Utc::now().timestamp_millis()),
        "taskId": task_id,
        "sessionId": session_id,
        "runId": ledger.control_plane_run_id,
        "type": map_team_event_to_task_event_type(kind),
        "actor": {
            "kind": "executor",
            "id": ledger.agent_id,
            "name": ledger.agent_name,
        },
        "payload": {
            "agentId": ledger.agent_id,
            "taskId": ledger.task_id,
            "ledger": ledger,
        },
        "createdAt": Utc::now().timestamp_millis(),
    }))
}

fn build_control_task_result(ledger: &SubAgentTaskLedger) -> Option<Value> {
    let result = ledger.result.as_ref()?;
    let task_id = ledger.control_plane_task_id.as_deref()?;
    let _session_id = ledger.control_plane_session_id.as_deref()?;
    let status = match result.status.as_str() {
        "completed" => "done",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "failed",
    };
    Some(json!({
        "taskId": task_id,
        "status": status,
        "output": {
            "conclusion": result.conclusion,
            "changedFiles": result.modified_files,
            "evidence": result.evidence,
            "risks": result.risks,
            "nextSteps": result.next_steps,
        },
        "error": result.error.as_ref().map(|message| json!({
            "code": "SUBAGENT_FAILED",
            "message": message,
            "recoverable": true,
        })),
        "createdAt": result.started_at,
        "updatedAt": result.completed_at,
    }))
}

fn sections_from_load_memories_output(output: &ToolOutput) -> Vec<String> {
    output
        .data
        .as_ref()
        .and_then(|data| data.get("sections"))
        .and_then(Value::as_object)
        .map(|sections| {
            sections
                .keys()
                .filter(|key| is_context_section_name(key))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn context_patch_from_load_memories_output(output: &ToolOutput) -> Option<String> {
    let sections = output
        .data
        .as_ref()?
        .get("sections")?
        .as_object()?;
    let mut parts = Vec::new();
    for value in sections.values() {
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn parse_lazy_context_from_snapshot(
    snapshot: &ConversationSnapshotRecord,
) -> Option<LazyContextOverride> {
    let lazy = snapshot.conversation.get("lazyContext")?;
    serde_json::from_value::<LazyContextOverride>(lazy.clone()).ok()
}

fn runtime_tool_call_from_provider(
    call: &ProviderToolCall,
    catalog: &[ToolCatalogEntry],
    iteration: usize,
) -> Option<MainBrainRuntimeToolCall> {
    let name = call.name.trim();
    if name.is_empty() || !catalog.iter().any(|tool| tool.name == name) {
        return None;
    }
    let args = parse_mainbrain_tool_arguments(&call.arguments).unwrap_or_default();
    Some(MainBrainRuntimeToolCall {
        id: if call.id.trim().is_empty() {
            format!("call_{iteration}_{name}")
        } else {
            call.id.clone()
        },
        name: name.to_string(),
        args,
        source: "native.tool_calls".to_string(),
    })
}

fn parse_printed_mainbrain_tool_call(
    text: &str,
    catalog: &[ToolCatalogEntry],
    iteration: usize,
) -> Option<MainBrainRuntimeToolCall> {
    for value in mainbrain_json_object_candidates(text).into_iter().rev() {
        for (requested_name, args, source) in normalize_printed_mainbrain_tool_call(&value)
            .into_iter()
            .rev()
        {
            let Some(entry) = tool_catalog_entry(catalog, &requested_name) else {
                continue;
            };
            let name = entry.name.clone();
            let id = format!(
                "main_tool_{}_{}",
                iteration,
                stable_text_hash(&format!("{}:{}", name, json!(args)))
            );
            return Some(MainBrainRuntimeToolCall {
                id,
                name,
                args,
                source,
            });
        }
    }
    None
}

fn normalize_printed_mainbrain_tool_call(
    value: &Value,
) -> Vec<(String, BTreeMap<String, Value>, String)> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    for key in ["tool_calls", "toolCalls"] {
        if let Some(items) = object.get(key).and_then(Value::as_array) {
            for item in items {
                calls.extend(normalize_printed_mainbrain_tool_call(item));
            }
        }
    }
    for key in ["tool_call", "toolCall"] {
        if let Some(item) = object.get(key) {
            calls.extend(normalize_printed_mainbrain_tool_call(item));
        }
    }
    if let Some(name) = object.get("tool").and_then(Value::as_str) {
        if let Some(args) = mainbrain_args_from_value(object.get("args")) {
            calls.push((name.to_string(), args, "printed_json.tool".to_string()));
        }
    }
    if let Some(function) = object.get("function").and_then(Value::as_object) {
        if let Some(name) = function.get("name").and_then(Value::as_str) {
            if let Some(args) = mainbrain_args_from_value(
                function
                    .get("arguments")
                    .or_else(|| function.get("args"))
                    .or_else(|| object.get("arguments"))
                    .or_else(|| object.get("args")),
            ) {
                calls.push((name.to_string(), args, "printed_json.function".to_string()));
            }
        }
    }
    if let Some(name) = object.get("name").and_then(Value::as_str) {
        if let Some(args) = mainbrain_args_from_value(
            object
                .get("arguments")
                .or_else(|| object.get("args"))
                .or_else(|| object.get("input")),
        ) {
            calls.push((name.to_string(), args, "printed_json.name".to_string()));
        }
    }
    if object.get("path").and_then(Value::as_str).is_some()
        && object.get("content").and_then(Value::as_str).is_some()
    {
        calls.push((
            "file_write".to_string(),
            json_object_to_btree(Some(value)),
            "printed_json.file_write".to_string(),
        ));
    }
    if object.get("patches").and_then(Value::as_array).is_some() {
        calls.push((
            "apply_patch".to_string(),
            json_object_to_btree(Some(value)),
            "printed_json.apply_patch".to_string(),
        ));
    }
    calls
}

fn mainbrain_args_from_value(value: Option<&Value>) -> Option<BTreeMap<String, Value>> {
    match value? {
        Value::Object(_) => Some(json_object_to_btree(value)),
        Value::String(text) => parse_mainbrain_tool_arguments(text).ok(),
        _ => None,
    }
}

fn parse_mainbrain_tool_arguments(raw: &str) -> Result<BTreeMap<String, Value>, String> {
    for value in mainbrain_json_object_candidates(raw) {
        if value.is_object() {
            return Ok(json_object_to_btree(Some(&value)));
        }
    }
    Err("Invalid JSON object".to_string())
}

fn mainbrain_json_object_candidates(text: &str) -> Vec<Value> {
    let mut values = Vec::new();
    values.extend(parse_mainbrain_json_object_sequence(text));
    for fenced in fenced_code_blocks(text) {
        values.extend(parse_mainbrain_json_object_sequence(&fenced));
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if start < end {
            values.extend(parse_mainbrain_json_object_sequence(&text[start..=end]));
        }
    }
    let mut deduped = BTreeMap::new();
    values
        .into_iter()
        .filter(|value| value.is_object())
        .filter(|value| deduped.insert(value.to_string(), ()).is_none())
        .collect()
}

fn parse_mainbrain_json_object_sequence(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if !trimmed.contains('{') {
        return Vec::new();
    }
    let mut values = Vec::new();
    let mut depth = 0i32;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in trimmed.char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            continue;
        }
        if ch == '{' {
            if depth == 0 {
                start = Some(idx);
            }
            depth += 1;
            continue;
        }
        if ch != '}' {
            continue;
        }
        depth -= 1;
        if depth == 0 {
            if let Some(start_idx) = start.take() {
                if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start_idx..=idx]) {
                    values.push(value);
                }
            }
        } else if depth < 0 {
            return Vec::new();
        }
    }
    if depth == 0 {
        values
    } else {
        Vec::new()
    }
}

fn fenced_code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after_fence = &rest[start + 3..];
        let content_start = after_fence
            .find('\n')
            .map(|idx| idx + 1)
            .unwrap_or_default();
        let after_language = &after_fence[content_start..];
        let Some(end) = after_language.find("```") else {
            break;
        };
        blocks.push(after_language[..end].to_string());
        rest = &after_language[end + 3..];
    }
    blocks
}

fn looks_like_printed_mainbrain_tool_call(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<tool_call")
        || lower.contains("<function")
        || lower.contains("\"tool_call\"")
        || lower.contains("\"tool_calls\"")
        || lower.contains("\"toolcalls\"")
        || lower.contains("\"tools_used\"")
        || lower.contains("\"artifact_writes\"")
        || lower.contains("\"tool\"")
}

fn should_retry_mainbrain_tool_result(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    if lower.contains("permission denied")
        || lower.contains("validation failed")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("no such file")
        || lower.contains("already exists")
        || lower.contains("syntax error")
        || lower.contains("parse error")
        || lower.contains("invalid json")
        || lower.contains("not implemented")
        || lower.contains("not supported")
    {
        return false;
    }
    lower.contains("timeout")
        || lower.contains("econnrefused")
        || lower.contains("econnreset")
        || lower.contains("etimedout")
        || lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("exit code")
        || lower.contains("exited with")
        || lower.contains("process exited")
}

/// Trust tool-side bounds (file_read window / exec+MCP 64KiB+spill). No secondary crush.
fn mainbrain_tool_transcript_entry(
    tool_call: &MainBrainRuntimeToolCall,
    output: &ToolOutput,
    iteration: usize,
) -> String {
    let args_budget = match tool_call.name.as_str() {
        "file_write" | "file_edit" | "apply_patch" => 480,
        _ => 360,
    };
    let result_text = output.content.trim_end();
    let verify_hint = if matches!(
        tool_call.name.as_str(),
        "file_write" | "file_edit" | "apply_patch"
    ) && !output.is_error
    {
        "\n(Write/edit succeeded — do not re-read the same path only to verify.)"
    } else {
        ""
    };
    format!(
        "[Iteration {iteration} tool]\nassistant requested: {} {}\nresult: {}{}{}",
        tool_call.name,
        compact_context_line(&json!(tool_call.args).to_string(), args_budget),
        if output.is_error { "ERROR " } else { "" },
        result_text,
        verify_hint
    )
}

fn summarize_tool_output(tool_name: &str, reason: Option<&str>, content: &str) -> String {
    let compact = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");
    let clipped = if compact.chars().count() > 1200 {
        format!(
            "{}...\n[truncated]",
            compact.chars().take(1200).collect::<String>()
        )
    } else {
        compact
    };
    match reason {
        Some(reason) if !reason.trim().is_empty() => {
            format!("{tool_name} ({reason}): {clipped}")
        }
        _ => format!("{tool_name}: {clipped}"),
    }
}

fn fallback_agent_definition(ledger: &SubAgentTaskLedger) -> AgentDefinition {
    AgentDefinition {
        id: ledger.agent_id.clone(),
        name: ledger.agent_name.clone(),
        description: ledger.task.task.clone(),
        prompt: "Work as a focused XuanYu sub-agent.".to_string(),
        tools: Vec::new(),
        permission_mode: "default".to_string(),
        provider: ledger.provider.clone(),
        model: ledger.model.clone(),
        base_url: None,
        api_key: None,
        max_tokens: None,
        temperature: None,
        model_profile: None,
        can_write: !ledger.task.write_scope.is_empty(),
        can_talk_to_user: false,
        default_sandbox_mode: None,
        max_turns: 64,
        memory_scope: "session".to_string(),
        source: "ledger".to_string(),
        source_path: None,
        role: ledger.role.clone(),
    }
}

/** 子 Agent 轮次：0 视为未配置；不设低硬顶（与主循环一致，仅保留极高安全顶）。 */
fn subagent_max_turns(configured: u32) -> usize {
    const SOFT_SAFETY_CEILING: u32 = 10_000;
    const DEFAULT_TURNS: u32 = 64;
    let turns = if configured == 0 {
        DEFAULT_TURNS
    } else {
        configured
    };
    turns.clamp(1, SOFT_SAFETY_CEILING) as usize
}

fn model_task_for_subagent(definition: &AgentDefinition) -> ModelTaskKind {
    let role = definition
        .role
        .as_deref()
        .unwrap_or(&definition.id)
        .to_ascii_lowercase();
    if role.contains("review") {
        ModelTaskKind::Review
    } else if role.contains("developer") || role.contains("engineer") || definition.can_write {
        ModelTaskKind::Implementation
    } else {
        ModelTaskKind::Planning
    }
}

const MAX_SUBAGENT_TASK_CONTEXT_CHARS: usize = 32_000;
const SUBAGENT_CONTEXT_TRUNCATED_MARKER: &str = "\n...[context truncated]";

fn subagent_system_context(
    ledger: &SubAgentTaskLedger,
    definition: &AgentDefinition,
    system_prompt: &str,
    pre_model_evidence: &[String],
) -> String {
    let mut lines = vec![
        format!("SubAgent: {} ({})", definition.name, definition.id),
        format!(
            "Role: {}",
            definition
                .role
                .as_deref()
                .unwrap_or("focused sub-agent")
        ),
        format!("Description: {}", definition.description),
        "Execution mode: Rust local model executor with Python strategy planning and governed pre-model tool evidence. Only claim file changes when tool evidence below proves them.".to_string(),
    ];
    if !system_prompt.trim().is_empty() {
        lines.push(format!("Agent system prompt:\n{}", system_prompt.trim()));
    }
    if let Some(context) = ledger
        .task
        .context
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format_delegated_context_excerpt(context));
    }
    if !ledger.task.write_scope.is_empty() {
        lines.push(format!(
            "Requested write scope: {}",
            ledger.task.write_scope.join(", ")
        ));
    }
    if !pre_model_evidence.is_empty() {
        lines.push(format!(
            "Pre-model tool evidence:\n{}",
            pre_model_evidence.join("\n\n")
        ));
    }
    lines.join("\n\n")
}

fn format_delegated_context_excerpt(context: &str) -> String {
    let trimmed = context.trim();
    let excerpt = if trimmed.chars().count() > MAX_SUBAGENT_TASK_CONTEXT_CHARS {
        format!(
            "{}{}",
            trimmed
                .chars()
                .take(MAX_SUBAGENT_TASK_CONTEXT_CHARS)
                .collect::<String>(),
            SUBAGENT_CONTEXT_TRUNCATED_MARKER
        )
    } else {
        trimmed.to_string()
    };
    [
        "[Delegated context excerpt]".to_string(),
        "Capped excerpt from submit_task.context - not full session history. Use memory/search tools if more background is required.".to_string(),
        excerpt,
        "[End delegated context excerpt]".to_string(),
    ]
    .join("\n")
}

fn first_non_empty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(220).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCreateInput {
    pub content: String,
    pub category: String,
    pub importance: u8,
    pub tags: Vec<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpdateInput {
    pub content: Option<String>,
    pub category: Option<String>,
    pub importance: Option<u8>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItemDto {
    pub id: String,
    pub content: String,
    pub category: String,
    pub importance: u8,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub source: String,
    pub access_count: u64,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatsDto {
    pub total: usize,
    pub categories: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListResult {
    pub memories: Vec<MemoryItemDto>,
    pub stats: MemoryStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMutationResult {
    pub memory: MemoryItemDto,
    pub stats: MemoryStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotDto {
    pub version: u64,
    pub timestamp: DateTime<Utc>,
    pub entry_count: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotListResult {
    pub snapshots: Vec<MemorySnapshotDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotCreateResult {
    pub snapshot: MemorySnapshotDto,
    pub snapshots: Vec<MemorySnapshotDto>,
    pub stats: MemoryStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotRollbackResult {
    pub snapshot: MemorySnapshotDto,
    pub restored_count: usize,
    pub snapshots: Vec<MemorySnapshotDto>,
    pub stats: MemoryStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProfileFileDto {
    pub kind: String,
    pub scope: String,
    pub path: String,
    pub relative_path: String,
    pub title: String,
    pub content: String,
    pub exists: bool,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct MemoryProfileDefinition {
    kind: String,
    scope: String,
    path: String,
    relative_path: String,
    title: String,
    category: String,
    importance: f32,
}

impl MemoryProfileDefinition {
    fn into_file(self) -> MemoryProfileFileDto {
        MemoryProfileFileDto {
            kind: self.kind,
            scope: self.scope,
            path: self.path,
            relative_path: self.relative_path,
            title: self.title,
            content: String::new(),
            exists: false,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecuteInput {
    pub name: String,
    #[serde(default)]
    pub args: BTreeMap<String, serde_json::Value>,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
}

#[derive(Debug)]
struct PendingQuestion {
    session_id: String,
    resolve: Option<oneshot::Sender<Result<Value, String>>>,
}

#[derive(Debug, Clone)]
struct PendingToolApproval {
    request_id: String,
    session_id: Option<String>,
    tool_name: String,
    args: BTreeMap<String, serde_json::Value>,
    cwd: PathBuf,
    reason: String,
    risk: Option<Value>,
    created_at: i64,
}

impl PendingToolApproval {
    fn to_dto(&self) -> PendingPermissionDto {
        PendingPermissionDto {
            session_id: self.session_id.clone(),
            request_id: self.request_id.clone(),
            tool_name: self.tool_name.clone(),
            args: self.args.clone(),
            reason: self.reason.clone(),
            risk: self.risk.clone(),
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermissionDto {
    pub session_id: Option<String>,
    pub request_id: String,
    pub tool_name: String,
    pub args: BTreeMap<String, serde_json::Value>,
    pub reason: String,
    pub risk: Option<Value>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStoreHealthDto {
    pub backend: String,
    pub ok: bool,
    pub location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgvector: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStoreHealthDto {
    pub backend: String,
    pub postgres_configured: bool,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgvector: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInfoDto {
    pub path: String,
    pub exists: bool,
    pub data_dir: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitModelsFetchInput {
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub protocol: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitModelsFetchResult {
    pub models: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct LegacyPostgresConfig {
    connection_string: String,
    psql_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatusDto {
    pub enabled: bool,
    pub port: u64,
    pub bind: String,
    pub ws_clients: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSummaryDto {
    pub total: usize,
    pub enabled: usize,
    pub ready: usize,
    pub needs_config: usize,
    pub unsupported: usize,
    pub channels: Vec<ChannelStatusDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatusDto {
    #[serde(rename = "type")]
    pub channel_type: String,
    pub name: String,
    pub label: String,
    pub enabled: bool,
    pub configured: bool,
    pub adapter_supported: bool,
    pub status: String,
    pub mention_only: bool,
    pub allowlist_count: usize,
    pub missing: Vec<String>,
    pub extra_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatusDto {
    pub pid: u32,
    pub uptime_sec: u64,
    pub node: String,
    pub platform: String,
    pub cwd: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheckDto {
    pub name: String,
    pub status: DoctorCheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorStatusDto {
    pub status: DoctorCheckStatus,
    pub failed: usize,
    pub warnings: usize,
    pub checks: Vec<DoctorCheckDto>,
}

impl DoctorStatusDto {
    fn from_checks(checks: Vec<DoctorCheckDto>) -> Self {
        let failed = checks
            .iter()
            .filter(|check| check.status == DoctorCheckStatus::Fail)
            .count();
        let warnings = checks
            .iter()
            .filter(|check| check.status == DoctorCheckStatus::Warn)
            .count();
        let status = if failed > 0 {
            DoctorCheckStatus::Fail
        } else if warnings > 0 {
            DoctorCheckStatus::Warn
        } else {
            DoctorCheckStatus::Ok
        };
        Self {
            status,
            failed,
            warnings,
            checks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatusDto {
    pub project_name: String,
    pub project_dir: String,
    pub conversation: ConversationStoreHealthDto,
    pub memory: MemoryStoreHealthDto,
    pub gateway: GatewayStatusDto,
    pub channels: ChannelSummaryDto,
    pub runtime: RuntimeStatusDto,
    pub workers: Vec<WorkerHealth>,
    pub security: SecurityStatusDto,
    pub doctor: DoctorStatusDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatusDto {
    pub sensitive_findings: Vec<SensitiveFinding>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayLogEntryDto {
    pub id: String,
    pub timestamp: i64,
    pub level: String,
    pub scope: String,
    pub message: String,
    pub details: Option<Value>,
}

fn memory_item_from_entry(entry: MemoryEntry) -> MemoryItemDto {
    MemoryItemDto {
        id: entry.id,
        content: entry.content,
        category: entry.category,
        importance: score_to_ui_importance(entry.importance),
        created_at: entry.created_at,
        updated_at: entry.updated_at,
        tags: entry.tags,
        source: "rust-memory".to_string(),
        access_count: entry.access_count,
        last_accessed_at: entry.last_accessed_at,
    }
}

fn memory_snapshot_from_info(info: MemorySnapshotInfo) -> MemorySnapshotDto {
    MemorySnapshotDto {
        version: info.version,
        timestamp: info.timestamp,
        entry_count: info.entry_count,
        reason: info.reason,
    }
}

fn extraction_message_from_chat(message: &ChatMessage) -> ExtractionMessage {
    ExtractionMessage {
        role: match message.role {
            MessageRole::User => ExtractionRole::User,
            MessageRole::Assistant => ExtractionRole::Assistant,
            MessageRole::System => ExtractionRole::System,
            MessageRole::Tool => ExtractionRole::Tool,
        },
        content: message.content.clone(),
        tool_names: Vec::new(),
    }
}

fn pending_memory_to_value(item: PendingMemory) -> Value {
    json!({
        "id": item.id,
        "content": item.content,
        "category": item.category,
        "importance": item.importance,
        "source": item.source,
        "tags": item.tags,
        "confidence": item.confidence,
        "reason": item.reason,
        "sessionId": item.session_id,
        "taskId": item.task_id,
        "runId": item.run_id,
        "traceId": item.trace_id,
        "status": match item.status {
            xuanyu_memory::PendingMemoryStatus::Pending => "pending",
            xuanyu_memory::PendingMemoryStatus::Approved => "approved",
            xuanyu_memory::PendingMemoryStatus::Rejected => "rejected",
        },
        "createdAt": item.created_at,
        "decidedAt": item.decided_at,
        "rejectedUntil": item.rejected_until
    })
}

fn memory_profile_definitions(project_dir: &std::path::Path) -> Vec<MemoryProfileDefinition> {
    let project_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    [
        ("identity", "global", "XuanYu Identity", "identity", 1.0),
        ("user", "global", "User Profile", "user_profile", 1.0),
        ("project", "project", "Project Memory", "project_fact", 0.9),
        ("agents", "project", "Agent Team", "workflow_rule", 0.8),
        (
            "workflow",
            "project",
            "Workflow Rules",
            "workflow_rule",
            0.8,
        ),
        ("lessons", "project", "Lessons Learned", "task_lesson", 0.8),
    ]
    .into_iter()
    .map(
        |(kind, scope, title, category, importance)| MemoryProfileDefinition {
            kind: kind.to_string(),
            scope: scope.to_string(),
            path: if scope == "global" {
                format!("memory://global/{kind}")
            } else {
                format!("memory://project/{project_name}/{kind}")
            },
            relative_path: if scope == "global" {
                format!("db://memory/global/{kind}")
            } else {
                format!("db://memory/project/{project_name}/{kind}")
            },
            title: title.to_string(),
            category: category.to_string(),
            importance,
        },
    )
    .collect()
}

fn memory_profile_context(scope: &str, project_dir: &std::path::Path) -> String {
    if scope == "global" {
        "global".to_string()
    } else {
        format!("project:{}", path_to_string(project_dir))
    }
}

fn find_profile_memory<'a>(
    memories: &'a [MemoryEntry],
    kind: &str,
    scope: &str,
    context: &str,
) -> Option<&'a MemoryEntry> {
    memories
        .iter()
        .filter(|memory| {
            memory.context.as_deref() == Some(context)
                && memory.tags.iter().any(|tag| tag == "profile")
                && memory.tags.iter().any(|tag| tag == kind)
                && memory.tags.iter().any(|tag| tag == scope)
        })
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
}

fn ui_importance_to_score(value: u8) -> f32 {
    (value.clamp(1, 10) as f32) / 10.0
}

fn score_to_ui_importance(value: f32) -> u8 {
    ((value.clamp(0.0, 1.0) * 10.0).round() as u8).clamp(1, 10)
}

fn merge_json(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                if matches!(
                    key.as_str(),
                    "__proto__" | "constructor" | "prototype"
                ) {
                    continue;
                }
                merge_json(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch,
    }
}

fn ensure_json_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("json object")
}

fn legacy_postgres_query_scalar(
    config: &LegacyPostgresConfig,
    sql: &str,
) -> anyhow::Result<String> {
    let output = Command::new(&config.psql_path)
        .arg("-X")
        .arg("-q")
        .arg("-A")
        .arg("-t")
        .arg("--set")
        .arg("ON_ERROR_STOP=1")
        .arg("--pset")
        .arg("footer=off")
        .arg("--dbname")
        .arg(&config.connection_string)
        .arg("-c")
        .arg(sql)
        .output()
        .map_err(|err| {
            anyhow::anyhow!(
                "Cannot execute psql '{}': {err}",
                path_to_string(&config.psql_path)
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "psql '{}' exited with status {}: {}",
        path_to_string(&config.psql_path),
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated".to_string()),
        if stderr.is_empty() { stdout } else { stderr }
    );
}

fn json_object_to_btree(value: Option<&Value>) -> BTreeMap<String, Value> {
    value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}
fn init_is_sqlite_connection(value: &str) -> bool {
    value.trim().to_ascii_lowercase().starts_with("sqlite:")
}

fn init_is_postgres_connection(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("postgres://") || value.starts_with("postgresql://")
}

fn init_sqlite_path_from_connection(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let raw = if lower.starts_with("sqlite://") {
        &trimmed["sqlite://".len()..]
    } else if lower.starts_with("sqlite:") {
        &trimmed["sqlite:".len()..]
    } else {
        return None;
    };
    let decoded = raw.replace("%20", " ");
    let normalized = if decoded.len() > 3
        && decoded.starts_with('/')
        && decoded.as_bytes().get(2).is_some_and(|byte| *byte == b':')
    {
        &decoded[1..]
    } else {
        decoded.as_str()
    };
    if normalized.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(normalized))
    }
}

fn init_sqlite_connection_string(path: &Path) -> String {
    format!("sqlite://{}", path_to_string(path).replace('\\', "/"))
}

fn init_postgres_psql_path() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "psql.exe" } else { "psql" })
}

/// 配置中声明的 PostgreSQL 设置（连接串为空表示未配置）。
struct ConfiguredPostgres {
    connection_string: Option<String>,
    schema: String,
    table_prefix: String,
}

fn configured_postgres(config: &Value) -> ConfiguredPostgres {
    let pg = config.get("memory").and_then(|memory| memory.get("postgres"));
    let connection_string = pg
        .and_then(|item| {
            item.get("connectionString")
                .or_else(|| item.get("connection_string"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let schema = pg
        .and_then(|item| item.get("schema"))
        .and_then(Value::as_str)
        .unwrap_or("public")
        .to_string();
    let table_prefix = pg
        .and_then(|item| {
            item.get("tablePrefix")
                .or_else(|| item.get("table_prefix"))
        })
        .and_then(Value::as_str)
        .unwrap_or("xy")
        .to_string();
    ConfiguredPostgres {
        connection_string,
        schema,
        table_prefix,
    }
}

/// 连接串是否指向引擎自带的内置实例（回环地址 + 内置端口）。
///
/// 安装器会把内置实例的连接串写进配置，这类"本地连接串"仍由引擎负责启动与自愈；
/// 只有真正指向外部主机的连接串才被当作外部数据库，引擎不做任何本地实例管理。
fn postgres_connection_targets_embedded(conn: &str, paths: &EmbeddedPostgresPaths) -> bool {
    const DEFAULT_PORT: u16 = 5432;
    let without_scheme = conn
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(conn);
    let authority = without_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(without_scheme);
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().unwrap_or(DEFAULT_PORT)),
        None => (host_port, DEFAULT_PORT),
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") && port == paths.port
}

fn init_postgres_database_name(connection_string: &str) -> String {
    let without_query = connection_string
        .split_once('?')
        .map(|(head, _)| head)
        .unwrap_or(connection_string);
    without_query
        .rsplit('/')
        .next()
        .filter(|database| !database.is_empty() && !database.contains(':'))
        .map(|database| database.replace("%20", " "))
        .unwrap_or_else(|| "postgres".to_string())
}

fn redact_json_clone(value: &Value) -> Value {
    let mut value = value.clone();
    redact_json_values(&mut value);
    value
}

fn redact_json_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if is_sensitive_json_key(key) {
                    if !value.is_null() {
                        *value = Value::String("***redacted***".to_string());
                    }
                } else {
                    redact_json_values(value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json_values(item);
            }
        }
        _ => {}
    }
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "apikey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "password"
            | "secret"
            | "clientsecret"
            | "connectionstring"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
        || normalized.ends_with("token")
}

fn json_string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn merge_tool_output_data(base: Option<Value>, patch: Value) -> Value {
    let mut base = base.unwrap_or_else(|| json!({}));
    let base_object = ensure_json_object(&mut base);
    if let Some(patch_object) = patch.as_object() {
        for (key, value) in patch_object {
            base_object.insert(key.clone(), value.clone());
        }
    }
    base
}

fn gateway_log_from_event(
    event: xuanyu_eventbus::ObservabilityEvent,
) -> Option<GatewayLogEntryDto> {
    match event.kind {
        ObservabilityKind::Log => Some(GatewayLogEntryDto {
            id: event.id,
            timestamp: event.created_at.timestamp_millis(),
            level: event
                .payload
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("info")
                .to_string(),
            scope: event
                .payload
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("core")
                .to_string(),
            message: event
                .payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("runtime event")
                .to_string(),
            details: Some(event.payload),
        }),
        ObservabilityKind::Health => {
            let status = event
                .payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok");
            let component = event
                .payload
                .get("component")
                .and_then(Value::as_str)
                .unwrap_or("component");
            let detail = event.payload.get("detail").and_then(Value::as_str);
            Some(GatewayLogEntryDto {
                id: event.id,
                timestamp: event.created_at.timestamp_millis(),
                level: match status {
                    "down" => "error",
                    "degraded" => "warn",
                    _ => "info",
                }
                .to_string(),
                scope: "health".to_string(),
                message: detail
                    .map(|value| format!("{component}: {value}"))
                    .unwrap_or_else(|| format!("{component}: {status}")),
                details: Some(event.payload),
            })
        }
        _ => None,
    }
}

fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

/** User-facing path: strip Windows `\\?\` extended prefix. */
fn path_to_display_string(path: &std::path::Path) -> String {
    let raw = path_to_string(path);
    strip_windows_extended_path(&raw)
}

fn strip_windows_extended_path(path: &str) -> String {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = trimmed.strip_prefix("//?/") {
        rest.replace('/', "\\")
    } else {
        trimmed.to_string()
    }
}

/** Compact postgres URL for status / doctor text (host:port/db). */
fn summarize_db_location(location: &str) -> String {
    let trimmed = location.trim();
    if trimmed.is_empty() {
        return "—".to_string();
    }
    let rest = trimmed
        .strip_prefix("postgres://")
        .or_else(|| trimmed.strip_prefix("postgresql://"))
        .unwrap_or(trimmed);
    // user:pass@host:port/db → take after '@' when present
    let authority = rest.rsplit_once('@').map(|(_, right)| right).unwrap_or(rest);
    let (hostport, db) = match authority.split_once('/') {
        Some((hostport, path)) => (hostport, path.split('?').next().unwrap_or(path)),
        None => (authority.split('?').next().unwrap_or(authority), ""),
    };
    if !hostport.is_empty() {
        return if db.is_empty() {
            hostport.to_string()
        } else {
            format!("{hostport}/{db}")
        };
    }
    if trimmed.len() > 48 {
        format!("{}…", &trimmed[..45])
    } else {
        trimmed.to_string()
    }
}
const DEFAULT_MODEL_FETCH_TIMEOUT_MS: u64 = 15_000;

struct StoredModelFetchCredentials {
    api_key: Option<String>,
    base_url: Option<String>,
}

fn non_empty_json_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// UI 下发的配置密钥是空的（传输脱敏）；探索时从本地已存配置回填。
fn resolve_stored_model_fetch_credentials(
    config: &Value,
    provider_type: &str,
) -> StoredModelFetchCredentials {
    let provider_type = provider_type.trim();
    let main = config.get("provider");
    let main_type = main
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !provider_type.is_empty() && main_type.eq_ignore_ascii_case(provider_type) {
        return StoredModelFetchCredentials {
            api_key: non_empty_json_str(main.and_then(|value| value.get("apiKey"))),
            base_url: non_empty_json_str(main.and_then(|value| value.get("baseUrl"))),
        };
    }

    if let Some(models) = config
        .pointer("/modelRouting/auxiliaryModels")
        .and_then(Value::as_object)
    {
        for value in models.values() {
            let aux_type = value
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !provider_type.is_empty() && aux_type.eq_ignore_ascii_case(provider_type) {
                let api_key = non_empty_json_str(value.get("apiKey"));
                let base_url = non_empty_json_str(value.get("baseUrl"));
                if api_key.is_some() || base_url.is_some() {
                    return StoredModelFetchCredentials { api_key, base_url };
                }
            }
        }
    }

    if let Some(models) = config.pointer("/subAgents/models").and_then(Value::as_object) {
        for value in models.values() {
            let aux_type = value
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !provider_type.is_empty() && aux_type.eq_ignore_ascii_case(provider_type) {
                let api_key = non_empty_json_str(value.get("apiKey"));
                let base_url = non_empty_json_str(value.get("baseUrl"));
                if api_key.is_some() || base_url.is_some() {
                    return StoredModelFetchCredentials { api_key, base_url };
                }
            }
        }
    }

    // 未匹配到同类型时，仍可用主供应商凭据（对话页只配了主模型）。
    StoredModelFetchCredentials {
        api_key: non_empty_json_str(main.and_then(|value| value.get("apiKey"))),
        base_url: non_empty_json_str(main.and_then(|value| value.get("baseUrl"))),
    }
}

async fn fetch_models_for_provider(input: InitModelsFetchInput) -> anyhow::Result<Vec<String>> {
    let base_url = input.base_url.unwrap_or_default();
    if base_url.trim().is_empty() {
        anyhow::bail!("baseUrl is required to fetch models");
    }

    let timeout_ms = input
        .timeout_ms
        .unwrap_or(DEFAULT_MODEL_FETCH_TIMEOUT_MS)
        .clamp(1_000, 60_000);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent("XuanYu-Agent")
        .build()?;

    if input.provider_type.eq_ignore_ascii_case("ollama") {
        let models = try_ollama_models(&client, &base_url).await?;
        if models.is_empty() {
            anyhow::bail!("no models returned from Ollama");
        }
        return Ok(models);
    }

    let api_key = input.api_key.unwrap_or_default();
    if api_key.trim().is_empty() {
        anyhow::bail!("apiKey is required to fetch models");
    }

    let models = if input
        .protocol
        .as_deref()
        .unwrap_or("openai")
        .eq_ignore_ascii_case("anthropic")
    {
        try_anthropic_models(&client, &base_url, api_key.trim()).await?
    } else {
        try_openai_models(&client, &base_url, api_key.trim()).await?
    };
    if models.is_empty() {
        anyhow::bail!("provider returned an empty model list");
    }
    Ok(models)
}

async fn try_openai_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<String>> {
    let endpoint = with_model_fetch_path(base_url, "/models");
    let data = fetch_json(client.get(endpoint).bearer_auth(api_key)).await?;
    Ok(parse_openai_models(&data))
}

async fn try_anthropic_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<String>> {
    let mut last_error = None;
    for endpoint in [
        with_model_fetch_path(base_url, "/v1/models"),
        with_model_fetch_path(base_url, "/models"),
    ] {
        match fetch_json(
            client
                .get(endpoint)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01"),
        )
        .await
        {
            Ok(data) => {
                let models = parse_anthropic_models(&data);
                if !models.is_empty() {
                    return Ok(models);
                }
                last_error = Some(anyhow::anyhow!("empty Anthropic model list"));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("failed to fetch Anthropic models")))
}

async fn try_ollama_models(
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<Vec<String>> {
    let clean_base = normalize_model_fetch_base(base_url);
    let root = if clean_base.to_ascii_lowercase().ends_with("/v1") {
        &clean_base[..clean_base.len().saturating_sub(3)]
    } else {
        &clean_base
    };
    let mut last_error = None;
    for endpoint in [
        with_model_fetch_path(root, "/api/tags"),
        with_model_fetch_path(&clean_base, "/models"),
    ] {
        match fetch_json(client.get(endpoint.clone())).await {
            Ok(data) => {
                let models = if endpoint.ends_with("/api/tags") {
                    parse_ollama_models(&data)
                } else {
                    parse_openai_models(&data)
                };
                if !models.is_empty() {
                    return Ok(models);
                }
                last_error = Some(anyhow::anyhow!("empty Ollama model list"));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("failed to fetch Ollama models")))
}

async fn fetch_json(request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status.as_u16());
    }
    Ok(response.json::<Value>().await?)
}

fn normalize_model_fetch_base(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

fn with_model_fetch_path(base_url: &str, path: &str) -> String {
    let clean = normalize_model_fetch_base(base_url);
    let safe_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{clean}{safe_path}")
}

fn maybe_extract_model_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    if id.is_empty() {
        return None;
    }
    let lower = id.to_ascii_lowercase();
    if ["embedding", "moderation", "tts", "asr"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return None;
    }
    Some(id.to_string())
}

fn push_model_id(models: &mut Vec<String>, raw: Option<&Value>) {
    if models.len() >= 120 {
        return;
    }
    let Some(id) = raw.and_then(Value::as_str).and_then(maybe_extract_model_id) else {
        return;
    };
    if !models.iter().any(|item| item == &id) {
        models.push(id);
    }
}

fn parse_openai_models(data: &Value) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(items) = data.get("data").and_then(Value::as_array) {
        for item in items {
            push_model_id(&mut models, item.get("id"));
        }
    }
    models
}

fn parse_anthropic_models(data: &Value) -> Vec<String> {
    parse_openai_models(data)
}

fn parse_ollama_models(data: &Value) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(items) = data.get("models").and_then(Value::as_array) {
        for item in items {
            push_model_id(&mut models, item.get("name").or_else(|| item.get("model")));
        }
    }
    models
}

fn backup_copy_path(
    source: &Path,
    dest: &Path,
    relative: &Path,
    entries: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let metadata = fs::metadata(source)?;
    if metadata.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            let source_path = entry.path();
            let dest_path = dest.join(&name);
            let relative_path = relative.join(&name);
            backup_copy_path(&source_path, &dest_path, &relative_path, entries)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, dest)?;
    entries.push(json!({
        "path": path_to_string(relative),
        "sizeBytes": metadata.len()
    }));
    Ok(())
}

fn backup_restore_path(
    source: &Path,
    data_dir: &Path,
    files_root: &Path,
    restored_files: &mut Vec<String>,
    total_bytes: &mut u64,
) -> anyhow::Result<()> {
    let metadata = fs::metadata(source)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            backup_restore_path(
                &entry.path(),
                data_dir,
                files_root,
                restored_files,
                total_bytes,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Ok(());
    }
    let relative = source.strip_prefix(files_root).map_err(|_| {
        anyhow::anyhow!(
            "Backup file '{}' is outside '{}'",
            path_to_string(source),
            path_to_string(files_root)
        )
    })?;
    if relative.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        anyhow::bail!(
            "Backup file path is not relative: {}",
            path_to_string(relative)
        );
    }
    let dest = data_dir.join(relative);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, &dest)?;
    restored_files.push(path_to_string(relative));
    *total_bytes += metadata.len();
    Ok(())
}

fn release_readiness_snapshot(workspace_root: &Path) -> Value {
    let release_root = locate_release_workspace_root(workspace_root);
    let package_json_path = release_root.join("package.json");
    let desktop_package_json_path = release_root.join("apps/desktop-ui/package.json");
    let tauri_config_path = release_root.join("crates/xuanyu-app/tauri.conf.json");
    let root_package = read_json_file(&package_json_path);
    let desktop_package = read_json_file(&desktop_package_json_path);
    let tauri_config = read_json_file(&tauri_config_path);
    let root_scripts = root_package
        .as_ref()
        .and_then(|value| value.get("scripts"))
        .and_then(Value::as_object);

    let mut scripts = BTreeMap::new();
    for script in [
        "dev",
        "build",
        "typecheck",
        "smoke",
        "build:all",
        "build:installer",
        "release:manifest",
    ] {
        scripts.insert(
            script.to_string(),
            package_script_status(root_scripts, script),
        );
    }

    let file_checks = vec![
        file_check("devScript", release_root.join("scripts/dev.ps1")),
        file_check("buildScript", release_root.join("scripts/build.ps1")),
        file_check("smokeScript", release_root.join("scripts/smoke.ps1")),
        file_check(
            "installerScript",
            release_root.join("scripts/build-installer.ps1"),
        ),
        file_check(
            "releaseManifestScript",
            release_root.join("scripts/release-manifest.ps1"),
        ),
        file_check(
            "deploymentDocs",
            release_root.join("docs/deployment/README.md"),
        ),
        file_check(
            "frontendDist",
            release_root.join("apps/desktop-ui/dist/index.html"),
        ),
    ];

    let tauri_bundle_active = tauri_config
        .as_ref()
        .and_then(|value| value.pointer("/bundle/active"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tauri_targets = tauri_config
        .as_ref()
        .and_then(|value| value.pointer("/bundle/targets"))
        .cloned()
        .unwrap_or(Value::Null);
    let updater_configured = tauri_config
        .as_ref()
        .and_then(|value| value.pointer("/plugins/updater"))
        .is_some();

    let checks = vec![
        release_check(
            "rootPackage",
            package_json_path.exists(),
            "Root package.json is readable.",
        ),
        release_check(
            "smokeCommand",
            root_scripts
                .and_then(|scripts| scripts.get("smoke"))
                .and_then(Value::as_str)
                .is_some(),
            "pnpm run smoke is configured.",
        ),
        release_check(
            "bundleCommand",
            root_scripts
                .and_then(|scripts| scripts.get("bundle"))
                .and_then(Value::as_str)
                .is_some(),
            "pnpm run bundle is configured.",
        ),
        release_check(
            "releaseManifestCommand",
            root_scripts
                .and_then(|scripts| scripts.get("release:manifest"))
                .and_then(Value::as_str)
                .is_some(),
            "pnpm run release:manifest is configured.",
        ),
        release_check(
            "releaseManifestScript",
            release_root.join("scripts/release-manifest.ps1").is_file(),
            "Static updater manifest generator is present.",
        ),
        release_check(
            "tauriBundle",
            tauri_bundle_active,
            "Tauri bundle output is enabled.",
        ),
        release_check(
            "deploymentDocs",
            release_root.join("docs/deployment/README.md").is_file(),
            "Deployment notes are present.",
        ),
        release_check(
            "updaterConfig",
            updater_configured,
            "Tauri updater plugin is configured.",
        ),
    ];

    let gaps = checks
        .iter()
        .filter(|check| {
            check
                .get("ok")
                .and_then(Value::as_bool)
                .map(|ok| !ok)
                .unwrap_or(false)
        })
        .filter_map(|check| check.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();

    json!({
        "workspaceRoot": path_to_string(&release_root),
        "coreVersion": env!("CARGO_PKG_VERSION"),
        "environment": {
            "envScript": "D:\\develop\\environment\\xuanyu-agent-app-env.ps1",
            "envScriptExists": Path::new("D:\\develop\\environment\\xuanyu-agent-app-env.ps1").is_file()
        },
        "package": {
            "name": root_package
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str),
            "version": root_package
                .as_ref()
                .and_then(|value| value.get("version"))
                .and_then(Value::as_str),
            "packageManager": root_package
                .as_ref()
                .and_then(|value| value.get("packageManager"))
                .and_then(Value::as_str),
            "desktopVersion": desktop_package
                .as_ref()
                .and_then(|value| value.get("version"))
                .and_then(Value::as_str),
            "scripts": scripts
        },
        "tauri": {
            "configPath": path_to_string(&tauri_config_path),
            "productName": tauri_config
                .as_ref()
                .and_then(|value| value.get("productName"))
                .and_then(Value::as_str),
            "version": tauri_config
                .as_ref()
                .and_then(|value| value.get("version"))
                .and_then(Value::as_str),
            "identifier": tauri_config
                .as_ref()
                .and_then(|value| value.get("identifier"))
                .and_then(Value::as_str),
            "bundleActive": tauri_bundle_active,
            "bundleTargets": tauri_targets,
            "updaterConfigured": updater_configured
        },
        "files": file_checks,
        "checks": checks,
        "gaps": gaps
    })
}

fn locate_release_workspace_root(start: &Path) -> PathBuf {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };

    loop {
        if current.join("package.json").is_file()
            && current.join("crates/xuanyu-app/tauri.conf.json").is_file()
        {
            return current;
        }

        if !current.pop() {
            return start.to_path_buf();
        }
    }
}

fn read_json_file(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn package_script_status(scripts: Option<&serde_json::Map<String, Value>>, name: &str) -> Value {
    let command = scripts
        .and_then(|scripts| scripts.get(name))
        .and_then(Value::as_str);
    json!({
        "configured": command.is_some(),
        "command": command
    })
}

fn file_check(name: &str, path: PathBuf) -> Value {
    json!({
        "name": name,
        "path": path_to_string(&path),
        "exists": path.is_file()
    })
}

fn release_check(name: &str, ok: bool, detail: &str) -> Value {
    json!({
        "name": name,
        "ok": ok,
        "detail": detail
    })
}

fn write_single_file_zip(path: &Path, entry_name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let name = entry_name.as_bytes();
    let name_len = zip_u16(name.len(), "entry name")?;
    let size = zip_u32(bytes.len(), "entry data")?;
    let crc = crc32(bytes);
    let mut file = fs::File::create(path)?;

    write_u32_le(&mut file, 0x0403_4b50)?;
    write_u16_le(&mut file, 20)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u32_le(&mut file, crc)?;
    write_u32_le(&mut file, size)?;
    write_u32_le(&mut file, size)?;
    write_u16_le(&mut file, name_len)?;
    write_u16_le(&mut file, 0)?;
    file.write_all(name)?;
    file.write_all(bytes)?;

    let central_offset = 30u32 + u32::from(name_len) + size;
    write_u32_le(&mut file, 0x0201_4b50)?;
    write_u16_le(&mut file, 20)?;
    write_u16_le(&mut file, 20)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u32_le(&mut file, crc)?;
    write_u32_le(&mut file, size)?;
    write_u32_le(&mut file, size)?;
    write_u16_le(&mut file, name_len)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u32_le(&mut file, 0)?;
    write_u32_le(&mut file, 0)?;
    file.write_all(name)?;

    let central_size = 46u32 + u32::from(name_len);
    write_u32_le(&mut file, 0x0605_4b50)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 1)?;
    write_u16_le(&mut file, 1)?;
    write_u32_le(&mut file, central_size)?;
    write_u32_le(&mut file, central_offset)?;
    write_u16_le(&mut file, 0)?;
    Ok(())
}

fn zip_u16(value: usize, label: &str) -> anyhow::Result<u16> {
    u16::try_from(value).map_err(|_| anyhow::anyhow!("{label} too large for zip"))
}

fn zip_u32(value: usize, label: &str) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| anyhow::anyhow!("{label} too large for zip"))
}

fn write_u16_le(writer: &mut impl Write, value: u16) -> anyhow::Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32_le(writer: &mut impl Write, value: u32) -> anyhow::Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn plugin_install_type(payload: &Value) -> &str {
    payload
        .get("install")
        .and_then(Value::as_object)
        .and_then(|install| install.get("type"))
        .or_else(|| payload.get("installType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
}

fn is_local_plugin_install_type(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "local" | "path" | "directory" | "manifest"
    )
}

fn local_plugin_install_path(payload: &Value) -> Option<String> {
    let install = payload.get("install").and_then(Value::as_object);
    string_field_in_object(
        install,
        &["path", "directory", "dir", "manifestPath", "pluginPath"],
    )
    .or_else(|| {
        string_field_in_object(
            payload.as_object(),
            &["path", "directory", "dir", "manifestPath", "pluginPath"],
        )
    })
}

fn local_plugin_manifest_paths(source_path: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let metadata = fs::metadata(source_path).map_err(|err| {
        anyhow::anyhow!(
            "Cannot read local plugin source '{}': {err}",
            path_to_string(source_path)
        )
    })?;
    if metadata.is_dir() {
        let manifest_path = source_path.join("plugin.json");
        if !manifest_path.exists() {
            anyhow::bail!(
                "Local plugin directory '{}' does not contain plugin.json.",
                path_to_string(source_path)
            );
        }
        return Ok((source_path.to_path_buf(), manifest_path));
    }
    if metadata.is_file() {
        let source_dir = source_path.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "Local plugin manifest '{}' has no parent directory.",
                path_to_string(source_path)
            )
        })?;
        return Ok((source_dir.to_path_buf(), source_path.to_path_buf()));
    }
    anyhow::bail!(
        "Local plugin source '{}' is not a file or directory.",
        path_to_string(source_path)
    );
}

fn plugin_source_is_configured(source_dir: &Path, plugin_directories: &[PathBuf]) -> bool {
    plugin_directories
        .iter()
        .filter_map(|directory| fs::canonicalize(directory).ok())
        .any(|directory| source_dir.starts_with(directory))
}

fn copy_plugin_directory(source_dir: &Path, target_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(target_dir).map_err(|err| {
        anyhow::anyhow!(
            "Cannot create plugin target '{}': {err}",
            path_to_string(target_dir)
        )
    })?;
    for entry in fs::read_dir(source_dir).map_err(|err| {
        anyhow::anyhow!(
            "Cannot read plugin source '{}': {err}",
            path_to_string(source_dir)
        )
    })? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target_dir.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            anyhow::bail!(
                "Refusing to install plugin through symlink '{}'.",
                path_to_string(&source_path)
            );
        }
        if file_type.is_dir() {
            copy_plugin_directory(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|err| {
                anyhow::anyhow!(
                    "Cannot copy plugin file '{}' to '{}': {err}",
                    path_to_string(&source_path),
                    path_to_string(&target_path)
                )
            })?;
        }
    }
    Ok(())
}

fn string_field_in_object(
    object: Option<&serde_json::Map<String, Value>>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .and_then(|item| item.get(*key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn resolve_path_from(base: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn slugify_identifier(value: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            output.push(ch);
            last_dash = false;
        } else if !last_dash {
            output.push('-');
            last_dash = true;
        }
    }
    let output = output.trim_matches('-').to_string();
    if output.is_empty() {
        "plugin".to_string()
    } else {
        output
    }
}

struct PluginManualInstallHint {
    command_line: Option<String>,
    args: Vec<String>,
    message: String,
}

fn plugin_manual_install_hint(payload: &Value) -> PluginManualInstallHint {
    let install_type = plugin_install_type(payload).to_ascii_lowercase();
    let install = payload.get("install").and_then(Value::as_object);
    match install_type.as_str() {
        "npm" => {
            let package = string_field_in_object(install, &["package", "name"]);
            let args = package
                .as_ref()
                .map(|package| vec!["install".to_string(), package.clone()])
                .unwrap_or_else(|| vec!["install".to_string()]);
            PluginManualInstallHint {
                command_line: Some(format_command_line("npm", &args)),
                args,
                message: package
                    .map(|package| {
                        format!(
                            "Install npm package '{package}' manually, then place a plugin.json directory under the configured plugin folder."
                        )
                    })
                    .unwrap_or_else(|| {
                        "Install this npm plugin manually, then place a plugin.json directory under the configured plugin folder.".to_string()
                    }),
            }
        }
        "mcp" => {
            let command =
                string_field_in_object(install, &["command"]).unwrap_or_else(|| "npx".to_string());
            let args = install
                .and_then(|item| item.get("args"))
                .map(json_string_array)
                .unwrap_or_default();
            PluginManualInstallHint {
                command_line: Some(format_command_line(&command, &args)),
                args,
                message: "MCP plugin installation is pending the governed MCP client flow. Configure this server manually for now.".to_string(),
            }
        }
        _ => PluginManualInstallHint {
            command_line: None,
            args: Vec::new(),
            message: "This plugin install type is not supported by the governed installer yet."
                .to_string(),
        },
    }
}

fn json_string_array(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect(),
        Value::String(text) => text
            .split_whitespace()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn format_command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command.to_string())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.chars().any(char::is_whitespace) {
                format!("\"{}\"", part.replace('"', "\\\""))
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn project_dir_from_config(config: &Value) -> Option<PathBuf> {
    config
        .get("projectDir")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn watchdog_settings_from_config(config: &Value) -> WatchdogSettings {
    let watchdog = config
        .get("subAgents")
        .or_else(|| config.get("subagents"))
        .and_then(|value| value.get("watchdog"));
    WatchdogSettings {
        max_retries: watchdog
            .and_then(|value| value.get("maxRetries").or_else(|| value.get("max_retries")))
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32,
        auto_recover: watchdog
            .and_then(|value| value.get("autoRecover").or_else(|| value.get("auto_recover")))
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }
}

fn compact_context_line(value: &str, max_chars: usize) -> String {
    let mut text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max_chars {
        text = text.chars().take(max_chars).collect();
        text.push_str("...");
    }
    text
}

fn user_turn_order(messages: &[ConversationMessageRecord]) -> Vec<String> {
    let mut order = Vec::new();
    for message in messages {
        if message.role == "user" {
            if let Some(turn_id) = &message.turn_id {
                if !order.contains(turn_id) {
                    order.push(turn_id.clone());
                }
            }
        }
    }
    order
}

fn normalize_rewind_mode(mode: Option<&str>) -> String {
    match mode.unwrap_or("both") {
        "conversation" => "conversation".to_string(),
        "workspace" => "workspace".to_string(),
        _ => "both".to_string(),
    }
}

const ATTACHMENT_TEXT_EXTRACT_LIMIT_CHARS: usize = 8_000;

fn model_input_with_attachments(content: &str, attachments: Option<&[ChatAttachment]>) -> String {
    let Some(attachments) = attachments.filter(|items| !items.is_empty()) else {
        return content.to_string();
    };
    let mut parts = Vec::new();
    if !content.trim().is_empty() {
        parts.push(content.to_string());
    }
    parts.push(format!(
        "[Attachments]\n{}",
        attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| attachment_model_context(index + 1, attachment))
            .collect::<Vec<_>>()
            .join("\n\n")
    ));
    parts.join("\n\n")
}

fn attachment_model_context(index: usize, attachment: &ChatAttachment) -> String {
    if attachment.kind.eq_ignore_ascii_case("image") {
        return format!(
            "{index}. [Image: {}] mime={}, size={} bytes. Image pixels are not sent through the current text model path; ask for a description or use a vision-capable route when visual inspection is required.",
            attachment.name, attachment.mime_type, attachment.size_bytes
        );
    }
    if let Some(text) = attachment
        .extracted_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .map(|text| truncate_attachment_text(text, ATTACHMENT_TEXT_EXTRACT_LIMIT_CHARS))
        .or_else(|| extract_text_from_attachment(attachment, ATTACHMENT_TEXT_EXTRACT_LIMIT_CHARS))
    {
        format!(
            "{index}. [File: {}] mime={}, size={} bytes\n{}",
            attachment.name, attachment.mime_type, attachment.size_bytes, text
        )
    } else {
        format!(
            "{index}. [File: {}] mime={}, size={} bytes. Content was not text-decodable from the attachment data URL.",
            attachment.name, attachment.mime_type, attachment.size_bytes
        )
    }
}

fn extract_text_from_attachment(attachment: &ChatAttachment, max_chars: usize) -> Option<String> {
    if !attachment.kind.eq_ignore_ascii_case("file") || !is_text_like_attachment(attachment) {
        return None;
    }
    let bytes = decode_data_url_base64(&attachment.data_url)?;
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(truncate_attachment_text(&text, max_chars))
}

fn is_text_like_attachment(attachment: &ChatAttachment) -> bool {
    let name = attachment.name.to_ascii_lowercase();
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();
    if matches!(
        extension,
        "md" | "markdown"
            | "txt"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "xml"
            | "csv"
            | "tsv"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "py"
            | "java"
            | "go"
            | "rs"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "sql"
            | "sh"
            | "bash"
            | "zsh"
            | "toml"
            | "ini"
            | "cfg"
            | "conf"
            | "log"
            | "svg"
    ) {
        return true;
    }
    let mime = attachment.mime_type.to_ascii_lowercase();
    mime.starts_with("text/")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-yaml"
                | "application/yaml"
                | "application/markdown"
                | "application/x-markdown"
                | "application/x-json"
                | "application/ld+json"
        )
}

fn decode_data_url_base64(value: &str) -> Option<Vec<u8>> {
    let data = value
        .split_once(";base64,")
        .map(|(_, data)| data)
        .or_else(|| value.split_once(',').map(|(_, data)| data))
        .unwrap_or(value)
        .trim();
    if data.is_empty() {
        return Some(Vec::new());
    }
    STANDARD.decode(data).ok()
}

fn truncate_attachment_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n\n...[truncated; use file_read for the full attachment content]");
    truncated
}

fn split_token_estimate(messages: &[ChatMessage]) -> (u64, u64) {
    let mut input = 0u64;
    let mut output = 0u64;
    for message in messages {
        let tokens = estimate_tokens(&message.content);
        match message.role {
            MessageRole::Assistant | MessageRole::Tool => output += tokens,
            MessageRole::User | MessageRole::System => input += tokens,
        }
    }
    (input, output)
}

fn context_policy_used_tokens(
    policy: &Value,
    memory_entry_count: u64,
    key_fact_tokens: u64,
    key_fact_count: usize,
) -> u64 {
    context_policy_budget_sections(policy, memory_entry_count, key_fact_tokens, key_fact_count)
        .iter()
        .map(|section| {
            section
                .get("usedTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum()
}

fn context_policy_budget_sections(
    policy: &Value,
    memory_entry_count: u64,
    key_fact_tokens: u64,
    key_fact_count: usize,
) -> Vec<Value> {
    let auto_sections = policy_auto_sections(policy);
    let memory_limit = policy
        .get("memoryAutoInjectLimit")
        .and_then(Value::as_u64)
        .unwrap_or(8);
    let memory_active = policy_bool(policy, "injectRetrievedMemory")
        || auto_sections.iter().any(|section| section == "memory");
    let memory_used = if memory_active {
        memory_entry_count.min(memory_limit) * 64
    } else {
        0
    };
    [
        (
            "memory",
            "memory",
            "Retrieved long-term memories",
            70,
            memory_limit * 64,
            memory_used,
        ),
        (
            "memory_profile",
            "memory_profile",
            "Explicit memory profiles",
            60,
            3_000,
            0,
        ),
        (
            "project_instructions",
            "project",
            "Project instruction files",
            55,
            2_000,
            0,
        ),
        ("skills", "skills", "Matched workflow skills", 45, 1_200, 0),
        (
            "key_facts",
            "key_facts",
            "Runtime key facts",
            90,
            1_500,
            key_fact_tokens,
        ),
        (
            "delegation",
            "delegation",
            "Sub-agent delegation routing",
            65,
            800,
            if policy_bool(policy, "injectDelegation") {
                160
            } else {
                0
            },
        ),
    ]
    .into_iter()
    .map(|(id, section_type, title, priority, allocated, used)| {
        let active = match id {
            "memory" => memory_active,
            "memory_profile" => {
                policy_bool(policy, "injectMemoryProfile")
                    || auto_sections.iter().any(|section| section == id)
            }
            "project_instructions" => {
                policy_bool(policy, "injectProjectInstructions")
                    || auto_sections.iter().any(|section| section == id)
            }
            "skills" => {
                policy_bool(policy, "injectSkills")
                    || auto_sections.iter().any(|section| section == id)
            }
            "key_facts" => policy_bool(policy, "injectKeyFacts"),
            "delegation" => policy_bool(policy, "injectDelegation"),
            _ => auto_sections.iter().any(|section| section == id),
        };
        json!({
            "id": id,
            "type": section_type,
            "title": title,
            "priority": priority,
            "allocatedTokens": allocated,
            "usedTokens": used,
            "dropped": false,
            "truncated": false,
            "loaded": active,
            "itemCount": if id == "key_facts" { key_fact_count } else { 0 },
            "reason": if active {
                context_section_activation_reason(policy, id)
            } else {
                "available through load_memories"
            }
        })
    })
    .collect()
}

fn token_breakdown(
    input_tokens: u64,
    output_tokens: u64,
    tool_schema_tokens: u64,
    context_section_tokens: u64,
    max_tokens: u64,
    max_output_tokens: u64,
) -> Value {
    let total = input_tokens + output_tokens + tool_schema_tokens + context_section_tokens;
    json!({
        "inputTokens": input_tokens,
        "outputTokens": output_tokens,
        "totalTokens": total,
        "systemTokens": context_section_tokens,
        "userTokens": input_tokens,
        "assistantTokens": output_tokens,
        "toolCallTokens": 0,
        "toolResultTokens": 0,
        "toolSchemaTokens": tool_schema_tokens,
        "messageOverheadTokens": 0,
        "contextLength": max_tokens,
        "maxInputTokens": max_tokens.saturating_sub(max_output_tokens),
        "maxOutputTokens": max_output_tokens,
        "remainingTokens": max_tokens.saturating_sub(total),
        "estimated": true,
        "method": "rust-char-estimate",
        "model": "xuanyu-core",
        "providerType": "desktop-rust"
    })
}

struct ContextAllocationInput<'a> {
    messages: &'a [ChatMessage],
    max_tokens: u64,
    tool_schema_tokens: u64,
    used_tokens: u64,
    policy: &'a Value,
    memory_entry_count: u64,
    key_fact_tokens: u64,
    key_fact_count: usize,
}

fn context_allocation(input: ContextAllocationInput<'_>) -> Value {
    let ContextAllocationInput {
        messages,
        max_tokens,
        tool_schema_tokens,
        used_tokens,
        policy,
        memory_entry_count,
        key_fact_tokens,
        key_fact_count,
    } = input;
    let mut sections = vec![
        json!({
            "id": "conversation",
            "type": "messages",
            "title": "Conversation history",
            "priority": 100,
            "allocatedTokens": max_tokens.saturating_sub(tool_schema_tokens),
            "usedTokens": used_tokens.saturating_sub(tool_schema_tokens),
            "dropped": false,
            "truncated": false,
            "messageCount": messages.len()
        }),
        json!({
            "id": "toolCatalog",
            "type": "tools",
            "title": "Tool catalog (deferred)",
            "priority": 80,
            "allocatedTokens": tool_schema_tokens,
            "usedTokens": tool_schema_tokens,
            "dropped": false,
            "truncated": false
        }),
    ];
    sections.extend(context_policy_budget_sections(
        policy,
        memory_entry_count,
        key_fact_tokens,
        key_fact_count,
    ));
    json!({
        "sections": sections,
        "maxInputTokens": max_tokens,
        "toolSchemaTokens": tool_schema_tokens,
        "usedInputTokens": used_tokens,
        "remainingInputTokens": max_tokens.saturating_sub(used_tokens),
        "droppedMessageCount": 0,
        "truncatedMessageCount": 0
    })
}

fn build_memory_retrieval_query(seed: &str, messages: &[ChatMessage]) -> Option<String> {
    let mut parts = Vec::new();
    let seed = seed.trim();
    if !seed.is_empty() {
        parts.push(compact_context_line(seed, 800));
    }
    if let Some(assistant) = messages.iter().rev().find(|message| {
        message.role == MessageRole::Assistant && !message.content.trim().is_empty()
    }) {
        parts.push(compact_context_line(assistant.content.trim(), 400));
    }
    for message in messages
        .iter()
        .rev()
        .filter(|message| message.role == MessageRole::Tool && !message.content.trim().is_empty())
        .take(2)
    {
        parts.push(format!(
            "[tool] {}",
            compact_context_line(message.content.trim(), 300)
        ));
    }
    let query = parts.join("\n").trim().to_string();
    (!query.is_empty()).then_some(query)
}

fn infer_mainbrain_intent_from_messages(messages: &[ChatMessage]) -> &'static str {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map(|message| fallback_mainbrain_intent(&message.content))
        .unwrap_or("chat")
}

fn should_maintain_run_plan_for_intent(intent: &str) -> bool {
    intent == "task"
}

fn create_initial_run_plan(goal: &str, intent: &str) -> Value {
    let now = Utc::now().timestamp_millis();
    let is_task_intent = intent == "task";
    json!({
        "goal": goal,
        "constraints": [
            "遵循当前项目结构和现有实现风格",
            if is_task_intent {
                "多步任务由主脑拆解并异步委派给子 Agent，主脑不直接改代码"
            } else {
                "需要工具调用时先经过权限策略"
            },
            if is_task_intent {
                "子 Agent 进度进入独立 ledger，主脑只在用户询问时汇报"
            } else {
                "完成后尽可能运行可用校验"
            },
        ],
        "risks": [
            "模型输出、工具执行或权限等待可能中断长任务",
            if is_task_intent {
                "子 Agent 后台执行可能阻塞，需要 watchdog 和任务状态兜底"
            } else {
                "上下文压缩可能遗漏细节，需要 checkpoint 兜底"
            },
        ],
        "acceptanceCriteria": [
            "用户目标有明确结果",
            if is_task_intent {
                "复杂任务至少产生一个后台 taskId 并进入 Task Control Plane"
            } else {
                "关键工具调用和状态变化可在 Run 时间线中审计"
            },
            if is_task_intent {
                "主脑委派后立即回复用户并回到可交互状态"
            } else {
                "失败时保留可恢复的 checkpoint"
            },
        ],
        "todos": [
            { "id": "understand", "title": "理解目标和约束", "status": "pending", "updatedAt": now },
            {
                "id": "execute",
                "title": if is_task_intent { "拆解并委派给子 Agent" } else { "执行模型和工具循环" },
                "status": "pending",
                "updatedAt": now
            },
            {
                "id": "verify",
                "title": if is_task_intent { "记录委派摘要并进入后台监控" } else { "验证结果并保存 checkpoint" },
                "status": "pending",
                "updatedAt": now
            },
        ],
        "createdAt": now,
        "updatedAt": now,
    })
}

fn fallback_mainbrain_intent(content: &str) -> &'static str {
    let text = content.to_ascii_lowercase();
    if regex_is_match(
        &text,
        r"review|code review|security audit|审查|评审|安全审计|评估方案",
    ) {
        return "review";
    }
    if regex_is_match(
        &text,
        r"fix|bug|error|debug|failed|failing|修复|错误|调试|报错|异常",
    ) {
        return "debug";
    }
    if regex_is_match(
        &text,
        r"refactor|migrate|implement .{20,}|多步|重构|整体|架构|全量|大规模|submit_task|委派",
    ) {
        return "task";
    }
    if regex_is_match(
        &text,
        r"write|create|add|implement|build|develop|read|show|explain|编写|创建|添加|实现|修改|改一下|查看|解释",
    ) {
        return "coding";
    }
    "chat"
}

fn regex_is_match(text: &str, pattern: &str) -> bool {
    Regex::new(pattern)
        .map(|regex| regex.is_match(text))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeKeyFact {
    id: String,
    kind: String,
    text: String,
    source: String,
    confidence: f32,
    occurrences: u32,
}

fn key_facts_context_from_messages(messages: &[ChatMessage], limit: usize) -> Option<String> {
    let facts = key_facts_from_messages(messages, limit);
    if facts.is_empty() {
        return None;
    }
    let mut lines = vec![
        "[Locked key facts]".to_string(),
        "Preserve these facts across compaction and request trimming. If a new instruction conflicts with one of them, explicitly resolve the conflict with the user or the newer source.".to_string(),
    ];
    for fact in facts {
        lines.push(format!("- {}: {}", fact.kind, fact.text));
    }
    lines.push("[End locked key facts]".to_string());
    Some(lines.join("\n"))
}

fn format_intent_guidance(intent: &str) -> String {
    let normalized = intent
        .strip_prefix("desktop-")
        .unwrap_or(intent)
        .trim()
        .to_ascii_lowercase();
    let hints: &[&str] = match normalized.as_str() {
        "coding" => &[
            "- Tiny local edits: read/write/exec yourself. Repo-wide or multi-file analysis: submit_task / agent.",
            "- Prefer small, verifiable edits over large rewrites.",
            "- Use search_memory then load_memories when project rules or long-term memory are needed. Skills: activate_skill.",
        ],
        "task" => &[
            "- Multi-step scope: decompose, then submit_task for parallel/specialist tracks.",
            "- Prefer paths and goals in briefs; sub-agents read the tree themselves.",
        ],
        "debug" => &[
            "- Investigate systematically: gather logs/errors, reproduce, then fix.",
            "- Use tools to collect evidence before changing code; for broad digs prefer explorer via submit_task.",
        ],
        "review" => &[
            "- Focus on risks, evidence, and actionable feedback.",
            "- Broad review: submit_task to reviewer/explorer with a goal only.",
        ],
        _ => &[
            "- Answer directly. Core file/exec tools are already active; for uncommon tools use the Tool catalog + load_tools.",
            "- Context stays minimal; call load_memories when project rules or memory are needed.",
        ],
    };
    let mut lines = vec![
        "## Current Turn Intent".to_string(),
        format!("Classified intent: {normalized}"),
        String::new(),
    ];
    lines.extend(hints.iter().map(|line| (*line).to_string()));
    lines.push(String::new());
    lines.push(
        "Progressive context: call search_memory / load_memories only when needed — never mid-write. Skills use activate_skill."
            .to_string(),
    );
    lines.join("\n")
}

fn key_facts_token_estimate(facts: &[RuntimeKeyFact]) -> u64 {
    facts
        .iter()
        .map(|fact| estimate_tokens(&fact.text) + 3)
        .sum()
}

fn key_facts_from_messages(messages: &[ChatMessage], limit: usize) -> Vec<RuntimeKeyFact> {
    let mut facts = BTreeMap::<String, RuntimeKeyFact>::new();
    for message in messages.iter().rev().take(24).rev() {
        for fact in
            extract_key_facts_from_text(&message.content, key_fact_source_for_role(message.role))
        {
            let key = format!("{}:{}", fact.kind, normalize_key_fact_text(&fact.text));
            match facts.get_mut(&key) {
                Some(existing) => {
                    existing.occurrences += 1;
                    existing.confidence =
                        (existing.confidence.max(fact.confidence) + 0.03).min(1.0);
                }
                None => {
                    facts.insert(key, fact);
                }
            }
        }
    }
    let mut facts = facts.into_values().collect::<Vec<_>>();
    facts.sort_by(|left, right| key_fact_score(right).total_cmp(&key_fact_score(left)));
    facts.truncate(limit.max(1));
    facts
}

fn extract_key_facts_from_text(text: &str, source: &str) -> Vec<RuntimeKeyFact> {
    let mut facts = Vec::new();
    for path in extract_path_like_facts(text).into_iter().take(20) {
        facts.push(runtime_key_fact("file_path", &path, source, 0.9));
    }
    for command in extract_command_facts(text).into_iter().take(20) {
        facts.push(runtime_key_fact("command", &command, source, 0.72));
    }
    for api in extract_api_facts(text).into_iter().take(20) {
        facts.push(runtime_key_fact("api", &api, source, 0.62));
    }
    for line in important_fact_lines(text) {
        if is_constraint_line(&line) {
            facts.push(runtime_key_fact(
                "constraint",
                &strip_list_marker(&line),
                source,
                0.84,
            ));
        } else if is_decision_line(&line) {
            facts.push(runtime_key_fact(
                "decision",
                &strip_list_marker(&line),
                source,
                0.78,
            ));
        } else if is_failure_line(&line) {
            facts.push(runtime_key_fact(
                "failure",
                &compact_context_line(&strip_list_marker(&line), 280),
                source,
                0.8,
            ));
        }
    }
    facts
}

fn runtime_key_fact(kind: &str, text: &str, source: &str, confidence: f32) -> RuntimeKeyFact {
    let text = cleanup_key_fact_token(text);
    RuntimeKeyFact {
        id: format!("fact_{}", stable_text_hash(&format!("{kind}:{text}"))),
        kind: kind.to_string(),
        text,
        source: source.to_string(),
        confidence,
        occurrences: 1,
    }
}

fn key_fact_source_for_role(role: MessageRole) -> &'static str {
    match role {
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::System => "project_instruction",
        MessageRole::User => "user",
    }
}

fn extract_path_like_facts(text: &str) -> Vec<String> {
    unique_regex_matches(
        text,
        r#"(?:[A-Za-z]:[\\/][^\s"'<>|]+|(?:\.{1,2}[\\/])?[A-Za-z0-9_.-]+(?:[\\/][A-Za-z0-9_.-]+)+\.[A-Za-z0-9]{1,12})"#,
        0,
    )
    .into_iter()
    .map(|value| value.replace('\\', "/"))
    .collect()
}

fn extract_command_facts(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    for value in unique_regex_matches(text, r"`([^`\r\n]{2,180})`", 1) {
        if looks_like_command(&value) {
            push_unique_string(&mut commands, value.trim().to_string());
        }
    }
    for line in text.lines() {
        let value = strip_list_marker(line.trim())
            .trim_start_matches('$')
            .trim_start_matches('>')
            .trim()
            .to_string();
        if looks_like_command(&value) {
            push_unique_string(&mut commands, compact_context_line(&value, 180));
        }
    }
    commands
}

fn extract_api_facts(text: &str) -> Vec<String> {
    unique_regex_matches(text, r"\b[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+\b", 0)
}

fn unique_regex_matches(text: &str, pattern: &str, group: usize) -> Vec<String> {
    let Ok(regex) = Regex::new(pattern) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for captures in regex.captures_iter(text) {
        if let Some(value) = captures.get(group).map(|item| item.as_str().trim()) {
            if !value.is_empty() {
                push_unique_string(&mut output, cleanup_key_fact_token(value));
            }
        }
    }
    output
}

fn important_fact_lines(text: &str) -> Vec<String> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            let len = line.chars().count();
            (4..=360).contains(&len)
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.len() > 120 {
        lines = lines.split_off(lines.len() - 120);
    }
    lines
}

fn looks_like_command(value: &str) -> bool {
    regex_is_match(
        value,
        r"(?i)^(npm|pnpm|yarn|node|npx|tsc|python|pip|cargo|go|mvn|gradle|git|xy|javac|java)\b",
    )
}

fn is_constraint_line(line: &str) -> bool {
    regex_is_match(
        line,
        r"(?i)必须|不要|不能|不需要|不用|禁止|默认|优先|保持|每做完|提交|不要执行|do not|must|never|default|prefer|without running",
    )
}

fn is_decision_line(line: &str) -> bool {
    regex_is_match(
        line,
        r"(?i)决定|采用|选择|确认|已经|改为|使用|设计为|decision|decided|use |chosen",
    )
}

fn is_failure_line(line: &str) -> bool {
    regex_is_match(
        line,
        r"(?i)error|failed|failure|exception|traceback|失败|报错|卡住|限流|429|timeout|timed out",
    )
}

fn strip_list_marker(line: &str) -> String {
    let trimmed = line.trim();
    let without_bullet = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .unwrap_or(trimmed);
    let mut chars = without_bullet.chars().peekable();
    let mut digit_count = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digit_count += 1;
        chars.next();
    }
    if digit_count > 0 && matches!(chars.peek().copied(), Some('.') | Some(')')) {
        chars.next();
        chars.collect::<String>().trim().to_string()
    } else {
        without_bullet.trim().to_string()
    }
}

fn cleanup_key_fact_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, ',' | '.' | ';' | ')' | '，' | '。' | '；'))
        .replace('\\', "/")
}

fn normalize_key_fact_text(value: &str) -> String {
    value
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_fact_score(fact: &RuntimeKeyFact) -> f32 {
    fact.confidence * 10.0 + fact.occurrences as f32
}

fn push_unique_string(items: &mut Vec<String>, value: String) {
    if !value.is_empty() && !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

fn stable_text_hash(value: &str) -> String {
    let mut hash = 2166136261u32;
    for byte in value.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    format!("{hash:x}")
}

#[derive(Debug, Clone)]
struct SessionAnalyticsEstimate {
    label: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    message_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionActivity {
    tool_call_count: usize,
    compact_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct AnalyticsTotals {
    input_tokens: f64,
    output_tokens: f64,
    total_tokens: f64,
    cost_usd: f64,
    estimated: bool,
    priced_token_count: f64,
    unpriced_token_count: f64,
}

impl AnalyticsTotals {
    fn estimated(input_tokens: u64, output_tokens: u64) -> Self {
        let total_tokens = (input_tokens + output_tokens) as f64;
        Self {
            input_tokens: input_tokens as f64,
            output_tokens: output_tokens as f64,
            total_tokens,
            cost_usd: 0.0,
            estimated: true,
            priced_token_count: 0.0,
            unpriced_token_count: total_tokens,
        }
    }

    fn usage(
        input_tokens: f64,
        output_tokens: f64,
        total_tokens: f64,
        cost_usd: Option<f64>,
        estimated: bool,
    ) -> Self {
        let total_tokens = if total_tokens > 0.0 {
            total_tokens
        } else {
            input_tokens + output_tokens
        };
        let cost_value = cost_usd.unwrap_or(0.0);
        let priced = if cost_usd.is_some() {
            total_tokens
        } else {
            0.0
        };
        let unpriced = if cost_usd.is_some() {
            0.0
        } else {
            total_tokens
        };
        Self {
            input_tokens,
            output_tokens,
            total_tokens,
            cost_usd: cost_value,
            estimated,
            priced_token_count: priced,
            unpriced_token_count: unpriced,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
        self.cost_usd += other.cost_usd;
        self.estimated = self.estimated || other.estimated;
        self.priced_token_count += other.priced_token_count;
        self.unpriced_token_count += other.unpriced_token_count;
    }

    fn share(&self, divisor: usize) -> Self {
        if divisor == 0 {
            return Self::default();
        }
        let divisor = divisor as f64;
        Self {
            input_tokens: self.input_tokens / divisor,
            output_tokens: self.output_tokens / divisor,
            total_tokens: self.total_tokens / divisor,
            cost_usd: self.cost_usd / divisor,
            estimated: self.estimated,
            priced_token_count: self.priced_token_count / divisor,
            unpriced_token_count: self.unpriced_token_count / divisor,
        }
    }

    fn normalized_json(&self) -> Value {
        json!({
            "inputTokens": rounded_token_count(self.input_tokens),
            "outputTokens": rounded_token_count(self.output_tokens),
            "totalTokens": rounded_token_count(self.total_tokens),
            "costUsd": round_cost_usd(self.cost_usd),
            "estimated": self.estimated,
            "pricedTokenCount": rounded_token_count(self.priced_token_count),
            "unpricedTokenCount": rounded_token_count(self.unpriced_token_count)
        })
    }
}

#[derive(Debug, Clone)]
struct RunCost {
    provider: String,
    model: String,
    totals: AnalyticsTotals,
}

#[derive(Debug, Clone)]
struct ToolCost {
    tool_name: String,
    is_error: bool,
    usage: Option<AnalyticsTotals>,
}

#[derive(Debug, Clone)]
struct AnalyticsRowData {
    key: String,
    label: String,
    run_count: u64,
    tool_call_count: u64,
    totals: AnalyticsTotals,
}

impl AnalyticsRowData {
    fn new(key: String, label: String) -> Self {
        Self {
            key,
            label,
            run_count: 0,
            tool_call_count: 0,
            totals: AnalyticsTotals::default(),
        }
    }

    fn add_run(&mut self, cost: &RunCost, tool_call_count: usize) {
        self.run_count += 1;
        self.tool_call_count += tool_call_count as u64;
        self.totals.add_assign(&cost.totals);
    }
}

#[derive(Debug, Clone)]
struct ToolAnalyticsRowData {
    row: AnalyticsRowData,
    success_count: u64,
    failure_count: u64,
}

impl ToolAnalyticsRowData {
    fn new(tool_name: String) -> Self {
        Self {
            row: AnalyticsRowData::new(tool_name.clone(), tool_name),
            success_count: 0,
            failure_count: 0,
        }
    }
}

fn build_analytics_snapshot(
    timelines: &[RunTimeline],
    session_estimates: &BTreeMap<String, SessionAnalyticsEstimate>,
    fallback_input_tokens: u64,
    fallback_output_tokens: u64,
    total_messages: usize,
) -> Value {
    let mut by_model = BTreeMap::<String, AnalyticsRowData>::new();
    let mut by_session = BTreeMap::<String, AnalyticsRowData>::new();
    let mut by_tool = BTreeMap::<String, ToolAnalyticsRowData>::new();
    let mut by_task = Vec::<Value>::new();
    let mut totals = AnalyticsTotals::default();

    let mut completed_runs = 0u64;
    let mut failed_runs = 0u64;
    let mut cancelled_runs = 0u64;
    let mut total_iterations = 0.0;
    let mut iteration_runs = 0u64;
    let mut total_tool_calls = 0u64;
    let mut total_tool_results = 0u64;
    let mut failed_tool_results = 0u64;
    let mut compacted_runs = 0u64;
    let mut total_duration_ms = 0.0;
    let mut duration_runs = 0u64;

    for timeline in timelines {
        let cost = calculate_run_cost(timeline);
        totals.add_assign(&cost.totals);
        match timeline.run.status.as_str() {
            "completed" => completed_runs += 1,
            "failed" => failed_runs += 1,
            "cancelled" => cancelled_runs += 1,
            _ => {}
        }

        if let Some(iterations) = get_run_iterations(&timeline.events) {
            total_iterations += iterations;
            iteration_runs += 1;
        }

        if let Some(completed_at) = timeline.run.completed_at {
            total_duration_ms += completed_at.saturating_sub(timeline.run.created_at) as f64;
            duration_runs += 1;
        }

        let tool_calls = get_tool_calls(&timeline.events);
        let tool_results = get_tool_results(&timeline.events);
        let tool_names = if tool_calls.is_empty() {
            tool_results
                .iter()
                .map(|result| result.tool_name.clone())
                .collect::<Vec<_>>()
        } else {
            tool_calls
        };
        let tool_call_count = tool_names.len();
        total_tool_calls += tool_call_count as u64;
        total_tool_results += tool_results.len() as u64;
        failed_tool_results += tool_results.iter().filter(|result| result.is_error).count() as u64;
        if timeline.events.iter().any(is_compaction_event) {
            compacted_runs += 1;
        }

        let model_key = format!("{}/{}", cost.provider, cost.model);
        merge_analytics_row(
            &mut by_model,
            model_key.clone(),
            model_key,
            &cost,
            tool_call_count,
        );
        merge_analytics_row(
            &mut by_session,
            timeline.run.session_id.clone(),
            session_label_for_timeline(timeline, session_estimates),
            &cost,
            tool_call_count,
        );
        by_task.push(task_analytics_row(
            &timeline.run.id,
            &timeline.run.goal,
            &timeline.run.status,
            timeline
                .run
                .completed_at
                .map(|completed| completed.saturating_sub(timeline.run.created_at)),
            tool_call_count,
            &cost.totals,
        ));

        let shared_tool_cost = cost.totals.share(tool_call_count);
        for tool_name in &tool_names {
            let row = by_tool
                .entry(tool_name.clone())
                .or_insert_with(|| ToolAnalyticsRowData::new(tool_name.clone()));
            row.row.run_count += 1;
            row.row.tool_call_count += 1;

            let mut added_usage = false;
            for result in tool_results
                .iter()
                .filter(|result| result.tool_name == *tool_name)
            {
                if let Some(usage) = result.usage {
                    row.row.totals.add_assign(&usage);
                    added_usage = true;
                }
            }
            if !added_usage {
                row.row.totals.add_assign(&shared_tool_cost);
            }
        }

        for result in &tool_results {
            let row = by_tool
                .entry(result.tool_name.clone())
                .or_insert_with(|| ToolAnalyticsRowData::new(result.tool_name.clone()));
            if result.is_error {
                row.failure_count += 1;
            } else {
                row.success_count += 1;
            }
        }
    }

    if timelines.is_empty() {
        totals = AnalyticsTotals::estimated(fallback_input_tokens, fallback_output_tokens);
        let fallback = RunCost {
            provider: "desktop-rust".to_string(),
            model: "xuanyu-core".to_string(),
            totals,
        };
        merge_analytics_row(
            &mut by_model,
            "desktop-rust/xuanyu-core".to_string(),
            "desktop-rust/xuanyu-core".to_string(),
            &fallback,
            0,
        );
        for (session_id, estimate) in session_estimates {
            let cost = RunCost {
                provider: "desktop-rust".to_string(),
                model: "session-estimate".to_string(),
                totals: AnalyticsTotals::estimated(estimate.input_tokens, estimate.output_tokens),
            };
            let row = by_session.entry(session_id.clone()).or_insert_with(|| {
                AnalyticsRowData::new(session_id.clone(), estimate.label.clone())
            });
            row.tool_call_count = 0;
            row.totals.add_assign(&cost.totals);
        }
    }

    let total_runs = timelines.len() as u64;
    json!({
        "generatedAt": Utc::now().timestamp_millis(),
        "totals": totals.normalized_json(),
        "byModel": sorted_analytics_rows(by_model.values().map(analytics_row_value).collect()),
        "bySession": sorted_analytics_rows(by_session.values().map(analytics_row_value).collect()),
        "byTool": sorted_analytics_rows(by_tool.values().map(tool_analytics_row_value).collect()),
        "byTask": sorted_analytics_rows(by_task),
        "quality": {
            "totalRuns": total_runs,
            "completedRuns": completed_runs,
            "failedRuns": failed_runs,
            "cancelledRuns": cancelled_runs,
            "successRate": ratio(completed_runs as f64, total_runs as f64),
            "averageIterations": ratio(total_iterations, iteration_runs as f64),
            "averageToolCalls": ratio(total_tool_calls as f64, total_runs as f64),
            "toolFailureRate": ratio(failed_tool_results as f64, total_tool_results as f64),
            "compactTriggerRate": ratio(compacted_runs as f64, total_runs as f64),
            "averageDurationMs": ratio(total_duration_ms, duration_runs as f64),
        },
        "metadata": {
            "messageCount": total_messages,
            "totalTokens": fallback_input_tokens + fallback_output_tokens,
            "runCount": total_runs,
            "estimated": totals.estimated,
        }
    })
}

fn session_activity_from_timelines(timelines: &[RunTimeline]) -> BTreeMap<String, SessionActivity> {
    let mut activity = BTreeMap::<String, SessionActivity>::new();
    for timeline in timelines {
        let calls = get_tool_calls(&timeline.events);
        let tool_call_count = if calls.is_empty() {
            get_tool_results(&timeline.events).len()
        } else {
            calls.len()
        };
        let compact_count = timeline
            .events
            .iter()
            .filter(|event| is_compaction_event(event))
            .count();
        let entry = activity.entry(timeline.run.session_id.clone()).or_default();
        entry.tool_call_count += tool_call_count;
        entry.compact_count += compact_count;
    }
    activity
}

fn merge_analytics_row(
    rows: &mut BTreeMap<String, AnalyticsRowData>,
    key: String,
    label: String,
    cost: &RunCost,
    tool_call_count: usize,
) {
    rows.entry(key.clone())
        .or_insert_with(|| AnalyticsRowData::new(key, label))
        .add_run(cost, tool_call_count);
}

fn calculate_run_cost(timeline: &RunTimeline) -> RunCost {
    let mut provider = value_string(timeline.run.metadata.get("provider"))
        .or_else(|| value_string(timeline.run.metadata.get("providerType")))
        .unwrap_or_else(|| "unknown".to_string());
    let mut model =
        value_string(timeline.run.metadata.get("model")).unwrap_or_else(|| "unknown".to_string());
    let mut totals = AnalyticsTotals::default();
    let mut saw_model_response = false;

    for event in timeline
        .events
        .iter()
        .filter(|event| event.event_type == "trace.model.response")
    {
        let Some(payload) = event.payload.as_ref().and_then(Value::as_object) else {
            continue;
        };
        saw_model_response = true;
        if provider == "unknown" {
            if let Some(value) = field_string(payload, &["provider", "providerType"]) {
                provider = value;
            }
        }
        if model == "unknown" {
            if let Some(value) = field_string(payload, &["model"]) {
                model = value;
            }
        }

        let input_tokens = token_number(
            payload,
            &[
                "promptTokens",
                "prompt_tokens",
                "inputTokens",
                "input_tokens",
                "prompt",
                "input",
            ],
        );
        let mut output_tokens = token_number(
            payload,
            &[
                "completionTokens",
                "completion_tokens",
                "outputTokens",
                "output_tokens",
                "completion",
                "output",
            ],
        );
        let mut total_tokens = token_number(payload, &["totalTokens", "total_tokens", "total"]);
        let mut estimated = false;
        if input_tokens == 0.0 && output_tokens == 0.0 && total_tokens == 0.0 {
            if let Some(content) = field_string(payload, &["content", "text", "message"]) {
                output_tokens = estimate_tokens(&content) as f64;
                total_tokens = output_tokens;
                estimated = true;
            }
        }
        if total_tokens == 0.0 {
            total_tokens = input_tokens + output_tokens;
        }
        if total_tokens > 0.0 {
            let event_totals = AnalyticsTotals::usage(
                input_tokens,
                output_tokens,
                total_tokens,
                field_number(payload, &["costUsd", "cost_usd", "estimatedCostUsd"]),
                estimated,
            );
            totals.add_assign(&event_totals);
        }
    }

    if !saw_model_response || totals.total_tokens == 0.0 {
        totals.estimated = true;
    }

    RunCost {
        provider,
        model,
        totals,
    }
}

fn get_run_iterations(events: &[RunEvent]) -> Option<f64> {
    for event in events.iter().rev() {
        let Some(payload) = event.payload.as_ref().and_then(Value::as_object) else {
            continue;
        };
        if event.event_type == "run.completed" {
            if let Some(iterations) = field_number(payload, &["iterations"]) {
                if iterations > 0.0 {
                    return Some(iterations);
                }
            }
            if let Some(nested) = payload.get("payload").and_then(Value::as_object) {
                if let Some(iterations) = field_number(nested, &["iterations"]) {
                    if iterations > 0.0 {
                        return Some(iterations);
                    }
                }
            }
        }
        if event.event_type == "agent.event" {
            let Some(agent_event) = payload.get("event").and_then(Value::as_object) else {
                continue;
            };
            if field_string(agent_event, &["type"]).as_deref() == Some("done") {
                if let Some(iterations) = field_number(agent_event, &["iterations"]) {
                    if iterations > 0.0 {
                        return Some(iterations);
                    }
                }
            }
        }
    }
    None
}

fn get_tool_calls(events: &[RunEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| is_tool_call_event(event))
        .map(|event| {
            event
                .payload
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|payload| {
                    field_string(payload, &["toolName", "tool", "name", "requestedTool"])
                })
                .unwrap_or_else(|| "unknown".to_string())
        })
        .collect()
}

fn get_tool_results(events: &[RunEvent]) -> Vec<ToolCost> {
    events
        .iter()
        .filter(|event| is_tool_result_event(event))
        .map(|event| {
            let payload = event.payload.as_ref().and_then(Value::as_object);
            let tool_name = payload
                .and_then(|payload| {
                    field_string(payload, &["toolName", "tool", "name", "requestedTool"])
                })
                .unwrap_or_else(|| "unknown".to_string());
            let is_error = payload.map(tool_result_is_error).unwrap_or(false);
            let usage = payload.and_then(tool_usage_totals);
            ToolCost {
                tool_name,
                is_error,
                usage,
            }
        })
        .collect()
}

fn is_tool_call_event(event: &RunEvent) -> bool {
    matches!(
        event.event_type.as_str(),
        "trace.tool.call" | "agent.tool.requested"
    )
}

fn is_tool_result_event(event: &RunEvent) -> bool {
    matches!(
        event.event_type.as_str(),
        "trace.tool.result" | "agent.tool.result"
    )
}

fn is_compaction_event(event: &RunEvent) -> bool {
    event.event_type == "context.compacted" || event.event_type.contains("context.compact")
}

fn tool_result_is_error(payload: &serde_json::Map<String, Value>) -> bool {
    payload
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || field_string(payload, &["status"])
            .map(|status| matches!(status.as_str(), "failed" | "error"))
            .unwrap_or(false)
        || payload.get("error").is_some()
}

fn tool_usage_totals(payload: &serde_json::Map<String, Value>) -> Option<AnalyticsTotals> {
    let input_tokens = token_number(
        payload,
        &[
            "promptTokens",
            "prompt_tokens",
            "inputTokens",
            "input_tokens",
            "prompt",
            "input",
        ],
    );
    let output_tokens = token_number(
        payload,
        &[
            "completionTokens",
            "completion_tokens",
            "outputTokens",
            "output_tokens",
            "completion",
            "output",
        ],
    );
    let mut total_tokens = token_number(payload, &["totalTokens", "total_tokens", "total"]);
    if total_tokens == 0.0 {
        total_tokens = input_tokens + output_tokens;
    }
    if input_tokens == 0.0 && output_tokens == 0.0 && total_tokens == 0.0 {
        return None;
    }
    Some(AnalyticsTotals::usage(
        input_tokens,
        output_tokens,
        total_tokens,
        field_number(payload, &["costUsd", "cost_usd", "estimatedCostUsd"]),
        false,
    ))
}

fn session_label_for_timeline(
    timeline: &RunTimeline,
    session_estimates: &BTreeMap<String, SessionAnalyticsEstimate>,
) -> String {
    session_estimates
        .get(&timeline.run.session_id)
        .map(|estimate| estimate.label.clone())
        .or_else(|| value_string(timeline.run.metadata.get("sessionName")))
        .unwrap_or_else(|| timeline.run.session_id.clone())
}

fn task_analytics_row(
    run_id: &str,
    label: &str,
    status: &str,
    duration_ms: Option<i64>,
    tool_call_count: usize,
    totals: &AnalyticsTotals,
) -> Value {
    let cost = totals.normalized_json();
    json!({
        "key": run_id,
        "runId": run_id,
        "label": label,
        "status": status,
        "durationMs": duration_ms,
        "runCount": 1,
        "toolCallCount": tool_call_count,
        "inputTokens": cost["inputTokens"],
        "outputTokens": cost["outputTokens"],
        "totalTokens": cost["totalTokens"],
        "costUsd": cost["costUsd"],
        "estimated": cost["estimated"],
        "pricedTokenCount": cost["pricedTokenCount"],
        "unpricedTokenCount": cost["unpricedTokenCount"]
    })
}

fn analytics_row_value(row: &AnalyticsRowData) -> Value {
    let cost = row.totals.normalized_json();
    json!({
        "key": row.key,
        "label": row.label,
        "runCount": row.run_count,
        "toolCallCount": row.tool_call_count,
        "inputTokens": cost["inputTokens"],
        "outputTokens": cost["outputTokens"],
        "totalTokens": cost["totalTokens"],
        "costUsd": cost["costUsd"],
        "estimated": cost["estimated"],
        "pricedTokenCount": cost["pricedTokenCount"],
        "unpricedTokenCount": cost["unpricedTokenCount"]
    })
}

fn tool_analytics_row_value(row: &ToolAnalyticsRowData) -> Value {
    let mut value = analytics_row_value(&row.row);
    if let Some(object) = value.as_object_mut() {
        object.insert("successCount".to_string(), json!(row.success_count));
        object.insert("failureCount".to_string(), json!(row.failure_count));
        object.insert(
            "failureRate".to_string(),
            json!(ratio(
                row.failure_count as f64,
                (row.success_count + row.failure_count) as f64
            )),
        );
    }
    value
}

fn sorted_analytics_rows(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by(|a, b| {
        value_number(b.get("costUsd"))
            .partial_cmp(&value_number(a.get("costUsd")))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                value_number(b.get("totalTokens"))
                    .partial_cmp(&value_number(a.get("totalTokens")))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                value_number(b.get("toolCallCount"))
                    .partial_cmp(&value_number(a.get("toolCallCount")))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    rows
}

fn field_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| value_string(object.get(*key)))
}

fn field_number(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(number_from_value))
}

fn token_number(object: &serde_json::Map<String, Value>, keys: &[&str]) -> f64 {
    field_number(object, keys)
        .or_else(|| {
            object
                .get("usage")
                .and_then(Value::as_object)
                .and_then(|usage| field_number(usage, keys))
        })
        .or_else(|| {
            object
                .get("tokens")
                .and_then(Value::as_object)
                .and_then(|tokens| field_number(tokens, keys))
        })
        .unwrap_or(0.0)
}

fn value_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_number(value: Option<&Value>) -> f64 {
    value.and_then(number_from_value).unwrap_or(0.0)
}

fn number_from_value(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        if number.is_finite() {
            return Some(number);
        }
    }
    value
        .as_str()
        .and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|number| number.is_finite())
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator <= 0.0 {
        return 0.0;
    }
    ((numerator / denominator) * 10_000.0).round() / 10_000.0
}

fn rounded_token_count(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        value.round() as u64
    }
}

fn round_cost_usd(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        (value * 1_000_000.0).round() / 1_000_000.0
    }
}

#[derive(Debug, Clone)]
struct MainBrainRuntimeContextParts {
    system: String,
    memory: Option<String>,
    proactive: Option<String>,
    key_facts: Option<String>,
    context_sections: Option<String>,
    plugin_notes: Vec<String>,
    policy_tools: Option<String>,
    policy_guidance: Vec<String>,
    tool_bridge: String,
}

fn token_counter_config_from_app_config(config: &Value) -> TokenCounterConfig {
    let budget = config
        .get("contextBudget")
        .or_else(|| config.get("context_budget"));
    let backend = token_counter_backend_from_value(
        config
            .get("tokenCounter")
            .or_else(|| config.get("token_counter")),
    );
    TokenCounterConfig {
        max_context_tokens: config
            .get("maxContextTokens")
            .and_then(Value::as_u64)
            .or_else(|| budget.and_then(|item| item.get("max")).and_then(Value::as_u64))
            .unwrap_or(128_000)
            .clamp(8_000, 2_000_000),
        max_output_tokens: config
            .get("maxOutputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(4_096)
            .clamp(256, 32_768),
        backend,
    }
}

fn estimate_active_tool_schema_tokens(
    definitions: &[ToolDefinition],
    active_names: &BTreeSet<String>,
) -> u64 {
    let tools = definitions
        .iter()
        .filter(|tool| {
            active_names.contains(&tool.name)
                || tool
                    .aliases
                    .iter()
                    .any(|alias| active_names.contains(alias))
        })
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    estimate_tool_schema_tokens(&tools)
}

fn build_budgeted_mainbrain_runtime_context(
    parts: MainBrainRuntimeContextParts,
    config: &Value,
    iteration_input: &str,
    tool_schema_tokens: u64,
) -> (String, ContextBudgetAllocation) {
    let mut sections = Vec::<BudgetSectionInput>::new();
    sections.push(BudgetSectionInput {
        id: "system",
        section_type: "system",
        title: "Runtime system context",
        priority: 100,
        text: parts.system,
        can_drop: false,
        can_truncate: true,
        reason: "Core runtime persona and attention state.",
    });
    if let Some(memory) = parts
        .memory
        .as_deref()
        .map(str::trim)
        .filter(|memory| !memory.is_empty())
    {
        sections.push(BudgetSectionInput {
            id: "memory",
            section_type: "memory",
            title: "Retrieved memory context",
            priority: 92,
            text: memory.to_string(),
            can_drop: true,
            can_truncate: true,
            reason: "Semantic memory hits for the current turn.",
        });
    }
    if let Some(proactive) = parts.proactive {
        sections.push(BudgetSectionInput {
            id: "proactive",
            section_type: "delegation",
            title: "Proactive initiative",
            priority: 95,
            text: proactive,
            can_drop: false,
            can_truncate: true,
            reason: "Proactive Person-mode initiative prompt.",
        });
    }
    if let Some(key_facts) = parts
        .key_facts
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(BudgetSectionInput {
            id: "key_facts",
            section_type: "key_facts",
            title: "Key conversation facts",
            priority: 88,
            text: key_facts.to_string(),
            can_drop: true,
            can_truncate: true,
            reason: "Recent durable facts extracted from the conversation.",
        });
    }
    if let Some(context_sections) = parts
        .context_sections
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(BudgetSectionInput {
            id: "context_sections",
            section_type: "skills",
            title: "Active context sections",
            priority: 84,
            text: context_sections.to_string(),
            can_drop: true,
            can_truncate: true,
            reason: "Lazy-loaded project instructions, skills, and memory profile.",
        });
    }
    if !parts.plugin_notes.is_empty() {
        let text = parts
            .plugin_notes
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(BudgetSectionInput {
            id: "plugin_notes",
            section_type: "project_instructions",
            title: "Plugin hook notes",
            priority: 78,
            text: format!("Plugin hook notes:\n{text}"),
            can_drop: true,
            can_truncate: true,
            reason: "Notes returned by extension beforeTurn hooks.",
        });
    }
    if let Some(policy_tools) = parts
        .policy_tools
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(BudgetSectionInput {
            id: "policy_tools",
            section_type: "tool_results",
            title: "Policy tool results",
            priority: 72,
            text: policy_tools.to_string(),
            can_drop: true,
            can_truncate: true,
            reason: "Tool outputs requested by the Python policy planner.",
        });
    }
    if !parts.policy_guidance.is_empty() {
        let text = parts
            .policy_guidance
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(BudgetSectionInput {
            id: "policy_guidance",
            section_type: "project_instructions",
            title: "Python policy guidance",
            priority: 68,
            text: format!("Python policy guidance:\n{text}"),
            can_drop: true,
            can_truncate: true,
            reason: "Additional guidance from the Python policy worker.",
        });
    }
    sections.push(BudgetSectionInput {
        id: "tool_bridge",
        section_type: "tool_results",
        title: "Tool bridge",
        priority: 66,
        text: parts.tool_bridge,
        can_drop: false,
        can_truncate: false,
        reason: "Deferred tool catalog (name + summary); core/meta schemas are already active.",
    });
    let allocator = ContextBudgetAllocator::new(token_counter_config_from_app_config(config));
    let allocation = allocator.build(
        sections,
        tool_schema_tokens,
        &[("user".to_string(), iteration_input.to_string())],
    );
    (assemble_budget_sections(&allocation.sections), allocation)
}

fn assemble_budget_sections(sections: &[ContextBudgetSection]) -> String {
    const ORDER: &[&str] = &[
        "system",
        "memory",
        "proactive",
        "key_facts",
        "context_sections",
        "plugin_notes",
        "policy_tools",
        "policy_guidance",
        "tool_bridge",
    ];
    ORDER
        .iter()
        .filter_map(|id| {
            sections
                .iter()
                .find(|section| section.id == *id && section.loaded && !section.rendered_text.is_empty())
        })
        .map(|section| section.rendered_text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn runtime_agent_prompt() -> String {
    concat!(
        "## Tools\n",
        "Already active each turn: file_read, file_write, file_edit, list_files, glob, search, exec, submit_task (alias: agent) — call these directly; do not load_tools first.\n",
        "- ask_user (meta, when enabled): use it for clarifying questions that must block until answered.\n",
        "- Deferred tools: the Tool catalog below lists name + short summary. Call load_tools([\"exact_name\"]) then invoke on the next iteration.\n",
        "- Objective: only the user sets it (/goal or Goal bar). When active, update_goal is preloaded for verified completion only.\n",
        "- Memory: search_memory → load_memories (sections: memory, memory_profile, project_instructions). Write profiles with load_tools([\"write_memory\"]).\n",
        "- Skills: catalog (name + summary) is already in this system prompt. Call activate_skill with an exact id before following one.\n",
        "- Plan mode: load_tools([\"enter_plan_mode\"]) then enter; exit_plan_mode after the user approves a plan.\n",
        "- Nested specialists: call submit_task / agent({ agents: [...] }) — default WAITS until all finish.\n",
        "\n",
        "## Context (on demand)\n",
        "Pure chat stays minimal until you explicitly load context.\n",
        "Work intents (coding/debug/task/review) auto-activate project_instructions and memory at turn start.\n",
        "For chat or extra context: search_memory → load_memories, or load_memories([\"project_instructions\"] / [\"memory_profile\"]).\n",
        "Skills catalog is always in the system prompt; call activate_skill(id) for the full package — never load_memories for skills.\n",
    )
    .to_string()
}

#[derive(Debug, Clone)]
struct GeneratedSessionTitle {
    title: Option<String>,
    source: String,
    error: Option<String>,
    provider: String,
    model: String,
}

fn should_auto_title(name: &str, generated_by: Option<&str>) -> bool {
    let title = name.trim();
    if title.is_empty() {
        return true;
    }
    // Match reference placeholder titles (新建默认「新对话」).
    if matches!(
        title,
        "新对话" | "新会话" | "New chat" | "New session" | "Robot Session" | "Person Session"
    ) {
        return true;
    }
    if is_project_storage_key(title) || looks_like_json_blob(title) {
        return true;
    }
    if generated_by == Some("fallback") {
        return true;
    }
    regex::Regex::new(r"(?i)^Session\s+\d+$")
        .map(|pattern| pattern.is_match(title))
        .unwrap_or(false)
}

/// `basename_8hex` storage keys misused as display titles should be retitled.
fn is_project_storage_key(value: &str) -> bool {
    let title = value.trim();
    if title.is_empty() {
        return false;
    }
    regex::Regex::new(r"^[a-zA-Z0-9_-]+_[0-9a-f]{8}$")
        .map(|pattern| pattern.is_match(title))
        .unwrap_or(false)
}

fn looks_like_json_blob(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return true;
    }
    regex::Regex::new(r#"^["']?\s*\{\s*["']?(?:title|response|name)"#)
        .map(|pattern| pattern.is_match(trimmed))
        .unwrap_or(false)
}

fn generate_fallback_session_title(content: &str) -> Option<String> {
    let normalized = content
        .replace("```", " ")
        .replace('`', "")
        .replace(|c: char| c.is_whitespace(), " ");
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Some("新对话".to_string());
    }
    let mut cleaned = normalized.clone();
    for prefix in ["请你", "帮我", "麻烦你", "能不能", "可以帮我", "你能", "我要", "我想", "请", "帮忙"] {
        if let Some(stripped) = cleaned.strip_prefix(prefix) {
            cleaned = stripped.trim().to_string();
        }
    }
    while cleaned.ends_with(['。', '！', '？', '!', '?', '，', ',', '；', ';', '：', ':']) {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        cleaned = normalized;
    }
    let ascii_count = cleaned.chars().filter(|ch| ch.is_ascii_alphanumeric()).count();
    let limit = if ascii_count * 2 >= cleaned.len() { 56 } else { 18 };
    Some(if cleaned.chars().count() > limit {
        format!("{}...", cleaned.chars().take(limit).collect::<String>().trim())
    } else {
        cleaned
    })
}

fn sanitize_model_session_title(value: &str) -> Option<String> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(title) = extract_json_title(raw) {
        return Some(title);
    }
    let cleaned = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim_matches(['"', '\'', '“', '”', '‘', '’', '《', '【', '[', '》', '】', ']'])
        .trim_start_matches("标题:")
        .trim_start_matches("标题：")
        .trim_start_matches("会话标题:")
        .trim_start_matches("会话标题：")
        .trim_end_matches(['。', '！', '？', '!', '?', '，', ',', '；', ';', '：', ':'])
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    let compacted = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let ascii_count = compacted
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count();
    let limit = if ascii_count * 2 >= compacted.len() { 64 } else { 16 };
    Some(if compacted.chars().count() > limit {
        compacted.chars().take(limit).collect::<String>().trim().to_string()
    } else {
        compacted
    })
}

fn extract_json_title(value: &str) -> Option<String> {
    let candidates = [
        value.trim().to_string(),
        value
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string(),
    ];
    for candidate in candidates {
        if let Ok(parsed) = serde_json::from_str::<Value>(&candidate) {
            if let Some(title) = parsed.get("title").and_then(Value::as_str) {
                if let Some(clean) = sanitize_plain_title(title) {
                    return Some(clean);
                }
            }
        }
    }
    None
}

fn extract_title_from_reasoning(reasoning: &str) -> Option<String> {
    if let Some(title) = extract_json_title(reasoning) {
        return Some(title);
    }
    for line in reasoning.lines() {
        let line = line.trim();
        for prefix in [
            "标题:",
            "标题：",
            "会话标题:",
            "会话标题：",
            "最终标题:",
            "最终标题：",
            "建议标题:",
            "建议标题：",
            "可命名为",
            "命名为",
            "叫做",
        ] {
            if let Some(rest) = line.strip_prefix(prefix) {
                if let Some(title) = sanitize_plain_title(rest.trim()) {
                    return Some(title);
                }
            }
            if let Some(index) = line.find(prefix) {
                let rest = line[index + prefix.len()..].trim();
                if let Some(title) = sanitize_plain_title(rest) {
                    return Some(title);
                }
            }
        }
    }
    None
}

fn sanitize_plain_title(value: &str) -> Option<String> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return None;
    }
    sanitize_model_session_title(cleaned)
}

fn policy_message_role(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn policy_memory_hits_from_context(memory_context: Option<&str>) -> Vec<String> {
    memory_context
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.eq_ignore_ascii_case("retrieved memories:"))
        .map(|line| {
            compact_context_line(
                line.trim_start_matches("- ")
                    .trim_start_matches(char::is_whitespace),
                500,
            )
        })
        .filter(|line| !line.is_empty())
        .take(8)
        .collect()
}

fn policy_constraints_from_config(config: &Value) -> Vec<String> {
    let mut constraints = Vec::new();
    if let Some(policy_mode) = config
        .get("mainBrain")
        .and_then(Value::as_object)
        .and_then(|main_brain| main_brain.get("policyMode"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        constraints.push(format!("mainBrain.policyMode={policy_mode}"));
    }
    if let Some(max_context_tokens) = config.get("maxContextTokens").and_then(Value::as_u64) {
        constraints.push(format!("maxContextTokens={max_context_tokens}"));
    }
    if let Some(max_iterations) = policy_max_turns_from_config(config) {
        constraints.push(format!("maxIterations={max_iterations}"));
    }
    if let Some(load_mode) = config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| context.get("loadMode"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        constraints.push(format!("contextManagement.loadMode={load_mode}"));
    }
    if !memory_auto_inject_enabled(config) {
        constraints.push("memoryAutoInject=false".to_string());
    }
    constraints
}

fn policy_max_turns_from_config(config: &Value) -> Option<u32> {
    let main_brain = config.get("mainBrain").and_then(Value::as_object);
    main_brain
        .and_then(|main_brain| {
            main_brain
                .get("maxIterations")
                .or_else(|| main_brain.get("maxTurns"))
        })
        .or_else(|| config.get("maxIterations"))
        .or_else(|| config.get("maxTurns"))
        .and_then(Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32)
}

/// Absolute unix-ms deadline for a turn from `mainBrain.turnTimeoutMs` / `turnTimeoutMs`.
fn turn_deadline_ms_from_config(config: &Value, turn_started_ms: u64) -> Option<u64> {
    let timeout_ms = config
        .get("mainBrain")
        .and_then(Value::as_object)
        .and_then(|main| {
            main.get("turnTimeoutMs")
                .or_else(|| main.get("deadlineMs"))
        })
        .or_else(|| config.get("turnTimeoutMs"))
        .or_else(|| config.get("deadlineMs"))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)?;
    Some(turn_started_ms.saturating_add(timeout_ms))
}

fn validate_workspace_directory(path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Workspace path is required");
    }
    let resolved = PathBuf::from(trimmed);
    let metadata = std::fs::metadata(&resolved).map_err(|_| {
        anyhow::anyhow!("Directory does not exist: {}", path_to_string(&resolved))
    })?;
    if !metadata.is_dir() {
        anyhow::bail!("Directory does not exist: {}", path_to_string(&resolved));
    }
    Ok(resolved.canonicalize().unwrap_or(resolved))
}

fn unscoped_workspace_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn project_name_from_dir(dir: &Path) -> String {
    let base = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = if base.is_empty() {
        "project".to_string()
    } else {
        base
    };
    let resolved = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let normalized = resolved.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        format!("{:x}", hasher.finalize())
    };
    format!("{}_{}", base, &digest[..16.min(digest.len())])
}

fn ensure_workspace_meta(data_dir: &Path, workspace_dir: &Path) -> anyhow::Result<()> {
    let workspace_id = project_name_from_dir(workspace_dir);
    let meta_dir = data_dir.join("workspaces").join(&workspace_id);
    std::fs::create_dir_all(&meta_dir)?;
    let meta_path = meta_dir.join("meta.json");
    let payload = json!({
        "id": workspace_id,
        "path": path_to_string(workspace_dir),
        "updatedAt": chrono::Utc::now().timestamp_millis(),
    });
    let tmp = meta_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&payload)?)?;
    std::fs::rename(tmp, meta_path)?;
    Ok(())
}

fn workspace_path_key(path: &str) -> String {
    let display = strip_windows_extended_path(path);
    let mut s = display.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        // Keep drive root like `D:/`
        let bytes = s.as_bytes();
        if s.len() == 3 && bytes[1] == b':' {
            break;
        }
        s.pop();
    }
    s.to_ascii_lowercase()
}

fn normalize_workspace_preference_path(path: &str) -> String {
    let display = strip_windows_extended_path(path.trim());
    let mut s = display.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        let bytes = s.as_bytes();
        if s.len() == 3 && bytes[1] == b':' {
            break;
        }
        s.pop();
    }
    s
}

fn build_workspace_preference_update(config: &Value, dir: &str) -> Value {
    let resolved = normalize_workspace_preference_path(dir);
    let resolved_key = workspace_path_key(&resolved);
    let prev = config
        .get("workspace")
        .and_then(|value| value.get("recentDirs"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut recent = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if !resolved.is_empty() {
        seen.insert(resolved_key);
        recent.push(resolved.clone());
    }
    for item in prev {
        let norm = normalize_workspace_preference_path(&item);
        let key = workspace_path_key(&norm);
        if norm.is_empty() || !seen.insert(key) {
            continue;
        }
        recent.push(norm);
    }
    recent.truncate(12);
    json!({
        "workspace": {
            "lastDir": resolved,
            "recentDirs": recent
        }
    })
}

fn git_status_snapshot_at(cwd: &Path) -> Value {
    let root = git_output_at(cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd.to_path_buf());
    let branch = git_output_at(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "not-a-git-worktree".to_string());
    let head = git_output_at(&root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let upstream =
        git_output_at(&root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok();
    let (ahead, behind) = upstream
        .as_ref()
        .and_then(|_| {
            git_output_at(&root, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]).ok()
        })
        .and_then(|text| {
            let mut parts = text.split_whitespace();
            let ahead = parts.next()?.parse::<u64>().ok()?;
            let behind = parts.next()?.parse::<u64>().ok()?;
            Some((ahead, behind))
        })
        .unwrap_or((0, 0));
    let files = git_output_at(&root, &["status", "--porcelain=v1"])
        .map(|text| {
            text.lines()
                .filter_map(parse_porcelain_status_line)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "root": path_to_string(&root),
        "branch": branch,
        "head": head,
        "upstream": upstream,
        "ahead": ahead,
        "behind": behind,
        "clean": files.is_empty(),
        "files": files
    })
}

fn list_git_branches_at(cwd: &Path) -> Result<Vec<Value>, String> {
    let raw = git_output_at(
        cwd,
        &[
            "branch",
            "--format=%(HEAD)%09%(refname:short)%09%(upstream:short)",
        ],
    )
    .map_err(|err| err.to_string())?;
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

fn git_checkout_at(cwd: &Path, branch: &str) -> Result<(String, Value), String> {
    let branch = validate_git_branch_name(branch)?;
    git_output_at(cwd, &["checkout", &branch]).map_err(|err| err.to_string())?;
    Ok((branch, git_status_snapshot_at(cwd)))
}

fn validate_git_branch_name(name: &str) -> Result<String, String> {
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

fn git_output_at(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_porcelain_status_line(line: &str) -> Option<Value> {
    if line.len() < 4 {
        return None;
    }
    let bytes = line.as_bytes();
    let index = bytes[0] as char;
    let working_tree = bytes[1] as char;
    let mut path = line[3..].to_string();
    let mut from = None;
    if index == 'R' || index == 'C' {
        if let Some((old, new)) = path.split_once(" -> ") {
            from = Some(old.to_string());
            path = new.to_string();
        }
    }
    let status = git_status_kind(index, working_tree);
    Some(json!({
        "path": path,
        "from": from,
        "index": index.to_string(),
        "workingTree": working_tree.to_string(),
        "status": status,
        "staged": index != ' ' && index != '?',
        "unstaged": working_tree != ' ' && working_tree != '?'
    }))
}

fn git_status_kind(index: char, working_tree: char) -> &'static str {
    if index == '?' || working_tree == '?' {
        "untracked"
    } else if index == 'U' || working_tree == 'U' {
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

#[async_trait::async_trait]
impl SessionBridge for AppCore {
    async fn build_context_status(&self, session_id: &SessionId) -> Value {
        self.build_context_status_inner(session_id).await
    }

    async fn compact_context(
        &self,
        session_id: &SessionId,
        strategy: Option<String>,
    ) -> Value {
        self.compact_context_inner(session_id, strategy).await
    }

    async fn rollback_context(&self, session_id: &SessionId) -> Value {
        self.rollback_context_inner(session_id).await
    }

    async fn permission_respond_with_scope(
        &self,
        request_id: String,
        allowed: bool,
        scope: Option<String>,
    ) -> Value {
        self.permission_respond_with_scope_inner(request_id, allowed, scope)
            .await
    }

    async fn list_pending_permissions(&self) -> Value {
        self.permission_list(None).await
    }
}
