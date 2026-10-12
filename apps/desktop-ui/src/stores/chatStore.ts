/**
 * chat-store.ts - 聊天状态仓库
 *
 * 用途：管理每个会话的消息、流式文本、工具调用和子代理任务状态。
 *
 * ── 架构 ──────────────────────────────────────────────────────────────
 *
 * 单个用户问题触发 Agent 的一轮（turn）处理。该轮中 Agent 可能迭代多次
 *（思考 → 调用工具 → 思考 → 调用工具 → …）。所有迭代都属于同一次助手回复，
 * 在 UI 中应显示在同一头像下。
 *
 * 为此引入 **Segments** —— 有序内容块列表（按事件到达顺序落盘）：
 *
 *   [text_segment, tool_call_segment, text_segment, tool_call_segment, …]
 *
 * ── 事件流 ────────────────────────────────────────────────────────────
 *
 *     text / reasoning  → 立刻写入 turnSegments（追加到末尾 text，或新建）
 *     tool_call         → 直接追加工具段（前面的字已经在 segments 里）
 *     tool_result       → 更新对应工具段
 *     message / done    → 收尾、去重、提交，不再承担「把缓冲刷成时间线」的职责
 *
 *   streamingText 是当前增长中 text 段的镜像，不是第二条时间线。
 *
 * ── 并发 ───────────────────────────────────────────────────────────────
 *
 * 每个会话拥有独立的 state slice，以 `sessionId` 为键。
 * 事件在 `useWebSocket.ts` 中按 `sessionId` 路由。
 */

import { create } from 'zustand';

import type { AgentEvent, TokenUsage } from '../protocol';
import { notifyChatError, notifyChatInfo } from '../utils/chatAlert';
import { hasTokenUsage, mergeTurnUsage, normalizeTokenUsage } from '../utils/tokenUsage';
import type {
  NestedTaskSnapshot,
  NestedTaskEvent,
  NestedTaskResult,
  RunPlan,
  SubAgentTaskLedger,
  SubAgentTimelineEvent,
} from '../protocol';

// ---------------------------------------------------------------------------
// Segment types
// ---------------------------------------------------------------------------

/** 正文段：可含 thinking 旁路。 */
export interface TextSegment {
  type: 'text';
  id: string;
  content: string;
  thinking?: string;
}

/** 工具调用段：调用中 / 完成 / 错误，可挂 streamBefore。 */
export interface ToolCallSegment {
  type: 'tool_call';
  id: string;
  name: string;
  args: Record<string, unknown>;
  rawArgs: string;
  status: 'calling' | 'done' | 'error';
  result?: string;
  /** 流式 file_write 覆盖统计用的已有文件内容（'' = 新建）。 */
  streamBefore?: string;
  /** 引擎流式预览给出的 +/-（补丁尚未就绪时的翻牌数字）。 */
  streamAdd?: number;
  streamDel?: number;
}

/** 子 Agent 派发 / 结果段。 */
export interface AgentDelegateSegment {
  type: 'agent_delegate' | 'agent_result';
  id: string;
  name: string;
  content?: string;
  args?: Record<string, unknown>;
  rawArgs?: string;
  status: 'calling' | 'done' | 'error' | 'pending' | 'cancelled';
  result?: string;
  title?: string;
  payload?: Record<string, unknown> & {
    taskId?: string;
    agentId?: string;
    ledger?: SubAgentTaskLedger;
    timelineEvent?: SubAgentTimelineEvent;
  };
}

/** 沙箱相关工具事件段。 */
export interface SandboxEventSegment {
  type: 'sandbox_event';
  id: string;
  name: string;
  content?: string;
  args?: Record<string, unknown>;
  rawArgs?: string;
  status: 'calling' | 'done' | 'error' | 'pending' | 'cancelled';
  result?: string;
  title?: string;
  payload?: Record<string, unknown>;
}

/** 记忆命中提示段（search_memory / 注入反馈）。 */
export interface MemoryHitSegment {
  type: 'memory_hit';
  id: string;
  status?: string;
  title?: string;
  payload: {
    query?: string;
    source?: string;
    count?: number;
    limit?: number;
    memoryIds?: string[];
    categories?: string[];
    injected?: boolean;
  };
}

/** 检查点 / 权限 / 资源 / 系统通知段。 */
export interface CheckpointSegment {
  type: 'checkpoint' | 'permission' | 'resource_usage' | 'system_notice';
  id: string;
  status?: string;
  title?: string;
  payload?: Record<string, unknown>;
}

/** 嵌套子 Agent 运行卡片：含 children 时间线。 */
export interface AgentRunSegment {
  type: 'agent_run';
  id: string;
  parentToolCallId?: string;
  taskId?: string;
  agentId: string;
  agentName: string;
  task: string;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
  children: Segment[];
  summary?: string;
  startedAt: number;
  endedAt?: number;
  readScope?: string[];
  writeScope?: string[];
  background?: boolean;
}

/** 控制任务生命周期事件 / 结果段。 */
export interface TaskEventSegment {
  type: 'task_event' | 'task_result';
  id: string;
  status?: string;
  title?: string;
  payload?: Record<string, unknown>;
}

/** 时间线有序内容块联合类型。 */
export type Segment =
  | TextSegment
  | ToolCallSegment
  | AgentDelegateSegment
  | AgentRunSegment
  | SandboxEventSegment
  | MemoryHitSegment
  | CheckpointSegment
  | TaskEventSegment;

// ---------------------------------------------------------------------------
// Chat message
// ---------------------------------------------------------------------------

/** 会话气泡：可挂 segments / usage / 附件。 */
export interface ChatMessage {
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: number;
  turnId?: string;
  isError?: boolean;
  usage?: TokenUsage;
  segments?: Segment[];
  workStartedAt?: number;
  workEndedAt?: number;
  attachments?: Array<{
    kind: 'image' | 'file';
    name: string;
    mimeType: string;
    sizeBytes: number;
    dataUrl: string;
  }>;
}

// ---------------------------------------------------------------------------
// Permission
// ---------------------------------------------------------------------------

/** 等待用户确认的工具权限请求。 */
export interface PendingPermission {
  sessionId: string;
  requestId: string;
  toolName: string;
  args: Record<string, unknown>;
  reason: string;
  runId?: string;
  approvalDigest: string;
}

/** 等待用户回答的 ask_user 结构化提问。 */
export interface PendingQuestion {
  sessionId: string;
  requestId: string;
  questions: Array<{
    id: string;
    prompt: string;
    options?: string[];
    allow_multiple?: boolean;
  }>;
  runId?: string;
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/** 单会话切片：消息、流式缓冲、子 Agent / 任务状态。 */
export interface SessionState {
  messages: ChatMessage[];
  streamingText: string;
  reasoningText: string;
  turnSegments: Segment[];
  pendingUsage?: TokenUsage;
  isProcessing: boolean;
  /** 本轮首次输出正文时开始计时（Worked for） */
  turnWorkStartedAt?: number;
  /** 乐观更新的用户消息索引，用于发送失败时回滚。 */
  pendingMessageIndex: number | null;
  pendingPermission: PendingPermission | null;
  pendingQuestion: PendingQuestion | null;
  subAgentLedgers: Record<string, SubAgentTaskLedger>;
  subAgentTimelines: Record<string, SubAgentTimelineEvent[]>;
  taskSnapshots: Record<string, NestedTaskSnapshot>;
  runPlan: RunPlan | null;
}

/** 侧栏「新建」进入的本地准备态；首条消息发出前不创建服务端会话。 */
export type ComposeDraft =
  | { kind: 'recent' }
  | { kind: 'project'; projectDir: string };

/** 准备态本地会话 id（不进侧栏列表）。 */
export const COMPOSE_DRAFT_SESSION_ID = 'local:compose-draft';

interface ChatStore {
  sessions: Record<string, SessionState>;
  activeSessionId: string | null;
  /** 新建会话准备态：点哪边就在哪边准备，多次点击只替换准备目标。 */
  composeDraft: ComposeDraft | null;

  initSession: (sessionId: string) => void;
  removeSession: (sessionId: string) => void;
  setActiveSession: (sessionId: string) => void;
  setComposeDraft: (draft: ComposeDraft | null) => void;
  /** 进入准备态并清空本地草稿会话内容。 */
  beginComposeDraft: (draft: ComposeDraft) => void;
  applyEvent: (sessionId: string, event: AgentEvent) => void;
  setPendingPermission: (sessionId: string, perm: PendingPermission | null) => void;
  setPendingQuestion: (sessionId: string, question: PendingQuestion | null) => void;
  addUserMessage: (sessionId: string, content: string, attachments?: ChatMessage['attachments']) => void;
  addAgentReply: (sessionId: string, userMessage: string, content: string) => void;
  addErrorMessage: (sessionId: string, content: string, opts?: { title?: string }) => void;
  addSystemMessage: (sessionId: string, content: string) => void;
  upsertSubAgentLedger: (sessionId: string, ledger: SubAgentTaskLedger) => void;
  setSubAgentTimeline: (
    sessionId: string,
    taskId: string,
    timeline: SubAgentTimelineEvent[],
    ledger?: SubAgentTaskLedger,
  ) => void;
  upsertTaskSnapshot: (sessionId: string, snapshot: NestedTaskSnapshot) => void;
  setRunPlan: (sessionId: string, plan: RunPlan | null) => void;
  addTaskEvent: (sessionId: string, event: NestedTaskEvent) => void;
  addTaskResult: (sessionId: string, result: NestedTaskResult) => void;
  clearHistory: (sessionId: string) => void;
  /** 编辑重发前乐观截断到目标用户消息之前，避免旧回复与新流交错。 */
  prepareInlineRewind: (sessionId: string, turnId: string) => void;
  loadMessages: (sessionId: string, messages: ChatMessage[]) => void;
  syncProcessingState: (sessionId: string, isProcessing: boolean) => void;
  getSession: (sessionId: string) => SessionState;
  /** 回滚乐观更新的用户消息，用于 WebSocket 发送失败时清理幽灵消息。 */
  rollbackOptimisticMessage: (sessionId: string) => void;
}

/** 新建空会话切片。 */
const emptySession = (): SessionState => ({
  messages: [],
  streamingText: '',
  reasoningText: '',
  turnSegments: [],
  pendingUsage: undefined,
  turnWorkStartedAt: undefined,
  isProcessing: false,
  pendingMessageIndex: null,
  pendingPermission: null,
  pendingQuestion: null,
  subAgentLedgers: {},
  subAgentTimelines: {},
  taskSnapshots: {},
  runPlan: null,
});

let _segId = 0;
/** 本页唯一 segment id。 */
function segId(): string {
  return `seg_${Date.now()}_${++_segId}`;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** AgentEvent.tool_call 原始结构 → ToolCallSegment。 */
function normaliseToolCall(raw: Record<string, unknown>): ToolCallSegment {
  const id = String(raw.id ?? '');
  const func = raw.function as Record<string, unknown> | undefined;
  const name = String(func?.name ?? raw.name ?? 'unknown');
  const rawArgsStr = String(func?.arguments ?? raw.arguments ?? '');
  let args: Record<string, unknown> = {};
  if (rawArgsStr) {
    try {
      const parsed = JSON.parse(rawArgsStr);
      if (typeof parsed === 'object' && parsed !== null) args = parsed;
    } catch {
      /* not JSON */
    }
  }
  return { type: 'tool_call', id, name, args, rawArgs: rawArgsStr, status: 'calling' };
}

/** 时间线折叠标题：按工具名抽关键 path/command/pattern。 */
export function summariseArgs(tc: ToolCallSegment): string {
  const { name, args } = tc;
  if (name === 'exec') return String(args.command ?? args.cmd ?? tc.rawArgs.substring(0, 80));
  if (name === 'file_read') return String(args.path ?? args.file_path ?? '');
  if (name === 'file_write') return String(args.path ?? args.file_path ?? '');
  if (name === 'file_edit') return String(args.path ?? args.file_path ?? '');
  if (name === 'list_files') return String(args.path ?? '.');
  if (name === 'search') return String(args.pattern ?? args.regex ?? `${args.path ?? '.'}`);
  if (name === 'glob') return String(args.pattern ?? '');
  const values = Object.values(args).filter((v) => typeof v === 'string' && v.length > 0);
  if (values.length > 0) {
    const first = String(values[0]);
    return first.length > 60 ? first.substring(0, 57) + '...' : first;
  }
  return '';
}

/** tool_call / sandbox_event 可挂 tool_result。 */
function isToolLikeSegment(seg: Segment): seg is ToolCallSegment | SandboxEventSegment {
  return seg.type === 'tool_call' || seg.type === 'sandbox_event';
}

/** 把 ledger 状态 / 时间线合入匹配的 agent_run 段。 */
function attachLedgerToSegment(segment: Segment, ledger: SubAgentTaskLedger): Segment {
  if (segment.type !== 'agent_run') return segment;
  if (segment.taskId && segment.taskId !== ledger.taskId) return segment;
  const status: AgentRunSegment['status'] =
    ledger.status === 'completed'
      ? 'completed'
      : ledger.status === 'failed' || ledger.status === 'blocked'
        ? 'failed'
        : ledger.status === 'cancelled'
          ? 'cancelled'
          : 'running';
  return {
    ...segment,
    taskId: ledger.taskId || segment.taskId,
    agentName: ledger.agentName || segment.agentName,
    status,
    summary: ledger.result?.conclusion || segment.summary,
    endedAt:
      status === 'running' ? segment.endedAt : ledger.completedAt || segment.endedAt || Date.now(),
    children:
      ledger.timeline && ledger.timeline.length
        ? mapTimelineToChildSegments(ledger.timeline)
        : segment.children,
  };
}

/** 将 ledger 绑定到匹配的 agent_run（优先 parentToolCallId + nestedIndex）。 */
function bindLedgerToAgentRuns(segments: Segment[] | undefined, ledger: SubAgentTaskLedger): Segment[] | undefined {
  if (!segments?.length) return segments;
  const meta = ledger.task?.metadata || {};
  const parentToolCallId =
    typeof meta.parentToolCallId === 'string' ? meta.parentToolCallId : undefined;
  const nestedIndex = typeof meta.nestedIndex === 'number' ? meta.nestedIndex : undefined;

  const matchScore = (segment: Extract<Segment, { type: 'agent_run' }>): number => {
    if (segment.taskId === ledger.taskId) return 100;
    if (segment.taskId) return -1;
    if (parentToolCallId && segment.parentToolCallId === parentToolCallId) {
      if (nestedIndex !== undefined && segment.id.endsWith(`_run_${nestedIndex}`)) return 90;
      if (segment.agentId === ledger.agentId) return 70;
      return 50;
    }
    if (segment.agentId === ledger.agentId) return 20;
    return -1;
  };

  let bestIdx = -1;
  let bestScore = -1;
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    if (segment.type !== 'agent_run') continue;
    const score = matchScore(segment);
    if (score > bestScore) {
      bestScore = score;
      bestIdx = i;
    }
  }
  if (bestIdx < 0 || bestScore < 0) {
    return segments.map((segment) => attachLedgerToSegment(segment, ledger));
  }
  return segments.map((segment, index) => {
    if (index === bestIdx && segment.type === 'agent_run') {
      return attachLedgerToSegment({ ...segment, taskId: ledger.taskId }, ledger);
    }
    return attachLedgerToSegment(segment, ledger);
  });
}

/** 子 Agent 时间线事件 → 工具调用字段（含 streamArgs / result）。 */
function parseToolCallFromTimelineEvent(ev: SubAgentTimelineEvent): {
  id: string;
  name: string;
  args: Record<string, unknown>;
  rawArgs: string;
  status: 'calling' | 'done' | 'error';
  result?: string;
  streamBefore?: string;
} {
  const call = ev.payload?.call as
    | { id?: string; function?: { name?: string; arguments?: string } }
    | undefined;
  const resultPayload = ev.payload?.result as
    | { content?: string; isError?: boolean; toolName?: string }
    | string
    | undefined;
  const rawArgs =
    typeof call?.function?.arguments === 'string' ? call.function.arguments : '';
  let args: Record<string, unknown> = {};
  if (
    ev.payload?.streamArgs &&
    typeof ev.payload.streamArgs === 'object' &&
    !Array.isArray(ev.payload.streamArgs)
  ) {
    args = ev.payload.streamArgs as Record<string, unknown>;
  } else if (rawArgs) {
    try {
      const parsed = JSON.parse(rawArgs);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        args = parsed as Record<string, unknown>;
      }
    } catch {
      /* keep empty args */
    }
  } else if (
    ev.payload?.args &&
    typeof ev.payload.args === 'object' &&
    !Array.isArray(ev.payload.args)
  ) {
    args = ev.payload.args as Record<string, unknown>;
  }

  const name =
    (typeof call?.function?.name === 'string' && call.function.name) ||
    (typeof resultPayload === 'object' && resultPayload?.toolName) ||
    (typeof ev.title === 'string' && ev.title) ||
    (typeof ev.payload?.toolName === 'string' && ev.payload.toolName) ||
    (typeof ev.payload?.name === 'string' && ev.payload.name) ||
    'tool';

  const resultContent =
    typeof resultPayload === 'string'
      ? resultPayload
      : typeof resultPayload?.content === 'string'
        ? resultPayload.content
        : undefined;
  const isError =
    typeof resultPayload === 'object' && resultPayload
      ? resultPayload.isError === true
      : ev.status === 'error';
  const status: 'calling' | 'done' | 'error' = isError
    ? 'error'
    : resultContent !== undefined || ev.status === 'done'
      ? 'done'
      : 'calling';

  return {
    id: (typeof call?.id === 'string' && call.id) || ev.id,
    name,
    args,
    rawArgs: rawArgs || JSON.stringify(args),
    status,
    result: resultContent,
    streamBefore:
      typeof ev.payload?.streamBefore === 'string' ? ev.payload.streamBefore : undefined,
  };
}

/** 子 Agent timeline 事件列表 → agent_run.children 段数组。 */
function mapTimelineToChildSegments(
  timeline: SubAgentTimelineEvent[],
): Segment[] {
  const out: Segment[] = [];
  for (const ev of timeline) {
    if (ev.type === 'tool_call') {
      const parsed = parseToolCallFromTimelineEvent(ev);
      out.push({
        type: 'tool_call',
        id: parsed.id,
        name: parsed.name,
        args: parsed.args,
        rawArgs: parsed.rawArgs,
        status: parsed.status,
        result: parsed.result,
        streamBefore: parsed.streamBefore,
      });
      continue;
    }
    if (ev.type === 'reasoning' || ev.type === 'text') {
      const content = String(ev.payload?.content || ev.payload?.text || '');
      out.push({
        type: 'text',
        id: ev.id,
        content: ev.type === 'text' ? content : '',
        thinking: ev.type === 'reasoning' ? content : undefined,
      });
      continue;
    }
    if (ev.type === 'progress') {
      const summary = String(ev.payload?.summary || ev.payload?.stepTitle || ev.title || '').trim();
      if (!summary) continue;
      out.push({ type: 'text', id: ev.id, content: summary });
      continue;
    }
    out.push({
      type: 'text',
      id: ev.id,
      content: String(ev.payload?.summary || ev.payload?.content || ev.type),
    });
  }
  return out;
}

/** 文件读写工具结果在时间线折叠，避免刷屏。 */
export function shouldHideToolResult(toolName: string): boolean {
  return (
    toolName === 'file_read' ||
    toolName === 'file_write' ||
    toolName === 'file_edit'
  );
}

// ---------------------------------------------------------------------------
// Store - 聊天状态仓库实现
// ---------------------------------------------------------------------------

/** 聊天 Zustand store；useWebSocket 写入，ChatView 订阅。 */
export const useChatStore = create<ChatStore>((set, get) => ({
  sessions: {},
  activeSessionId: null,
  composeDraft: null,

  initSession: (sessionId) =>
    set((s) => ({
      sessions: s.sessions[sessionId] ? s.sessions : { ...s.sessions, [sessionId]: emptySession() },
      activeSessionId: s.activeSessionId || sessionId,
    })),

  removeSession: (sessionId) =>
    set((s) => {
      const { [sessionId]: _, ...rest } = s.sessions;
      return {
        sessions: rest,
        activeSessionId:
          s.activeSessionId === sessionId ? Object.keys(rest)[0] || null : s.activeSessionId,
      };
    }),

  setActiveSession: (sessionId) =>
    set((s) => ({
      activeSessionId: sessionId,
      composeDraft: sessionId === COMPOSE_DRAFT_SESSION_ID ? s.composeDraft : null,
    })),

  setComposeDraft: (draft) => set({ composeDraft: draft }),

  beginComposeDraft: (draft) =>
    set({
      composeDraft: draft,
      activeSessionId: COMPOSE_DRAFT_SESSION_ID,
      sessions: {
        ...get().sessions,
        [COMPOSE_DRAFT_SESSION_ID]: emptySession(),
      },
    }),

  addUserMessage: (sessionId, content, attachments) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const newIndex = session.messages.length;
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            isProcessing: true,
            pendingMessageIndex: newIndex,
            streamingText: '',
            reasoningText: '',
            turnSegments: [],
            pendingUsage: undefined,
            turnWorkStartedAt: undefined,
            messages: [...session.messages, { role: 'user', content, timestamp: Date.now(), attachments: attachments && attachments.length > 0 ? attachments : undefined }],
          },
        },
      };
    }),

  addAgentReply: (sessionId, userMessage, content) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const last = session.messages[session.messages.length - 1];
      const trimmed = (content || '').trim();
      // 已与末条助手正文相同则跳过，避免事件重复落两条气泡。
      if (
        last?.role === 'assistant' &&
        trimmed &&
        (last.content || '').trim() === trimmed
      ) {
        return {
          sessions: {
            ...s.sessions,
            [sessionId]: {
              ...session,
              isProcessing: false,
              streamingText: '',
              reasoningText: '',
              turnSegments: [],
              pendingUsage: undefined,
            },
          },
        };
      }
      const shouldAppendUser =
        !!userMessage && !(last?.role === 'user' && last.content === userMessage);
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            isProcessing: false,
            streamingText: '',
            reasoningText: '',
            turnSegments: [],
            pendingUsage: undefined,
            messages: [
              ...session.messages,
              ...(shouldAppendUser
                ? [{ role: 'user' as const, content: userMessage, timestamp: Date.now() }]
                : []),
              {
                role: 'assistant',
                content,
                timestamp: Date.now(),
                segments: [{ type: 'text', id: segId(), content }],
              },
            ],
          },
        },
      };
    }),

  addErrorMessage: (sessionId, content, opts) => {
    const text = humanizeTimelineError(content);
    const isAbort = /\b(aborted|cancelled|canceled|user aborted|已停止)\b/i.test(text);
    const display = isAbort
      ? localeText('已停止本次运行。', 'Run stopped.')
      : text.replace(/^错误:\s*/i, '').replace(/^Error:\s*/i, '');
    if (isAbort) notifyChatInfo(display);
    else notifyChatError(display, opts?.title);
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            isProcessing: false,
            streamingText: '',
            reasoningText: '',
            turnSegments: [],
            pendingUsage: undefined,
          },
        },
      };
    });
  },

  addSystemMessage: (sessionId, content) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const text = content.trim();
      if (!text) return s;
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            messages: [
              ...session.messages,
              {
                role: 'assistant',
                content: text,
                timestamp: Date.now(),
                segments: [{ type: 'text', id: segId(), content: text }],
              },
            ],
          },
        },
      };
    }),

  upsertSubAgentLedger: (sessionId, ledger) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const mappedMessages = session.messages.map((message) => ({
        ...message,
        segments: bindLedgerToAgentRuns(message.segments, ledger),
      }));
      const turnSegments = bindLedgerToAgentRuns(session.turnSegments, ledger) || [];
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            subAgentLedgers: {
              ...session.subAgentLedgers,
              [ledger.taskId]: ledger,
            },
            subAgentTimelines: {
              ...session.subAgentTimelines,
              [ledger.taskId]: ledger.timeline || session.subAgentTimelines[ledger.taskId] || [],
            },
            messages: mappedMessages,
            turnSegments,
          },
        },
      };
    }),

  setSubAgentTimeline: (sessionId, taskId, timeline, ledger) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const nextLedger =
        ledger ||
        session.subAgentLedgers[taskId] ||
        ({
          taskId,
          agentId: '',
          agentName: '',
          task: { agentId: '', task: '' },
          status: 'running',
          startedAt: Date.now(),
          updatedAt: Date.now(),
          timeline,
        } as SubAgentTaskLedger);
      const withTimeline = { ...nextLedger, taskId, timeline };
      const turnSegments =
        bindLedgerToAgentRuns(
          session.turnSegments.map((segment) => {
            if (segment.type === 'agent_run' && segment.taskId === taskId) {
              return { ...segment, children: mapTimelineToChildSegments(timeline) };
            }
            return segment;
          }),
          withTimeline,
        ) || [];
      // After bind, ensure children are on the matched run
      const patchChildren = (segments: Segment[] | undefined): Segment[] | undefined =>
        segments?.map((segment) =>
          segment.type === 'agent_run' && segment.taskId === taskId
            ? { ...segment, children: mapTimelineToChildSegments(timeline) }
            : segment,
        );
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            subAgentLedgers: ledger
              ? { ...session.subAgentLedgers, [taskId]: ledger }
              : session.subAgentLedgers[taskId]
                ? session.subAgentLedgers
                : { ...session.subAgentLedgers, [taskId]: withTimeline },
            subAgentTimelines: {
              ...session.subAgentTimelines,
              [taskId]: timeline,
            },
            turnSegments: patchChildren(turnSegments) || [],
            messages: session.messages.map((message) => ({
              ...message,
              segments: patchChildren(bindLedgerToAgentRuns(message.segments, withTimeline)),
            })),
          },
        },
      };
    }),

  upsertTaskSnapshot: (sessionId, snapshot) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            taskSnapshots: {
              ...session.taskSnapshots,
              [snapshot.task.taskId]: snapshot,
            },
          },
        },
      };
    }),

  setRunPlan: (sessionId, plan) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      return { sessions: { ...s.sessions, [sessionId]: { ...session, runPlan: plan } } };
    }),

  addTaskEvent: (sessionId, event) =>
    set((s) => {
      // 任务生命周期事件不再插入时间线居中条；子 Agent 进度看工具行/任务面板。
      const session = s.sessions[sessionId] || emptySession();
      void event;
      return { sessions: { ...s.sessions, [sessionId]: session } };
    }),

  addTaskResult: (sessionId, result) =>
    set((s) => {
      // 任务完成/失败结论不插居中系统条；失败细节由 agent 气泡或任务面板承载。
      const session = s.sessions[sessionId] || emptySession();
      void result;
      return { sessions: { ...s.sessions, [sessionId]: session } };
    }),

  applyEvent: (sessionId, event) => {
    notifyAgentEventAlert(event);
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const updated = applyAgentEvent(session, event);
      return { sessions: { ...s.sessions, [sessionId]: updated } };
    });
  },

  setPendingPermission: (sessionId, perm) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      return { sessions: { ...s.sessions, [sessionId]: { ...session, pendingPermission: perm } } };
    }),

  setPendingQuestion: (sessionId, question) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      return { sessions: { ...s.sessions, [sessionId]: { ...session, pendingQuestion: question } } };
    }),

  clearHistory: (sessionId) =>
    set((s) => ({
      sessions: { ...s.sessions, [sessionId]: emptySession() },
    })),

  prepareInlineRewind: (sessionId, turnId) =>
    set((s) => {
      const session = s.sessions[sessionId];
      if (!session) return s;
      const idx = session.messages.findIndex(
        (message) => message.role === 'user' && message.turnId === turnId,
      );
      if (idx < 0) {
        return {
          sessions: {
            ...s.sessions,
            [sessionId]: {
              ...session,
              isProcessing: false,
              ...clearInflightTurn(session),
            },
          },
        };
      }
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            messages: session.messages.slice(0, idx),
            isProcessing: false,
            ...clearInflightTurn(session),
          },
        },
      };
    }),

  loadMessages: (sessionId, messages) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      const normalized = coalesceAssistantTurns(
        messages
          .map((message) => normalizeTimelineMessage(message))
          .filter((message): message is ChatMessage => !!message),
      );
      // 进行中且本地已有更新乐观消息时，勿用偏旧/空历史整表覆盖（新建会话 restore 竞态）。
      let nextMessages = normalized;
      if (
        session.isProcessing &&
        session.messages.length > normalized.length &&
        session.pendingMessageIndex !== null
      ) {
        const optimistic = session.messages.slice(normalized.length);
        nextMessages = [...normalized, ...optimistic];
      }
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            messages: nextMessages,
            ...(session.isProcessing ? {} : clearInflightTurn(session)),
          },
        },
      };
    }),

  syncProcessingState: (sessionId, isProcessing) =>
    set((s) => {
      const session = s.sessions[sessionId] || emptySession();
      if (session.isProcessing === isProcessing) {
        if (!isProcessing) {
          const hasInflight =
            !!session.streamingText.trim() ||
            !!session.reasoningText.trim() ||
            session.turnSegments.length > 0 ||
            session.pendingMessageIndex !== null;
          if (!hasInflight) return s;
        } else {
          return s;
        }
      }
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: {
            ...session,
            isProcessing,
            ...(isProcessing ? {} : clearInflightTurn(session)),
          },
        },
      };
    }),

  getSession: (sessionId) => get().sessions[sessionId] || emptySession(),

  rollbackOptimisticMessage: (sessionId) =>
    set((s) => {
      const session = s.sessions[sessionId];
      if (!session || session.pendingMessageIndex === null) return s;
      const idx = session.pendingMessageIndex;
      const messages = [...session.messages.slice(0, idx), ...session.messages.slice(idx + 1)];
      return {
        sessions: {
          ...s.sessions,
          [sessionId]: { ...session, messages, isProcessing: false, pendingMessageIndex: null },
        },
      };
    }),
}));

/** 轮次结束：清流式缓冲与 pendingUsage / pendingMessageIndex。 */
function clearInflightTurn(session: SessionState): Partial<SessionState> {
  return {
    streamingText: '',
    reasoningText: '',
    turnSegments: [],
    pendingUsage: undefined,
    pendingMessageIndex: null,
  };
}

// ---------------------------------------------------------------------------
// Event reducer - 将单个 AgentEvent 应用到 SessionState
// ---------------------------------------------------------------------------

/** applyEvent 入口：按 event.type 分支更新 SessionState。 */
function applyAgentEvent(state: SessionState, event: AgentEvent): SessionState {
  // 收到任何 agent 事件都表示服务器已确认用户消息，清除乐观更新标记。
  const cleared =
    state.pendingMessageIndex !== null ? { ...state, pendingMessageIndex: null } : state;

  switch (event.type) {
    // ── Text chunks → 立刻写入时间线 ──────────────────────────────────
    case 'text': {
      const content = event.content || '';
      if (!content) return state;
      const segments = [...cleared.turnSegments];
      const last = segments[segments.length - 1];
      if (last?.type === 'text') {
        segments[segments.length - 1] = { ...last, content: last.content + content };
      } else {
        segments.push({ type: 'text', id: segId(), content });
      }
      const open = segments[segments.length - 1] as TextSegment;
      return {
        ...cleared,
        turnSegments: segments,
        streamingText: open.content,
        turnWorkStartedAt: state.turnWorkStartedAt ?? Date.now(),
      };
    }

    // ── Reasoning → 独立思考段（仅当末段仍是纯思考中时追加）──────────────
    case 'reasoning': {
      const content = event.content || '';
      if (!content) return state;
      const segments = [...cleared.turnSegments];
      const last = segments[segments.length - 1];
      // 末段已有正文或不是 text：新开思考段，避免思考粘在上一轮回复后面、位置错乱
      if (last?.type === 'text' && !(last.content || '').trim()) {
        segments[segments.length - 1] = {
          ...last,
          thinking: `${last.thinking || ''}${content}`,
        };
      } else {
        segments.push({ type: 'text', id: segId(), content: '', thinking: content });
      }
      return {
        ...cleared,
        turnSegments: segments,
        reasoningText: state.reasoningText + content,
        turnWorkStartedAt: state.turnWorkStartedAt ?? Date.now(),
      };
    }

    // ── Thinking marker → iteration boundary, no state change ─────────────
    case 'thinking':
      return state;

    // ── Message → 仅补齐无流式 chunk 的提供方；不重复落已有叙述 ───────────
    case 'message': {
      const msg = event.message;
      if (!msg) return state;
      if (msg.role === 'user' || msg.role === 'tool') return state;
      if (looksLikeInternalCheckpoint(msg.content || '')) return state;

      const segments = [...state.turnSegments];
      const msgContent = (msg.content || '').trim();
      if (msgContent && msg.role === 'assistant') {
        const already = segments.some(
          (s) => s.type === 'text' && normalizeProseKey(s.content) === normalizeProseKey(msgContent),
        );
        if (!already) {
          const lastText = [...segments].reverse().find((s) => s.type === 'text') as
            | TextSegment
            | undefined;
          if (!lastText || !lastText.content.trim()) {
            segments.push({ type: 'text', id: segId(), content: msgContent });
          } else if (msgContent.startsWith(lastText.content.trim())) {
            const idx = segments.lastIndexOf(lastText);
            segments[idx] = { ...lastText, content: msgContent };
          } else if (!lastText.content.trim().startsWith(msgContent)) {
            segments.push({ type: 'text', id: segId(), content: msgContent });
          }
        }
      }

      return {
        ...state,
        turnSegments: segments,
        turnWorkStartedAt:
          state.turnWorkStartedAt ?? (msgContent ? Date.now() : state.turnWorkStartedAt),
        streamingText: '',
        reasoningText: '',
      };
    }

    // ── Tool call → 直接追加（叙述已在 segments 中）──────────────────────
    case 'tool_call': {
      const raw = (event.call || {}) as Record<string, unknown>;
      const tc = normaliseToolCall(raw);
      // Same id already sealed into the timeline — ignore duplicate protocol events.
      if (
        tc.id &&
        state.turnSegments.some((seg) => isToolLikeSegment(seg) && seg.id === tc.id)
      ) {
        return state;
      }
      const type = segmentTypeForTool(tc.name);
      const nextSegments: Segment[] = [
        {
          ...tc,
          type,
          content:
            type === 'agent_delegate'
              ? '子 Agent 委托'
              : type === 'sandbox_event'
                ? '沙箱操作'
                : undefined,
        } as Segment,
      ];
      // agent 工具：为主时间线展开并行 agent_run 嵌套块
      if (tc.name === 'agent' || tc.name === 'submit_task') {
        const agentsRaw = Array.isArray(tc.args.agents) ? tc.args.agents : [];
        const background = tc.args.background === true;
        for (let i = 0; i < agentsRaw.length; i++) {
          const item = agentsRaw[i];
          if (!item || typeof item !== 'object' || Array.isArray(item)) continue;
          const row = item as Record<string, unknown>;
          const agentId = typeof row.agentId === 'string' ? row.agentId : 'unknown';
          const task = typeof row.task === 'string' ? row.task : '';
          nextSegments.push({
            type: 'agent_run',
            id: `${tc.id}_run_${i}`,
            parentToolCallId: tc.id,
            agentId,
            agentName: agentId,
            task,
            status: 'running',
            children: [],
            startedAt: Date.now(),
            background,
            readScope: Array.isArray(row.readScope)
              ? row.readScope.filter((v): v is string => typeof v === 'string')
              : undefined,
            writeScope: Array.isArray(row.writeScope)
              ? row.writeScope.filter((v): v is string => typeof v === 'string')
              : undefined,
          });
        }
      }
      return {
        ...state,
        turnSegments: [...state.turnSegments, ...nextSegments],
        turnWorkStartedAt: state.turnWorkStartedAt ?? Date.now(),
        streamingText: '',
      };
    }

    // ── Tool call completes → update matching segment ─────────────────────
    case 'tool_result': {
      const result = event.result || { tool_call_id: '', content: '', isError: false };
      const backgroundAccepted = isBackgroundAgentAccepted(result.content || '');
      const nestedRows = parseNestedAgentResultRows(result.content || '');
      const updated = state.turnSegments.map((seg) => {
        if (isToolLikeSegment(seg) && seg.id === result.tool_call_id) {
          return {
            ...seg,
            status: (result.isError ? 'error' : 'done') as 'done' | 'error',
            result: result.content || '',
          };
        }
        // 后台 accepted 只表示已启动；子 run 仍靠 ledger / 续跑更新终态
        if (
          !backgroundAccepted &&
          seg.type === 'agent_run' &&
          seg.parentToolCallId === result.tool_call_id &&
          seg.status === 'running'
        ) {
          const row = pickNestedResultRow(nestedRows, seg.agentId, seg.id);
          const status = row
            ? mapNestedResultStatus(row.status)
            : ((result.isError ? 'failed' : 'completed') as AgentRunSegment['status']);
          return {
            ...seg,
            ...(typeof row?.runId === 'string' && row.runId ? { taskId: row.runId } : {}),
            status,
            endedAt: Date.now(),
            summary: row?.summary || summariseNestedToolResult(result.content || '', seg.agentId, seg.id),
          };
        }
        return seg;
      });
      return { ...cleared, turnSegments: updated };
    }

    // ── Streaming write args: real +N from model tool-arg deltas ───────────
    case 'tool_progress': {
      const toolUseId = String(event.toolUseId ?? '');
      const data = (event.data ?? null) as Record<string, unknown> | null;
      if (!toolUseId || !data || data.streamWrite !== true) return state;
      const patchArgs =
        data.args && typeof data.args === 'object' && !Array.isArray(data.args)
          ? (data.args as Record<string, unknown>)
          : null;
      const rawArgs = typeof data.rawArgs === 'string' ? data.rawArgs : undefined;
      const streamBefore =
        typeof data.existingBefore === 'string' ? data.existingBefore : undefined;
      const streamAdd = typeof data.add === 'number' ? data.add : undefined;
      const streamDel = typeof data.del === 'number' ? data.del : undefined;
      if (
        !patchArgs &&
        !rawArgs &&
        streamBefore === undefined &&
        streamAdd === undefined &&
        streamDel === undefined
      ) {
        return state;
      }
      return {
        ...state,
        turnSegments: state.turnSegments.map((seg) => {
          if (!isToolLikeSegment(seg) || seg.id !== toolUseId) return seg;
          if (seg.type !== 'tool_call') {
            return {
              ...seg,
              args: patchArgs ? { ...seg.args, ...patchArgs } : seg.args,
              ...(rawArgs !== undefined ? { rawArgs } : {}),
            };
          }
          return {
            ...seg,
            args: patchArgs ? { ...seg.args, ...patchArgs } : seg.args,
            ...(rawArgs !== undefined ? { rawArgs } : {}),
            ...(streamBefore !== undefined ? { streamBefore } : {}),
            ...(streamAdd !== undefined ? { streamAdd } : {}),
            ...(streamDel !== undefined ? { streamDel } : {}),
          };
        }),
      };
    }

    case 'memory_hit':
      return {
        ...state,
        turnSegments: [
          ...state.turnSegments,
          {
            type: 'memory_hit',
            id: segId(),
            status: 'done',
            title: '记忆命中',
            payload: {
              query: event.payload?.query as string | undefined,
              source: event.payload?.source as string | undefined,
              count: event.payload?.count as number | undefined,
              limit: event.payload?.limit as number | undefined,
              memoryIds: event.payload?.memoryIds as string[] | undefined,
              categories: event.payload?.categories as string[] | undefined,
              injected: event.payload?.injected as boolean | undefined,
            },
          },
        ],
      };
    case 'usage':
      return applyUsageEvent(cleared, event.usage);
    case 'checkpoint':
    case 'system_notice':
      return state;

    // ── Error / 停止：先把已有时间线落进 messages，避免命令记录被清掉 ──
    case 'error': {
      const segments = [...state.turnSegments];
      const finalMessages = [...state.messages];
      if (segments.length > 0) {
        const textParts = segments
          .filter((s): s is TextSegment => s.type === 'text')
          .map((s) => s.content);
        finalMessages.push({
          role: 'assistant',
          content: textParts.join('\n\n') || '',
          timestamp: Date.now(),
          segments,
          usage: state.pendingUsage,
          workStartedAt: state.turnWorkStartedAt,
          workEndedAt: Date.now(),
        });
      }
      return {
        ...state,
        messages: finalMessages,
        isProcessing: false,
        streamingText: '',
        reasoningText: '',
        turnSegments: [],
        pendingUsage: undefined,
        turnWorkStartedAt: undefined,
      };
    }

    // ── Done → commit the turn ────────────────────────────────────────────
    case 'done': {
      const segments = [...state.turnSegments];
      const finalMessages = [...state.messages];
      const cancelled = (event as { cancelled?: boolean }).cancelled === true;
      const doneTurnId =
        (typeof event.payload?.turnId === 'string' && event.payload.turnId) ||
        undefined;

      if (segments.length > 0) {
        const textParts = segments
          .filter((s): s is TextSegment => s.type === 'text')
          .map((s) => s.content);
        const content = textParts.join('\n\n') || '';

        finalMessages.push({
          role: 'assistant',
          content,
          timestamp: Date.now(),
          turnId: doneTurnId,
          segments,
          usage: state.pendingUsage,
          workStartedAt: state.turnWorkStartedAt,
          workEndedAt: Date.now(),
        });
      }
      // cancelled 且无片段：不插「已停止」气泡，提示由 notifyAgentEventAlert 负责

      return {
        ...state,
        messages: finalMessages,
        isProcessing: false,
        streamingText: '',
        reasoningText: '',
        turnSegments: [],
        pendingUsage: undefined,
        turnWorkStartedAt: undefined,
      };
    }

    default:
      return state;
  }
}

/** usage 事件：仅 isProcessing 时 mergeTurnUsage 到 pendingUsage。 */
function applyUsageEvent(state: SessionState, usage?: TokenUsage): SessionState {
  if (!usage || !hasTokenUsage(usage)) return state;
  // 只在本轮对话进行中累计；忽略回放/后台任务在空闲时发来的 usage，避免污染历史消息。
  if (!state.isProcessing) return state;
  const pendingUsage = mergeTurnUsage(state.pendingUsage, normalizeTokenUsage(usage));
  if (!pendingUsage) return state;
  return {
    ...state,
    pendingUsage,
  };
}

/** 读 xy-locale：en 英文，否则中文。 */
function localeText(zh: string, en: string): string {
  try {
    return window.localStorage.getItem('xy-locale') === 'en' ? en : zh;
  } catch {
    return zh;
  }
}

/** 错误串人性化：通道降级特殊文案，否则剥前缀。 */
function humanizeTimelineError(raw: string): string {
  const text = (raw || '').trim();
  if (!text) return localeText('运行失败，请重试。', 'Run failed. Please retry.');
  const degraded = text.match(/Provider route degraded:\s*(.+)$/i);
  if (degraded) {
    return localeText(
      `模型通道暂时不可用（${degraded[1].trim()}）。通常是限流或连续失败后的保护，稍后再试或换一个模型。`,
      `Model route temporarily unavailable (${degraded[1].trim()}). Often rate-limit protection — retry later or switch models.`,
    );
  }
  return text
    .replace(/^消息发送失败[：:]\s*/i, '')
    .replace(/^错误[：:]\s*/i, '')
    .replace(/^Error:\s*/i, '');
}

/** 内部检查点 / 资源 JSON 不应进用户时间线。 */
function looksLikeInternalCheckpoint(content: string): boolean {
  const trimmed = content.trim();
  if (!trimmed) return false;
  return (
    /^Checkpoint:/i.test(trimmed) ||
    /^本轮资源消耗\s*\n/i.test(trimmed) ||
    /"checkpointId"\s*:/.test(trimmed) ||
    /"tokenCount"\s*:/.test(trimmed) ||
    /"runId"\s*:\s*"run_/.test(trimmed)
  );
}

/** 去空白后的正文键，供去重比较。 */
function normalizeProseKey(content: string): string {
  return content.replace(/\s+/g, ' ').trim();
}

/** 时间线禁止居中系统插条：任务生命周期等噪音直接丢弃。 */
export function isCenteredTimelineNoise(message: ChatMessage): boolean {
  if (message.role !== 'system') return false;
  if (
    message.segments?.some(
      (segment) => segment.type === 'task_event' || segment.type === 'task_result',
    )
  ) {
    return true;
  }
  const text = (message.content || '').trim();
  if (!text) return true;
  if (looksLikeInternalCheckpoint(text)) return true;
  if (/任务开始执行|任务完成|任务失败|任务取消|^任务(完成|失败|取消):/i.test(text)) return true;
  if (/主 Agent 已完成任务分配|子 Agent 将在后台/i.test(text)) return true;
  if (/executor\.heartbeat|task\.started/i.test(text)) return true;
  return false;
}

/** 历史里的 system / 错误提示条不进时间线；运行时提示走 antd 浮层。 */
export function normalizeTimelineMessage(message: ChatMessage): ChatMessage | null {
  if (isCenteredTimelineNoise(message)) return null;
  if (message.isError) return null;
  if (message.role === 'system') return null;
  if (message.role === 'assistant') {
    const text = (message.content || '').trim();
    if (/^已停止本次运行\.?$|^Run stopped\.?$/i.test(text)) return null;
  }
  return message;
}

/**
 * 将同一用户轮次里被落库拆碎的多条 assistant 合成一条，
 * 与流式 `turnSegments` 的时间线组织对齐（按 turnId；无 turnId 则合并相邻 assistant）。
 */
export function coalesceAssistantTurns(messages: ChatMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const message of messages) {
    if (message.role !== 'assistant') {
      out.push(message);
      continue;
    }
    const prev = out[out.length - 1];
    if (prev?.role === 'assistant' && shouldMergeAssistantTurns(prev, message)) {
      out[out.length - 1] = mergeAssistantTurnMessages(prev, message);
    } else {
      out.push(message);
    }
  }
  return out;
}

/** 同 turnId 可合并；双方皆无 turnId 时按相邻 assistant 合并。 */
function shouldMergeAssistantTurns(a: ChatMessage, b: ChatMessage): boolean {
  if (a.turnId && b.turnId) return a.turnId === b.turnId;
  return !a.turnId && !b.turnId;
}

/** 合并两条 assistant：segments 按 id 去重，usage / 时间取并集。 */
function mergeAssistantTurnMessages(a: ChatMessage, b: ChatMessage): ChatMessage {
  const mergedSegments = dedupeSegmentsById([...(a.segments || []), ...(b.segments || [])]);
  const textParts = mergedSegments
    .filter((segment): segment is TextSegment => segment.type === 'text')
    .map((segment) => segment.content.trim())
    .filter(Boolean);
  return {
    ...a,
    content: textParts.length > 0 ? textParts.join('\n\n') : a.content || b.content,
    timestamp: Math.min(a.timestamp || b.timestamp, b.timestamp || a.timestamp),
    turnId: a.turnId || b.turnId,
    usage: b.usage || a.usage,
    segments: mergedSegments.length > 0 ? mergedSegments : a.segments || b.segments,
    workStartedAt:
      minOptional(a.workStartedAt, b.workStartedAt) ??
      Math.min(a.timestamp || Date.now(), b.timestamp || Date.now()),
    workEndedAt:
      maxOptional(a.workEndedAt, b.workEndedAt) ??
      Math.max(a.timestamp || 0, b.timestamp || 0),
    attachments: a.attachments?.length ? a.attachments : b.attachments,
  };
}

/** 按 segment.id 去重，保留首次出现。 */
function dedupeSegmentsById(segments: Segment[]): Segment[] {
  const seen = new Set<string>();
  const out: Segment[] = [];
  for (const segment of segments) {
    if (segment.id && seen.has(segment.id)) continue;
    if (segment.id) seen.add(segment.id);
    out.push(segment);
  }
  return out;
}

/** 两边皆有则取较小；单边有则取该边。 */
function minOptional(a?: number, b?: number): number | undefined {
  if (typeof a === 'number' && typeof b === 'number') return Math.min(a, b);
  return typeof a === 'number' ? a : b;
}

/** 两边皆有则取较大；单边有则取该边。 */
function maxOptional(a?: number, b?: number): number | undefined {
  if (typeof a === 'number' && typeof b === 'number') return Math.max(a, b);
  return typeof a === 'number' ? a : b;
}

/**
 * done(cancelled) / error → antd 浮层；中止走 info，其它走 error。
 * 不把这些提示写进时间线气泡。
 */
function notifyAgentEventAlert(event: AgentEvent): void {
  if (event.type === 'done' && (event as { cancelled?: boolean }).cancelled === true) {
    notifyChatInfo(localeText('已停止本次运行。', 'Run stopped.'));
    return;
  }
  if (event.type !== 'error') return;

  const structured = event.error as
    | {
        name?: string;
        message?: string;
        structured?: {
          message?: string;
          suggestion?: string;
          logPath?: string;
        };
      }
    | undefined;
  const rawMessage =
    structured?.structured?.message ||
    structured?.message ||
    event.error?.message ||
    '';
  const isAbort =
    structured?.name === 'AbortError' ||
    /\b(aborted|cancelled|canceled|user aborted)\b/i.test(rawMessage);

  if (isAbort) {
    notifyChatInfo(localeText('已停止本次运行。', 'Run stopped.'));
    return;
  }

  const humanized = humanizeTimelineError(rawMessage || localeText('未知错误', 'Unknown error'));
  const content = structured?.structured
    ? [
        humanized,
        structured.structured.suggestion
          ? `${localeText('建议', 'Suggestion')}：${structured.structured.suggestion}`
          : '',
        structured.structured.logPath
          ? `${localeText('日志', 'Log')}：${structured.structured.logPath}`
          : '',
      ]
        .filter(Boolean)
        .join('\n')
    : humanized;
  notifyChatError(content);
}

/** sandbox_* → sandbox_event，其余 tool_call（agent 另走 agent_run）。 */
function segmentTypeForTool(name: string): 'tool_call' | 'agent_delegate' | 'sandbox_event' {
  if (name.startsWith('sandbox_')) return 'sandbox_event';
  return 'tool_call';
}

/** agent 工具结果是否表示后台已受理（JSON 或启发式正则）。 */
function isBackgroundAgentAccepted(content: string): boolean {
  const trimmed = content.trim();
  if (!trimmed) return false;
  try {
    const parsed = JSON.parse(trimmed) as { background?: unknown; status?: unknown };
    return parsed.background === true && parsed.status === 'accepted';
  } catch {
    return /"background"\s*:\s*true/.test(trimmed) && /"status"\s*:\s*"accepted"/.test(trimmed);
  }
}

type NestedResultRow = {
  agentId?: string;
  summary?: string;
  status?: string;
  runId?: string;
};

/** 解析 agent 工具 JSON 的 results 数组。 */
function parseNestedAgentResultRows(content: string): NestedResultRow[] {
  try {
    const parsed = JSON.parse(content) as { results?: NestedResultRow[] };
    return Array.isArray(parsed.results) ? parsed.results : [];
  } catch {
    return [];
  }
}

/** 按 run 段 `_run_N` 下标或 agentId 挑对应结果行。 */
function pickNestedResultRow(
  rows: NestedResultRow[],
  agentId: string,
  runSegmentId: string,
): NestedResultRow | undefined {
  if (!rows.length) return undefined;
  const indexMatch = runSegmentId.match(/_run_(\d+)$/);
  const index = indexMatch ? Number(indexMatch[1]) : -1;
  if (index >= 0 && rows[index]) return rows[index];
  return rows.find((r) => r.agentId === agentId) || rows[0];
}

/** 嵌套结果 status 字符串 → agent_run.status。 */
function mapNestedResultStatus(status?: string): AgentRunSegment['status'] {
  if (status === 'completed' || status === 'done') return 'completed';
  if (status === 'cancelled') return 'cancelled';
  if (status === 'failed' || status === 'blocked' || status === 'error') return 'failed';
  return 'completed';
}

/** 从 agent 工具结果抽出该 run 的 summary 文案。 */
function summariseNestedToolResult(
  content: string,
  agentId: string,
  runSegmentId: string,
): string | undefined {
  const row = pickNestedResultRow(parseNestedAgentResultRows(content), agentId, runSegmentId);
  return row?.summary || row?.status;
}

/** 同 taskId 的 system 任务气泡原地替换，否则追加。 */
function upsertTaskSystemMessage(
  messages: ChatMessage[],
  next: ChatMessage,
  taskId: string,
): ChatMessage[] {
  const index = messages.findIndex(
    (message) =>
      message.role === 'system' &&
      message.segments?.some(
        (segment) =>
          (segment.type === 'task_event' || segment.type === 'task_result') &&
          String(
            (segment.payload as { event?: { taskId?: string }; result?: { taskId?: string } })
              ?.event?.taskId ||
              (segment.payload as { event?: { taskId?: string }; result?: { taskId?: string } })
                ?.result?.taskId ||
              '',
          ) ===
            taskId,
      ),
  );
  if (index === -1) return [...messages, next];
  const updated = [...messages];
  updated[index] = {
    ...next,
  };
  return updated;
}

/** task.started 文案；heartbeat 返回空（不展示）。 */
function summariseTaskEvent(event: NestedTaskEvent): string {
  if (event.type === 'executor.heartbeat') return '';
  const actor = event.actor?.name || '执行器';
  if (event.type === 'task.started') return `${actor}: 任务开始执行`;
  return '';
}

/** 控制任务结果摘要：完成/失败/取消 + conclusion + 风险。 */
function summariseTaskResult(result: NestedTaskResult): string {
  const status = result.status === 'done' ? '完成' : result.status === 'failed' ? '失败' : '取消';
  const risks = result.output.risks?.length ? `\n风险: ${result.output.risks.join('; ')}` : '';
  return `任务${status}: ${result.output.conclusion || result.taskId}${risks}`;
}
