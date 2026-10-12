/**
 * XuanYu Agent 核心共享类型。
 *
 * 这里放跨模块复用的基础结构：消息、工具调用、流式输出、Provider、
 * Agent 事件、权限、任务、记忆和对话快照。
 */

// ============================================================================
// Message Types
// ============================================================================

/** 对话消息角色。 */
export type MessageRole = 'system' | 'user' | 'assistant' | 'tool';

/** 消息附件：图片或文件，含 data URL 与可选抽取正文。 */
export interface MessageAttachment {
  kind: 'image' | 'file';
  name: string;
  mimeType: string;
  sizeBytes: number;
  dataUrl: string;
  extractedText?: string;
}

/** 对话消息：角色、正文、工具调用与可选 reasoning。 */
export interface Message {
  /** 消息角色。 */
  role: MessageRole;
  /** 消息正文。 */
  content: string;
  /** 可选附件。 */
  attachments?: MessageAttachment[];
  /** 可选名称，通常用于 tool 或多参与方消息。 */
  name?: string;
  /** tool 消息对应的工具调用 ID。 */
  tool_call_id?: string;
  /** assistant 消息中携带的工具调用列表。 */
  tool_calls?: ToolCall[];
  /** reasoning 模型返回的思考内容。 */
  reasoning_content?: string;
  /** 消息创建时间戳，单位毫秒。 */
  timestamp?: number;
}

/** 用户消息。 */
export interface UserMessage extends Message {
  role: 'user';
}

/** 助手消息，可以携带工具调用和 reasoning 内容。 */
export interface AssistantMessage extends Message {
  role: 'assistant';
  tool_calls?: ToolCall[];
  reasoning_content?: string;
}

/** 工具返回消息，必须关联 tool_call_id。 */
export interface ToolMessage extends Message {
  role: 'tool';
  tool_call_id: string;
  name?: string;
}

// ============================================================================
// Tool Types
// ============================================================================

/** 一次工具调用：id、函数名与参数 JSON。 */
export interface ToolCall {
  /** 模型生成的工具调用 ID。 */
  id: string;
  /** 当前只支持 function 类型工具调用。 */
  type: 'function';
  /** 函数名和 JSON 字符串参数。 */
  function: {
    name: string;
    arguments: string;
  };
}

/** 工具结果：callId、输出与是否错误。 */
export interface ToolResult {
  /** 对应 ToolCall.id。 */
  tool_call_id: string;
  /** 工具返回文本。 */
  content: string;
  /** 是否表示错误结果。 */
  isError?: boolean;
  /** 工具名，便于日志和 UI 展示。 */
  toolName?: string;
  /** 结构化数据，供 UI 或后续运行时消费。 */
  data?: unknown;
  /** 工具内部模型调用产生的 token 用量。 */
  usage?: TokenUsage;
  /** 文件变更 diff。 */
  diff?: string;
  /** 工具影响到的文件列表。 */
  affectedFiles?: string[];
  /** 便于 UI 展示的新增/删除行数统计。 */
  lineChanges?: { added: number; removed: number };
  /** 多模态附件（如 file_read 读到的本地图片），随 tool 消息进入下一轮模型请求。 */
  attachments?: MessageAttachment[];
}

// ============================================================================
// Streaming Types
// ============================================================================

/** 模型流式输出 chunk 类型。 */
export type StreamChunkType =
  | 'text'
  | 'reasoning'
  | 'tool_call_start'
  | 'tool_call_delta'
  | 'tool_call_end'
  | 'usage'
  | 'error'
  | 'done';

/** 模型流式块：text/reasoning/tool/error/done。 */
export interface StreamChunk {
  /** 当前 chunk 类型。 */
  type: StreamChunkType;
  /** 文本或 reasoning 内容。 */
  content?: string;
  /** 工具调用的增量信息。 */
  tool_call?: Partial<ToolCall>;
  /** 错误消息。 */
  error?: string;
  /** usage 增量或最终统计。 */
  usage?: TokenUsage;
}

// ============================================================================
// Provider Types
// ============================================================================

import type { ModelRetrySettings, ProviderType } from '../config/types';

/** 供应商请求配置，最终会传给具体 provider 实现。 */
export interface ProviderConfig {
  type: ProviderType;
  /** API Key；本地模型可为空字符串。 */
  apiKey: string;
  baseUrl?: string;
  model: string;
  maxTokens?: number;
  temperature?: number;
  retry?: ModelRetrySettings;
  /** 额外请求体字段，供 JSON mode、thinking 等供应商特性使用。 */
  extraBody?: Record<string, unknown>;
}

/** 非流式补全响应的统一结构。 */
export interface CompletionResponse {
  content: string;
  toolCalls?: ToolCall[];
  finishReason: 'stop' | 'tool_calls' | 'length' | 'error';
  usage?: {
    promptTokens: number;
    completionTokens: number;
    totalTokens: number;
    cacheTokens?: number;
  };
}

// ============================================================================
// Agent Event Types (Async Generator Pattern)
// ============================================================================

/** Agent 循环对外发出的事件联合类型。 */
export type AgentEvent =
  | AgentTextEvent
  | AgentReasoningEvent
  | AgentThinkingEvent
  | AgentModelStartEvent
  | AgentModelDoneEvent
  | AgentToolCallEvent
  | AgentToolResultEvent
  | AgentToolProgressEvent
  | AgentMemoryHitEvent
  | AgentCompactEvent
  | AgentUsageEvent
  | AgentErrorEvent
  | AgentDoneEvent
  | AgentMessageEvent;

/** 可见文本输出事件。 */
export interface AgentTextEvent {
  type: 'text';
  content: string;
}

/** reasoning 文本输出事件。 */
export interface AgentReasoningEvent {
  type: 'reasoning';
  content: string;
}

/** thinking 状态事件，表示模型正在思考但暂未输出文本。 */
export interface AgentThinkingEvent {
  type: 'thinking';
}

/** 模型请求开始事件。 */
export interface AgentModelStartEvent {
  type: 'model_start';
  model?: string;
  messageCount: number;
  toolCount: number;
  context?: ContextBudgetAllocation;
}

/** 模型请求结束事件。 */
export interface AgentModelDoneEvent {
  type: 'model_done';
  model?: string;
  finishReason: 'stop' | 'tool_calls' | 'length' | 'error';
  toolCallCount: number;
}

/** 模型发起工具调用事件。 */
export interface AgentToolCallEvent {
  type: 'tool_call';
  call: ToolCall;
}

/** 工具返回结果事件。 */
export interface AgentToolResultEvent {
  type: 'tool_result';
  result: ToolResult;
}

/** 工具执行中的进度事件。 */
export interface AgentToolProgressEvent {
  type: 'tool_progress';
  toolUseId: string;
  toolName: string;
  data: unknown;
}

/** 记忆检索命中事件。 */
export interface AgentMemoryHitEvent {
  type: 'memory_hit';
  payload: {
    query?: string;
    source: 'query' | 'system';
    count: number;
    limit: number;
    memoryIds: string[];
    categories: string[];
    injected: boolean;
  };
}

/** 上下文压缩事件。 */
export interface AgentCompactEvent {
  type: 'compact';
  result: CompactResult;
}

/** Token usage 事件。 */
export interface AgentUsageEvent {
  type: 'usage';
  usage: TokenUsage;
}

/** Agent 错误事件。 */
export interface AgentErrorEvent {
  type: 'error';
  error: Error;
}

/** Agent 结束事件。 */
export interface AgentDoneEvent {
  type: 'done';
  finalResponse: string;
  iterations: number;
  fallbackReasoningOnly?: boolean;
  fallbackSource?: 'last_assistant_visible_text';
  /** 用户手动停止，非失败。 */
  cancelled?: boolean;
}

/** 新消息写入会话事件。 */
export interface AgentMessageEvent {
  type: 'message';
  message: Message;
}

// ============================================================================
// Agent Configuration
// ============================================================================

/** 系统提示词配置。 */
export interface SystemPromptConfig {
  basePrompt: string;
  projectContext?: string;
  customInstructions?: string;
}

/**
 * Agent 主循环配置（由 AgentFactory 从 AppConfig 映射而来）。
 * - provider：主对话模型（OpenAI-compatible 等）
 * - systemPrompt：Tier-0 base / projectContext / customInstructions
 * - maxContextTokens：上下文窗口上限，驱动 Conversation/Budget/Compact
 */
export interface AgentConfig {
  /** 主模型连接与采样参数。 */
  provider: ProviderConfig;
  /** 分层系统提示；base 始终注入，其余经 ContextLoadPolicy 门控。 */
  systemPrompt: SystemPromptConfig;
  /** 上下文 token 硬上限；影响压缩阈值与预算分配。 */
  maxContextTokens?: number;
}

// ============================================================================
// Context / Conversation Types
// ============================================================================

/** Token 用量统计。 */
export interface TokenUsage {
  /** 最近一次模型请求的 prompt（生成回复之前的上下文）。 */
  promptTokens: number;
  /** 本轮累计 completion（多次模型调用相加）。 */
  completionTokens: number;
  totalTokens: number;
  cacheTokens?: number;
  /** 本轮累计计费 prompt（多次模型调用的 prompt 相加）。 */
  billedPromptTokens?: number;
  /** 最近一次模型调用的 completion（用于：结束后的上下文 ≈ prompt + lastCompletion）。 */
  lastCompletionTokens?: number;
}

/** Token 计数方法。 */
export type TokenCountMethod = 'heuristic' | 'tiktoken' | `tiktoken:${string}`;

/** 上下文 token 细分统计。 */
export interface TokenBreakdown {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  systemTokens: number;
  userTokens: number;
  assistantTokens: number;
  toolCallTokens: number;
  toolResultTokens: number;
  toolSchemaTokens: number;
  messageOverheadTokens: number;
  contextLength: number;
  maxInputTokens: number;
  maxOutputTokens: number;
  remainingTokens: number;
  estimated: boolean;
  method: TokenCountMethod;
  model: string;
  providerType: string;
}

/** 当前上下文预算状态。 */
export interface TokenBudget {
  maxInputTokens: number;
  maxOutputTokens: number;
  totalTokens: number;
  usedTokens: number;
  remainingTokens: number;
  inputTokens?: number;
  outputTokens?: number;
  cacheTokens?: number;
  toolSchemaTokens?: number;
  toolResultTokens?: number;
  estimated?: boolean;
  method?: TokenCountMethod;
}

/** 上下文预算分区类型。 */
export type ContextBudgetSectionType =
  | 'system'
  | 'project_instructions'
  | 'current_goal'
  | 'active_goals'
  | 'plan'
  | 'task_dag'
  | 'recent_dialogue'
  | 'relevant_files'
  | 'tool_results'
  | 'memory'
  | 'memory_profile'
  | 'key_facts'
  | 'delegation'
  | 'intent_guidance'
  | 'project_context'
  | 'custom_instructions'
  | 'compact_summary'
  | 'error_log';

/** 上下文来源引用，通常指向文件或指令来源。 */
export interface ContextSourceRef {
  path?: string;
  label?: string;
  scope?: string;
  kind?: string;
}

/** 上下文预算中的单个分区。 */
export interface ContextBudgetSection {
  id: string;
  type: ContextBudgetSectionType;
  title: string;
  priority: number;
  allocatedTokens: number;
  usedTokens: number;
  dropped: boolean;
  truncated: boolean;
  reason?: string;
  sources?: ContextSourceRef[];
  messageCount?: number;
}

/** 一次上下文预算分配结果。 */
export interface ContextBudgetAllocation {
  messages: Message[];
  sections: ContextBudgetSection[];
  maxInputTokens: number;
  toolSchemaTokens: number;
  usedInputTokens: number;
  remainingInputTokens: number;
  droppedMessageCount: number;
  truncatedMessageCount: number;
  breakdown: TokenBreakdown;
}

/** 上下文压缩策略描述。 */
export interface CompactStrategy {
  type: 'autocompact' | 'reactive';
  description: string;
}

// ============================================================================
// Permission Types
// ============================================================================

/** 权限级别。 */
export type PermissionLevel = 'allow' | 'confirm' | 'deny';

/** 权限模式。 */
export type PermissionMode = 'default' | 'plan' | 'auto';

/** 单条权限规则。 */
export interface PermissionRule {
  pattern: string;
  level: PermissionLevel;
  constraints?: Record<string, unknown>;
  source?: RuleSource;
}

/** 权限规则来源。 */
export type RuleSource = 'config' | 'project' | 'user' | 'session' | 'command' | 'cliArg';

/** 权限配置。 */
export interface PermissionConfig {
  default: 'confirm' | 'deny';
  rules: PermissionRule[];
  whitelist: string[];
  mode?: PermissionMode;
}

/** 权限检查结果。 */
export interface PermissionCheckResult {
  allowed: boolean;
  rule?: PermissionRule;
  reason: string;
  needsConfirmation: boolean;
  isSafetyCheck?: boolean;
}

/** 权限审计记录。 */
export interface PermissionAuditEntry {
  timestamp: number;
  toolName: string;
  args: Record<string, unknown>;
  level: PermissionLevel;
  granted: boolean;
  confirmedByUser?: boolean;
  reason?: string;
}

// ============================================================================
// State / Store Types
// ============================================================================

/** immutable store 的状态更新函数。 */
export type StateUpdater<T> = (prev: T) => T;

/** immutable store 的订阅监听器。 */
export type StateListener<T> = (state: T, prevState: T) => void;

/** 最小状态容器接口。 */
export interface Store<T> {
  getState: () => T;
  setState: (updater: StateUpdater<T>) => void;
  subscribe: (listener: StateListener<T>) => () => void;
}

// ============================================================================
// Task Types
// ============================================================================

/** 后台或本地任务状态。 */
export type TaskStatus = 'pending' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';

/** 任务记录。 */
export interface Task {
  id: string;
  name: string;
  status: TaskStatus;
  type: 'local' | 'remote' | 'background';
  createdAt: number;
  updatedAt?: number;
  completedAt?: number;
  error?: string;
  result?: unknown;
}

// ============================================================================
// Hook Types
// ============================================================================

// ============================================================================
// Memory Types
// ============================================================================

/** 长期记忆条目。 */
/** Long-term memory visibility within an otherwise identical ownership scope. */
export type MemoryVisibility = 'conversation' | 'project';

/** Structured ownership dimensions enforced by every memory boundary. */
export interface MemoryScope {
  tenantId: string;
  principalId: string;
  agentId: string;
  projectId: string;
  conversationId?: string;
  visibility: MemoryVisibility;
}

/** 一条长期记忆：内容、分类、重要性、scope 与向量。 */
export interface MemoryEntry {
  id: string;
  content: string;
  category: string;
  importance: number;
  createdAt: number;
  updatedAt?: number;
  source: string;
  /** Tags for semantic retrieval */
  tags?: string[];
  /** Embedding vector for similarity search (canonical in-memory field) */
  embedding?: number[];
  /** Mirror of `embedding` for callers that still read `vector` */
  vector?: number[];
  /** Confidence score for extracted/imported memories (0-1) */
  confidence?: number;
  /** Where an explicit memory file or evidence item came from */
  sourceFile?: string;
  /** Optional line range inside sourceFile */
  sourceRange?: {
    startLine?: number;
    endLine?: number;
  };
  /** Task/run provenance for learned memories */
  sourceTaskId?: string;
  sourceRunId?: string;
  sourceTraceId?: string;
  /** Last time a user, validation result, or explicit file verified this memory */
  lastVerifiedAt?: number;
  /** Injection/use count for retrieval governance */
  usageCount?: number;
  /** Number of times this memory was contradicted or corrected */
  conflictCount?: number;
  /** Whether this memory came from a human-editable source */
  explicit?: boolean;
  /** Human approval state for sensitive inferred memories */
  approvalStatus?: 'direct' | 'pending' | 'approved' | 'rejected';
  /** Explicit profile record kind for database-backed long-term memory. */
  profileKind?: 'identity' | 'user' | 'project' | 'agents' | 'workflow' | 'lessons';
  /** Optional expiration timestamp */
  expiresAt?: number;
  /** Access count for recency/frequency scoring */
  accessCount?: number;
  /** 分阶段反馈，避免“仅被召回”形成自我强化排名。 */
  retrievedCount?: number;
  injectedCount?: number;
  answerUseCount?: number;
  userConfirmedCount?: number;
  validationConfirmedCount?: number;
  /** Last accessed timestamp */
  lastAccessedAt?: number;
  /** Project/session context */
  context?: string;
  /** Structured ownership and visibility boundary. */
  scope?: MemoryScope;
}

/** 记忆分类字符串。 */
export type MemoryCategory =
  | 'identity'
  | 'user_profile'
  | 'user_preference'
  | 'project_fact'
  | 'workflow_rule'
  | 'task_lesson'
  | 'error_pattern'
  | 'task_state'
  | 'error_experience'
  | 'code_knowledge'
  | 'decision'
  | 'session'
  | 'fact'
  | 'context'
  | 'file_change'
  | 'code_solution'
  | 'user_request';

/** 会话摘要。 */
export interface SessionSummary {
  sessionId: string;
  startedAt: number;
  endedAt?: number;
  keyDecisions: string[];
  filesModified: string[];
  toolsUsed: string[];
  /** LLM-generated summary of the session */
  summary?: string;
  /** Token count at time of summary */
  tokenCount?: number;
}

/** 带相关性分数的记忆搜索结果。 */
export interface MemorySearchResult {
  entry: MemoryEntry;
  score: number;
  matchType: 'tag' | 'keyword' | 'recency' | 'importance' | 'vector' | 'hybrid';
}

// ============================================================================
// Episodic Memory Types
// ============================================================================

/** 情景记忆条目，记录"什么情境下做了什么、结果如何"。 */
export interface EpisodicMemory {
  id: string;
  sessionId: string;
  runId?: string;
  episode: {
    goal: string;
    context: string;
    startTimestamp: number;
    endTimestamp: number;
  };
  actions: Array<{
    type: 'thought' | 'tool_call' | 'decision' | 'communication';
    description: string;
    result: 'success' | 'partial' | 'failure';
    timestamp: number;
  }>;
  outcome: {
    success: boolean;
    lessonsLearned: string[];
    surprises: string[];
  };
  embedding?: number[];
}

/** 从多条情景记忆提取的泛化模式。 */
export interface GeneralizedPattern {
  id: string;
  description: string;
  confidence: number;
  sourceEpisodeIds: string[];
  category: 'strategy' | 'pitfall' | 'convention' | 'workflow';
  createdAt: number;
}

/** 记忆存储通用配置。 */
export interface MemoryStoreConfig {
  /** Maximum number of memories to keep */
  maxEntries?: number;
  /** Auto-cleanup entries below this importance threshold after maxEntries is reached */
  cleanupImportanceThreshold?: number;
}

/** 支持的记忆持久化后端。 */
export type MemoryPersistenceBackend = 'postgres';

/** PostgreSQL 记忆后端配置。 */
export interface PostgresMemoryConfig {
  /** PostgreSQL connection string. */
  connectionString?: string;
  /** Database schema for memory tables. */
  schema?: string;
  /** Prefix for created tables and indexes. */
  tablePrefix?: string;
  /** pgvector dimension count. Defaults to the local embedding size. */
  vectorDimensions?: number;
  /** Whether to use TLS for the database connection. */
  ssl?: boolean | { rejectUnauthorized?: boolean };
}

/** 嵌入模型、维度与批次。 */
export interface MemoryEmbeddingConfig {
  /** Embedding backend. hash keeps the offline deterministic fallback. */
  provider?: 'openai' | 'local' | 'hash';
  /** Embedding model name. */
  model?: string;
  /** Embedding vector dimension count. */
  dimensions?: number;
  /** API key for OpenAI-compatible embedding endpoints. */
  apiKey?: string;
  /** Base URL for OpenAI-compatible or local embedding endpoints. */
  baseUrl?: string;
  /** Request timeout in milliseconds. */
  timeoutMs?: number;
  /** Batch size for async embedding refresh. */
  batchSize?: number;
}

/** 记忆持久化总配置。 */
export interface MemoryPersistenceConfig {
  /** PostgreSQL + pgvector memory backend. */
  backend?: MemoryPersistenceBackend;
  /** Maximum number of memories to retain. */
  maxEntries?: number;
  /** Auto-cleanup entries below this importance threshold after maxEntries is reached. */
  cleanupImportanceThreshold?: number;
  embedding?: MemoryEmbeddingConfig;
  postgres?: PostgresMemoryConfig;
}

/** compact 命令选项。 */
export interface CompactOptions {
  /** Force compact even if below threshold */
  force?: boolean;
  /** Specific strategy to use */
  strategy?: 'autocompact' | 'reactive';
  /** Whether to extract memories before compacting */
  extractMemories?: boolean;
  /** Custom summary to use instead of LLM-generated */
  customSummary?: string;
}

/** compact 执行结果。 */
export interface CompactResult {
  success: boolean;
  strategy: string;
  tokensBefore: number;
  tokensAfter: number;
  memoriesExtracted: number;
  summary?: string;
  versionId?: string;
  sourceRange?: {
    startIndex: number;
    endIndex: number;
    messageCount: number;
  };
  quality?: CompactQualityEvaluation;
  canRollback?: boolean;
}

/** compact 质量评估结果。 */
export interface CompactQualityEvaluation {
  score: number;
  retainedKeywords: string[];
  missingKeywords: string[];
  sourceTokenEstimate: number;
  summaryTokenEstimate: number;
  semanticSimilarity?: number;
  warnings: string[];
}

/** 可回滚的 compact 版本。 */
export interface CompactVersion {
  id: string;
  createdAt: number;
  strategy: string;
  summary: string;
  sourceRange: {
    startIndex: number;
    endIndex: number;
    messageCount: number;
  };
  tokensBefore: number;
  tokensAfter: number;
  quality: CompactQualityEvaluation;
  snapshotBefore: ConversationSnapshot;
  snapshotAfter: ConversationSnapshot;
}

/** 对话持久化快照。 */
export interface ConversationSnapshot {
  id: string;
  sessionId: string;
  createdAt: number;
  systemPrompt: string;
  messages: Message[];
  metadata?: Record<string, unknown>;
}

/** Turn 意图（Session 入口语义分类，驱动预算与委派策略）。 */
export type TurnIntent = 'chat' | 'coding' | 'task' | 'debug' | 'review';

/** 一轮意图枚举常量。 */
export const TURN_INTENTS: readonly TurnIntent[] = [
  'chat',
  'coding',
  'task',
  'debug',
  'review',
] as const;
