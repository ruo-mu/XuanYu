/**
 * protocol.ts - Web 端协议类型定义
 *
 * 用途：镜像 gateway 协议并补充前端需要的展开状态结构。
 * 包含 WebSocket 消息、会话、统计、模型能力、扩展、子代理、
 * 权限、记忆、上下文、Run/Trace、变更集、Git、交付和系统状态等类型。
 */

// 所有 WebSocket 消息共享的基础外壳。
export interface WsMessage {
  id: string;
  type: string;
  payload: unknown;
  timestamp: number;
  sequence?: number;
}

export interface ChatAttachment {
  kind: 'image' | 'file';
  name: string;
  mimeType: string;
  sizeBytes: number;
  dataUrl: string;
}

export interface ChatSendPayload {
  sessionId: string;
  content: string;
  deepThinking?: boolean;
  attachments?: ChatAttachment[];
}

export type RewindMode = 'both' | 'conversation' | 'workspace';

export interface RewindTargetPreview {
  turnId: string;
  userMessagePreview: string;
  createdAt: number;
  filesToRevert: string[];
  hasWorkspaceChanges: boolean;
}

// 前端会主动发送给 gateway 的消息类型白名单。
export type ClientMessageType =
  | 'chat.send'
  | 'chat.abort'
  | 'session.create'
  | 'session.delete'
  | 'session.archive'
  | 'session.rename'
  | 'session.pin'
  | 'session.list'
  | 'session.history'
  | 'session.restore'
  | 'session.rewind'
  | 'session.rewind.preview'
  | 'workspace.file.reject'
  | 'goal.get'
  | 'goal.action'
  | 'conversation.search'
  | 'config.get'
  | 'config.capabilities'
  | 'config.update'
  | 'config.save'
  | 'tools.catalog.get'
  | 'permission.respond'
  | 'question.respond'
  | 'stats.get'
  | 'extensions.list'
  | 'marketplace.search'
  | 'marketplace.install.plan'
  | 'marketplace.install.confirm'
  | 'marketplace.install.status'
  | 'capability.installed.list'
  | 'capability.installed.mutate'
  | 'channel.connections.list'
  | 'channel.adapters.list'
  | 'channel.connection.upsert'
  | 'channel.connection.connect'
  | 'channel.connection.disconnect'
  | 'channel.connection.remove'
  | 'channel.delivery.list'
  | 'channel.qr.begin'
  | 'channel.qr.cancel'
  | 'team.status'
  | 'subagent.timeline'
  | 'memory.create'
  | 'memory.list'
  | 'memory.search'
  | 'memory.update'
  | 'memory.delete'
  | 'memory.flush'
  | 'memory.hit.list'
  | 'memory.profile.list'
  | 'memory.profile.read'
  | 'memory.profile.update'
  | 'context.status'
  | 'context.compact'
  | 'context.rollback'
  | 'context.reset'
  | 'run.cancel'
  | 'run.resume'
  | 'replay.request'
  | 'task.list'
  | 'task.status'
  | 'task.pause'
  | 'task.resume'
  | 'task.cancel'
  | 'task.retry'
  | 'feedback.submit'
  | 'git.status'
  | 'git.branches'
  | 'git.checkout'
  | 'workspace.open'
  | 'workspace.create'
  | 'workspace.clear'
  | 'workspace.pickFolder'
  | 'workspace.reveal'
  | 'process.list'
  | 'process.read'
  | 'process.stop'
  | 'system.status'
  | 'gateway.logs'
  | 'agent.list'
  | 'agent.create'
  | 'agent.update'
  | 'agent.delete'
  | 'providers.catalog'

export type ServerMessageType =
  | 'tools.catalog'
  | string;

// 会话列表、分析统计与模型能力探测相关结构。
export interface SessionInfo {
  id: string;
  name: string;
  createdAt: number;
  messageCount: number;
  isProcessing: boolean;
  updatedAt?: number;
  lastMessageAt?: number;
  provider?: string;
  model?: string;
  status?: string;
  archivedAt?: number;
  pinned?: boolean;
  tags?: string[];
  currentRunId?: string;
  currentTaskId?: string;
  storageBackend?: string;
  titleStatus?: 'idle' | 'queued' | 'running' | 'succeeded' | 'failed';
  titleGeneratedBy?: string;
  titleGenerationError?: string;
  titleProvider?: string;
  titleModel?: string;
  projectName?: string;
  projectDir?: string;
  metadata?: Record<string, unknown>;
}



export interface SessionStats {
  sessionId: string;
  name: string;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  messageCount: number;
  toolCallCount: number;
  compactCount: number;
  tokenUsageEstimated: boolean;
}

export interface TokenUsage {
  /** Last model-call prompt (context before that reply). */
  promptTokens: number;
  /** Sum of completions this user turn. */
  completionTokens: number;
  totalTokens: number;
  cacheTokens?: number;
  /** Sum of prompts billed this turn. */
  billedPromptTokens?: number;
  /** Last model-call completion; ended context ≈ promptTokens + lastCompletionTokens. */
  lastCompletionTokens?: number;
}

export interface CostTotals {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  costUsd: number;
  estimated: boolean;
  pricedTokenCount: number;
  unpricedTokenCount: number;
}

export interface AnalyticsRow extends CostTotals {
  key: string;
  label: string;
  runCount: number;
  toolCallCount: number;
  standardCostUsd?: number;
}

export interface ToolAnalyticsRow extends AnalyticsRow {
  successCount: number;
  failureCount: number;
  failureRate: number;
}

export interface TaskAnalyticsRow extends AnalyticsRow {
  runId: string;
  status: string;
  durationMs?: number;
}

export interface QualityMetrics {
  totalRuns: number;
  completedRuns: number;
  failedRuns: number;
  cancelledRuns: number;
  successRate: number;
  averageIterations: number;
  averageToolCalls: number;
  toolFailureRate: number;
  compactTriggerRate: number;
  averageDurationMs: number;
}

export interface PeriodTotals extends CostTotals {
  requestCount: number;
  standardCostUsd: number;
  cacheReadTokens: number;
  cacheCreateTokens: number;
}

export interface PerformanceMetrics {
  rpm: number;
  tpm: number;
}

export interface TokenTrendPoint {
  timestamp: number;
  label: string;
  inputTokens: number;
  outputTokens: number;
  cacheCreateTokens: number;
  cacheReadTokens: number;
  cacheHitRate: number;
  requestCount: number;
}

export interface AnalyticsSnapshot {
  generatedAt: number;
  totals: CostTotals & {
    standardCostUsd: number;
    cacheReadTokens: number;
    cacheCreateTokens: number;
    requestCount: number;
  };
  today: PeriodTotals;
  performance: PerformanceMetrics;
  trend: TokenTrendPoint[];
  byModel: AnalyticsRow[];
  bySession: AnalyticsRow[];
  byTool: ToolAnalyticsRow[];
  byTask: TaskAnalyticsRow[];
  quality: QualityMetrics;
}

export interface ModelCapabilities {
  provider: string;
  model: string;
  stream: boolean;
  toolCall: boolean;
  reasoning: boolean;
  thinkingDisable: boolean;
  jsonMode: boolean;
  usage: boolean;
  vision: boolean;
  maxContext?: number;
  maxOutput?: number;
  rateLimit?: { rpm?: number; concurrency?: number };
  cost?: { input: number; output: number; cachedInput?: number } | null;
  source: string[];
  warnings: string[];
}

export interface ModelCapabilityRoute {
  task: string;
  provider: string;
  model: string;
  capabilities: ModelCapabilities;
  check: {
    ok: boolean;
    warnings: string[];
    errors: string[];
  };
}

export interface ConfigCapabilitiesPayload {
  models: ModelCapabilityRoute[];
}

export interface ToolCatalogEntry {
  name: string;
  description: string;
  category?: string;
  source: 'builtin' | 'mcp' | 'plugin' | 'internal' | 'runtime';
  isWrite: boolean;
  isExecute: boolean;
  isConcurrent: boolean;
  aliases: string[];
}

export interface ToolCatalogPayload {
  sessionId?: string;
  tools: ToolCatalogEntry[];
}

// 扩展生态、子代理团队与时间线相关结构。
export interface SkillDef {
  id: string;
  name: string;
  description: string;
  version?: string;
  tools: string[];
  prompt?: string;
  contextRules: Array<{
    pattern: string;
    include?: string[];
    exclude?: string[];
    priority?: number;
  }>;
  source: string;
  filePath?: string;
  pluginId?: string;
}

export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description?: string;
  enabled: boolean;
  source: string;
  directory?: string;
  marketplaceUrl?: string;
  main?: string;
  apiVersion?: string;
  permissions?: {
    fs: { read: string[]; write: string[] };
    network: { enabled: boolean; allowlist: string[] };
    process: boolean;
    tools: string[];
    env: string[];
  };
  runtime?: {
    loaded: boolean;
    error?: string;
    phase?: string;
    entry?: string;
    registered: {
      tools: string[];
      providers: string[];
      hooks: string[];
      skills: string[];
      channels: string[];
    };
    lifecycle: {
      load: boolean;
      init: boolean;
      activate: boolean;
      deactivate: boolean;
      destroy: boolean;
    };
  };
  contributes: {
    tools: string[];
    providers: string[];
    hooks: string[];
    uiPanels: string[];
    skills: SkillDef[];
  };
}

export interface ExtensionsListPayload {
  skills: SkillDef[];
  plugins: PluginManifest[];
  capabilities?: Array<{
    id: string;
    kind: string;
    name: string;
    description?: string;
  }>;
  totals: {
    skills: number;
    plugins: number;
    enabledPlugins: number;
    tools: number;
    providers: number;
    hooks: number;
    uiPanels: number;
  };
}

export interface SubAgentTeamMember {
  agentId: string;
  name: string;
  role: string;
  description: string;
  status: 'idle' | 'queued' | 'running' | 'completed' | 'failed' | 'blocked' | 'cancelled';
  currentTask?: string;
  enabled?: boolean;
  model: string;
  provider: string;
  deepThinking?: boolean;
  reasoningEffort?: 'low' | 'medium' | 'high' | 'max' | 'ultracode';
  tools: string[];
  canWrite: boolean;
  defaultSandboxMode?: 'workspace' | 'worktree' | 'readonly';
  source: string;
  latestResult?: {
    taskId: string;
    status: string;
    conclusion: string;
    modifiedFiles: string[];
    risks: string[];
    blockers: string[];
    nextSteps: string[];
    completedAt: number;
  };
  latestProgress?: SubAgentProgressUpdate;
  activeTaskIds: string[];
  updatedAt: number;
}

export interface SubAgentTeamSnapshot {
  generatedAt: number;
  members: SubAgentTeamMember[];
  runningCount: number;
  completedCount: number;
  failedCount: number;
  activeTasks?: SubAgentTaskLedger[];
  recentTasks?: SubAgentTaskLedger[];
}

export interface SubAgentProgressUpdate {
  taskId: string;
  agentId: string;
  stepTitle: string;
  summary: string;
  evidence: string[];
  risks: string[];
  nextStep?: string;
  createdAt: number;
}

export interface SubAgentTimelineEvent {
  id: string;
  taskId: string;
  agentId: string;
  type: string;
  timestamp: number;
  title?: string;
  status?: string;
  payload: Record<string, unknown>;
}

export interface SubAgentTaskLedger {
  taskId: string;
  agentId: string;
  agentName: string;
  role?: string;
  task: {
    agentId: string;
    task: string;
    context?: string;
    writeScope?: string[];
    metadata?: Record<string, unknown>;
  };
  status: 'queued' | 'running' | 'completed' | 'failed' | 'blocked' | 'cancelled';
  provider?: string;
  model?: string;
  startedAt: number;
  updatedAt: number;
  completedAt?: number;
  progress: SubAgentProgressUpdate[];
  timeline: SubAgentTimelineEvent[];
  result?: {
    taskId: string;
    agentId: string;
    agentName: string;
    status: string;
    conclusion: string;
    evidence: string[];
    modifiedFiles: string[];
    risks: string[];
    blockers: string[];
    nextSteps: string[];
    output: string;
    startedAt: number;
    completedAt: number;
  };
  watchdog: {
    lastEventAt: number;
    lastProgressAt?: number;
    repeatedToolCount: number;
    consecutiveErrors: number;
    retries: number;
    warnings: string[];
    blockedReason?: string;
  };
}

export interface AgentDefinitionInput {
  id: string;
  name: string;
  description: string;
  prompt: string;
  tools: string[];
  permissionMode: 'default' | 'plan' | 'auto';
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKey?: string;
  maxTokens?: number;
  temperature?: number;
  /** 列表展示用，由工具集推导。 */
  canWrite?: boolean;
  /** 是否允许被主 Agent 委派。 */
  enabled?: boolean;
  defaultSandboxMode?: 'workspace' | 'worktree' | 'readonly';
  memoryScope: string;
  role?: string;
  source?: string;
}

export interface ProviderCatalogEntry {
  type: string;
  name: string;
  defaultBaseUrl?: string;
}

export interface AgentEvent {
  type: string;
  content?: string;
  title?: string;
  payload?: Record<string, unknown>;
  message?: { role: string; content: string; timestamp?: number };
  call?: { id: string; name: string; arguments: string };
  result?: { tool_call_id: string; content: string; isError?: boolean };
  error?: { name?: string; message: string };
  usage?: TokenUsage;
  toolUseId?: string;
  toolName?: string;
  data?: unknown;
  context?: ContextBudgetAllocation;
  cancelled?: boolean;
  finalResponse?: string;
  iterations?: number;
}

// 权限请求、记忆、上下文压缩与预算分配相关结构。
export interface PermissionRequestPayload {
  sessionId: string;
  requestId: string;
  toolName: string;
  args: Record<string, unknown>;
  reason: string;
  runId?: string;
  approvalDigest: string;
}

export interface MemoryItem {
  id: string;
  content: string;
  category: string;
  importance: number;
  createdAt: number;
  updatedAt?: number;
  tags?: string[];
  source: string;
  confidence?: number;
  expiresAt?: number;
  accessCount?: number;
  lastAccessedAt?: number;
  context?: string;
  sourceFile?: string;
  sourceRange?: { startLine?: number; endLine?: number };
  sourceTaskId?: string;
  sourceRunId?: string;
  sourceTraceId?: string;
  lastVerifiedAt?: number;
  usageCount?: number;
  conflictCount?: number;
  explicit?: boolean;
  approvalStatus?: 'direct' | 'pending' | 'approved' | 'rejected';
  profileKind?: 'identity' | 'user' | 'project' | 'agents' | 'workflow' | 'lessons';
}

export interface MemoryProfileFile {
  kind: 'identity' | 'user' | 'project' | 'agents' | 'workflow' | 'lessons';
  scope: 'global' | 'project';
  path: string;
  relativePath: string;
  title: string;
  content: string;
  exists: boolean;
  updatedAt?: number;
}

export interface MemoryHitRecord {
  id: string;
  sessionId: string;
  runId?: string;
  turnId?: string;
  query?: string;
  source: 'query' | 'system';
  memoryId: string;
  score?: number;
  category?: string;
  injected: boolean;
  createdAt: number;
}

export interface ContextStatus {
  messageCount: number;
  estimatedTokens: number;
  maxTokens: number;
  usageRatio: number;
  autoCompactEnabled: boolean;
  autoCompactThreshold: number;
  compactCount: number;
  lastCompact?: {
    strategy: string;
    tokensBefore: number;
    tokensAfter: number;
    memoriesExtracted: number;
    summary?: string;
  };
  breakdown?: TokenBreakdown;
  allocation?: ContextBudgetStatus;
  turnIntent?: string;
  activeSections?: string[];
  contextLoadPolicy?: Record<string, unknown>;
}

export interface ContextBudgetStatus {
  sections: ContextBudgetSection[];
  maxInputTokens: number;
  toolSchemaTokens: number;
  usedInputTokens: number;
  remainingInputTokens: number;
  droppedMessageCount: number;
  truncatedMessageCount: number;
}

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
  method: string;
  model: string;
  providerType: string;
}

export interface ContextBudgetSection {
  id: string;
  type: string;
  title: string;
  priority: number;
  allocatedTokens: number;
  usedTokens: number;
  dropped: boolean;
  truncated: boolean;
  reason?: string;
  sources?: Array<{
    path?: string;
    label?: string;
    scope?: string;
    kind?: string;
  }>;
  messageCount?: number;
}

export interface ContextBudgetAllocation {
  messages: unknown[];
  sections: ContextBudgetSection[];
  maxInputTokens: number;
  toolSchemaTokens: number;
  usedInputTokens: number;
  remainingInputTokens: number;
  droppedMessageCount: number;
  truncatedMessageCount: number;
  breakdown: TokenBreakdown;
}

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
  quality?: {
    score: number;
    retainedKeywords: string[];
    missingKeywords: string[];
    sourceTokenEstimate: number;
    summaryTokenEstimate: number;
    warnings: string[];
  };
  canRollback?: boolean;
}

// Run / trace / evidence：驱动运行面板与调试视图。
export interface RunRecord {
  id: string;
  sessionId: string;
  traceId: string;
  goal: string;
  status: 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';
  createdAt: number;
  updatedAt: number;
  completedAt?: number;
  error?: string;
  currentStepId?: string;
  checkpointId?: string;
  metadata?: Record<string, unknown>;
}

export type RunTodoStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface RunTodoItem {
  id: string;
  title: string;
  status: RunTodoStatus;
  updatedAt: number;
}

export interface RunPlan {
  goal: string;
  constraints: string[];
  risks: string[];
  acceptanceCriteria: string[];
  todos: RunTodoItem[];
  createdAt: number;
  updatedAt: number;
}

export interface StepRecord {
  id: string;
  runId: string;
  sessionId: string;
  type: 'agent' | 'model' | 'tool' | 'permission' | 'compact' | 'system';
  name: string;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
  startedAt: number;
  completedAt?: number;
  error?: string;
  metadata?: Record<string, unknown>;
}

export interface RunEvent {
  id: string;
  runId: string;
  sessionId: string;
  traceId: string;
  type: string;
  timestamp: number;
  stepId?: string;
  payload?: unknown;
}

export interface RunTimeline {
  run: RunRecord;
  steps: StepRecord[];
  events: RunEvent[];
  latestCheckpoint?: unknown;
}

export type NestedTaskStatus =
  | 'queued'
  | 'running'
  | 'cancelling'
  | 'done'
  | 'failed'
  | 'cancelled'
  | 'timed_out';

export type NestedTaskType = 'coding' | 'shell' | 'review' | 'explore' | 'plan';

export interface NestedTask {
  taskId: string;
  parentTaskId?: string;
  sessionId: string;
  runId?: string;
  type: NestedTaskType;
  status: NestedTaskStatus;
  priority: number;
  params: Record<string, unknown>;
  scope: {
    cwd: string;
    writeScope?: string[];
    sandboxMode: 'readonly' | 'worktree';
    permissionMode: 'default' | 'plan' | 'auto';
    owner: {
      tenantId: string;
      principalId: string;
      projectId: string;
    };
    resourceClaims: Array<{ key: string; mode: 'read' | 'write' }>;
  };
  executor: {
    preferredType: NestedTaskType;
    assignedExecutorId?: string;
    leaseUntil?: number;
    attempt: number;
    maxAttempts: number;
    timeoutMs: number;
    deadline?: number;
  };
  createdAt: number;
  updatedAt: number;
  startedAt?: number;
  completedAt?: number;
  metadata?: Record<string, unknown>;
}

export interface NestedExecutorLease {
  taskId: string;
  executorId: string;
  leaseUntil: number;
  createdAt: number;
  updatedAt: number;
}

export interface NestedTaskEvent {
  eventId: string;
  taskId: string;
  sessionId: string;
  runId?: string;
  type: string;
  actor: {
    kind: 'orchestrator' | 'executor' | 'system';
    id: string;
    name: string;
  };
  payload: Record<string, unknown>;
  createdAt: number;
}

export interface NestedTaskResult {
  taskId: string;
  status: 'done' | 'failed' | 'cancelled' | 'timed_out';
  output: {
    conclusion: string;
    changedFiles: string[];
    evidence: string[];
    risks: string[];
    nextSteps: string[];
  };
  error?: {
    code: string;
    message: string;
    recoverable: boolean;
  };
  artifacts?: Array<{ kind: string; path?: string; content?: string }>;
  createdAt: number;
  updatedAt: number;
}

export interface NestedTaskSnapshot {
  task: NestedTask;
  events: NestedTaskEvent[];
  result?: NestedTaskResult;
  lease?: NestedExecutorLease;
}

// 变更集、Git、交付和系统状态相关结构。
export type BackgroundProcessStatus = 'running' | 'completed' | 'failed' | 'stopped';

export interface BackgroundProcessSnapshot {
  id: string;
  pid?: number;
  name?: string;
  command: string;
  cwd: string;
  logPath: string;
  status: BackgroundProcessStatus;
  startedAt: number;
  completedAt?: number;
  exitCode?: number | null;
  signal?: string | null;
  outputBytes: number;
}



export interface GitStatusFile {
  path: string;
  from?: string;
  index: string;
  workingTree: string;
  status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked' | 'conflicted';
  staged: boolean;
  unstaged: boolean;
}

export interface GitStatusSnapshot {
  root: string;
  branch: string;
  head: string;
  upstream?: string;
  ahead: number;
  behind: number;
  clean: boolean;
  files: GitStatusFile[];
}

export interface ConversationStoreHealth {
  backend: 'postgres';
  ok: boolean;
  location: string;
  pgvector?: boolean;
  error?: string;
}

export interface MemoryStoreHealth {
  backend: 'postgres';
  ok?: boolean;
  location?: string;
  pgvector?: boolean;
}

export type GatewayLogLevel = 'debug' | 'info' | 'warn' | 'error';

export interface GatewayLogEntry {
  id: string;
  timestamp: number;
  level: GatewayLogLevel;
  scope: string;
  message: string;
  details?: Record<string, unknown>;
}

export interface ExternalTaskDraft {
  title: string;
  body: string;
  branchName?: string;
  labels?: string[];
  platform?: string;
  issue?: string;
  branch?: string;
  commands?: string[];
}

export interface SystemStatusPayload {
  projectName: string;
  projectDir: string;
  conversation: ConversationStoreHealth;
  memory: MemoryStoreHealth;
  models?: ModelCapabilityRoute[];
  gateway?: {
    enabled: boolean;
    port: number;
    bind: string;
    wsClients?: number;
  };
  runtime?: {
    pid: number;
    uptimeSec: number;
    node: string;
    platform: string;
    cwd: string;
    startedAt: number;
  };
  workers?: Array<WorkerHealthPayload & { name: string; status: string }>;
  doctor?: {
    status: string;
    failed: number;
    warnings: number;
    checks: Array<{
      name: string;
      status: string;
      detail?: string;
      suggestion?: string;
    }>;
  };
  security?: {
    sensitiveFindings: unknown[];
    warnings: unknown[];
  };
}

// Desktop IPC 专用类型（参考项目无 WS 网关时由 Rust 核心直接暴露）。
export interface ExternalGitIntegrationStatus {
  platform: string;
  connected?: boolean;
  configured?: boolean;
  currentBranch?: string;
  authHint?: string;
  nextSteps?: string[];
  detail?: string;
}

export interface MemorySnapshotInfo {
  id?: string;
  sessionId?: string;
  createdAt?: number;
  label?: string;
  version?: number;
  timestamp?: number;
  entryCount?: number;
  reason?: string;
}

export interface LocalPullRequest {
  id?: string;
  title: string;
  body?: string;
  background?: string;
  branchName?: string;
  baseBranch?: string;
  files: string[];
  platform?: string;
  issue?: string;
  commitMessage?: string;
  summary?: string[];
  checks?: string[];
  risks?: string[];
  rollback?: string[];
  diff?: string;
  status?: GitStatusSnapshot;
  createdAt?: number;
  markdownPath?: string;
  htmlPath?: string;
  patchPath?: string;
  external?: Record<string, unknown>;
}

export interface SessionSummary {
  sessionId: string;
  summary: string | null;
  updatedAt?: number;
  startedAt?: number;
  endedAt?: number | null;
  keyDecisions?: string[];
  filesModified?: string[];
  toolsUsed?: string[];
  tokenCount?: number | null;
}

export interface PendingMemory {
  id: string;
  sessionId: string;
  content: string;
  category?: string;
  createdAt: number;
}

export interface ControlTaskEvent {
  taskId: string;
  sessionId?: string;
  type: string;
  payload?: Record<string, unknown>;
  createdAt?: number;
}

export interface ControlTaskResult {
  taskId: string;
  sessionId?: string;
  status: string;
  output?: Record<string, unknown>;
}

export interface AttemptComparison {
  runId: string;
  attempts: Array<Record<string, unknown>>;
}

export interface EvidencePack {
  runId: string;
  items: Array<Record<string, unknown>>;
}

export interface TraceTimeline {
  runId: string;
  events: Array<Record<string, unknown>>;
}

export interface WorkerHealthPayload {
  name: string;
  status: string;
  detail?: string;
  lastSeenMs?: number;
}
