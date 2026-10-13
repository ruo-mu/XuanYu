import { message } from 'antd';

import type {
  AgentDefinitionInput,
  ChatSendPayload,
  CompactResult,
  ConfigCapabilitiesPayload,
  ContextStatus,
  ExtensionsListPayload,
  GatewayLogEntry,
  GitStatusSnapshot,
  MemoryItem,
  MemoryProfileFile,
  MemorySnapshotInfo,
  SessionSummary,
  PendingMemory,
  NestedTaskSnapshot,
  RunPlan,
  SessionInfo,
  SessionStats,
  SubAgentTaskLedger,
  SubAgentTimelineEvent,
  SystemStatusPayload,
  AnalyticsSnapshot,
  ToolCatalogPayload,
  WsMessage,
} from '../protocol';
import type { AppConfig } from '../stores/configStore';
import { useChatStore, type PendingPermission } from '../stores/chatStore';
import { useConfigStore } from '../stores/configStore';
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
import { DESKTOP_GATEWAY_DEFAULT } from '../foundation/config/types';
import { getMainSessionId, setMainSessionIdValue } from './desktopSessionState';

type EmitOptions = { buffer?: boolean };
type ReplayBufferItem = { sequence: number; timestamp: number; message: WsMessage };

const REPLAY_BUFFER_LIMIT = 500;
const REPLAY_TTL_MS = 60_000;
const LOCAL_CONFIG_KEY = 'xy-desktop-config';

export async function sendDesktopCommand(
  type: string,
  payload: unknown,
  ctx: {
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
    sessionId: string;
    syncProcessingState: (sessionId: string, isProcessing: boolean) => void;
    addSystemMessage: (sessionId: string, content: string) => void;
    addErrorMessage: (sessionId: string, content: string) => void;
    updateSession: (sessionId: string, update: Partial<SessionInfo>) => void;
    setMainSessionId: (sessionId: string) => void;
    rollbackOptimisticMessage: ReturnType<typeof useChatStore.getState>['rollbackOptimisticMessage'];
    setPendingPermission: (sessionId: string, perm: PendingPermission | null) => void;
    setPendingQuestion: (sessionId: string, question: import('../stores/chatStore').PendingQuestion | null) => void;
    upsertSubAgentLedger: (sessionId: string, ledger: SubAgentTaskLedger) => void;
    setSubAgentTimeline: (
      sessionId: string,
      taskId: string,
      timeline: import('../protocol').SubAgentTimelineEvent[],
      ledger?: SubAgentTaskLedger,
    ) => void;
    replayBuffer: ReplayBufferItem[];
    nextSequence: () => number;
  },
) {
  const sessionId = ctx.sessionId;
  switch (type) {
    case 'session.create': {
      const modeRaw = stringPayload(payload, 'mode');
      const mode =
        modeRaw === 'recent' || modeRaw === 'branch' || modeRaw === 'main' ? modeRaw : 'main';
      const title = stringPayload(payload, 'title') || undefined;
      const projectDir = stringPayload(payload, 'projectDir')?.trim() || undefined;
      const session = await sessionCreate({
        mode: mode === 'main' ? 'main' : mode,
        title,
        projectDir: mode === 'branch' ? projectDir : undefined,
      });
      const emitMode = mode === 'main' ? undefined : mode;
      const nextSession =
        mode === 'recent'
          ? { ...session, projectDir: undefined, metadata: {} }
          : mode === 'branch' && projectDir
            ? {
                ...session,
                projectDir,
                metadata: { ...(session.metadata || {}), projectDir, workspace: true },
              }
            : session;
      ctx.emit('session.created', {
        sessionId: nextSession.id,
        name: nextSession.name,
        session: nextSession,
        mode: emitMode,
      });
      if (mode === 'main') {
        const sessions = await sessionList().catch(() => [nextSession]);
        const history = await sessionHistory(nextSession.id).catch(() => []);
        ctx.emit('session.history', { sessionId: nextSession.id, messages: history });
        ctx.emit('session.list', { sessions: sessions.length ? sessions : [nextSession] });
      }
      break;
    }
    case 'chat.send': {
      if (!sessionId) return;
      const content = ((payload as ChatSendPayload)?.content || '').trim();
      try {
        ctx.syncProcessingState(sessionId, true);
        ctx.updateSession(sessionId, { isProcessing: true, status: 'active', updatedAt: Date.now() });
        const attachments = Array.isArray((payload as ChatSendPayload)?.attachments)
          ? (payload as ChatSendPayload).attachments
          : undefined;
        const deepThinking = Boolean((payload as ChatSendPayload)?.deepThinking);
        void (async () => {
          try {
            await sessionSendMessage(sessionId, content || '(附件消息)', attachments, deepThinking);
          } catch (err) {
            ctx.rollbackOptimisticMessage(sessionId);
            ctx.syncProcessingState(sessionId, false);
            ctx.updateSession(sessionId, { isProcessing: false, status: 'idle', updatedAt: Date.now() });
            ctx.addErrorMessage(sessionId, err instanceof Error ? err.message : String(err));
          }
        })();
      } catch (err) {
        ctx.rollbackOptimisticMessage(sessionId);
        ctx.addErrorMessage(sessionId, err instanceof Error ? err.message : String(err));
      }
      break;
    }
    case 'chat.abort': {
      if (!sessionId) return;
      const result = await sessionAbort(sessionId);
      ctx.syncProcessingState(sessionId, false);
      if (result.session) {
        ctx.emit('session.updated', { session: result.session });
      } else {
        ctx.updateSession(sessionId, { isProcessing: false, status: 'idle', updatedAt: Date.now() });
      }
      ctx.addSystemMessage(sessionId, '已停止当前回复。');
      ctx.emit('chat.aborted', result);
      if (result.cancelledRun) {
        ctx.emit('run.cancelled', {
          ok: true,
          run: result.cancelledRun,
          runId: result.cancelledRun.id,
        });
      }
      break;
    }
    case 'session.history':
    case 'session.restore': {
      if (!sessionId) return;
      const history = await sessionHistory(sessionId);
      ctx.emit('session.history', { sessionId, messages: history });
      break;
    }
    case 'session.rewind.preview': {
      if (!sessionId) return;
      ctx.emit('session.rewind.preview', await sessionRewindPreview(sessionId));
      break;
    }
    case 'session.rewind': {
      if (!sessionId) return;
      const turnId = stringPayload(payload, 'turnId');
      if (!turnId) return;
      const result = await sessionRewind({
        sessionId,
        turnId,
        mode: (stringPayload(payload, 'mode') as 'both' | 'conversation' | 'workspace' | '') || undefined,
        newContent: stringPayload(payload, 'newContent') || undefined,
      });
      ctx.emit('session.rewound', result);
      const history = await sessionHistory(sessionId);
      ctx.emit('session.history', { sessionId, messages: history });
      const sessions = await sessionList().catch(() => []);
      if (sessions.length) ctx.emit('session.list', { sessions });
      break;
    }
    case 'session.list': {
      const sessions = await sessionList(sessionListOptions(payload));
      ctx.emit('session.list', { sessions });
      break;
    }
    case 'conversation.search': {
      const query = stringPayload(payload, 'query');
      const results = await conversationSearch(query, numberPayload(payload, 'limit', 30));
      ctx.emit('conversation.search', { query, results });
      break;
    }
    case 'session.delete': {
      if (!sessionId) return;
      const deleted = await sessionDelete(sessionId);
      const sessions = await sessionList().catch(() => []);
      if (deleted) {
        useChatStore.getState().removeSession(sessionId);
        ctx.emit('session.deleted', { sessionId, deleted: true });
        await switchActiveSessionIfNeeded(sessionId, sessions, ctx);
      }
      if (sessions.length) ctx.emit('session.list', { sessions });
      break;
    }
    case 'session.archive': {
      if (!sessionId) return;
      const archived = booleanPayload(payload, 'archived', true);
      const ok = await sessionArchive(sessionId, archived);
      const sessions = await sessionList().catch(() => []);
      ctx.emit('session.archived', { sessionId, archived, ok });
      if (ok && archived) {
        useChatStore.getState().removeSession(sessionId);
        await switchActiveSessionIfNeeded(sessionId, sessions, ctx);
      }
      if (sessions.length) ctx.emit('session.list', { sessions });
      break;
    }
    case 'session.rename': {
      if (!sessionId) return;
      const name = stringPayload(payload, 'name');
      if (!name.trim()) return;
      const session = await sessionRename(sessionId, name);
      ctx.emit('session.renamed', { sessionId, session });
      const sessions = await sessionList().catch(() => []);
      if (sessions.length) ctx.emit('session.list', { sessions });
      break;
    }
    case 'session.pin': {
      if (!sessionId) return;
      const pinned = booleanPayload(payload, 'pinned', true);
      const session = await sessionPin(sessionId, pinned);
      if (session) {
        ctx.updateSession(sessionId, { pinned: session.pinned });
        ctx.emit('session.updated', { sessionId, session });
      }
      break;
    }
    case 'workspace.file.reject': {
      if (!sessionId) return;
      const turnId = stringPayload(payload, 'turnId');
      const path = stringPayload(payload, 'path');
      if (!turnId || !path) {
        ctx.addErrorMessage(sessionId, 'workspace.file.reject 需要 sessionId、turnId 和 path');
        break;
      }
      const result = await workspaceFileReject({ sessionId, turnId, path });
      ctx.emit('workspace.file.rejected', { sessionId, turnId, path, ...result });
      break;
    }
    case 'goal.get': {
      if (!sessionId) return;
      ctx.emit('goal.updated', await goalGet(sessionId));
      break;
    }
    case 'goal.action': {
      if (!sessionId) return;
      const action = stringPayload(payload, 'action');
      if (!action) {
        ctx.addErrorMessage(sessionId, 'goal.action 需要 sessionId 和 action');
        break;
      }
      const result = await goalAction({
        sessionId,
        action,
        objective: stringPayload(payload, 'objective') || undefined,
        tokenBudget:
          payload && typeof payload === 'object' && (payload as { tokenBudget?: unknown }).tokenBudget != null
            ? numberPayload(payload, 'tokenBudget', 0)
            : undefined,
      });
      if (!result.ok) {
        ctx.addErrorMessage(sessionId, result.error || 'Goal action failed');
        break;
      }
      if (result.snapshot) ctx.emit('goal.updated', result.snapshot);
      if (result.continueMessage?.trim()) {
        try {
          ctx.syncProcessingState(sessionId, true);
          ctx.updateSession(sessionId, { isProcessing: true, status: 'active', updatedAt: Date.now() });
          await sessionSendMessage(sessionId, result.continueMessage.trim());
        } catch (err) {
          ctx.syncProcessingState(sessionId, false);
          ctx.addErrorMessage(sessionId, err instanceof Error ? err.message : String(err));
        }
      }
      break;
    }
    case 'marketplace.search': {
      const result = await marketplaceSearch({
        query: stringPayload(payload, 'query') || undefined,
        kind: stringPayload(payload, 'kind') || undefined,
        kinds: arrayPayload(payload, 'kinds'),
        requestId: stringPayload(payload, 'requestId') || undefined,
        page: numberPayload(payload, 'page', 0) || undefined,
        pageSize: numberPayload(payload, 'pageSize', 0) || undefined,
        category: stringPayload(payload, 'category') || undefined,
        sourceId: stringPayload(payload, 'sourceId') || undefined,
        summaryOnly: booleanPayload(payload, 'summaryOnly', false) || undefined,
      });
      ctx.emit('marketplace.search', result);
      break;
    }
    case 'marketplace.install.plan': {
      const entryId = stringPayload(payload, 'entryId');
      if (!entryId) break;
      ctx.emit(
        'marketplace.install.plan',
        await marketplaceInstallPlan({
          entryId,
          scope: (stringPayload(payload, 'scope') as 'user' | 'project' | 'org' | '') || undefined,
        }),
      );
      break;
    }
    case 'marketplace.install.confirm': {
      const planId = stringPayload(payload, 'planId');
      const approvalDigest = stringPayload(payload, 'approvalDigest');
      if (!planId || !approvalDigest) break;
      ctx.emit(
        'marketplace.install.operation',
        await marketplaceInstallConfirm({ planId, approvalDigest }),
      );
      break;
    }
    case 'marketplace.install.status':
      ctx.emit('marketplace.install.status', await marketplaceInstallStatus());
      break;
    case 'capability.installed.list':
      ctx.emit('capability.installed.list', await capabilityInstalledList());
      break;
    case 'capability.installed.mutate': {
      const entryId = stringPayload(payload, 'entryId');
      const action = stringPayload(payload, 'action') as 'enable' | 'disable' | 'uninstall' | 'rollback';
      if (!entryId || !action) break;
      ctx.emit('marketplace.install.operation', await capabilityInstalledMutate({ entryId, action }));
      break;
    }
    case 'config.get': {
      const config = mergeConfig(defaultConfig, await configGet());
      const capabilities = await configCapabilities().catch(() => createConfiguredCapabilities(config));
      ctx.emit('config.current', { config });
      ctx.emit('config.capabilities', capabilities);
      break;
    }
    case 'config.update':
    case 'config.save': {
      const partial = (payload as { config?: Partial<AppConfig> })?.config || {};
      const next = mergeConfig(defaultConfig, await configUpdate(partial));
      const capabilities = await configCapabilities().catch(() => createConfiguredCapabilities(next));
      ctx.emit('config.updated', { config: next });
      ctx.emit('config.capabilities', capabilities);
      message.success('配置已更新');
      break;
    }
    case 'config.capabilities': {
      const config = mergeConfig(defaultConfig, await configGet().catch(() => ({} as Partial<AppConfig>)));
      const capabilities = await configCapabilities().catch(() => createConfiguredCapabilities(config));
      ctx.emit('config.capabilities', capabilities);
      break;
    }
    case 'tools.catalog.get': {
      const catalog = await toolCatalog();
      ctx.emit('tools.catalog', { sessionId, ...catalog });
      break;
    }
    case 'tool.execute': {
      const name = stringPayload(payload, 'name');
      if (!name) {
        ctx.emit('tool.executed', {
          sessionId,
          name,
          output: { content: 'tool.execute requires a tool name', isError: true },
        });
        break;
      }
      const output = await toolExecute({
        name,
        args: objectPayload(payload, 'args'),
        cwd: stringPayload(payload, 'cwd') || undefined,
        sessionId: sessionId || undefined,
      });
      const pending = pendingPermissionFromToolOutput(output, sessionId || undefined);
      if (pending) ctx.emit('permission.request', pending);
      ctx.emit('tool.executed', { sessionId, name, output });
      break;
    }
    case 'permission.respond':
      {
        const requestId = stringPayload(payload, 'requestId');
        if (!requestId) {
          if (sessionId) ctx.setPendingPermission(sessionId, null);
          break;
        }
        const allowed = booleanPayload(payload, 'allowed', false);
        const scope =
          stringPayload(payload, 'scope') ||
          (booleanPayload(payload, 'remember', false) ? 'session' : 'once');
        const result = await permissionRespond({
          requestId,
          allowed,
          remember: scope === 'session',
          scope,
        });
        const responseSessionId = result.sessionId || sessionId;
        if (responseSessionId) ctx.setPendingPermission(responseSessionId, null);
        ctx.emit('permission.resolved', result);
        if (result.allowed && result.output && !(result as { waitingTurn?: boolean }).waitingTurn) {
          ctx.emit('tool.executed', {
            sessionId: responseSessionId,
            name: result.toolName || '',
            output: result.output,
          });
        }
      }
      break;
    case 'question.respond': {
      const requestId = stringPayload(payload, 'requestId');
      if (!requestId) {
        if (sessionId) ctx.setPendingQuestion(sessionId, null);
        break;
      }
      const answers = answersPayload(payload);
      const result = await questionRespond({ requestId, answers });
      const responseSessionId = result.sessionId || sessionId;
      ctx.emit('question.answered', {
        sessionId: responseSessionId,
        requestId,
        answers,
      });
      if (responseSessionId) ctx.setPendingQuestion(responseSessionId, null);
      break;
    }
    case 'stats.get':
      {
        const snapshot = await statsGet();
        ctx.emit('stats.current', snapshot);
      }
      break;
    case 'extensions.list':
      {
        const extensions = await extensionsList();
        ctx.emit('extensions.list', extensions);
      }
      break;
    case 'plugin.setEnabled': {
      const result = await pluginSetEnabled(
        stringPayload(payload, 'id'),
        booleanPayload(payload, 'enabled', true),
      );
      if (result.extensions) ctx.emit('extensions.list', result.extensions);
      ctx.emit('plugin.updated', result);
      break;
    }
    case 'plugin.install': {
      const result = await pluginInstall(plainPayload(payload));
      ctx.emit('plugin.installed', result);
      break;
    }
    case 'providers.catalog':
      ctx.emit('providers.catalog', { providers: getProviderCatalog() });
      break;
    case 'agent.list':
      ctx.emit('agent.list', await agentList());
      break;
    case 'agent.create': {
      const result = await agentCreate(plainPayload(payload));
      ctx.emit('agent.created', result);
      break;
    }
    case 'agent.update': {
      const id = stringPayload(payload, 'id');
      const input = plainPayload(payload);
      delete (input as { id?: unknown }).id;
      const result = await agentUpdate(id, input);
      ctx.emit('agent.updated', { id, ...result });
      break;
    }
    case 'agent.delete': {
      const id = stringPayload(payload, 'id');
      const result = await agentDelete(id);
      ctx.emit('agent.deleted', { id, ...result });
      break;
    }
    case 'team.status': {
      const result = await teamStatus(sessionId || undefined);
      const actualSessionId = result.sessionId || sessionId;
      ctx.emit('team.status', { ...result, sessionId: actualSessionId });
      break;
    }
    case 'subagent.timeline': {
      const taskId = stringPayload(payload, 'taskId');
      const result = await subagentTimeline({ sessionId: sessionId || undefined, taskId });
      const actualSessionId = result.sessionId || sessionId;
      ctx.emit('subagent.timeline', { ...result, sessionId: actualSessionId });
      break;
    }
    case 'task.list': {
      const result = await taskList({
        sessionId: sessionId || undefined,
        limit: numberPayload(payload, 'limit', 50),
      });
      if (sessionId) {
        result.tasks.forEach((ledger) => ctx.upsertSubAgentLedger(sessionId, ledger));
      }
      ctx.emit('task.list', result);
      break;
    }
    case 'task.create': {
      const input = plainPayload(payload);
      const result = await taskCreate({
        sessionId: sessionId || undefined,
        agentId: stringPayload(input, 'agentId'),
        task: stringPayload(input, 'task'),
        context: stringPayload(input, 'context') || undefined,
        writeScope: arrayPayload(input, 'writeScope'),
        runId: stringPayload(input, 'runId') || undefined,
        autoExecute: booleanPayload(input, 'autoExecute', true),
      });
      const actualSessionId = result.sessionId || sessionId;
      if (actualSessionId && result.task) {
        ctx.upsertSubAgentLedger(actualSessionId, result.task);
        ctx.setSubAgentTimeline(actualSessionId, result.task.taskId, result.task.timeline || [], result.task);
      }
      ctx.emit('task.created', { ...result, sessionId: actualSessionId });
      break;
    }
    case 'subagent.delegate': {
      const input = plainPayload(payload) as {
        runId?: string;
        autoExecute?: boolean;
        tasks?: unknown;
      };
      const tasks = Array.isArray(input.tasks)
        ? input.tasks.filter(
            (item): item is Record<string, unknown> =>
              Boolean(item) && typeof item === 'object' && !Array.isArray(item),
          )
        : [];
      const result = await subagentDelegateMany({
        sessionId: sessionId || undefined,
        runId: input.runId || undefined,
        tasks,
        autoExecute: input.autoExecute ?? true,
      });
      const actualSessionId = result.sessionId || sessionId;
      if (actualSessionId && result.tasks) {
        result.tasks.forEach((ledger) => {
          ctx.upsertSubAgentLedger(actualSessionId, ledger);
          ctx.setSubAgentTimeline(actualSessionId, ledger.taskId, ledger.timeline || [], ledger);
        });
      }
      ctx.emit('subagent.delegated', { ...result, sessionId: actualSessionId });
      break;
    }
    case 'subagent.assessMerge': {
      const input = plainPayload(payload) as { results?: unknown };
      const results = Array.isArray(input.results)
        ? input.results.filter(
            (item): item is Record<string, unknown> =>
              Boolean(item) && typeof item === 'object' && !Array.isArray(item),
          )
        : [];
      const result = await subagentAssessMerge({ results });
      ctx.emit('subagent.assessMerge', { sessionId, ...result });
      break;
    }
    case 'task.status': {
      const result = await taskStatus(stringPayload(payload, 'taskId'));
      if (sessionId && result.task) ctx.upsertSubAgentLedger(sessionId, result.task);
      ctx.emit('task.status', result);
      break;
    }
    case 'task.pause':
    case 'task.resume':
    case 'task.cancel':
    case 'task.retry': {
      const result = await taskControl({
        taskId: stringPayload(payload, 'taskId'),
        action: type.split('.')[1] as 'pause' | 'resume' | 'cancel' | 'retry',
        reason: stringPayload(payload, 'reason') || undefined,
      });
      if (sessionId && result.task) ctx.upsertSubAgentLedger(sessionId, result.task);
      ctx.emit('task.control', result);
      break;
    }
    case 'system.status':
      ctx.emit('system.status', await systemStatus());
      break;
    case 'gateway.logs': {
      const limit = numberPayload(payload, 'limit', 200);
      ctx.emit('gateway.logs', await gatewayLogs(limit));
      break;
    }
    case 'audit.list': {
      const limit = numberPayload(payload, 'limit', 200);
      ctx.emit('audit.list', await auditList(limit));
      break;
    }
    case 'context.reset': {
      if (!sessionId) return;
      const reset = await sessionResetContext(sessionId);
      const history = await sessionHistory(sessionId).catch(() => []);
      ctx.emit('context.reset', { ...reset, sessionId });
      ctx.emit('session.history', { sessionId, messages: history });
      if (reset.session) ctx.emit('session.updated', { session: reset.session });
      const sessions = await sessionList().catch(() => []);
      if (sessions.length) ctx.emit('session.list', { sessions });
      break;
    }
    case 'context.status': {
      if (!sessionId) return;
      const status = await contextStatus(sessionId);
      ctx.emit('context.status', { sessionId, status });
      break;
    }
    case 'context.compact': {
      if (!sessionId) return;
      const result = await contextCompact(sessionId, stringPayload(payload, 'strategy') || undefined);
      ctx.emit('context.compact', result);
      break;
    }
    case 'context.rollback': {
      if (!sessionId) return;
      ctx.emit('context.rollback', await contextRollback(sessionId));
      break;
    }
    case 'run.list':
      ctx.emit('run.list', await runList(numberPayload(payload, 'limit', 50)));
      break;
    case 'run.timeline':
      ctx.emit('run.timeline', await runTimeline(stringPayload(payload, 'runId')));
      break;
    case 'run.cancel': {
      const runId = stringPayload(payload, 'runId');
      const result = await runCancel({ runId, reason: stringPayload(payload, 'reason') || undefined });
      ctx.emit('run.cancelled', result);
      ctx.emit('run.list', await runList(50));
      break;
    }
    case 'run.resume': {
      const runId = stringPayload(payload, 'runId');
      const result = await runResume(runId);
      ctx.emit('run.resumed', result);
      ctx.emit('run.timeline', await runTimeline(runId));
      ctx.emit('run.list', await runList(50));
      break;
    }
    case 'run.replay': {
      const runId = stringPayload(payload, 'runId');
      ctx.emit('run.replay', await runReplay(runId));
      break;
    }
    case 'replay.request': {
      const lastSequence = numberPayload(payload, 'lastSequence', 0);
      const sessionId = stringPayload(payload, 'sessionId');
      const turnId = stringPayload(payload, 'turnId');
      try {
        const batch = sessionId
          ? await eventReplayFiltered(lastSequence, {
              sessionId,
              turnId: turnId || undefined,
            })
          : await eventReplay(lastSequence);
        if (batch.overflow) {
          ctx.emit(
            'replay.overflow',
            { lastSequence: batch.lastSequence, nextSequence: batch.nextSequence },
            { buffer: false },
          );
          break;
        }
        const rustMessages = batch.events.map(storedEventToWsMessage);
        const localMessages = collectLocalReplayMessages(
          lastSequence,
          ctx.replayBuffer,
          ctx.nextSequence(),
        );
        ctx.emit(
          'replay.batch',
          {
            lastSequence,
            messages: mergeReplayMessages(rustMessages, localMessages),
          },
          { buffer: false },
        );
      } catch {
        emitReplayBatch(lastSequence, ctx.emit, ctx.replayBuffer, ctx.nextSequence());
      }
      break;
    }
    case 'trace.get':
      ctx.emit('trace.timeline', await traceGet({
        runId: stringPayload(payload, 'runId') || undefined,
        traceId: stringPayload(payload, 'traceId') || undefined,
      }));
      break;
    case 'trace.evidence':
      ctx.emit('trace.evidence', await traceEvidence({
        runId: stringPayload(payload, 'runId'),
        format: (stringPayload(payload, 'format', 'json') as 'json' | 'markdown') || 'json',
      }));
      break;
    case 'trace.compare':
      ctx.emit('trace.compare', await traceCompare({
        baseRunId: stringPayload(payload, 'baseRunId'),
        targetRunId: stringPayload(payload, 'targetRunId'),
      }));
      break;
    case 'feedback.submit':
      ctx.emit('feedback.saved', await feedbackSubmit({
        runId: stringPayload(payload, 'runId') || undefined,
        sessionId: stringPayload(payload, 'sessionId') || undefined,
        status: stringPayload(payload, 'status', 'partial'),
        note: stringPayload(payload, 'note') || undefined,
        tags: arrayPayload(payload, 'tags'),
      }));
      break;
    case 'process.list':
      ctx.emit('process.list', await processList());
      break;
    case 'process.read': {
      const processId = stringPayload(payload, 'processId');
      ctx.emit('process.output', await processRead(processId, numberPayload(payload, 'offset', 0)));
      break;
    }
    case 'process.stop': {
      const processId = stringPayload(payload, 'processId');
      ctx.emit('process.output', await processStop(processId));
      ctx.emit('process.list', await processList());
      break;
    }
    case 'git.status': {
      const result = await gitStatus(sessionId || undefined);
      ctx.emit('git.status', { sessionId, status: result.status });
      break;
    }
    case 'git.branches': {
      const result = await gitBranches(sessionId || undefined);
      ctx.emit('git.branches', { sessionId, branches: result.branches || [] });
      break;
    }
    case 'git.checkout': {
      const branch = stringPayload(payload, 'branch');
      if (!branch) {
        ctx.addErrorMessage(sessionId, 'git.checkout 需要 branch');
        break;
      }
      const result = await gitCheckout(branch, sessionId || undefined);
      if (result.error) {
        ctx.addErrorMessage(sessionId, result.error);
        break;
      }
      ctx.emit('git.checkout', {
        sessionId,
        branch: result.branch || branch,
        status: result.status,
      });
      if (result.status) {
        ctx.emit('git.status', { sessionId, status: result.status });
      }
      break;
    }
    case 'workspace.open': {
      const path = stringPayload(payload, 'path');
      if (!path?.trim()) {
        ctx.addErrorMessage(sessionId, 'workspace.open 需要 path');
        break;
      }
      try {
        const opened = await workspaceOpen({
          path,
          sessionId: sessionId || undefined,
        });
        const status = await systemStatus();
        const git = await gitStatus(sessionId || undefined);
        const branches = await gitBranches(sessionId || undefined);
        ctx.emit('workspace.opened', { ...opened, status });
        ctx.emit('system.status', status);
        ctx.emit('git.status', { sessionId, status: git.status });
        ctx.emit('git.branches', { sessionId, branches: branches.branches || [] });
      } catch (err) {
        ctx.addErrorMessage(
          sessionId,
          err instanceof Error ? err.message : String(err),
        );
      }
      break;
    }
    case 'workspace.clear': {
      const clearSessionId = stringPayload(payload, 'sessionId') || sessionId;
      if (!clearSessionId?.trim()) {
        ctx.addErrorMessage(sessionId, 'workspace.clear 需要 sessionId');
        break;
      }
      try {
        const cleared = await workspaceClear({ sessionId: clearSessionId.trim() });
        ctx.emit('workspace.cleared', cleared);
      } catch (err) {
        ctx.addErrorMessage(
          sessionId,
          err instanceof Error ? err.message : String(err),
        );
      }
      break;
    }
    case 'workspace.pickFolder': {
      try {
        const picked = await workspacePickFolder({
          title: stringPayload(payload, 'title') || undefined,
          initialDir: stringPayload(payload, 'initialDir') || undefined,
        });
        if (picked.cancelled) {
          ctx.emit('workspace.folderPicked', { path: null, cancelled: true });
        } else if (picked.path) {
          ctx.emit('workspace.folderPicked', { path: picked.path, cancelled: false });
        } else {
          ctx.emit('workspace.folderPicked', {
            path: null,
            cancelled: false,
            error: picked.error || 'Folder picker failed',
          });
        }
      } catch (err) {
        ctx.emit('workspace.folderPicked', {
          path: null,
          cancelled: false,
          error: err instanceof Error ? err.message : String(err),
        });
      }
      break;
    }
    case 'workspace.reveal': {
      const path = stringPayload(payload, 'path');
      if (!path?.trim()) {
        ctx.addErrorMessage(sessionId, 'workspace.reveal 需要 path');
        break;
      }
      try {
        const revealed = await workspaceReveal({ path: path.trim() });
        ctx.emit('workspace.revealed', revealed);
      } catch (err) {
        ctx.addErrorMessage(
          sessionId,
          err instanceof Error ? err.message : String(err),
        );
      }
      break;
    }
    case 'workspace.create': {
      const parentDir = stringPayload(payload, 'parentDir');
      const name = stringPayload(payload, 'name');
      if (!parentDir?.trim() || !name?.trim()) {
        ctx.addErrorMessage(sessionId, 'workspace.create 需要 parentDir 和 name');
        break;
      }
      try {
        const created = await workspaceCreate({
          parentDir,
          name,
          sessionId: sessionId || undefined,
        });
        const status = await systemStatus();
        const git = await gitStatus(sessionId || undefined);
        const branches = await gitBranches(sessionId || undefined);
        ctx.emit('workspace.opened', { ...created, status });
        ctx.emit('system.status', status);
        ctx.emit('git.status', { sessionId, status: git.status });
        ctx.emit('git.branches', { sessionId, branches: branches.branches || [] });
      } catch (err) {
        ctx.addErrorMessage(
          sessionId,
          err instanceof Error ? err.message : String(err),
        );
      }
      break;
    }
    case 'channel.adapters.list':
      ctx.emit('channel.adapters.list', await channelAdaptersList());
      break;
    case 'channel.connections.list':
      ctx.emit('channel.connections.list', await channelConnectionsList());
      break;
    case 'channel.connection.upsert': {
      const config = objectPayload(payload, 'config') as Record<string, unknown>;
      const result = await channelConnectionUpsert({
        config,
        connectAfter: booleanPayload(payload, 'connectAfter', false),
      });
      ctx.emit('channel.connection.updated', { key: result.key });
      if (booleanPayload(payload, 'connectAfter', false)) {
        void channelConnectionConnect({ key: result.key })
          .then((list) => ctx.emit('channel.connections.list', list))
          .catch((err) => {
            ctx.emit('channel.connections.list', { connections: [] });
            ctx.addErrorMessage(
              sessionId,
              err instanceof Error ? err.message : String(err),
            );
          });
      } else {
        ctx.emit('channel.connections.list', await channelConnectionsList());
      }
      break;
    }
    case 'channel.connection.connect': {
      const key = stringPayload(payload, 'key');
      if (!key) return;
      void channelConnectionConnect({ key })
        .then((list) => ctx.emit('channel.connections.list', list))
        .catch(async (err) => {
          ctx.emit('channel.connections.list', await channelConnectionsList().catch(() => ({ connections: [] })));
          ctx.addErrorMessage(sessionId, err instanceof Error ? err.message : String(err));
        });
      break;
    }
    case 'channel.connection.disconnect': {
      const key = stringPayload(payload, 'key');
      if (!key) return;
      ctx.emit('channel.connections.list', await channelConnectionDisconnect({ key }));
      break;
    }
    case 'channel.connection.remove': {
      const key = stringPayload(payload, 'key');
      if (!key) return;
      ctx.emit('channel.connections.list', await channelConnectionRemove({ key }));
      break;
    }
    case 'channel.qr.begin': {
      const channelType = stringPayload(payload, 'type');
      if (!channelType) return;
      const result = await channelQrBegin({
        type: channelType,
        tenantId: stringPayload(payload, 'tenantId') || undefined,
      });
      const draftId = result.draftId || `draft:${channelType}:${Date.now()}`;
      if (result.pending) {
        // QR URL + final result arrive via core events channel_qr / channel_qr_result.
        ctx.emit('channel.qr', { key: draftId, qrUrl: '' });
        break;
      }
      ctx.emit('channel.qr', { key: draftId, qrUrl: '' });
      ctx.emit('channel.qr.result', {
        draftId,
        ok: Boolean(result.ok),
        key: result.key,
        error: result.error,
      });
      break;
    }
    case 'channel.qr.cancel':
      await channelQrCancel();
      break;
    case 'channel.delivery.list':
      ctx.emit(
        'channel.delivery.list',
        await channelDeliveryList(numberPayload(payload, 'limit', 50)),
      );
      break;
    case 'memory.list': {
      if (!sessionId) return;
      const limit = numberPayload(payload, 'limit', 50);
      const result = await memoryList(limit);
      ctx.emit('memory.list', { sessionId, ...result });
      break;
    }
    case 'memory.search': {
      if (!sessionId) return;
      const query = stringPayload(payload, 'query');
      const limit = numberPayload(payload, 'limit', 20);
      const result = await memorySearch(query, limit);
      ctx.emit('memory.search', { sessionId, ...result });
      break;
    }
    case 'memory.create': {
      if (!sessionId) return;
      const result = await memoryCreate({
        content: stringPayload(payload, 'content'),
        category: stringPayload(payload, 'category', 'session'),
        importance: numberPayload(payload, 'importance', 5),
        tags: arrayPayload(payload, 'tags'),
      });
      ctx.emit('memory.created', { sessionId, ...result });
      break;
    }
    case 'memory.update': {
      if (!sessionId) return;
      const id = stringPayload(payload, 'id');
      const updates = ((payload as { updates?: unknown })?.updates || {}) as Record<string, unknown>;
      const result = await memoryUpdate(id, {
        content: typeof updates.content === 'string' ? updates.content : undefined,
        category: typeof updates.category === 'string' ? updates.category : undefined,
        importance: typeof updates.importance === 'number' ? updates.importance : undefined,
        tags: Array.isArray(updates.tags) ? updates.tags.filter((tag): tag is string => typeof tag === 'string') : undefined,
      });
      ctx.emit('memory.updated', {
        sessionId,
        id,
        updated: Boolean(result),
        memory: result?.memory,
        stats: result?.stats || emptyMemoryStats(),
      });
      break;
    }
    case 'memory.delete': {
      if (!sessionId) return;
      const id = stringPayload(payload, 'id');
      const deleted = await memoryDelete(id);
      ctx.emit('memory.deleted', { sessionId, id, deleted });
      break;
    }
    case 'memory.flush':
      if (sessionId) {
        const result = await memoryFlush();
        ctx.emit('memory.flushed', { sessionId, ...result });
      }
      break;
    case 'memory.maintenance.run':
      if (sessionId) {
        const result = await memoryMaintenanceRun();
        ctx.emit('memory.maintenance.ran', { sessionId, ...result });
      }
      break;
    case 'memory.snapshot.list':
      if (sessionId) {
        const result = await memorySnapshotList();
        ctx.emit('memory.snapshot.list', { sessionId, ...result });
      }
      break;
    case 'memory.snapshot.create':
      if (sessionId) {
        const reason = stringPayload(payload, 'reason') || undefined;
        const result = await memorySnapshotCreate(reason);
        ctx.emit('memory.snapshot.created', { sessionId, ...result });
      }
      break;
    case 'memory.snapshot.rollback':
      if (sessionId) {
        const version = numberPayload(payload, 'version', 0);
        const result = version > 0 ? await memorySnapshotRollback(version) : null;
        ctx.emit('memory.snapshot.rolledBack', {
          sessionId,
          ok: Boolean(result),
          ...(result || {
            snapshots: [] satisfies MemorySnapshotInfo[],
            stats: emptyMemoryStats(),
            error: 'memory_snapshot_not_found',
          }),
        });
      }
      break;
    case 'memory.hit.list':
      if (sessionId) {
        const runId = stringPayload(payload, 'runId') || undefined;
        const result = await memoryHitList({
          sessionId,
          runId,
          limit: numberPayload(payload, 'limit', 100),
        });
        ctx.emit('memory.hit.list', { sessionId, runId, hits: result.hits });
      }
      break;
    case 'memory.sessionSummary.get':
      if (sessionId) {
        const result = await memorySessionSummaryGet(sessionId);
        ctx.emit('memory.sessionSummary', { sessionId, ...result });
      }
      break;
    case 'memory.sessionSummaries.list':
      if (sessionId) {
        const result = await memorySessionSummariesList(numberPayload(payload, 'limit', 20));
        ctx.emit('memory.sessionSummaries.list', { sessionId, ...result });
      }
      break;
    case 'memory.profile.list':
      if (sessionId) {
        const result = await memoryProfileList();
        ctx.emit('memory.profile.list', { sessionId, files: result.files || [] });
      }
      break;
    case 'memory.profile.read': {
      if (!sessionId) return;
      const kind = stringPayload(payload, 'kind', 'project') as MemoryProfileFile['kind'];
      const result = await memoryProfileRead(kind);
      ctx.emit('memory.profile.read', {
        sessionId,
        file: result.file || defaultProfileFiles().find((item) => item.kind === kind) || defaultProfileFiles()[0],
      });
      break;
    }
    case 'memory.profile.update': {
      if (!sessionId) return;
      const kind = stringPayload(payload, 'kind', 'project') as MemoryProfileFile['kind'];
      const result = await memoryProfileUpdate(kind, stringPayload(payload, 'content'));
      ctx.emit('memory.profile.updated', { sessionId, file: result.file, files: result.files || [] });
      break;
    }
    case 'memory.pending.list':
      if (sessionId) {
        const result = await memoryPendingList(numberPayload(payload, 'limit', 100));
        ctx.emit('memory.pending.list', { sessionId, pending: result.pending || [] });
      }
      break;
    case 'memory.pending.approve': {
      if (!sessionId) return;
      const result = await memoryPendingApprove(stringPayload(payload, 'id'));
      ctx.emit('memory.pending.approved', { sessionId, ...result });
      break;
    }
    case 'memory.pending.reject':
      if (sessionId) {
        const result = await memoryPendingReject(stringPayload(payload, 'id'));
        ctx.emit('memory.pending.rejected', { sessionId, ...result });
      }
      break;
    default:
      break;
  }
}


async function switchActiveSessionIfNeeded(
  previousSessionId: string,
  sessions: SessionInfo[],
  ctx: {
    emit: (type: string, payload: unknown, options?: EmitOptions) => void;
    setMainSessionId: (sessionId: string) => void;
  },
) {
  const chat = useChatStore.getState();
  if (chat.activeSessionId !== previousSessionId && getMainSessionId() !== previousSessionId) return;
  const next = sessions.find((session) => session.id !== previousSessionId) || sessions[0];
  const nextId = next?.id || '';
  setMainSessionIdValue(nextId);
  ctx.setMainSessionId(nextId);
  if (!next) return;
  chat.initSession(next.id);
  chat.setActiveSession(next.id);
  const history = await sessionHistory(next.id).catch(() => []);
  ctx.emit('session.history', { sessionId: next.id, messages: history });
}



function storedEventToWsMessage(event: {
  sequence: number;
  timestamp: string;
  kind: string;
  payload: unknown;
}): WsMessage {
  return {
    id: `replay_${event.sequence}`,
    type: 'core.event',
    payload: event.payload,
    timestamp: Date.parse(event.timestamp) || Date.now(),
    sequence: event.sequence,
  };
}

function trimReplayBuffer(buffer: ReplayBufferItem[]) {
  const cutoff = Date.now() - REPLAY_TTL_MS;
  const trimmed = buffer
    .filter((item) => item.timestamp >= cutoff)
    .slice(-REPLAY_BUFFER_LIMIT);
  buffer.splice(0, buffer.length, ...trimmed);
}

function emitReplayBatch(
  lastSequence: number,
  emit: (type: string, payload: unknown, options?: EmitOptions) => void,
  buffer: ReplayBufferItem[],
  nextSequence: number,
) {
  trimReplayBuffer(buffer);
  const requestedSequence = Number.isFinite(lastSequence)
    ? Math.max(0, Math.floor(lastSequence))
    : 0;
  if (requestedSequence <= 0) {
    emit('replay.batch', { lastSequence: requestedSequence, messages: [] }, { buffer: false });
    return;
  }
  const oldest = buffer[0]?.sequence;
  const overflow =
    (oldest !== undefined && requestedSequence < oldest - 1)
    || (buffer.length === 0 && nextSequence !== requestedSequence + 1);
  if (overflow) {
    emit('replay.overflow', { lastSequence: requestedSequence, nextSequence }, { buffer: false });
    return;
  }
  emit(
    'replay.batch',
    {
      lastSequence: requestedSequence,
      messages: buffer
        .filter((item) => item.sequence > requestedSequence)
        .map((item) => item.message),
    },
    { buffer: false },
  );
}

function collectLocalReplayMessages(
  lastSequence: number,
  buffer: ReplayBufferItem[],
  nextSequence: number,
): WsMessage[] {
  trimReplayBuffer(buffer);
  const requestedSequence = Number.isFinite(lastSequence)
    ? Math.max(0, Math.floor(lastSequence))
    : 0;
  if (requestedSequence <= 0) return [];
  const oldest = buffer[0]?.sequence;
  const overflow =
    (oldest !== undefined && requestedSequence < oldest - 1)
    || (buffer.length === 0 && nextSequence !== requestedSequence + 1);
  if (overflow) return [];
  return buffer
    .filter((item) => item.sequence > requestedSequence)
    .map((item) => item.message);
}

function mergeReplayMessages(rustMessages: WsMessage[], localMessages: WsMessage[]): WsMessage[] {
  const merged = [...rustMessages, ...localMessages];
  merged.sort((left, right) => (left.sequence || 0) - (right.sequence || 0));
  const seen = new Set<number>();
  return merged.filter((message) => {
    const sequence = message.sequence;
    if (sequence === undefined) return true;
    if (seen.has(sequence)) return false;
    seen.add(sequence);
    return true;
  });
}

function plainPayload(payload: unknown): Record<string, unknown> {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return {};
  return payload as Record<string, unknown>;
}

function stringPayload(payload: unknown, key: string, fallback = ''): string {
  if (!payload || typeof payload !== 'object') return fallback;
  const value = (payload as Record<string, unknown>)[key];
  return typeof value === 'string' ? value : fallback;
}

function numberPayload(payload: unknown, key: string, fallback: number): number {
  if (!payload || typeof payload !== 'object') return fallback;
  const value = (payload as Record<string, unknown>)[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function clampNumber(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function booleanPayload(payload: unknown, key: string, fallback: boolean): boolean {
  if (!payload || typeof payload !== 'object') return fallback;
  const value = (payload as Record<string, unknown>)[key];
  return typeof value === 'boolean' ? value : fallback;
}

function arrayPayload(payload: unknown, key: string): string[] {
  if (!payload || typeof payload !== 'object') return [];
  const value = (payload as Record<string, unknown>)[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

function answersPayload(
  payload: unknown,
): Array<{ id: string; selected?: string[]; text?: string }> {
  if (!payload || typeof payload !== 'object') return [];
  const answers = (payload as { answers?: unknown }).answers;
  if (!Array.isArray(answers)) return [];
  return answers
    .map((item) => {
      if (!item || typeof item !== 'object') return null;
      const record = item as Record<string, unknown>;
      const id =
        typeof record.id === 'string'
          ? record.id
          : typeof record.questionId === 'string'
            ? record.questionId
            : '';
      if (!id) return null;
      const selected = Array.isArray(record.selected)
        ? record.selected.filter((entry): entry is string => typeof entry === 'string')
        : undefined;
      const text =
        typeof record.text === 'string'
          ? record.text
          : typeof record.answer === 'string'
            ? record.answer
            : undefined;
      return {
        id,
        ...(selected?.length ? { selected } : {}),
        ...(text ? { text } : {}),
      };
    })
    .filter((item): item is { id: string; selected?: string[]; text?: string } => Boolean(item));
}

function emptyMemoryStats() {
  return { total: 0, categories: {} as Record<string, number> };
}

function defaultProfileFiles(): MemoryProfileFile[] {
  const now = Date.now();
  return [
    {
      kind: 'identity',
      scope: 'global',
      path: 'profiles/identity.md',
      relativePath: 'profiles/identity.md',
      title: 'Identity',
      content: '',
      exists: false,
      updatedAt: now,
    },
    {
      kind: 'user',
      scope: 'global',
      path: 'profiles/user.md',
      relativePath: 'profiles/user.md',
      title: 'User',
      content: '',
      exists: false,
      updatedAt: now,
    },
    {
      kind: 'project',
      scope: 'project',
      path: 'profiles/project.md',
      relativePath: 'profiles/project.md',
      title: 'Project',
      content: '',
      exists: false,
      updatedAt: now,
    },
    {
      kind: 'agents',
      scope: 'project',
      path: 'profiles/agents.md',
      relativePath: 'profiles/agents.md',
      title: 'Agents',
      content: '',
      exists: false,
      updatedAt: now,
    },
  ];
}

function objectPayload(payload: unknown, key: string): Record<string, unknown> {
  if (!payload || typeof payload !== 'object') return {};
  const value = (payload as Record<string, unknown>)[key];
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
}

function sessionListOptions(payload: unknown): {
  includeArchived?: boolean;
  search?: string;
  limit?: number;
} {
  return {
    includeArchived: booleanPayload(payload, 'includeArchived', false),
    search: stringPayload(payload, 'search') || undefined,
    limit: numberPayload(payload, 'limit', 500),
  };
}

export function getPayloadSessionId(payload: unknown): string | undefined {
  if (!payload || typeof payload !== 'object') return undefined;
  const value = (payload as { sessionId?: unknown }).sessionId;
  return typeof value === 'string' ? value : undefined;
}

function loadConfig(): AppConfig {
  try {
    const raw = window.localStorage.getItem(LOCAL_CONFIG_KEY);
    if (raw) return mergeConfig(defaultConfig, JSON.parse(raw) as Partial<AppConfig>);
  } catch {
    /* keep defaults */
  }
  return defaultConfig;
}

function saveConfig(config: AppConfig): void {
  try {
    window.localStorage.setItem(LOCAL_CONFIG_KEY, JSON.stringify(config));
  } catch {
    /* storage may be unavailable in tests */
  }
}

export function mergeConfig(base: AppConfig, partial: Partial<AppConfig>): AppConfig {
  return {
    ...base,
    ...partial,
    provider: { ...base.provider, ...partial.provider },
    permissions: { ...base.permissions, ...partial.permissions },
    theme: { ...base.theme, ...partial.theme },
    memory: partial.memory ? ({ ...base.memory, ...partial.memory } as AppConfig['memory']) : base.memory,
    sandbox: { ...base.sandbox, ...partial.sandbox } as AppConfig['sandbox'],
    subAgents: { ...base.subAgents, ...partial.subAgents } as AppConfig['subAgents'],
    agentLoop: { ...base.agentLoop, ...partial.agentLoop },
    modelRouting: { ...base.modelRouting, ...partial.modelRouting } as AppConfig['modelRouting'],
    webSearch: { ...base.webSearch, ...partial.webSearch } as AppConfig['webSearch'],
    skills: { ...base.skills, ...partial.skills } as AppConfig['skills'],
    plugins: { ...base.plugins, ...partial.plugins } as AppConfig['plugins'],
    gateway: { ...base.gateway, ...partial.gateway } as AppConfig['gateway'],
    workspace: { ...base.workspace, ...partial.workspace },
    contextManagement: { ...base.contextManagement, ...partial.contextManagement },
    governance: { ...base.governance, ...partial.governance } as AppConfig['governance'],
  };
}

export const defaultConfig: AppConfig = {
  locale: 'zh-CN',
  provider: {
    type: 'openai',
    model: 'gpt-4o',
    maxTokens: 4096,
    temperature: 0.7,
  },
  permissions: {
    default: 'confirm',
    mode: 'default',
    rules: [
      { pattern: 'file_read', level: 'allow' },
      { pattern: 'search', level: 'allow' },
      { pattern: 'exec', level: 'confirm' },
    ],
    whitelist: [],
  },
  theme: {
    primaryColor: '#00D4AA',
    secondaryColor: '#7C3AED',
    accentColor: '#F59E0B',
    codeTheme: 'dark',
  },
  systemPrompt: '',
  maxContextTokens: 128000,
  disabledTools: [],
  memory: {
    backend: 'postgres',
    maxEntries: 1000,
    cleanupImportanceThreshold: 3,
    postgres: {
      schema: 'public',
      tablePrefix: 'xy',
      vectorDimensions: 1536,
    },
  },
  sandbox: {
    enabled: true,
    defaultMode: 'workspace-write',
    osSandbox: {
      enabled: false,
      networkOff: true,
    },
    cwdAllowlist: [],
    autoForRiskyCommands: true,
    container: {
      enabled: false,
      runner: 'docker',
      image: 'node:20-bookworm',
      network: 'off',
    },
  },
  subAgents: {
    configDir: 'capabilities/agents',
    watchdog: {
      silentMs: 120_000,
      noProgressIterations: 8,
      maxRetries: 3,
    },
    models: {},
  },
  agentLoop: {
    maxAutoFixPasses: 5,
  },
  modelRouting: {
    enabled: true,
    auxiliaryModels: {},
  },
  webSearch: {
    enabled: false,
    provider: 'brave',
  },
  skills: {
    enabled: true,
    configDir: 'capabilities/skills',
  },
  plugins: {
    enabled: true,
    directories: ['plugins'],
  },
  workspace: {
    recentDirs: [],
  },
  // 通信方式写死：桌面端走 Electron IPC，不暴露网关配置。
  gateway: { ...DESKTOP_GATEWAY_DEFAULT },
  contextManagement: {
    autoCompactThreshold: 0.85,
  },
};

export const defaultToolCatalog: ToolCatalogPayload = {
  tools: [
    {
      name: 'file_read',
      description: 'Read files through Rust governance',
      category: 'files',
      source: 'builtin',
      isWrite: false,
      isExecute: false,
      isConcurrent: true,
      aliases: ['read_file'],
    },
    {
      name: 'search',
      description: 'Search project text',
      category: 'search',
      source: 'builtin',
      isWrite: false,
      isExecute: false,
      isConcurrent: true,
      aliases: ['rg'],
    },
    {
      name: 'exec',
      description: 'Run a guarded process',
      category: 'runtime',
      source: 'builtin',
      isWrite: true,
      isExecute: true,
      isConcurrent: false,
      aliases: ['shell'],
    },
  ],
};

const CAPABILITY_PREVIEW_WARNING = 'Browser preview derives capabilities from local configuration.';

function configuredCapabilityFlag(value: boolean | undefined, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function configuredCapabilityCost(
  cost: NonNullable<NonNullable<AppConfig['modelRouting']>['capabilities']>[string]['cost'] | undefined,
): { input: number; output: number; cachedInput?: number } | null {
  if (!cost || typeof cost.input !== 'number' || typeof cost.output !== 'number') return null;
  return {
    input: cost.input,
    output: cost.output,
    ...(typeof cost.cachedInput === 'number' ? { cachedInput: cost.cachedInput } : {}),
  };
}

function createConfiguredCapabilityRoute(
  task: string,
  settings: { provider?: string; model?: string; protocol?: string; maxTokens?: number },
  config: AppConfig,
): ConfigCapabilitiesPayload['models'][number] {
  const provider = settings.provider || config.provider?.type || 'configured-provider';
  const model = settings.model || config.provider?.model || 'configured-model';
  const protocol = settings.protocol || 'openai';
  const overrides =
    config.modelRouting?.capabilities?.[task] ||
    config.modelRouting?.capabilities?.[`${provider}:${model}`] ||
    {};
  const rateLimit =
    overrides.rateLimit ||
    config.modelRouting?.rateLimits?.[task] ||
    config.modelRouting?.rateLimits?.[provider];
  const maxOutput = overrides.maxOutput || settings.maxTokens || config.provider?.maxTokens;
  const warnings = [CAPABILITY_PREVIEW_WARNING];

  return {
    task,
    provider,
    model,
    capabilities: {
      provider,
      model,
      stream: configuredCapabilityFlag(overrides.stream, true),
      toolCall: configuredCapabilityFlag(overrides.toolCall, protocol !== 'anthropic'),
      reasoning: configuredCapabilityFlag(overrides.reasoning, false),
      thinkingDisable: configuredCapabilityFlag(overrides.thinkingDisable, true),
      jsonMode: configuredCapabilityFlag(overrides.jsonMode, protocol !== 'anthropic'),
      usage: configuredCapabilityFlag(overrides.usage, false),
      vision: false,
      maxContext: overrides.maxContext || config.maxContextTokens,
      maxOutput,
      ...(rateLimit ? { rateLimit } : {}),
      cost: configuredCapabilityCost(overrides.cost),
      source: ['config', 'preview-adapter'],
      warnings,
    },
    check: {
      ok: Boolean(model),
      warnings,
      errors: model ? [] : ['Model is not configured.'],
    },
  };
}

export function createConfiguredCapabilities(config: AppConfig): ConfigCapabilitiesPayload {
  const routes: ConfigCapabilitiesPayload['models'] = [
    createConfiguredCapabilityRoute('main', config.provider, config),
  ];
  const auxiliary = config.modelRouting?.enabled === false ? undefined : config.modelRouting?.auxiliaryModels;
  Object.entries(auxiliary || {}).forEach(([task, settings]) => {
    if (!settings || (!settings.model && !settings.provider)) return;
    routes.push(createConfiguredCapabilityRoute(task, settings, config));
  });
  return { models: routes };
}

export const defaultExtensions: ExtensionsListPayload = {
  skills: [],
  plugins: [],
  totals: {
    skills: 0,
    plugins: 0,
    enabledPlugins: 0,
    tools: defaultToolCatalog.tools.length,
    providers: 0,
    hooks: 0,
    uiPanels: 0,
  },
};
