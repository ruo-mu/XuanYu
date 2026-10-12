import { useCallback, useEffect, useRef, useState, type MutableRefObject } from 'react';
import { message } from 'antd';

import type {
  AgentDefinitionInput,
  AgentEvent,
  ChatSendPayload,
  CompactResult,
  ConfigCapabilitiesPayload,
  ContextBudgetStatus,
  ContextStatus,
  ExternalGitIntegrationStatus,
  ExternalTaskDraft,
  ExtensionsListPayload,
  GatewayLogEntry,
  GitStatusSnapshot,
  LocalPullRequest,
  MemoryItem,
  MemoryProfileFile,
  MemorySnapshotInfo,
  SessionSummary,
  PendingMemory,
  PermissionRequestPayload,
  NestedTaskSnapshot,
  NestedTaskEvent,
  NestedTaskResult,
  RunPlan,
  SessionInfo,
  SessionStats,
  SubAgentTaskLedger,
  SubAgentTimelineEvent,
  SubAgentTeamSnapshot,
  ControlTaskEvent,
  ControlTaskResult,
  SystemStatusPayload,
  AnalyticsSnapshot,
  PeriodTotals,
  ToolCatalogPayload,
  WsMessage,
} from '../protocol';
import type { AppConfig } from '../stores/configStore';
import { useChatStore, COMPOSE_DRAFT_SESSION_ID, type PendingPermission } from '../stores/chatStore';
import { useConfigStore } from '../stores/configStore';
import { normalizeFsPath } from '../foundation/config/paths';
import { useSessionStore } from '../stores/sessionStore';
import {
  appInit,
  agentCreate,
  agentDelete,
  agentList,
  agentUpdate,
  auditList,
  channelAdaptersList,
  channelConnectionConnect,
  channelConnectionDisconnect,
  channelConnectionRemove,
  channelConnectionUpsert,
  channelConnectionsList,
  channelDeliveryList,
  channelQrBegin,
  channelQrCancel,
  coerceHistorySegments,
  configCapabilities,
  configGet,
  configUpdate,
  contextCompact,
  contextRollback,
  contextStatus,
  conversationSearch,
  eventReplay,
  eventReplayFiltered,
  extensionsList,
  feedbackSubmit,
  gatewayLogs,
  gitStatus,
  gitBranches,
  gitCheckout,
  isDesktopIpcAvailable,
  memoryCreate,
  memoryDelete,
  memoryFlush,
  memoryHitList,
  memoryMaintenanceRun,
  memoryList,
  memorySessionSummariesList,
  memorySessionSummaryGet,
  memoryPendingApprove,
  memoryPendingList,
  memoryPendingReject,
  memoryProfileList,
  memoryProfileRead,
  memoryProfileUpdate,
  memorySearch,
  memorySnapshotCreate,
  memorySnapshotList,
  memorySnapshotRollback,
  memoryUpdate,
  pendingPermissionFromToolOutput,
  permissionRespond,
  questionRespond,
  capabilityInstalledList,
  capabilityInstalledMutate,
  goalAction,
  goalGet,
  marketplaceInstallConfirm,
  marketplaceInstallPlan,
  marketplaceInstallStatus,
  marketplaceSearch,
  pluginInstall,
  pluginSetEnabled,
  sessionArchive,
  sessionAbort,
  sessionCreate,
  sessionDelete,
  sessionHistory,
  sessionList,
  sessionPin,
  sessionRename,
  sessionResetContext,
  sessionRewind,
  sessionRewindPreview,
  sessionSendMessage,
  workspaceFileReject,
  workspaceOpen,
  workspaceClear,
  workspaceCreate,
  workspacePickFolder,
  workspaceReveal,
  processList,
  processRead,
  processStop,
  runCancel,
  runList,
  runReplay,
  runResume,
  runTimeline,
  statsGet,
  subagentTimeline,
  subagentDelegateMany,
  subagentAssessMerge,
  subscribeDesktopCoreEvent,
  systemStatus,
  taskControl,
  taskCreate,
  taskList,
  taskStatus,
  teamStatus,
  toolCatalog,
  toolExecute,
  toUiSession,
  traceCompare,
  traceEvidence,
  traceGet,
} from '../ipc/desktopIpc';
import { getProviderCatalog } from '../init/initApi';
import type { UnlistenFn } from '../ipc/ipcTypes';

import {
  sendDesktopCommand,
  mergeConfig,
  createConfiguredCapabilities,
  getPayloadSessionId,
  defaultConfig,
  defaultToolCatalog,
  defaultExtensions,
} from './desktopCommandHandlers';
import { getMainSessionId, setMainSessionIdValue } from './desktopSessionState';

type MessageHandler = (msg: WsMessage) => void;
type TypedMessageHandler = (payload: unknown, msg: WsMessage) => void;
type EmitOptions = { buffer?: boolean };
type ReplayBufferItem = { sequence: number; timestamp: number; message: WsMessage };

export interface AddHandler {
  (handler: MessageHandler): () => void;
  (type: string, handler: TypedMessageHandler): () => void;
}

export interface WebSocketConnectionOptions {
  enabled?: boolean;
  gatewayUrl?: string;
  token?: string;
}

type TeamSetter = (team: SubAgentTeamSnapshot | null) => void;

let setTeamSnapshotGlobal: TeamSetter | null = null;
const REPLAY_BUFFER_LIMIT = 500;
const REPLAY_TTL_MS = 60_000;

export function registerTeamSnapshotSetter(setter: TeamSetter | null): void {
  setTeamSnapshotGlobal = setter;
}

export { getMainSessionId } from './desktopSessionState';


export function useWebSocket(options: WebSocketConnectionOptions = {}) {
  const [connected, setConnected] = useState(false);
  const [mainSessionIdState, setMainSessionIdState] = useState('');
  const handlersRef = useRef<Set<MessageHandler>>(new Set());
  const timersRef = useRef<number[]>([]);
  const desktopUnlistenRef = useRef<UnlistenFn[]>([]);
  const desktopEventCtxRef = useRef<{
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
  } | null>(null);
  const replayBufferRef = useRef<ReplayBufferItem[]>([]);
  const nextSequenceRef = useRef(1);
  const sendRef = useRef<(type: string, payload: unknown) => void>(() => {});
  const seenMessageIdsRef = useRef<Map<string, number>>(new Map());

  const {
    applyEvent,
    initSession,
    setActiveSession,
    syncProcessingState,
    addSystemMessage,
    addErrorMessage,
    addAgentReply,
    setPendingPermission,
    setPendingQuestion,
    loadMessages,
    rollbackOptimisticMessage,
    upsertSubAgentLedger,
    setSubAgentTimeline,
    upsertTaskSnapshot,
    addTaskEvent,
    addTaskResult,
    clearHistory,
  } = useChatStore();
  const { addSession, setSessions, updateSession, removeSession, setStats, setAnalytics } = useSessionStore();
  const { setConfig, setExtensions, setCapabilities, setToolCatalog } =
    useConfigStore();

  const handleMessage = useCallback(
    (msg: WsMessage) => {
      switch (msg.type) {
        case 'agent.event': {
          const { sessionId, event } = msg.payload as { sessionId: string; event: AgentEvent };
          applyEvent(sessionId, event);
          break;
        }
        case 'agent.message': {
          const p = msg.payload as { sessionId: string; userMessage?: string; content: string };
          if (p.sessionId && p.content) {
            addAgentReply(p.sessionId, p.userMessage || '', p.content);
            useSessionStore.getState().updateSession(p.sessionId, {
              isProcessing: false,
              status: 'idle',
              updatedAt: Date.now(),
              lastMessageAt: Date.now(),
            });
          }
          break;
        }
        case 'permission.request': {
          const p = msg.payload as PermissionRequestPayload;
          setPendingPermission(p.sessionId, p);
          break;
        }
        case 'question.request': {
          const p = msg.payload as {
            sessionId: string;
            requestId: string;
            questions: Array<{
              id: string;
              prompt: string;
              options?: string[];
              allow_multiple?: boolean;
            }>;
            runId?: string;
          };
          if (p.sessionId && p.requestId) setPendingQuestion(p.sessionId, p);
          break;
        }
        case 'question.answered': {
          const p = msg.payload as { sessionId?: string; requestId?: string };
          if (p.sessionId) setPendingQuestion(p.sessionId, null);
          break;
        }
        case 'session.created': {
          const { sessionId, name, session, mode } = msg.payload as {
            sessionId: string;
            name: string;
            session?: SessionInfo;
            mode?: string;
          };
          const isMain = !mode || mode === 'main';
          if (isMain) {
            setMainSessionIdValue(sessionId);
            setMainSessionIdState(sessionId);
          }
          const base =
            session ||
            ({
              id: sessionId,
              name,
              createdAt: Date.now(),
              messageCount: 0,
              isProcessing: false,
            } as SessionInfo);
          const nextSession: SessionInfo =
            mode === 'recent'
              ? {
                  ...base,
                  id: sessionId,
                  name: base.name || name,
                  projectDir: undefined,
                  metadata: (() => {
                    const meta = { ...(base.metadata || {}) };
                    delete meta.projectDir;
                    delete meta.workspace;
                    return meta;
                  })(),
                }
              : base;
          addSession(nextSession);
          initSession(sessionId);
          const currentActive = useChatStore.getState().activeSessionId;
          const onComposeDraft = currentActive === COMPOSE_DRAFT_SESSION_ID;
          // 启动用的空 main 不当成当前对话；否则首条消息会打到不存在/空壳会话上。
          if (isMain) {
            if (!currentActive || currentActive === sessionId) {
              useChatStore.getState().beginComposeDraft({ kind: 'recent' });
            }
          } else if (mode === 'branch' || mode === 'recent' || (!currentActive && !onComposeDraft)) {
            setActiveSession(sessionId);
            // 新建空会话不要立刻 restore：异步空历史会盖掉首条乐观用户消息。
            // 侧栏点进已有会话仍会自行 session.restore。
            if ((nextSession.messageCount || 0) > 0) {
              sendRef.current('session.restore', { sessionId });
            }
            sendRef.current('team.status', { sessionId });
            sendRef.current('context.status', { sessionId });
          }
          sendRef.current('stats.get', {});
          if (isMain) sendRef.current('session.list', { limit: 200 });
          break;
        }
        case 'session.deleted': {
          const { sessionId } = msg.payload as { sessionId: string };
          if (sessionId) removeSession(sessionId);
          break;
        }
        case 'session.archived': {
          const { sessionId, archived } = msg.payload as { sessionId: string; archived?: boolean };
          if (sessionId)
            updateSession(sessionId, {
              archivedAt: archived ? Date.now() : undefined,
              status: archived ? 'archived' : 'idle',
              updatedAt: Date.now(),
            });
          break;
        }
        case 'session.renamed': {
          const { sessionId, session } = msg.payload as {
            sessionId: string;
            session?: SessionInfo;
          };
          if (!sessionId || !session) break;
          const existing = useSessionStore.getState().sessions.find((item) => item.id === sessionId);
          if (existing && !existing.projectDir && !existing.metadata?.projectDir && session.projectDir) {
            const { projectDir: _drop, ...rest } = session;
            const meta = { ...(rest.metadata || {}) };
            delete meta.projectDir;
            delete meta.workspace;
            updateSession(sessionId, { ...rest, projectDir: undefined, metadata: meta });
          } else {
            updateSession(sessionId, session);
          }
          break;
        }
        case 'session.updated': {
          const { session } = msg.payload as { session?: SessionInfo };
          if (!session?.id) break;
          const exists = useSessionStore.getState().sessions.some((item) => item.id === session.id);
          const existing = useSessionStore.getState().sessions.find((item) => item.id === session.id);
          let next = session;
          if (existing && !existing.projectDir && !existing.metadata?.projectDir && session.projectDir) {
            const { projectDir: _drop, ...rest } = session;
            const meta = { ...(rest.metadata || {}) };
            delete meta.projectDir;
            delete meta.workspace;
            next = { ...rest, projectDir: undefined, metadata: meta };
          }
          if (exists) updateSession(session.id, next);
          else addSession(next);
          initSession(session.id);
          // 本地乐观回合进行中时，勿被侧栏/改名等 SessionUpdated 的 isProcessing=false 清掉。
          if (!hasLocalInflightTurn(session.id) || session.isProcessing) {
            syncProcessingState(session.id, session.isProcessing);
          }
          break;
        }
        case 'session.message': {
          const payload = msg.payload as {
            sessionId?: string;
            message?: { role?: string; content?: string; timestamp?: number };
          };
          if (
            payload.sessionId &&
            payload.message?.role === 'user' &&
            typeof payload.message.content === 'string'
          ) {
            const sessionId = payload.sessionId;
            const content = payload.message.content;
            const timestamp = payload.message.timestamp || Date.now();
            initSession(sessionId);
            useChatStore.setState((s) => {
              const session = s.sessions[sessionId];
              if (!session) return s;
              const last = session.messages[session.messages.length - 1];
              const duplicate =
                last?.role === 'user' &&
                last.content === content &&
                Math.abs((last.timestamp || 0) - timestamp) < 5000;
              if (duplicate) return s;
              return {
                sessions: {
                  ...s.sessions,
                  [sessionId]: {
                    ...session,
                    messages: [...session.messages, { role: 'user', content, timestamp }],
                  },
                },
              };
            });
          }
          break;
        }
        case 'session.title.status': {
          const p = msg.payload as {
            sessionId: string;
            status?: SessionInfo['titleStatus'];
            generatedBy?: string;
            error?: string;
          };
          if (p.sessionId)
            updateSession(p.sessionId, {
              titleStatus: p.status,
              titleGeneratedBy: p.generatedBy,
              titleGenerationError: p.error,
              updatedAt: Date.now(),
            });
          break;
        }
        case 'context.reset': {
          const { sessionId } = msg.payload as { sessionId: string };
          if (sessionId) clearHistory(sessionId);
          message.success(
            localeText(
              '上下文已清空，旧对话已转为记忆。',
              'Context cleared, old conversation converted to memory.',
            ),
          );
          break;
        }
        case 'chat.command.result': {
          const { sessionId, content } = msg.payload as { sessionId: string; content: string };
          if (sessionId && content) addSystemMessage(sessionId, content);
          break;
        }
        case 'session.list': {
          const { sessions } = msg.payload as { sessions: SessionInfo[] };
          setSessions(sessions);
          sessions.forEach((s) => {
            initSession(s.id);
            // 列表刷新常带陈旧 isProcessing；本地已有进行中回合时不要覆盖。
            if (!hasLocalInflightTurn(s.id) || s.isProcessing) {
              syncProcessingState(s.id, s.isProcessing);
            }
          });
          break;
        }
        case 'session.history': {
          const { sessionId, messages } = msg.payload as {
            sessionId: string;
            messages: Array<{
              role: string;
              content: string;
              timestamp?: number;
              turnId?: string;
              thinking?: string;
              usage?: import('../protocol').TokenUsage;
              metadata?: Record<string, unknown>;
              attachments?: Array<{
                kind: 'image' | 'file';
                name: string;
                mimeType: string;
                sizeBytes: number;
                dataUrl: string;
              }>;
              segments?: Array<Record<string, unknown>>;
              toolCalls?: Array<{
                id?: string;
                name?: string;
                call?: { id: string; name: string; arguments: string };
                function?: { name?: string; arguments?: string | Record<string, unknown> };
                args?: Record<string, unknown>;
                rawArgs?: string;
                status?: string;
                result?: string;
              }>;
            }>;
          };
          if (sessionId && messages) {
            initSession(sessionId);
            const chatMessages = messages
              .filter((m) => m.role !== 'tool')
              .map((m) => {
                const meta = (m.metadata || {}) as Record<string, unknown>;
                const segments = Array.isArray(m.segments)
                  ? coerceHistorySegments(m.segments)
                  : undefined;
                const usage =
                  m.usage ||
                  (meta.usage as import('../protocol').TokenUsage | undefined) ||
                  undefined;
                const hist = m as { workStartedAt?: number; workEndedAt?: number };
                return {
                  role: m.role as 'user' | 'assistant' | 'system',
                  content: m.content,
                  timestamp: m.timestamp || Date.now(),
                  turnId: m.turnId,
                  usage,
                  workStartedAt: hist.workStartedAt,
                  workEndedAt: hist.workEndedAt ?? m.timestamp,
                  attachments: Array.isArray(m.attachments) ? m.attachments : undefined,
                  segments,
                };
              });
            loadMessages(sessionId, chatMessages);
            const local = useChatStore.getState().sessions[sessionId];
            const serverSession = useSessionStore.getState().sessions.find((s) => s.id === sessionId);
            // 本地已在跑回合（乐观用户消息）时，勿被空历史把 isProcessing 打回 false。
            const nextProcessing =
              local?.isProcessing && local.pendingMessageIndex !== null
                ? true
                : (serverSession?.isProcessing ?? false);
            syncProcessingState(sessionId, nextProcessing);
          }
          break;
        }
        case 'config.current':
        case 'config.updated': {
          const { config } = msg.payload as { config: AppConfig };
          setConfig(config);
          sendRef.current('tools.catalog.get', {});
          break;
        }
        case 'config.capabilities': {
          const p = msg.payload as ConfigCapabilitiesPayload;
          setCapabilities(p.models || []);
          break;
        }
        case 'tools.catalog': {
          const p = msg.payload as ToolCatalogPayload;
          setToolCatalog(p.tools || []);
          break;
        }
        case 'extensions.list': {
          setExtensions(msg.payload as ExtensionsListPayload);
          break;
        }
        case 'team.status': {
          const { sessionId, team } = msg.payload as {
            sessionId?: string;
            team: SubAgentTeamSnapshot;
          };
          const active = useChatStore.getState().activeSessionId;
          if (!sessionId || !active || sessionId === active) setTeamSnapshotGlobal?.(team || null);
          if (sessionId) {
            useSessionStore.getState().updateSession(sessionId, {
              status: team?.runningCount && team.runningCount > 0 ? 'active' : 'idle',
              updatedAt: Date.now(),
            });
            if (team?.recentTasks)
              for (const t of team.recentTasks) upsertSubAgentLedger(sessionId, t);
          }
          break;
        }
        case 'team.agent.started':
        case 'team.agent.progress':
        case 'team.agent.completed':
        case 'team.agent.failed': {
          const { sessionId, team, ledger } = msg.payload as {
            sessionId?: string;
            team?: SubAgentTeamSnapshot;
            ledger?: SubAgentTaskLedger;
          };
          if (
            team &&
            (!sessionId ||
              !useChatStore.getState().activeSessionId ||
              sessionId === useChatStore.getState().activeSessionId)
          )
            setTeamSnapshotGlobal?.(team);
          if (sessionId && ledger) upsertSubAgentLedger(sessionId, ledger);
          if (team?.recentTasks && sessionId)
            for (const t of team.recentTasks) upsertSubAgentLedger(sessionId, t);
          break;
        }
        case 'subagent.timeline': {
          const p = msg.payload as {
            sessionId?: string;
            taskId: string;
            ledger?: SubAgentTaskLedger;
            timeline?: SubAgentTimelineEvent[];
          };
          if (p.sessionId && p.taskId)
            setSubAgentTimeline(p.sessionId, p.taskId, p.timeline || [], p.ledger);
          break;
        }
        case 'agent.nested.event': {
          const p = msg.payload as {
            sessionId?: string;
            taskId?: string;
            ledger?: SubAgentTaskLedger;
            parentToolCallId?: string;
            team?: SubAgentTeamSnapshot;
          };
          const sid = p.sessionId || useChatStore.getState().activeSessionId;
          if (sid && p.ledger) upsertSubAgentLedger(sid, p.ledger);
          if (p.team && (!sid || sid === useChatStore.getState().activeSessionId)) {
            setTeamSnapshotGlobal?.(p.team);
          }
          break;
        }
        case 'run.plan': {
          const p = msg.payload as { sessionId?: string; plan: RunPlan | null };
          const sid = p.sessionId || useChatStore.getState().activeSessionId;
          if (sid) useChatStore.getState().setRunPlan(sid, p.plan || null);
          break;
        }
        case 'task.snapshot': {
          const p = msg.payload as { sessionId?: string; task?: NestedTaskSnapshot };
          const sid = p.sessionId || p.task?.task?.sessionId;
          if (sid && p.task) upsertTaskSnapshot(sid, p.task);
          break;
        }
        case 'task.event': {
          const p = msg.payload as { sessionId: string; event: NestedTaskEvent };
          if (p.sessionId && p.event) addTaskEvent(p.sessionId, p.event);
          break;
        }
        case 'task.result': {
          const p = msg.payload as { sessionId: string; result: NestedTaskResult };
          if (p.sessionId && p.result) addTaskResult(p.sessionId, p.result);
          break;
        }
        case 'stats.current': {
          const p = msg.payload as { sessions: SessionStats[]; analytics?: AnalyticsSnapshot };
          setStats(p.sessions);
          setAnalytics(p.analytics || null);
          break;
        }
        case 'error': {
          const p = (msg.payload || {}) as GatewayErrorPayload;
          const text = formatGatewayError(p);
          if (p.sessionId) {
            addErrorMessage(p.sessionId, text);
            rollbackOptimisticMessage(p.sessionId);
            useSessionStore.getState().updateSession(p.sessionId, {
              isProcessing: false,
              status: 'idle',
              updatedAt: Date.now(),
            });
          } else message.error(text);
          break;
        }
        case 'replay.overflow': {
          const payload = (msg.payload || {}) as { nextSequence?: number };
          if (typeof payload.nextSequence === 'number' && payload.nextSequence > 0) {
            nextSequenceRef.current = Math.max(nextSequenceRef.current, payload.nextSequence);
          }
          requestDesktopCurrentState(
            sendRef.current,
            useChatStore.getState().activeSessionId || getMainSessionId(),
          );
          message.warning(
            localeText(
              '连接重放缓存已过期，正在刷新当前状态。',
              'Connection replay buffer expired; refreshing current state.',
            ),
          );
          break;
        }
      }
    },
    [
      applyEvent,
      setPendingPermission,
      setPendingQuestion,
      loadMessages,
      syncProcessingState,
      addAgentReply,
      addErrorMessage,
      addSystemMessage,
      upsertSubAgentLedger,
      setSubAgentTimeline,
      upsertTaskSnapshot,
      addTaskEvent,
      addTaskResult,
      rollbackOptimisticMessage,
      addSession,
      setSessions,
      setConfig,
      setExtensions,
      setCapabilities,
      setToolCatalog,
      setStats,
      setAnalytics,
      initSession,
      setActiveSession,
      clearHistory,
      removeSession,
      updateSession,
    ],
  );

  const dispatchMessage = useCallback(
    (msg: WsMessage) => {
      if (msg?.id && isDuplicateMessage(msg.id, seenMessageIdsRef.current)) return;
      handleMessage(msg);
      handlersRef.current.forEach((handler) => handler(msg));
    },
    [handleMessage],
  );

  const emit = useCallback(
    (type: string, payload: unknown, emitOptions: EmitOptions = {}) => {
      const shouldBuffer = emitOptions.buffer !== false;
      const sequence = shouldBuffer ? nextSequenceRef.current++ : undefined;
      const msg: WsMessage = {
        id: `desktop_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
        type,
        payload,
        timestamp: Date.now(),
        ...(sequence !== undefined ? { sequence } : {}),
      };
      if (shouldBuffer && sequence !== undefined) {
        replayBufferRef.current.push({ sequence, timestamp: msg.timestamp, message: msg });
        trimReplayBuffer(replayBufferRef.current);
      }
      if (type === 'replay.batch') {
        const batch = payload as { messages?: WsMessage[] };
        for (const replayed of (batch.messages || []).sort(
          (a, b) => (a.sequence || 0) - (b.sequence || 0),
        )) {
          dispatchMessage(replayed);
        }
        return;
      }
      dispatchMessage(msg);
    },
    [dispatchMessage],
  );

  desktopEventCtxRef.current = { emit };

  const ensureCoreEventSubscriptions = useCallback(async () => {
    const unsubs = await Promise.all([
      subscribeDesktopCoreEvent<DesktopCoreEnvelope>('xuanyu:event', (envelope) => {
        bufferCoreEnvelope(envelope, replayBufferRef.current);
        const ctx = desktopEventCtxRef.current;
        if (ctx) handleDesktopCoreEnvelope(envelope, ctx);
      }),
      subscribeDesktopCoreEvent<DesktopCoreEnvelope>('xuanyu:turn-delta', (envelope) => {
        bufferCoreEnvelope(envelope, replayBufferRef.current);
        const ctx = desktopEventCtxRef.current;
        if (ctx) handleDesktopCoreEnvelope(envelope, ctx);
      }),
      subscribeDesktopCoreEvent<DesktopCoreEnvelope>('xuanyu:log', (envelope) => {
        bufferCoreEnvelope(envelope, replayBufferRef.current);
        const ctx = desktopEventCtxRef.current;
        if (ctx) handleDesktopCoreEnvelope(envelope, ctx);
      }),
    ]);
    desktopUnlistenRef.current.forEach((unlisten) => void unlisten());
    desktopUnlistenRef.current = unsubs;
  }, []);

  const bootSession = useCallback(async (opts?: { reconnect?: boolean }) => {
    if (!isDesktopIpcAvailable()) {
      setConnected(false);
      message.error('Desktop IPC unavailable: Electron preload not loaded');
      throw new Error('Desktop IPC unavailable: Electron preload not loaded');
    }

    const reconnect = !!opts?.reconnect;
    try {
      // 先挂事件监听，再 init：避免 init 失败时 UI 能发消息却收不到 turn 事件而一直「正在思考」。
      await ensureCoreEventSubscriptions();
      setConnected(true);
      await appInit();
      const config = mergeConfig(
        defaultConfig,
        await configGet().catch(() => ({} as Partial<AppConfig>)),
      );
      const capabilities = await configCapabilities().catch(() =>
        createConfiguredCapabilities(config),
      );
      const catalog = await toolCatalog().catch(() => defaultToolCatalog);
      const extensions = await extensionsList().catch(() => defaultExtensions);
      emit('config.current', { config });
      emit('config.capabilities', capabilities);
      emit('tools.catalog', { ...catalog });
      emit('extensions.list', extensions);
      setConnected(true);

      if (reconnect) {
        // 引擎重启：只恢复配置与事件，不打断当前会话/草稿。
        const activeId = useChatStore.getState().activeSessionId;
        if (activeId && activeId !== COMPOSE_DRAFT_SESSION_ID) {
          sendRef.current('session.list', { limit: 200 });
          sendRef.current('team.status', { sessionId: activeId });
          sendRef.current('context.status', { sessionId: activeId });
        } else {
          sendRef.current('session.list', { limit: 200 });
        }
        message.success('引擎已恢复连接');
        return;
      }

      const session = await sessionCreate();
      const sessions = await sessionList().catch(() => [session]);
      const history = await sessionHistory(session.id).catch(() => []);
      setTeamSnapshotGlobal?.(null);
      emit('session.created', { sessionId: session.id, name: session.name, session });
      emit('session.history', { sessionId: session.id, messages: history });
      emit('session.list', { sessions: sessions.length ? sessions : [session] });
      emit('stats.current', { sessions: [], analytics: null });
    } catch (err) {
      setConnected(false);
      const detail = err instanceof Error ? err.message : String(err);
      message.error(`连接引擎失败：${detail}`);
      if (!reconnect) {
        useChatStore.getState().beginComposeDraft({ kind: 'recent' });
      }
      throw err;
    }
  }, [emit, ensureCoreEventSubscriptions]);

  const send = useCallback(
    (type: string, payload: unknown) => {
      const sessionId = getPayloadSessionId(payload) || getMainSessionId();
      void sendDesktopCommand(type, payload, {
        emit,
        sessionId,
        syncProcessingState,
        addSystemMessage,
        addErrorMessage,
        updateSession,
        setMainSessionId: setMainSessionIdState,
        rollbackOptimisticMessage,
        setPendingPermission,
        setPendingQuestion,
        upsertSubAgentLedger,
        setSubAgentTimeline,
        replayBuffer: replayBufferRef.current,
        nextSequence: () => nextSequenceRef.current,
      });
    },
    [
      addErrorMessage,
      addSystemMessage,
      emit,
      rollbackOptimisticMessage,
      setPendingPermission,
      setPendingQuestion,
      setSubAgentTimeline,
      syncProcessingState,
      updateSession,
      upsertSubAgentLedger,
    ],
  );

  useEffect(() => {
    sendRef.current = send;
  }, [send]);

  useEffect(() => {
    if (options.enabled === false) {
      setConnected(false);
      return () => undefined;
    }
    void bootSession().catch(() => undefined);
    return () => {
      desktopUnlistenRef.current.forEach((unlisten) => void unlisten());
      desktopUnlistenRef.current = [];
      timersRef.current.forEach((timer) => window.clearTimeout(timer));
      timersRef.current = [];
    };
    // 仅挂载时 boot；emit 变化不重跑，避免拆掉事件订阅。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.enabled]);

  // 引擎从异常恢复为 ready 时重新拉配置并确保事件订阅（含「重启引擎」按钮）。
  useEffect(() => {
    if (options.enabled === false) return () => undefined;
    let previous: string | null = null;
    const reconnect = () => {
      void bootSession({ reconnect: true }).catch(() => undefined);
    };
    const unsub = window.xuanyu?.onEvent?.('engine.status', (payload) => {
      const next = payload as { state?: string };
      const state = next?.state || '';
      if (state === 'ready' && previous && previous !== 'ready' && previous !== 'starting') {
        reconnect();
      }
      if (state) previous = state;
    });
    const onCustom = () => reconnect();
    window.addEventListener('xuanyu:engine-reconnected', onCustom);
    return () => {
      unsub?.();
      window.removeEventListener('xuanyu:engine-reconnected', onCustom);
    };
  }, [bootSession, options.enabled]);

  const addHandler = useCallback<AddHandler>(
    ((typeOrHandler: string | MessageHandler, maybeHandler?: TypedMessageHandler) => {
      let wrapped: MessageHandler | null = null;

      if (typeof typeOrHandler === 'function') {
        wrapped = typeOrHandler;
      } else if (typeof typeOrHandler === 'string' && typeof maybeHandler === 'function') {
        wrapped = (msg) => {
          if (msg.type !== typeOrHandler) return;
          maybeHandler(msg.payload, msg);
        };
      }

      if (!wrapped) return () => undefined;
      handlersRef.current.add(wrapped);
      return () => {
        handlersRef.current.delete(wrapped!);
      };
    }) as AddHandler,
    [],
  );

  return { connected, send, addHandler, mainSessionId: mainSessionIdState };
}

type DesktopCoreEventPayload = {
  type?: string;
  sessionId?: unknown;
  session_id?: unknown;
  task_id?: unknown;
  turnId?: unknown;
  turn_id?: unknown;
  state?: string;
  content?: string;
  data?: unknown;
  text?: string;
  isFinal?: boolean;
  is_final?: boolean;
  confidence?: number;
  mode?: string;
  salience?: number;
  level?: string;
  message?: string;
  totalEntries?: number;
  total_entries?: number;
  task?: string | SubAgentTaskLedger;
  provider?: string;
  model?: string;
  fromProvider?: string;
  from_provider?: string;
  fromModel?: string;
  from_model?: string;
  toProvider?: string;
  to_provider?: string;
  toModel?: string;
  to_model?: string;
  status?: string;
  summary?: string;
  strategy?: string;
  tokensBefore?: number;
  tokens_before?: number;
  tokensAfter?: number;
  tokens_after?: number;
  generatedBy?: string;
  generated_by?: string;
  error?: string | { message?: string };
  session?: Parameters<typeof toUiSession>[0];
  team?: SubAgentTeamSnapshot;
  ledger?: SubAgentTaskLedger;
  taskLedger?: SubAgentTaskLedger;
  runId?: string;
  run_id?: string;
  requestId?: string;
  request_id?: string;
  questions?: Array<{
    id: string;
    prompt: string;
    options?: string[];
    allow_multiple?: boolean;
    allowMultiple?: boolean;
  }>;
  answers?: Array<{ id: string; selected?: string[]; text?: string }>;
  toolName?: string;
  tool_name?: string;
  toolCallId?: string;
  tool_call_id?: string;
  args?: Record<string, unknown>;
  isError?: boolean;
  is_error?: boolean;
  promptTokens?: number;
  prompt_tokens?: number;
  completionTokens?: number;
  completion_tokens?: number;
  totalTokens?: number;
  total_tokens?: number;
  cacheTokens?: number;
  cache_tokens?: number;
  reason?: string;
  risk?: unknown;
  allowed?: boolean;
  plan?: import('../protocol').RunPlan | null;
  draftId?: string;
  draft_id?: string;
  qrUrl?: string;
  qr_url?: string;
  key?: string;
  ok?: boolean;
  worker?: {
    name?: string;
    status?: string;
    detail?: string;
    lastSeenMs?: number;
    last_seen_ms?: number;
  };
};

type DesktopCoreEnvelope = {
  payload?: DesktopCoreEventPayload;
};


function bufferCoreEnvelope(
  envelope: { meta?: Record<string, string>; payload?: unknown },
  replayBuffer: ReplayBufferItem[],
) {
  const sequence = Number(envelope.meta?.sequence);
  if (!Number.isFinite(sequence) || sequence <= 0) return;
  const msg: WsMessage = {
    id: `core_${sequence}`,
    type: 'core.event',
    payload: envelope.payload,
    timestamp: Date.now(),
    sequence,
  };
  replayBuffer.push({ sequence, timestamp: msg.timestamp, message: msg });
  trimReplayBuffer(replayBuffer);
}

function trimReplayBuffer(buffer: ReplayBufferItem[]) {
  const cutoff = Date.now() - REPLAY_TTL_MS;
  const trimmed = buffer
    .filter((item) => item.timestamp >= cutoff)
    .slice(-REPLAY_BUFFER_LIMIT);
  buffer.splice(0, buffer.length, ...trimmed);
}

function unwrapRustStringId(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object') {
    const record = value as { 0?: unknown };
    if (typeof record[0] === 'string') return record[0];
  }
  return '';
}

function coreEventSessionId(event: DesktopCoreEventPayload): string {
  return unwrapRustStringId(event.sessionId ?? event.session_id);
}

function readControlTaskEvent(payload: DesktopCoreEventPayload): ControlTaskEvent | undefined {
  const record = payload as DesktopCoreEventPayload & { event?: ControlTaskEvent };
  return record.event;
}

function readControlTaskResult(payload: DesktopCoreEventPayload): ControlTaskResult | undefined {
  const record = payload as DesktopCoreEventPayload & { result?: ControlTaskResult };
  return record.result;
}

function desktopEventErrorMessage(error: DesktopCoreEventPayload['error']): string {
  if (typeof error === 'string') return error;
  return error?.message || 'Turn failed';
}

function handleDesktopCoreEnvelope(
  envelope: DesktopCoreEnvelope,
  ctx: {
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
  },
) {
  const event = envelope.payload;
  if (!event?.type) return;
  const sessionId = coreEventSessionId(event);
  const turnId = unwrapRustStringId(event.turnId ?? event.turn_id);

  switch (event.type) {
    case 'turn_started':
      if (sessionId) {
        ctx.emit('session.updated', {
          session: {
            id: sessionId,
            isProcessing: true,
            status: 'active',
            updatedAt: Date.now(),
          },
        });
      }
      break;
    case 'turn_delta': {
      const content = event.content || '';
      if (sessionId && content) {
        ctx.emit('agent.event', {
          sessionId,
          event: { type: 'text', content, payload: turnId ? { turnId } : undefined },
        });
      }
      break;
    }
    case 'agent_tool_requested': {
      const toolCallId = String(event.toolCallId ?? event.tool_call_id ?? '');
      const toolName = String(event.toolName ?? event.tool_name ?? 'unknown');
      const args = (event.args && typeof event.args === 'object' ? event.args : {}) as Record<
        string,
        unknown
      >;
      if (sessionId && toolCallId) {
        ctx.emit('agent.event', {
          sessionId,
          event: {
            type: 'tool_call',
            call: {
              id: toolCallId,
              name: toolName,
              arguments: JSON.stringify(args),
            },
            payload: turnId ? { turnId } : undefined,
          },
        });
      }
      break;
    }
    case 'agent_tool_result': {
      const toolCallId = String(event.toolCallId ?? event.tool_call_id ?? '');
      if (sessionId && toolCallId) {
        ctx.emit('agent.event', {
          sessionId,
          event: {
            type: 'tool_result',
            result: {
              tool_call_id: toolCallId,
              content: String(event.content ?? ''),
              isError: Boolean(event.isError ?? event.is_error),
            },
            payload: turnId ? { turnId } : undefined,
          },
        });
      }
      break;
    }
    case 'agent_tool_progress': {
      const toolCallId = String(event.toolCallId ?? event.tool_call_id ?? '');
      const toolName = String(event.toolName ?? event.tool_name ?? 'unknown');
      const data = event.data ?? null;
      if (sessionId && toolCallId) {
        ctx.emit('agent.event', {
          sessionId,
          event: {
            type: 'tool_progress',
            toolUseId: toolCallId,
            toolName,
            data,
            payload: turnId ? { turnId } : undefined,
          },
        });
      }
      break;
    }
    case 'turn_usage': {
      if (sessionId) {
        const promptTokens = Number(event.promptTokens ?? event.prompt_tokens ?? 0);
        const completionTokens = Number(event.completionTokens ?? event.completion_tokens ?? 0);
        const totalTokens = Number(event.totalTokens ?? event.total_tokens ?? 0);
        if (promptTokens || completionTokens || totalTokens) {
          const cacheTokens = Number(event.cacheTokens ?? event.cache_tokens ?? 0);
          ctx.emit('agent.event', {
            sessionId,
            event: {
              type: 'usage',
              usage: {
                promptTokens,
                completionTokens,
                totalTokens: totalTokens || promptTokens + completionTokens,
                ...(cacheTokens > 0 ? { cacheTokens } : {}),
              },
              payload: turnId ? { turnId } : undefined,
            },
          });
        }
      }
      break;
    }
    case 'turn_reasoning': {
      const content = String(event.content ?? '');
      if (sessionId && content) {
        ctx.emit('agent.event', {
          sessionId,
          event: {
            type: 'reasoning',
            content,
            payload: turnId ? { turnId } : undefined,
          },
        });
      }
      break;
    }
    case 'context_compact': {
      if (sessionId) {
        const summary = String(event.summary ?? '');
        ctx.emit('agent.event', {
          sessionId,
          event: {
            type: 'context_compact',
            content: summary,
            payload: {
              ...(turnId ? { turnId } : {}),
              strategy: event.strategy,
              tokensBefore: event.tokensBefore ?? event.tokens_before,
              tokensAfter: event.tokensAfter ?? event.tokens_after,
            },
          },
        });
      }
      break;
    }
    case 'context_status_updated': {
      if (sessionId && event.status) {
        ctx.emit('context.status', { sessionId, status: event.status });
      }
      break;
    }
    case 'turn_finished':
      if (sessionId) {
        ctx.emit('agent.event', {
          sessionId,
          event: { type: 'done', payload: turnId ? { turnId } : undefined },
        });
        // 侧栏计数/标题；不在此处 reload history / agent.message（会与 done 提交重复且压扁时间线）。
        void sessionList()
          .then((sessions) => {
            if (sessions.length) ctx.emit('session.list', { sessions });
          })
          .catch(() => undefined);
      }
      break;
    case 'turn_failed':
      if (sessionId) {
        const messageText = desktopEventErrorMessage(event.error);
        ctx.emit('agent.event', {
          sessionId,
          event: { type: 'error', error: { message: messageText } },
        });
      }
      break;
    case 'audio_state_updated':
      if (sessionId && event.state) {
        ctx.emit('person.audio.state', { sessionId, state: event.state });
      }
      break;
    case 'asr_partial':
      if (sessionId) {
        ctx.emit('person.asr.partial', {
          sessionId,
          text: event.text || '',
          isFinal: event.isFinal ?? event.is_final ?? false,
          confidence: event.confidence,
        });
      }
      break;
    case 'worker_health': {
      const worker = event.worker;
      if (worker?.name) ctx.emit('worker.health.updated', { worker });
      break;
    }
    case 'log_entry':
      ctx.emit('gateway.log', {
        entry: {
          id: `rust_${Date.now()}`,
          level: (event.level || 'info') as GatewayLogEntry['level'],
          scope: 'rust-core',
          message: event.message || '',
          timestamp: Date.now(),
          details: {},
        } satisfies GatewayLogEntry,
      });
      break;
    case 'memory_updated':
      ctx.emit('memory.store.updated', {
        totalEntries: event.totalEntries ?? event.total_entries ?? 0,
      });
      break;
    case 'model_route_selected':
      if (sessionId) {
        ctx.emit('model.route.event', {
          sessionId,
          event: 'selected',
          task: typeof event.task === 'string' ? event.task : 'main',
          provider: event.provider,
          model: event.model,
        });
      }
      break;
    case 'model_route_fallback':
      if (sessionId) {
        ctx.emit('model.route.event', {
          sessionId,
          event: 'fallback',
          task: typeof event.task === 'string' ? event.task : 'main',
          fromProvider: event.fromProvider ?? event.from_provider,
          fromModel: event.fromModel ?? event.from_model,
          toProvider: event.toProvider ?? event.to_provider,
          toModel: event.toModel ?? event.to_model,
          reason: event.reason,
        });
      }
      break;
    case 'session_title_status': {
      const status = event.status || 'idle';
      if (sessionId) {
        ctx.emit('session.title.status', {
          sessionId,
          status,
          generatedBy: event.generatedBy || event.generated_by,
          error:
            typeof event.error === 'string'
              ? event.error
              : event.error?.message || undefined,
          provider: event.provider,
          model: event.model,
        });
      }
      break;
    }
    case 'session_updated': {
      const session = event.session;
      if (session?.id) {
        const ui = toUiSession(session);
        ctx.emit('session.updated', { session: ui });
        // 参考：自动起名后发 session.renamed；侧栏依赖该事件立刻刷新显示名。
        if (ui.name?.trim()) {
          ctx.emit('session.renamed', { sessionId: ui.id, session: ui });
        }
      }
      break;
    }
    case 'team_agent_started':
    case 'team_agent_progress':
    case 'team_agent_completed':
    case 'team_agent_failed':
      handleTeamAgentCoreEvent(event, ctx);
      break;
    case 'task_snapshot':
      handleTaskSnapshotCoreEvent(event, ctx);
      break;
    case 'task_event': {
      const taskEvent = readControlTaskEvent(event);
      const sid = sessionId || (taskEvent?.sessionId ?? '');
      const taskId = unwrapRustStringId(event.task_id) || taskEvent?.taskId;
      if (sid && taskEvent) ctx.emit('task.event', { sessionId: sid, taskId, event: taskEvent });
      break;
    }
    case 'task_result': {
      const taskResult = readControlTaskResult(event);
      const sid = sessionId;
      const taskId = unwrapRustStringId(event.task_id) || taskResult?.taskId;
      if (sid && taskResult) ctx.emit('task.result', { sessionId: sid, taskId, result: taskResult });
      break;
    }
    case 'run_plan': {
      const sid = sessionId;
      if (sid) {
        ctx.emit('run.plan', {
          sessionId: sid,
          runId: event.runId || event.run_id,
          plan: event.plan ?? null,
        });
      }
      break;
    }
    case 'question_requested':
      if (sessionId && (event.requestId || event.request_id)) {
        ctx.emit('question.request', {
          sessionId,
          requestId: event.requestId || event.request_id,
          questions: event.questions || [],
          runId: event.runId || event.run_id,
        });
      }
      break;
    case 'question_answered':
      if (sessionId) {
        ctx.emit('question.answered', {
          sessionId,
          requestId: event.requestId || event.request_id,
          answers: event.answers,
        });
      }
      break;
    case 'permission_requested':
      if (sessionId && (event.requestId || event.request_id)) {
        ctx.emit('permission.request', {
          sessionId,
          requestId: event.requestId || event.request_id,
          toolName: event.toolName || event.tool_name || '',
          args: event.args || {},
          reason: event.reason || '',
          runId: event.runId || event.run_id,
          risk: event.risk,
        });
      }
      break;
    case 'permission_answered':
      if (sessionId) {
        ctx.emit('permission.resolved', {
          sessionId,
          requestId: event.requestId || event.request_id,
          allowed: event.allowed,
        });
      }
      break;
    case 'channel_qr': {
      const draftId = event.draftId || event.draft_id;
      const qrUrl = event.qrUrl || event.qr_url || '';
      if (draftId) {
        ctx.emit('channel.qr', { key: draftId, qrUrl });
      }
      break;
    }
    case 'channel_qr_result': {
      const draftId = event.draftId || event.draft_id;
      if (draftId) {
        ctx.emit('channel.qr.result', {
          draftId,
          ok: Boolean(event.ok),
          key: event.key,
          error: typeof event.error === 'string' ? event.error : event.error?.message,
        });
        void channelConnectionsList()
          .then((listed) => {
            ctx.emit('channel.connections.list', listed);
          })
          .catch(() => undefined);
      }
      break;
    }
    default:
      break;
  }
}

function handleTeamAgentCoreEvent(
  event: DesktopCoreEventPayload,
  ctx: {
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
  },
) {
  const sessionId = coreEventSessionId(event);
  const team = event.team;
  const ledger = event.ledger;
  const mappedType =
    event.type === 'team_agent_started'
      ? 'team.agent.started'
      : event.type === 'team_agent_progress'
        ? 'team.agent.progress'
        : event.type === 'team_agent_completed'
          ? 'team.agent.completed'
          : 'team.agent.failed';
  ctx.emit(mappedType, { sessionId, team, ledger });
}

function handleTaskSnapshotCoreEvent(
  event: DesktopCoreEventPayload,
  ctx: {
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
  },
) {
  const sessionId = coreEventSessionId(event);
  const task =
    event.taskLedger ||
    (event.task && typeof event.task === 'object' ? event.task : undefined);
  ctx.emit('task.snapshot', { sessionId, task });
}

function isDuplicateMessage(id: string, cache: Map<string, number>): boolean {
  const now = Date.now();
  if (cache.has(id)) return true;
  cache.set(id, now);
  if (cache.size > 2000) {
    const threshold = now - 2 * 60 * 1000;
    for (const [key, ts] of cache.entries()) {
      if (ts < threshold) cache.delete(key);
    }
  }
  return false;
}

/** 本地已乐观开跑或有流式片段时，勿被 session.list / SessionUpdated 误清。 */
function hasLocalInflightTurn(sessionId: string): boolean {
  const session = useChatStore.getState().sessions[sessionId];
  if (!session) return false;
  return (
    session.isProcessing ||
    session.pendingMessageIndex !== null ||
    !!session.streamingText.trim() ||
    !!session.reasoningText.trim() ||
    session.turnSegments.length > 0
  );
}

function requestDesktopCurrentState(
  send: (type: string, payload: unknown) => void,
  sessionId: string,
): void {
  send('session.create', {});
  send('session.list', { limit: 50 });
  send('config.get', {});
  send('tools.catalog.get', { sessionId: sessionId || undefined });
  send('stats.get', {});
  if (sessionId) {
    send('session.restore', { sessionId });
    send('team.status', { sessionId });
    send('context.status', { sessionId });
  }
}

interface GatewayErrorPayload {
  message?: string;
  requestType?: string;
  sessionId?: string;
  structured?: {
    message?: string;
    code?: string;
    suggestion?: string;
    logPath?: string;
    recoverable?: boolean;
  };
}

function formatGatewayError(payload: GatewayErrorPayload): string {
  const structured = payload.structured;
  const raw =
    structured?.message || payload.message || localeText('未知错误', 'Unknown error');
  if (/\b(aborted|cancelled|canceled|user aborted)\b/i.test(raw)) {
    return localeText('已停止本次运行。', 'Run stopped.');
  }
  const degraded = raw.match(/Provider route degraded:\s*(.+)$/i);
  if (degraded) {
    return localeText(
      `模型通道暂时不可用（${degraded[1].trim()}）。通常是限流或连续失败后的保护，稍后再试或换一个模型。`,
      `Model route temporarily unavailable (${degraded[1].trim()}).`,
    );
  }
  const lines = [
    raw,
    structured?.code ? `${localeText('错误码', 'Code')}：${structured.code}` : '',
    structured?.suggestion ? `${localeText('建议', 'Suggestion')}：${structured.suggestion}` : '',
  ].filter(Boolean);
  return lines.join('\n');
}

function localeText(zh: string, en: string): string {
  try {
    return window.localStorage.getItem('xy-locale') === 'en' ? en : zh;
  } catch {
    return zh;
  }
}
