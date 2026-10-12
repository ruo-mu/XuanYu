/**
 * 跨平面线缆 DTO：会话摘要、频道入站、WS 封包与可序列化错误。
 * session / gateway 共用；具体 ServerMessageType 联合仍由 gateway/protocol 拥有。
 */

/** 会话列表/详情用的摘要信息。 */
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
  /** 存储键项目名（cwd 基名 + hash）；侧栏分组只看 projectDir。 */
  projectName?: string;
  /** 工作区绝对路径；有值则归入侧栏「项目」，无值则「最近」。 */
  projectDir?: string;
  /** 持久化会话元数据（如频道来源 channelType / channelId）。 */
  metadata?: Record<string, unknown>;
}

/** 会话用量与消息统计。 */
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

/** 频道类型标识（由插件注册的字符串）。 */
export type ChannelType = string;

/** 标准化频道入站消息。 */
export interface ChannelMessage {
  id: string;
  channelType: ChannelType;
  channelId: string;
  userId: string;
  userName: string;
  text: string;
  timestamp: number;
  isMention: boolean;
  raw?: unknown;
  tenantId?: string;
  threadId?: string;
  principalId?: string;
  attachments?: Array<{
    id: string;
    name: string;
    mimeType?: string;
    url?: string;
    sizeBytes?: number;
  }>;
}

/** 生成服务端 WS/广播 JSON 封包。 */
export function createServerMessage(type: string, payload: unknown): string {
  return JSON.stringify({
    id: `srv_${Date.now()}_${Math.random().toString(36).substring(2, 7)}`,
    type,
    payload,
    timestamp: Date.now(),
  });
}

/** Agent 事件上的 Error 在 JSON 中会变成 {}；广播前压成可序列化对象。 */
export function toWireAgentError(error: unknown): { name: string; message: string } {
  if (error instanceof Error) {
    return {
      name: error.name || 'Error',
      message: error.message?.trim() || error.name || 'Unknown error',
    };
  }
  if (error && typeof error === 'object') {
    const record = error as { name?: unknown; message?: unknown };
    const name = typeof record.name === 'string' && record.name.trim() ? record.name : 'Error';
    const message =
      typeof record.message === 'string' && record.message.trim()
        ? record.message
        : String(error);
    return { name, message };
  }
  return { name: 'Error', message: String(error || 'Unknown error') };
}

/** AbortError 名，或文案含 aborted/cancelled。 */
export function isAbortErrorMessage(message: string, name?: string): boolean {
  if (name === 'AbortError') return true;
  return /\b(aborted|cancelled|canceled|user aborted)\b/i.test(message);
}
