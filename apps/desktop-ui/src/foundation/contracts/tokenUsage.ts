/**
 * Token usage merge helpers.
 *
 * Streaming providers (e.g. MiMo) may emit cumulative usage on every chunk.
 * Summing those snapshots inflates counts by orders of magnitude.
 *
 * After merge:
 * - promptTokens = last call prompt (context BEFORE that reply)
 * - lastCompletionTokens = last call completion
 * - ended context ≈ promptTokens + lastCompletionTokens
 * - completionTokens = sum of completions this user turn
 * - billedPromptTokens = sum of prompts this user turn
 */

import type { TokenUsage } from './sharedTypes';

/** 把各厂商字段归一成 prompt/completion/cache/total。 */
export function normalizeTokenUsage(usage: TokenUsage): TokenUsage {
  const promptTokens = normalizeTokenNumber(usage.promptTokens);
  const completionTokens = normalizeTokenNumber(usage.completionTokens);
  const totalTokens = normalizeTokenNumber(usage.totalTokens) || promptTokens + completionTokens;
  const cacheTokens = normalizeTokenNumber(usage.cacheTokens);
  const billedPromptTokens = normalizeTokenNumber(usage.billedPromptTokens);
  const lastCompletionTokens = normalizeTokenNumber(usage.lastCompletionTokens);
  return {
    promptTokens,
    completionTokens,
    totalTokens,
    cacheTokens: cacheTokens > 0 ? cacheTokens : undefined,
    billedPromptTokens: billedPromptTokens > 0 ? billedPromptTokens : undefined,
    lastCompletionTokens: lastCompletionTokens > 0 ? lastCompletionTokens : undefined,
  };
}

/** 是否含有效 token 数字。 */
export function hasTokenUsage(usage: TokenUsage): boolean {
  return (
    usage.promptTokens > 0 ||
    usage.completionTokens > 0 ||
    usage.totalTokens > 0 ||
    normalizeTokenNumber(usage.cacheTokens) > 0 ||
    normalizeTokenNumber(usage.billedPromptTokens) > 0
  );
}

/** 本轮结束后对话上下文占用 ≈ 最后一次 prompt + 最后一次 completion。 */
export function contextTokensAfterTurn(usage: TokenUsage): number {
  const prompt = normalizeTokenNumber(usage.promptTokens);
  const lastOut = normalizeTokenNumber(usage.lastCompletionTokens);
  return prompt + lastOut;
}

/**
 * Merge provider usage into a running turn total.
 * - Same model call (streaming snapshots): keep latest prompt; grow lastCompletion.
 * - New model round: refresh prompt/lastCompletion; accumulate billed + total completions.
 */
export function mergeTurnUsage(
  current: TokenUsage | undefined,
  next: TokenUsage,
): TokenUsage | undefined {
  const normalized = normalizeTokenUsage(next);
  if (!hasTokenUsage(normalized)) return current;
  if (!current) {
    const billed = normalized.billedPromptTokens ?? normalized.promptTokens;
    const lastOut = normalized.lastCompletionTokens ?? normalized.completionTokens;
    return {
      promptTokens: normalized.promptTokens,
      completionTokens: normalized.completionTokens,
      totalTokens: normalized.totalTokens || normalized.promptTokens + normalized.completionTokens,
      cacheTokens: normalized.cacheTokens,
      billedPromptTokens: billed > 0 ? billed : undefined,
      lastCompletionTokens: lastOut > 0 ? lastOut : undefined,
    };
  }

  const cur = normalizeTokenUsage(current);
  const curBilled = cur.billedPromptTokens ?? cur.promptTokens;
  const curLastOut = cur.lastCompletionTokens ?? 0;

  if (
    normalized.promptTokens === cur.promptTokens &&
    normalized.completionTokens === cur.completionTokens &&
    normalized.totalTokens === cur.totalTokens
  ) {
    return {
      ...cur,
      billedPromptTokens: curBilled > 0 ? curBilled : undefined,
      lastCompletionTokens: curLastOut > 0 ? curLastOut : undefined,
    };
  }

  const promptUnchanged =
    cur.promptTokens > 0 &&
    normalized.promptTokens >= cur.promptTokens &&
    normalized.promptTokens <= Math.ceil(cur.promptTokens * 1.02);

  // Same API call streaming: prompt stable, this call's completion grows.
  if (promptUnchanged && normalized.completionTokens >= curLastOut) {
    const lastOut = normalized.completionTokens;
    const priorSum = Math.max(0, cur.completionTokens - curLastOut);
    const completionTokens = priorSum + lastOut;
    return {
      promptTokens: normalized.promptTokens,
      completionTokens,
      totalTokens: normalized.promptTokens + completionTokens,
      cacheTokens:
        Math.max(normalizeTokenNumber(cur.cacheTokens), normalizeTokenNumber(normalized.cacheTokens)) ||
        undefined,
      billedPromptTokens: curBilled > 0 ? curBilled : normalized.promptTokens > 0 ? normalized.promptTokens : undefined,
      lastCompletionTokens: lastOut > 0 ? lastOut : undefined,
    };
  }

  // New model round.
  const lastOut = normalized.completionTokens;
  const billedPromptTokens = curBilled + normalized.promptTokens;
  const completionTokens = cur.completionTokens + lastOut;
  return {
    promptTokens: normalized.promptTokens,
    completionTokens,
    totalTokens: normalized.promptTokens + completionTokens,
    cacheTokens:
      Math.max(normalizeTokenNumber(cur.cacheTokens), normalizeTokenNumber(normalized.cacheTokens)) ||
      undefined,
    billedPromptTokens,
    lastCompletionTokens: lastOut > 0 ? lastOut : undefined,
  };
}

/** 读取正整数；无效则 undefined。 */
export function readPositiveInt(value: unknown): number {
  return normalizeTokenNumber(value);
}

/** Per-session resource usage persisted on conversation_sessions. */
export interface SessionResourceUsage {
  totalInputTokens: number;
  totalOutputTokens: number;
  totalTokens: number;
  toolCallCount: number;
  compactCount: number;
  hasRealTokenUsage: boolean;
  tokenUsageEstimated: boolean;
}

/** 会话记录上的资源用量字段。 */
export interface SessionResourceRecordLike {
  totalInputTokens?: number;
  totalOutputTokens?: number;
  toolCallCount?: number;
  compactCount?: number;
  hasRealTokenUsage?: boolean;
  tokenUsageEstimated?: boolean;
  metadata?: Record<string, unknown>;
}

/** 消息上的 token 列。 */
export interface MessageTokenFields {
  tokenPrompt?: number;
  tokenCompletion?: number;
  tokenCount?: number;
}

/** Aggregate token usage from persisted assistant messages. */
export function sumMessageTokenTotals(messages: MessageTokenFields[]): SessionResourceUsage {
  let totalInputTokens = 0;
  let totalOutputTokens = 0;
  let hasRealTokenUsage = false;

  for (const message of messages) {
    const promptTokens = normalizeTokenNumber(message.tokenPrompt);
    const completionTokens = normalizeTokenNumber(message.tokenCompletion);
    if (promptTokens > 0 || completionTokens > 0) {
      hasRealTokenUsage = true;
      totalInputTokens += promptTokens;
      totalOutputTokens += completionTokens;
      continue;
    }

    const totalTokens = normalizeTokenNumber(message.tokenCount);
    if (totalTokens > 0) {
      totalOutputTokens += totalTokens;
    }
  }

  const totalTokens = totalInputTokens + totalOutputTokens;
  return {
    totalInputTokens,
    totalOutputTokens,
    totalTokens,
    toolCallCount: 0,
    compactCount: 0,
    hasRealTokenUsage,
    tokenUsageEstimated: !hasRealTokenUsage && totalTokens > 0,
  };
}


/** Read persisted session resource usage from typed columns. */
export function readSessionResourceUsage(
  record?: SessionResourceRecordLike,
): SessionResourceUsage | undefined {
  if (!record) return undefined;

  const fromColumns: SessionResourceUsage = {
    totalInputTokens: readPositiveInt(record.totalInputTokens),
    totalOutputTokens: readPositiveInt(record.totalOutputTokens),
    totalTokens:
      readPositiveInt(record.totalInputTokens) + readPositiveInt(record.totalOutputTokens),
    toolCallCount: readPositiveInt(record.toolCallCount),
    compactCount: readPositiveInt(record.compactCount),
    hasRealTokenUsage: record.hasRealTokenUsage === true,
    tokenUsageEstimated: record.tokenUsageEstimated !== false,
  };

  if (
    fromColumns.totalTokens > 0 ||
    fromColumns.toolCallCount > 0 ||
    fromColumns.compactCount > 0
  ) {
    return fromColumns;
  }

  return undefined;
}

/** 从会话字段组装资源用量。 */
export function buildSessionResourceUsage(session: {
  totalInputTokens: number;
  totalOutputTokens: number;
  toolCallCount: number;
  compactCount: number;
  hasRealTokenUsage: boolean;
  tokenUsageEstimated: boolean;
}): SessionResourceUsage {
  const totalTokens = session.totalInputTokens + session.totalOutputTokens;
  return {
    totalInputTokens: session.totalInputTokens,
    totalOutputTokens: session.totalOutputTokens,
    totalTokens,
    toolCallCount: session.toolCallCount,
    compactCount: session.compactCount,
    hasRealTokenUsage: session.hasRealTokenUsage,
    tokenUsageEstimated: session.tokenUsageEstimated,
  };
}

/** 把用量编成历史库 update patch。 */
export function buildSessionResourcePatch(usage: SessionResourceUsage): SessionResourceRecordLike {
  return {
    totalInputTokens: usage.totalInputTokens,
    totalOutputTokens: usage.totalOutputTokens,
    toolCallCount: usage.toolCallCount,
    compactCount: usage.compactCount,
    hasRealTokenUsage: usage.hasRealTokenUsage,
    tokenUsageEstimated: usage.tokenUsageEstimated,
  };
}

/** 把用量写回会话对象。 */
export function applySessionResourceUsage(
  target: {
    totalInputTokens: number;
    totalOutputTokens: number;
    hasRealTokenUsage: boolean;
    tokenUsageEstimated: boolean;
  },
  usage: SessionResourceUsage,
): void {
  target.totalInputTokens = usage.totalInputTokens;
  target.totalOutputTokens = usage.totalOutputTokens;
  target.hasRealTokenUsage = usage.hasRealTokenUsage;
  target.tokenUsageEstimated = usage.tokenUsageEstimated;
}

/** 正有限数 floor；否则 0。 */
function normalizeTokenNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}
