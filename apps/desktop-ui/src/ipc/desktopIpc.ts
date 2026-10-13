import type {
  AnalyticsSnapshot,
  AttemptComparison,
  BackgroundProcessSnapshot,
  ChatAttachment,
  CompactResult,
  ConfigCapabilitiesPayload,
  ContextStatus,
  EvidencePack,
  ExtensionsListPayload,
  GatewayLogEntry,
  GitStatusSnapshot,
  MemoryHitRecord,
  MemoryItem,
  MemoryProfileFile,
  MemorySnapshotInfo,
  SessionSummary,
  PendingMemory,
  RewindMode,
  RewindTargetPreview,
  RunRecord,
  RunTimeline,
  SessionInfo,
  SessionStats,
  SubAgentTaskLedger,
  SubAgentTimelineEvent,
  SystemStatusPayload,
  ToolCatalogEntry,
  ToolCatalogPayload,
  TraceTimeline,
  WorkerHealthPayload,
} from '../protocol';
import type { ChatMessage, PendingPermission } from '../stores/chatStore';
import type { AppConfig } from '../stores/configStore';
import type { UnlistenFn } from './ipcTypes';
import {
  invoke,
  isDesktopIpcAvailable,
  listen,
  workspacePickFolder,
  workspaceReveal,
} from './desktopIpcInvoke';

export {
  isDesktopIpcAvailable,
  listen,
  workspacePickFolder,
  workspaceReveal,
};
/** Generated raw wrappers (positional/`payload`); prefer typed helpers below when available. */
import * as DesktopIpcGen from './desktopIpc.gen';
export { DesktopIpcGen };
export {
  goalGet,
  marketplaceInstallStatus,
  capabilityInstalledList,
} from './desktopIpc.gen';

interface RustSessionInfo {
  id: string | { 0?: string };
  name: string;
  createdAt?: string | number;
  created_at?: string | number;
  updatedAt?: string | number;
  updated_at?: string | number;
  messageCount?: number;
  message_count?: number;
  isProcessing?: boolean;
  is_processing?: boolean;
  storageBackend?: string;
  storage_backend?: string;
  titleStatus?: SessionInfo['titleStatus'];
  title_status?: SessionInfo['titleStatus'];
  titleGeneratedBy?: string;
  title_generated_by?: string;
  titleGenerationError?: string;
  title_generation_error?: string;
  titleProvider?: string;
  title_provider?: string;
  titleModel?: string;
  title_model?: string;
  pinned?: boolean;
  projectDir?: string;
  project_dir?: string;
}

interface RustChatMessage {
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  timestamp: string | number;
  turnId?: string | { 0?: string };
  turn_id?: string | { 0?: string };
  attachments?: ChatAttachment[];
  metadata?: Record<string, unknown> | null;
  id?: string;
  toolCallId?: string;
  tool_call_id?: string;
  name?: string;
  /** 服务端挂到 assistant 的时间线 segments（历史回放主路径）。 */
  segments?: Array<Record<string, unknown>>;
}

interface RustMemoryItem {
  id: string;
  content: string;
  category: string;
  importance: number;
  createdAt?: string | number;
  created_at?: string | number;
  updatedAt?: string | number;
  updated_at?: string | number;
  tags?: string[];
  source?: string;
  accessCount?: number;
  access_count?: number;
  lastAccessedAt?: string | number | null;
  last_accessed_at?: string | number | null;
}

interface RustMemoryStats {
  total: number;
  categories: Record<string, number>;
}

interface RustMemoryListResult {
  memories: RustMemoryItem[];
  stats: RustMemoryStats;
}

interface RustMemoryHitRecord {
  id: string;
  sessionId?: string;
  session_id?: string;
  runId?: string | null;
  run_id?: string | null;
  turnId?: string | null;
  turn_id?: string | null;
  query?: string | null;
  source: 'query' | 'system' | string;
  memoryId?: string;
  memory_id?: string;
  score?: number | null;
  category?: string | null;
  injected: boolean;
  createdAt?: string | number;
  created_at?: string | number;
}

interface RustMemoryMutationResult {
  memory: RustMemoryItem;
  stats: RustMemoryStats;
}

interface RustMemorySnapshotInfo {
  version: number;
  timestamp: string | number;
  entryCount?: number;
  entry_count?: number;
  reason: string;
}

interface RustMemorySnapshotCreateResult {
  snapshot: RustMemorySnapshotInfo;
  snapshots: RustMemorySnapshotInfo[];
  stats: RustMemoryStats;
}

interface RustMemorySnapshotRollbackResult {
  snapshot: RustMemorySnapshotInfo;
  restoredCount?: number;
  restored_count?: number;
  snapshots: RustMemorySnapshotInfo[];
  stats: RustMemoryStats;
}

export interface ToolExecuteOutput {
  content: string;
  isError?: boolean;
  is_error?: boolean;
  data?: unknown;
}

export interface PermissionRespondResult {
  ok: boolean;
  requestId: string;
  sessionId?: string;
  toolName?: string;
  allowed: boolean;
  remember?: boolean;
  scope?: 'once' | 'session' | 'always' | string;
  rememberError?: string | null;
  output?: ToolExecuteOutput;
  error?: string;
}

export interface MemoryListResult {
  memories: MemoryItem[];
  stats: RustMemoryStats;
}

export interface MemoryMutationResult {
  memory: MemoryItem;
  stats: RustMemoryStats;
}

export interface MemoryMaintenanceResult {
  ok: boolean;
  merged: number;
  distilled: number;
  pruned: number;
  decayed: string[];
  dropped: string[];
  kept: number;
  embeddingRefresh?: {
    attempted: number;
    refreshed: number;
    skipped: number;
    failed: number;
  };
  stats: RustMemoryStats;
}

export interface MemorySnapshotListResult {
  snapshots: MemorySnapshotInfo[];
}

export interface MemorySnapshotCreateResult {
  snapshot: MemorySnapshotInfo;
  snapshots: MemorySnapshotInfo[];
  stats: RustMemoryStats;
}

export interface MemorySnapshotRollbackResult {
  snapshot: MemorySnapshotInfo;
  restoredCount: number;
  snapshots: MemorySnapshotInfo[];
  stats: RustMemoryStats;
}

export interface DesktopAppStatus {
  protocolVersion?: string;
  initialized: boolean;
  storageBackend?: string;
  activeSessions?: number;
  modules?: Array<{ name: string; status: string; detail?: string }>;
  workers?: WorkerHealthPayload[];
}

export interface DesktopConfigInfo {
  path: string;
  exists: boolean;
  dataDir: string;
  projectName: string;
}

export interface InitModelsFetchInput {
  providerType: string;
  baseUrl?: string;
  apiKey?: string;
  protocol?: 'openai' | 'anthropic' | string;
  timeoutMs?: number;
}

export interface InitModelsFetchResult {
  models: string[];
  error?: string | null;
}

export interface InitPostgresStatusResult {
  phase: 'ready' | 'needs_init' | 'needs_start' | 'needs_pgvector' | 'missing_program';
  program: boolean;
  installed: boolean;
  running: boolean;
  pgvector: boolean;
  port: number;
  database: string;
  username: string;
  programDir: string;
  dataDir: string;
  connectionString: string;
  error?: string;
}

export interface InitPostgresStartResult {
  ok: boolean;
  connectionString?: string;
  config?: Record<string, unknown>;
  pgvector?: boolean;
  logs: string[];
  error?: string;
}

export interface DiagnosticsExportResult {
  kind: 'xuanyu.diagnostics';
  version: number;
  exportedAt: number;
  protocolVersion?: string;
  path: string;
  jsonPath?: string;
  archiveFormat?: 'zip';
  [key: string]: unknown;
}

export interface DataBackupExportResult {
  ok: boolean;
  path: string;
  manifestPath: string;
  filesRoot: string;
  fileCount: number;
  totalBytes: number;
  includedRoots: string[];
  skippedRoots: string[];
  restartRecommended?: boolean;
}

export interface DataBackupRestoreResult {
  ok: boolean;
  path: string;
  manifestPath: string;
  restoredFiles: string[];
  restoredFileCount: number;
  totalBytes: number;
  restartRecommended: boolean;
}

export async function appInit(): Promise<DesktopAppStatus> {
  return DesktopIpcGen.appInit();
}

export async function appStatus(): Promise<DesktopAppStatus> {
  return DesktopIpcGen.appStatus();
}

export async function sessionCreate(options: {
  mode?: 'main' | 'recent' | 'branch';
  title?: string;
  projectDir?: string;
} = {}): Promise<SessionInfo> {
  const session = await DesktopIpcGen.sessionCreate<RustSessionInfo>(
    options.mode,
    options.title,
    options.projectDir,
  );
  return toUiSession(session);
}

export async function sessionList(options: {
  includeArchived?: boolean;
  search?: string;
  limit?: number;
} = {}): Promise<SessionInfo[]> {
  const sessions = await DesktopIpcGen.sessionList<RustSessionInfo[]>(
    options.includeArchived,
    options.search,
    options.limit,
  );
  return sessions.map(toUiSession);
}

export interface SessionParticipant {
  id: string;
  role: string;
  displayName?: string;
  externalId?: string;
  lastSeenAt: string;
}

export interface SessionThread {
  id: string;
  title: string;
  parentThreadId?: string;
  createdAt: string;
}

export interface SessionIsolationScope {
  workspaceRoot: string;
  allowedPaths: string[];
  readonly: boolean;
}

export async function sessionParticipantsList(
  sessionId: string,
): Promise<{ sessionId: string; participants: SessionParticipant[] }> {
  return DesktopIpcGen.sessionParticipantsList(sessionId);
}

export async function sessionThreadsList(
  sessionId: string,
): Promise<{ sessionId: string; threads: SessionThread[] }> {
  return DesktopIpcGen.sessionThreadsList(sessionId);
}

export async function sessionIsolationGet(
  sessionId: string,
): Promise<{ sessionId: string; isolation: SessionIsolationScope | null }> {
  return DesktopIpcGen.sessionIsolationGet(sessionId);
}

export async function sessionHistory(sessionId: string): Promise<ChatMessage[]> {
  const messages = await DesktopIpcGen.sessionHistory<RustChatMessage[]>(sessionId);
  return toUiMessages(messages);
}

export async function sessionResetContext(sessionId: string): Promise<{
  ok: boolean;
  sessionId: string;
  messageCountBefore: number;
  segmentCountBefore: number;
  session?: SessionInfo;
  status?: ContextStatus;
  compact?: { result?: CompactResult; status?: ContextStatus } | null;
}> {
  const result = await DesktopIpcGen.sessionResetContext<{
    ok: boolean;
    sessionId: string;
    messageCountBefore: number;
    segmentCountBefore: number;
    session?: RustSessionInfo;
    status?: ContextStatus;
    compact?: { result?: CompactResult; status?: ContextStatus } | null;
  }>(sessionId);
  return {
    ...result,
    session: result.session ? toUiSession(result.session) : undefined,
  };
}

export async function conversationSearch(query: string, limit = 30): Promise<SessionInfo[]> {
  const sessions = await DesktopIpcGen.conversationSearch<RustSessionInfo[]>(query, limit);
  return sessions.map(toUiSession);
}

export async function sessionRename(sessionId: string, name: string): Promise<SessionInfo | null> {
  const session = await DesktopIpcGen.sessionRename<RustSessionInfo | null>(sessionId, name);
  return session ? toUiSession(session) : null;
}

export async function sessionPin(sessionId: string, pinned: boolean): Promise<SessionInfo | null> {
  const session = await DesktopIpcGen.sessionPin<RustSessionInfo | null>(sessionId, pinned);
  return session ? toUiSession(session) : null;
}

export async function workspaceFileReject(input: {
  sessionId: string;
  turnId: string;
  path: string;
}): Promise<Record<string, unknown>> {
  return DesktopIpcGen.workspaceFileReject(input.sessionId, input.turnId, input.path);
}

export async function goalAction(input: {
  sessionId: string;
  action: string;
  objective?: string;
  tokenBudget?: number;
}): Promise<{ ok: boolean; error?: string; snapshot?: { sessionId?: string; goal?: unknown }; continueMessage?: string }> {
  return DesktopIpcGen.goalAction(input.sessionId, input.action, input.objective, input.tokenBudget);
}

export async function marketplaceSearch(input: {
  query?: string;
  kind?: string;
  kinds?: string[];
  requestId?: string;
  page?: number;
  pageSize?: number;
  category?: string;
  sourceId?: string;
  summaryOnly?: boolean;
}): Promise<Record<string, unknown>> {
  return DesktopIpcGen.marketplaceSearch(
    input.query,
    input.kind,
    input.kinds,
    input.requestId,
    input.page,
    input.pageSize,
    input.category,
    input.sourceId,
    input.summaryOnly,
  );
}

export async function marketplaceInstallPlan(input: {
  entryId: string;
  scope?: 'user' | 'project' | 'org';
}): Promise<Record<string, unknown>> {
  return DesktopIpcGen.marketplaceInstallPlan(input.entryId, input.scope);
}

export async function marketplaceInstallConfirm(input: {
  planId: string;
  approvalDigest: string;
}): Promise<Record<string, unknown>> {
  return DesktopIpcGen.marketplaceInstallConfirm(input.planId, input.approvalDigest);
}

export async function capabilityInstalledMutate(input: {
  entryId: string;
  action: 'enable' | 'disable' | 'uninstall' | 'rollback';
}): Promise<Record<string, unknown>> {
  return DesktopIpcGen.capabilityInstalledMutate(input.entryId, input.action);
}

export async function sessionArchive(sessionId: string, archived = true): Promise<boolean> {
  return DesktopIpcGen.sessionArchive(sessionId, archived);
}

export async function sessionDelete(sessionId: string): Promise<boolean> {
  return DesktopIpcGen.sessionDelete(sessionId);
}

export async function sessionAbort(sessionId: string): Promise<{
  ok: boolean;
  sessionId?: string;
  session?: SessionInfo | null;
  cancelledRun?: RunRecord | null;
  runId?: string;
  error?: string;
}> {
  const result = await DesktopIpcGen.sessionAbort<{
    ok: boolean;
    sessionId?: string;
    session?: RustSessionInfo | null;
    cancelledRun?: RunRecord | null;
    runId?: string;
    error?: string;
  }>(sessionId);
  return {
    ...result,
    session: result.session ? toUiSession(result.session) : result.session,
  };
}

export async function sessionSendMessage(sessionId: string, content: string, attachments?: ChatAttachment[], deepThinking = false): Promise<string> {
  const turn = await DesktopIpcGen.sessionSendMessage<string | { 0?: string }>(
    sessionId,
    content,
    attachments && attachments.length > 0 ? attachments : null,
    deepThinking,
  );
  return unwrapId(turn);
}

export async function sessionRewindPreview(sessionId: string): Promise<{
  sessionId: string;
  targets: RewindTargetPreview[];
}> {
  return DesktopIpcGen.sessionRewindPreview(sessionId);
}

export async function sessionRewind(input: {
  sessionId: string;
  turnId: string;
  mode?: RewindMode;
  newContent?: string;
}): Promise<{
  sessionId: string;
  rewound: {
    turnId: string;
    mode: RewindMode;
    removedMessageCount: number;
    removedSegmentCount: number;
    workspaceRestored: boolean;
    filesReverted: string[];
    conflicts: Array<{ path: string; reason: string }>;
    cancelledTasks: number;
    rolledBackChangeSets: number;
    resentTurnId?: string;
  };
}> {
  return DesktopIpcGen.sessionRewind(
    input.sessionId,
    input.turnId,
    input.mode,
    input.newContent,
  );
}

export async function memoryList(limit = 50): Promise<MemoryListResult> {
  const result = await DesktopIpcGen.memoryList<RustMemoryListResult>(limit);
  return {
    memories: result.memories.map(toUiMemory),
    stats: result.stats,
  };
}

export async function memorySearch(query: string, limit = 20): Promise<MemoryListResult> {
  const result = await DesktopIpcGen.memorySearch<RustMemoryListResult>(query, limit);
  return {
    memories: result.memories.map(toUiMemory),
    stats: result.stats,
  };
}

export async function memoryCreate(input: {
  content: string;
  category: string;
  importance: number;
  tags?: string[];
}): Promise<MemoryMutationResult> {
  const result = await DesktopIpcGen.memoryCreate<RustMemoryMutationResult>(
    input.content,
    input.category,
    input.tags || [],
    undefined,
    input.importance,
  );
  return {
    memory: toUiMemory(result.memory),
    stats: result.stats,
  };
}

export async function memoryUpdate(
  id: string,
  updates: {
    content?: string;
    category?: string;
    importance?: number;
    tags?: string[];
  },
): Promise<MemoryMutationResult | null> {
  const result = await DesktopIpcGen.memoryUpdate<RustMemoryMutationResult | null>(
    id,
    updates.content,
    updates.category,
    updates.tags,
  );
  return result
    ? {
        memory: toUiMemory(result.memory),
        stats: result.stats,
      }
    : null;
}

export async function memoryDelete(id: string): Promise<boolean> {
  return DesktopIpcGen.memoryDelete(id);
}

export async function memoryFlush(): Promise<{
  ok: boolean;
  path?: string | null;
  stats?: RustMemoryStats;
}> {
  return DesktopIpcGen.memoryFlush();
}

export async function memoryMaintenanceRun(): Promise<MemoryMaintenanceResult> {
  return DesktopIpcGen.memoryMaintenanceRun();
}

export async function memorySnapshotList(): Promise<MemorySnapshotListResult> {
  const result = await DesktopIpcGen.memorySnapshotList<{ snapshots: RustMemorySnapshotInfo[] }>();
  return { snapshots: result.snapshots.map(toUiMemorySnapshot) };
}

export async function memorySnapshotCreate(reason?: string): Promise<MemorySnapshotCreateResult> {
  const result = await DesktopIpcGen.memorySnapshotCreate<RustMemorySnapshotCreateResult>(reason);
  return {
    snapshot: toUiMemorySnapshot(result.snapshot),
    snapshots: result.snapshots.map(toUiMemorySnapshot),
    stats: result.stats,
  };
}

export async function memorySnapshotRollback(version: number): Promise<MemorySnapshotRollbackResult | null> {
  const result = await DesktopIpcGen.memorySnapshotRollback<RustMemorySnapshotRollbackResult | null>(version);
  return result
    ? {
        snapshot: toUiMemorySnapshot(result.snapshot),
        restoredCount: Number(result.restoredCount ?? result.restored_count ?? 0),
        snapshots: result.snapshots.map(toUiMemorySnapshot),
        stats: result.stats,
      }
    : null;
}

export async function memoryProfileList(): Promise<{ files: MemoryProfileFile[] }> {
  return DesktopIpcGen.memoryProfileList();
}

export async function memoryProfileRead(kind: MemoryProfileFile['kind']): Promise<{ file?: MemoryProfileFile | null }> {
  return DesktopIpcGen.memoryProfileRead(kind);
}

export async function memoryProfileUpdate(
  kind: MemoryProfileFile['kind'],
  content: string,
): Promise<{ file: MemoryProfileFile; files: MemoryProfileFile[] }> {
  return DesktopIpcGen.memoryProfileUpdate(kind, content);
}

export async function memoryPendingList(limit = 100): Promise<{ pending: PendingMemory[] }> {
  return DesktopIpcGen.memoryPendingList(limit);
}

export async function memoryPendingApprove(id: string): Promise<{
  id: string;
  pending?: PendingMemory;
  memory?: MemoryItem;
  stats?: RustMemoryStats;
  error?: string;
}> {
  const result = await DesktopIpcGen.memoryPendingApprove<{
    id: string;
    pending?: PendingMemory;
    memory?: RustMemoryItem;
    stats?: RustMemoryStats;
    error?: string;
  }>(id);
  return {
    ...result,
    memory: result.memory ? toUiMemory(result.memory) : undefined,
  };
}

export async function memoryPendingReject(id: string): Promise<{
  id: string;
  pending?: PendingMemory;
  error?: string;
}> {
  return DesktopIpcGen.memoryPendingReject(id);
}

export async function memoryHitList(input: {
  sessionId: string;
  runId?: string;
  limit?: number;
}): Promise<{ hits: MemoryHitRecord[] }> {
  const hits = await DesktopIpcGen.memoryHitList<RustMemoryHitRecord[]>(
    input.sessionId,
    input.runId,
    input.limit ?? 100,
  );
  return { hits: hits.map(toUiMemoryHit) };
}

interface RustSessionSummary {
  sessionId?: string;
  session_id?: string;
  startedAt?: number;
  started_at?: number;
  endedAt?: number | null;
  ended_at?: number | null;
  keyDecisions?: string[];
  key_decisions?: string[];
  filesModified?: string[];
  files_modified?: string[];
  toolsUsed?: string[];
  tools_used?: string[];
  summary?: string | null;
  tokenCount?: number | null;
  token_count?: number | null;
}

function toUiSessionSummary(summary: RustSessionSummary): SessionSummary {
  return {
    sessionId: summary.sessionId ?? summary.session_id ?? '',
    startedAt: Number(summary.startedAt ?? summary.started_at ?? 0),
    endedAt: summary.endedAt ?? summary.ended_at ?? null,
    keyDecisions: summary.keyDecisions ?? summary.key_decisions ?? [],
    filesModified: summary.filesModified ?? summary.files_modified ?? [],
    toolsUsed: summary.toolsUsed ?? summary.tools_used ?? [],
    summary: summary.summary ?? '',
    tokenCount: summary.tokenCount ?? summary.token_count ?? null,
  };
}

export async function memorySessionSummaryGet(sessionId: string): Promise<{
  ok: boolean;
  summary?: SessionSummary;
  reason?: string;
}> {
  const result = await DesktopIpcGen.memorySessionSummaryGet<{
    ok: boolean;
    summary?: RustSessionSummary;
    reason?: string;
  }>(sessionId);
  return {
    ok: Boolean(result.ok),
    reason: result.reason,
    summary: result.summary ? toUiSessionSummary(result.summary) : undefined,
  };
}

export async function memorySessionSummariesList(limit = 20): Promise<{
  ok: boolean;
  summaries: SessionSummary[];
  count: number;
}> {
  const result = await DesktopIpcGen.memorySessionSummariesList<{
    ok: boolean;
    summaries?: RustSessionSummary[];
    count?: number;
  }>(limit);
  return {
    ok: Boolean(result.ok),
    count: Number(result.count ?? result.summaries?.length ?? 0),
    summaries: (result.summaries || []).map(toUiSessionSummary),
  };
}

export async function toolCatalog(): Promise<ToolCatalogPayload> {
  const tools = await DesktopIpcGen.toolCatalog<ToolCatalogEntry[]>();
  return { tools };
}

export async function toolExecute(input: {
  name: string;
  args?: Record<string, unknown>;
  cwd?: string;
  sessionId?: string;
}): Promise<ToolExecuteOutput> {
  const output = await DesktopIpcGen.toolExecute<ToolExecuteOutput>(
    input.name,
    input.args || {},
    input.cwd,
    input.sessionId,
  );
  return {
    ...output,
    isError: Boolean(output.isError ?? output.is_error),
  };
}

export async function permissionList(sessionId?: string): Promise<{ permissions: PendingPermission[] }> {
  const result = await DesktopIpcGen.permissionList<{ permissions?: unknown[] }>(sessionId);
  return {
    permissions: (result.permissions || [])
      .map((item) => toUiPendingPermission(item, sessionId))
      .filter((item): item is PendingPermission => Boolean(item)),
  };
}

export async function permissionRespond(input: {
  requestId: string;
  allowed: boolean;
  remember?: boolean;
  scope?: 'once' | 'session' | 'always' | string;
}): Promise<PermissionRespondResult> {
  const result = await invoke<PermissionRespondResult>('permission_respond', {
    requestId: input.requestId,
    allowed: input.allowed,
    remember: input.remember,
    scope: input.scope,
  });
  return {
    ...result,
    output: result.output
      ? {
          ...result.output,
          isError: Boolean(result.output.isError ?? result.output.is_error),
        }
      : undefined,
  };
}

export async function questionRespond(input: {
  requestId: string;
  answers: Array<{ id: string; selected?: string[]; text?: string }>;
}): Promise<{ ok: boolean; requestId?: string; sessionId?: string; error?: string }> {
  return DesktopIpcGen.questionRespond(input.requestId, input.answers);
}

export function pendingPermissionFromToolOutput(
  output: ToolExecuteOutput,
  fallbackSessionId?: string,
): PendingPermission | null {
  const data = output.data && typeof output.data === 'object' ? output.data as Record<string, unknown> : null;
  return toUiPendingPermission(data?.pendingPermission, fallbackSessionId);
}

export async function extensionsList(): Promise<ExtensionsListPayload> {
  return DesktopIpcGen.extensionsList();
}

export async function pluginSetEnabled(
  id: string,
  enabled: boolean,
): Promise<{ ok: boolean; id: string; enabled?: boolean; extensions?: ExtensionsListPayload; error?: string }> {
  return DesktopIpcGen.pluginSetEnabled(id, enabled);
}

export async function pluginInstall(
  payload: Record<string, unknown>,
): Promise<{ ok: boolean; id?: string; installType?: string; requiresManualInstall?: boolean; error?: string }> {
  return DesktopIpcGen.pluginInstall(payload);
}

export async function configGet(): Promise<Partial<AppConfig>> {
  return DesktopIpcGen.configGet();
}

export async function configInfo(): Promise<DesktopConfigInfo> {
  return DesktopIpcGen.configInfo();
}

export async function configUpdate(config: Partial<AppConfig>): Promise<Partial<AppConfig>> {
  return DesktopIpcGen.configUpdate(config);
}

export async function configCapabilities(): Promise<ConfigCapabilitiesPayload> {
  return DesktopIpcGen.configCapabilities();
}

export async function initModelsFetch(input: InitModelsFetchInput): Promise<InitModelsFetchResult> {
  return DesktopIpcGen.initModelsFetch(
    input.providerType,
    input.baseUrl,
    input.apiKey,
    input.protocol,
    input.timeoutMs,
  );
}

export async function initPostgresStatus(): Promise<InitPostgresStatusResult> {
  return DesktopIpcGen.initPostgresStatus();
}

export async function initPostgresStart(): Promise<InitPostgresStartResult> {
  return DesktopIpcGen.initPostgresStart();
}

/** 首启存储初始化：内置 PostgreSQL 数据目录是否已初始化及其当前阶段。 */
export interface StorageSetupStatus {
  /** 内置 PostgreSQL 是否已安装（已初始化数据目录）。安装器便携式模式会预先装好。 */
  embeddedInstalled?: boolean;
  embeddedPhase: string;
}

/** 首启存储初始化：查询当前存储初始化状态。 */
export async function storageGetSetup(): Promise<StorageSetupStatus> {
  return invoke<StorageSetupStatus>('storage_get_setup', {});
}

export async function initChannelPluginsList(): Promise<{ plugins: InitChannelPlugin[] }> {
  return DesktopIpcGen.initChannelPluginsList();
}

export async function initChannelPluginInstall(pluginId: string): Promise<{
  ok: boolean;
  plugin?: InitChannelPlugin;
  error?: string;
}> {
  return DesktopIpcGen.initChannelPluginInstall(pluginId);
}

export interface InitChannelPlugin {
  id: string;
  name: string;
  version: string;
  description?: string;
  kind: string;
  channelType?: string;
  summary?: string;
  installed: boolean;
  installedPath?: string;
}

export async function systemStatus(): Promise<SystemStatusPayload> {
  return DesktopIpcGen.systemStatus();
}

export async function gatewayLogs(limit = 200): Promise<{ entries: GatewayLogEntry[] }> {
  const entries = await DesktopIpcGen.gatewayLogs<GatewayLogEntry[]>(limit);
  return { entries };
}

export interface GovernanceAuditEvent {
  id: string;
  actor: string;
  action: string;
  resource?: string;
  decision: string;
  reason: string;
  traceId?: string;
  createdAt: string;
}

export interface GovernanceRollbackAction {
  id: string;
  description: string;
  kind: string;
  payload: Record<string, unknown>;
  createdAt: string;
}

export async function auditList(limit = 200): Promise<{
  events: GovernanceAuditEvent[];
  rollbackActions: GovernanceRollbackAction[];
}> {
  return DesktopIpcGen.auditList(limit);
}

export async function rollbackList(): Promise<{ actions: GovernanceRollbackAction[] }> {
  return DesktopIpcGen.rollbackList();
}

export async function rollbackRun(rollbackId: string): Promise<Record<string, unknown>> {
  return DesktopIpcGen.rollbackRun(rollbackId);
}

export async function diagnosticsExport(limit = 200): Promise<DiagnosticsExportResult> {
  return DesktopIpcGen.diagnosticsExport(limit);
}

export async function dataBackupExport(): Promise<DataBackupExportResult> {
  return DesktopIpcGen.dataBackupExport();
}

export async function dataBackupRestore(backupPath: string): Promise<DataBackupRestoreResult> {
  return DesktopIpcGen.dataBackupRestore(backupPath);
}

export async function statsGet(): Promise<{ sessions: SessionStats[]; analytics: AnalyticsSnapshot }> {
  return DesktopIpcGen.statsGet();
}

export async function contextStatus(sessionId: string): Promise<ContextStatus> {
  return DesktopIpcGen.contextStatus(sessionId);
}

export async function contextCompact(
  sessionId: string,
  strategy?: string,
): Promise<{ sessionId: string; result: CompactResult; status: ContextStatus }> {
  return DesktopIpcGen.contextCompact(sessionId, strategy);
}

export async function contextRollback(
  sessionId: string,
): Promise<{
  sessionId: string;
  ok: boolean;
  reason?: string;
  snapshotVersion?: number;
  snapshot?: MemorySnapshotInfo | null;
  restoredCount?: number;
  status?: ContextStatus;
}> {
  return DesktopIpcGen.contextRollback(sessionId);
}

export async function runList(limit = 50): Promise<{ runs: RunRecord[] }> {
  return DesktopIpcGen.runList(limit);
}

export async function runTimeline(runId: string): Promise<{ timeline?: RunTimeline | null }> {
  return DesktopIpcGen.runTimeline(runId);
}

export async function traceGet(input: { runId?: string; traceId?: string }): Promise<{ trace?: TraceTimeline | null }> {
  const id = input.traceId || input.runId || '';
  return DesktopIpcGen.traceTimeline(id);
}

export async function runCancel(input: {
  runId: string;
  reason?: string;
}): Promise<{ ok: boolean; run?: RunRecord; runId?: string; error?: string }> {
  return DesktopIpcGen.runCancel(input.runId, input.reason);
}

export async function runResume(runId: string): Promise<{ ok: boolean; run?: RunRecord; runId?: string; error?: string }> {
  return DesktopIpcGen.runResume(runId);
}

export async function runReplay(runId: string): Promise<{ timeline?: RunTimeline | null; runId?: string; error?: string }> {
  return DesktopIpcGen.runReplay(runId);
}

export type EventReplayBatch = {
  lastSequence: number;
  nextSequence: number;
  overflow: boolean;
  events: Array<{
    sequence: number;
    timestamp: string;
    kind: string;
    payload: unknown;
  }>;
};

export async function eventReplay(lastSequence: number): Promise<EventReplayBatch> {
  return DesktopIpcGen.eventReplay(lastSequence);
}

export async function eventReplayFiltered(
  lastSequence: number,
  filter: { sessionId?: string; turnId?: string; kinds?: string[] },
): Promise<EventReplayBatch> {
  return invoke<EventReplayBatch>('event_replay_filtered', {
    lastSequence,
    filter: {
      sessionId: filter.sessionId,
      turnId: filter.turnId,
      kinds: filter.kinds ?? [],
    },
  });
}

export async function eventDeadLetters(limit = 64): Promise<unknown[]> {
  return DesktopIpcGen.eventDeadLetters(limit);
}

export async function controlPlaneListTasks(input?: {
  sessionId?: string;
  limit?: number;
}): Promise<unknown[]> {
  return DesktopIpcGen.controlPlaneListTasks(input?.sessionId, input?.limit ?? 64);
}

export async function traceEvidence(input: {
  runId: string;
  format?: 'json' | 'markdown';
}): Promise<{ runId: string; format: string; evidence?: EvidencePack | null; markdown?: string | null }> {
  return DesktopIpcGen.traceEvidence(input.runId, input.format);
}

export async function traceCompare(input: {
  baseRunId: string;
  targetRunId: string;
}): Promise<{ comparison?: AttemptComparison | null }> {
  return DesktopIpcGen.traceCompare(input.baseRunId, input.targetRunId);
}

export async function feedbackSubmit(input: {
  runId?: string;
  sessionId?: string;
  status: string;
  note?: string;
  tags?: string[];
}): Promise<{ feedback?: unknown; error?: string }> {
  return DesktopIpcGen.feedbackSubmit(input);
}

export async function processList(): Promise<{ processes: BackgroundProcessSnapshot[] }> {
  return DesktopIpcGen.processList();
}

export async function processRead(processId: string, offset = 0): Promise<{ processId: string; content: string; offset: number; process?: BackgroundProcessSnapshot }> {
  return DesktopIpcGen.processRead(processId, offset);
}

export async function processStop(processId: string): Promise<{ processId: string; content: string; offset: number; process?: BackgroundProcessSnapshot }> {
  return DesktopIpcGen.processStop(processId);
}

export async function gitStatus(sessionId?: string): Promise<{ status: GitStatusSnapshot }> {
  return DesktopIpcGen.gitStatus(sessionId);
}

export async function gitBranches(sessionId?: string): Promise<{
  branches: Array<{ name: string; current: boolean; upstream?: string }>;
}> {
  return DesktopIpcGen.gitBranches(sessionId);
}

export async function gitCheckout(
  branch: string,
  sessionId?: string,
): Promise<{
  ok?: boolean;
  branch?: string;
  status?: GitStatusSnapshot;
  error?: string;
}> {
  return DesktopIpcGen.gitCheckout(branch, sessionId);
}

export async function workspaceOpen(input: {
  path: string;
  sessionId?: string;
}): Promise<{ projectDir: string; projectName: string }> {
  return DesktopIpcGen.workspaceOpen(input.path, input.sessionId);
}

export async function workspaceClear(input: {
  sessionId: string;
}): Promise<{ sessionId: string }> {
  return DesktopIpcGen.workspaceClear(input.sessionId);
}

export async function workspaceCreate(input: {
  parentDir: string;
  name: string;
  sessionId?: string;
}): Promise<{ projectDir: string; projectName: string }> {
  return DesktopIpcGen.workspaceCreate(input.parentDir, input.name, input.sessionId);
}

export {
  channelAdaptersList,
  channelConnectionsList,
  channelConnectionUpsert,
  channelConnectionConnect,
  channelConnectionDisconnect,
  channelConnectionRemove,
  channelQrBegin,
  channelQrCancel,
  channelDeliveryList,
  agentList,
  agentCreate,
  agentUpdate,
  agentDelete,
  teamStatus,
} from './desktopIpc.gen';

export async function subagentTimeline(input: {
  sessionId?: string;
  taskId: string;
}): Promise<{
  sessionId?: string;
  taskId: string;
  ledger?: SubAgentTaskLedger | null;
  timeline: SubAgentTimelineEvent[];
}> {
  return DesktopIpcGen.subagentTimeline(input.taskId, input.sessionId);
}

export async function taskList(input: {
  sessionId?: string;
  limit?: number;
}): Promise<{ tasks: SubAgentTaskLedger[] }> {
  return DesktopIpcGen.taskList(input.sessionId, input.limit);
}

export async function taskCreate(input: {
  sessionId?: string;
  agentId: string;
  task: string;
  context?: string;
  writeScope?: string[];
  runId?: string;
  autoExecute?: boolean;
}): Promise<{ ok: boolean; sessionId?: string; task?: SubAgentTaskLedger; error?: string }> {
  const { sessionId, ...payload } = input;
  return DesktopIpcGen.taskCreate(payload, sessionId);
}

export async function subagentDelegateMany(input: {
  sessionId?: string;
  runId?: string;
  tasks: Array<Record<string, unknown>>;
  autoExecute?: boolean;
}): Promise<{
  ok: boolean;
  sessionId?: string;
  runId?: string;
  tasks?: SubAgentTaskLedger[];
  autoExecute?: boolean;
  reason?: string;
}> {
  return DesktopIpcGen.subagentDelegateMany({
    sessionId: input.sessionId,
    runId: input.runId,
    tasks: input.tasks,
    autoExecute: input.autoExecute,
  });
}

export async function subagentAssessMerge(input: {
  results: Array<Record<string, unknown>>;
}): Promise<{ ok: boolean; assessment?: unknown; reason?: string }> {
  return DesktopIpcGen.subagentAssessMerge({
    results: input.results,
  });
}

export async function taskStatus(taskId: string): Promise<{ task?: SubAgentTaskLedger | null }> {
  return DesktopIpcGen.taskStatus(taskId);
}

export async function taskControl(input: {
  taskId: string;
  action: 'pause' | 'resume' | 'cancel' | 'retry';
  reason?: string;
}): Promise<{ ok: boolean; taskId: string; action: string; reason?: string; task?: SubAgentTaskLedger | null }> {
  return DesktopIpcGen.taskControl(input.taskId, input.action, input.reason);
}



export async function subscribeDesktopCoreEvent<T>(
  eventName: 'xuanyu:event' | 'xuanyu:turn-delta' | 'xuanyu:log',
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(eventName, (event) => {
    handler(event.payload);
  });
}






export function toUiSession(session: RustSessionInfo): SessionInfo {
  const id = unwrapId(session.id);
  const createdAt = toMillis(session.createdAt ?? session.created_at);
  const updatedAt = toMillis(session.updatedAt ?? session.updated_at) || createdAt;
  const isProcessing = Boolean(session.isProcessing ?? session.is_processing);
  const messageCount = Number(session.messageCount ?? session.message_count ?? 0);
  const projectDir = (session.projectDir ?? session.project_dir)?.trim() || undefined;
  return {
    id,
    name: session.name,
    createdAt,
    updatedAt,
    // 空会话不占侧栏：需有消息或真实 lastMessageAt
    lastMessageAt: messageCount > 0 ? updatedAt : undefined,
    messageCount,
    isProcessing,
    provider: 'desktop-rust',
    model: 'xuanyu-core',
    status: isProcessing ? 'active' : 'idle',
    storageBackend: session.storageBackend || session.storage_backend || 'postgres',
    titleStatus: session.titleStatus ?? session.title_status,
    titleGeneratedBy: session.titleGeneratedBy ?? session.title_generated_by,
    titleGenerationError: session.titleGenerationError ?? session.title_generation_error,
    titleProvider: session.titleProvider ?? session.title_provider,
    titleModel: session.titleModel ?? session.title_model,
    pinned: Boolean(session.pinned),
    projectDir,
    metadata: projectDir ? { projectDir, workspace: true } : undefined,
  };
}

function toUiMessages(messages: RustChatMessage[]): ChatMessage[] {
  type PendingTurn = {
    turnId?: string;
    segments: NonNullable<ChatMessage['segments']>;
    content: string;
    timestamp: number;
    usage?: ChatMessage['usage'];
    attachments?: ChatAttachment[];
  };

  const out: ChatMessage[] = [];
  let pending = null as PendingTurn | null;

  const flushPending = () => {
    if (!pending) return;
    if (pending.segments.length === 0 && !pending.content.trim()) {
      pending = null;
      return;
    }
    out.push({
      role: 'assistant',
      content: pending.content,
      timestamp: pending.timestamp,
      turnId: pending.turnId,
      segments: pending.segments.length > 0 ? pending.segments : undefined,
      usage: pending.usage,
      attachments: pending.attachments,
    });
    pending = null;
  };

  const ensurePending = (turnId: string | undefined, timestamp: number): PendingTurn => {
    if (!pending || (turnId && pending.turnId && pending.turnId !== turnId)) {
      flushPending();
      pending = { turnId, segments: [], content: '', timestamp };
    } else if (!pending) {
      pending = { turnId, segments: [], content: '', timestamp };
    }
    return pending;
  };

  for (const message of messages) {
    const turnId = unwrapId(message.turnId ?? message.turn_id) || undefined;
    const timestamp = toMillis(message.timestamp);
    const metadata = message.metadata || undefined;

    if (message.role === 'user' || message.role === 'system') {
      flushPending();
      out.push({
        role: message.role,
        content: message.content,
        timestamp,
        turnId,
        attachments:
          message.attachments && message.attachments.length > 0 ? message.attachments : undefined,
      });
      continue;
    }

    if (message.role === 'tool') {
      // 工具结果已在 segments.result；不再用 role=tool 行拼时间线。
      continue;
    }

    // assistant
    const bucket = ensurePending(turnId, timestamp);
    const usage = readUsageFromMetadata(metadata);
    if (usage) bucket.usage = usage;
    if (message.attachments?.length) bucket.attachments = message.attachments;

    const serverSegments = coerceHistorySegments(message.segments);
    if (serverSegments.length > 0) {
      for (const seg of serverSegments) {
        if (
          isToolLikeHistorySeg(seg) &&
          bucket.segments.some((existing) => isToolLikeHistorySeg(existing) && existing.id === seg.id)
        ) {
          continue;
        }
        if (
          seg.type === 'text' &&
          bucket.segments.some(
            (existing) =>
              existing.type === 'text' &&
              existing.content === seg.content &&
              (existing.thinking || '') === (seg.thinking || ''),
          )
        ) {
          continue;
        }
        bucket.segments.push(seg);
      }
      const textSeg = [...serverSegments].reverse().find((seg) => seg.type === 'text' && seg.content?.trim());
      if (textSeg && textSeg.type === 'text' && textSeg.content?.trim()) {
        bucket.content = textSeg.content;
        bucket.timestamp = timestamp;
      } else if (message.content.trim()) {
        bucket.content = message.content;
        bucket.timestamp = timestamp;
      }
      continue;
    }

    // 无 segments 时仅落正文（不从 metadata.toolCalls 拼装）。
    if (message.content.trim()) {
      bucket.content = message.content;
      bucket.segments.push({
        type: 'text',
        id: `seg_${timestamp}_${bucket.segments.length}`,
        content: message.content,
      });
      bucket.timestamp = timestamp;
    }
  }

  flushPending();
  return out;
}

function isToolLikeHistorySeg(
  seg: NonNullable<ChatMessage['segments']>[number],
): seg is Extract<NonNullable<ChatMessage['segments']>[number], { type: 'tool_call' | 'sandbox_event' }> {
  return seg.type === 'tool_call' || seg.type === 'sandbox_event';
}

export function coerceHistorySegments(
  raw: Array<Record<string, unknown>> | undefined,
): NonNullable<ChatMessage['segments']> {
  if (!Array.isArray(raw) || raw.length === 0) return [];
  const out: NonNullable<ChatMessage['segments']> = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const type = String(item.type || '');
    const id = String(item.id || '');
    if (!type || !id) continue;
    if (type === 'text') {
      out.push({
        type: 'text',
        id,
        content: typeof item.content === 'string' ? item.content : '',
        thinking: typeof item.thinking === 'string' ? item.thinking : undefined,
      });
      continue;
    }
    if (type === 'tool_call' || type === 'sandbox_event') {
      const args =
        item.args && typeof item.args === 'object' && !Array.isArray(item.args)
          ? (item.args as Record<string, unknown>)
          : {};
      out.push({
        type,
        id,
        name: typeof item.name === 'string' ? item.name : 'unknown',
        args,
        rawArgs: typeof item.rawArgs === 'string' ? item.rawArgs : JSON.stringify(args),
        status: (item.status === 'calling' || item.status === 'error' || item.status === 'done'
          ? item.status
          : 'done') as 'calling' | 'done' | 'error',
        result: typeof item.result === 'string' ? item.result : undefined,
        content: typeof item.content === 'string' ? item.content : undefined,
      });
      continue;
    }
    if (type === 'agent_run') {
      const status =
        item.status === 'running' || item.status === 'completed' || item.status === 'failed' || item.status === 'cancelled'
          ? item.status
          : 'running';
      out.push({
        type: 'agent_run',
        id,
        parentToolCallId: typeof item.parentToolCallId === 'string' ? item.parentToolCallId : undefined,
        taskId: typeof item.taskId === 'string' ? item.taskId : undefined,
        agentId: typeof item.agentId === 'string' ? item.agentId : '',
        agentName: typeof item.agentName === 'string' ? item.agentName : '',
        task: typeof item.task === 'string' ? item.task : '',
        status,
        children: coerceHistorySegments(
          Array.isArray(item.children) ? (item.children as Array<Record<string, unknown>>) : undefined,
        ),
        summary: typeof item.summary === 'string' ? item.summary : undefined,
        startedAt: typeof item.startedAt === 'number' ? item.startedAt : 0,
        endedAt: typeof item.endedAt === 'number' ? item.endedAt : undefined,
        readScope:
          Array.isArray(item.readScope) && item.readScope.every((v) => typeof v === 'string')
            ? (item.readScope as string[])
            : undefined,
        writeScope:
          Array.isArray(item.writeScope) && item.writeScope.every((v) => typeof v === 'string')
            ? (item.writeScope as string[])
            : undefined,
        background: typeof item.background === 'boolean' ? item.background : undefined,
      });
      continue;
    }
    if (type === 'agent_delegate') {
      const status =
        item.status === 'calling' || item.status === 'done' || item.status === 'error' ||
        item.status === 'pending' || item.status === 'cancelled'
          ? item.status
          : 'done';
      out.push({
        type: 'agent_delegate',
        id,
        name: typeof item.name === 'string' ? item.name : 'unknown',
        content: typeof item.content === 'string' ? item.content : undefined,
        args:
          item.args && typeof item.args === 'object' && !Array.isArray(item.args)
            ? (item.args as Record<string, unknown>)
            : undefined,
        rawArgs: typeof item.rawArgs === 'string' ? item.rawArgs : undefined,
        status,
        result: typeof item.result === 'string' ? item.result : undefined,
        title: typeof item.title === 'string' ? item.title : undefined,
        payload:
          item.payload && typeof item.payload === 'object' && !Array.isArray(item.payload)
            ? (item.payload as Record<string, unknown>)
            : undefined,
      });
      continue;
    }
    if (type === 'permission') {
      out.push({
        type: 'permission',
        id,
        status: typeof item.status === 'string' ? item.status : undefined,
        title: typeof item.title === 'string' ? item.title : undefined,
        payload:
          item.payload && typeof item.payload === 'object' && !Array.isArray(item.payload)
            ? (item.payload as Record<string, unknown>)
            : undefined,
      });
      continue;
    }
  }
  return out;
}

function readUsageFromMetadata(
  metadata?: Record<string, unknown>,
): ChatMessage['usage'] | undefined {
  const raw = metadata?.usage;
  if (!raw || typeof raw !== 'object') return undefined;
  const usage = raw as Record<string, unknown>;
  const promptTokens = Number(usage.promptTokens ?? usage.prompt_tokens ?? 0);
  const completionTokens = Number(usage.completionTokens ?? usage.completion_tokens ?? 0);
  const totalTokens = Number(usage.totalTokens ?? usage.total_tokens ?? 0);
  if (!promptTokens && !completionTokens && !totalTokens) return undefined;
  return {
    promptTokens,
    completionTokens,
    totalTokens: totalTokens || promptTokens + completionTokens,
    billedPromptTokens:
      usage.billedPromptTokens != null || usage.billed_prompt_tokens != null
        ? Number(usage.billedPromptTokens ?? usage.billed_prompt_tokens)
        : undefined,
    lastCompletionTokens:
      usage.lastCompletionTokens != null || usage.last_completion_tokens != null
        ? Number(usage.lastCompletionTokens ?? usage.last_completion_tokens)
        : undefined,
  };
}

function toUiMessage(message: RustChatMessage): ChatMessage | null {
  if (message.role === 'tool') return null;
  const converted = toUiMessages([message]);
  return converted[0] ?? null;
}

function toUiMemory(memory: RustMemoryItem): MemoryItem {
  const createdAt = toMillis(memory.createdAt ?? memory.created_at);
  const updatedAt = toMillis(memory.updatedAt ?? memory.updated_at) || createdAt;
  const lastAccessedAt = memory.lastAccessedAt ?? memory.last_accessed_at;
  return {
    id: memory.id,
    content: memory.content,
    category: memory.category,
    importance: memory.importance,
    createdAt,
    updatedAt,
    tags: memory.tags || [],
    source: memory.source || 'rust-memory',
    accessCount: Number(memory.accessCount ?? memory.access_count ?? 0),
    lastAccessedAt: lastAccessedAt ? toMillis(lastAccessedAt) : undefined,
  };
}

function toUiMemorySnapshot(snapshot: RustMemorySnapshotInfo): MemorySnapshotInfo {
  return {
    version: Number(snapshot.version),
    timestamp: toMillis(snapshot.timestamp),
    entryCount: Number(snapshot.entryCount ?? snapshot.entry_count ?? 0),
    reason: snapshot.reason || 'manual_snapshot',
  };
}

function toUiMemoryHit(hit: RustMemoryHitRecord): MemoryHitRecord {
  return {
    id: hit.id,
    sessionId: hit.sessionId ?? hit.session_id ?? '',
    runId: hit.runId ?? hit.run_id ?? undefined,
    turnId: hit.turnId ?? hit.turn_id ?? undefined,
    query: hit.query ?? undefined,
    source: hit.source === 'system' ? 'system' : 'query',
    memoryId: hit.memoryId ?? hit.memory_id ?? '',
    score: hit.score ?? undefined,
    category: hit.category ?? undefined,
    injected: Boolean(hit.injected),
    createdAt: toMillis(hit.createdAt ?? hit.created_at),
  };
}

function toUiPendingPermission(raw: unknown, fallbackSessionId?: string): PendingPermission | null {
  if (!raw || typeof raw !== 'object') return null;
  const item = raw as Record<string, unknown>;
  const requestId = typeof item.requestId === 'string'
    ? item.requestId
    : typeof item.request_id === 'string'
      ? item.request_id
      : '';
  const toolName = typeof item.toolName === 'string'
    ? item.toolName
    : typeof item.tool_name === 'string'
      ? item.tool_name
      : '';
  if (!requestId || !toolName) return null;
  const sessionId = typeof item.sessionId === 'string'
    ? item.sessionId
    : typeof item.session_id === 'string'
      ? item.session_id
      : fallbackSessionId || '';
  const args = item.args && typeof item.args === 'object'
    ? item.args as Record<string, unknown>
    : {};
  return {
    sessionId,
    requestId,
    toolName,
    args,
    reason: typeof item.reason === 'string' ? item.reason : 'tool_requires_approval',
    approvalDigest: typeof item.approvalDigest === 'string'
      ? item.approvalDigest
      : typeof item.approval_digest === 'string'
        ? item.approval_digest
        : '',
  };
}

function unwrapId(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object' && typeof (value as { 0?: unknown })[0] === 'string') {
    return (value as { 0: string })[0];
  }
  return '';
}

function toMillis(value: string | number | undefined): number {
  if (typeof value === 'number') return value;
  if (typeof value === 'string') {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return Date.now();
}
