/**
 * MessageBubble — 单条消息展示。
 *
 * 被谁调用：ChatView 消息列表。
 * 做什么：用户 → UserTimelineRow；助手 → CodexTurnTimeline；噪声/空消息不渲染。
 */

import React from 'react';

import type { ChatMessage } from '../../stores/chatStore';
import { isCenteredTimelineNoise, normalizeTimelineMessage } from '../../stores/chatStore';
import { useLocaleText } from '../../i18n';
import { contextTokensAfterTurn } from '../../utils/tokenUsage';
import type { TokenUsage } from '../../protocol';
import { UserTimelineRow } from './chatTimeline';
import { CodexTurnTimeline } from './codex/CodexTimeline';

interface Props {
  message: ChatMessage;
  canInlineEdit?: boolean;
  isEditing?: boolean;
  editDraft?: string;
  onStartEdit?: () => void;
  onCancelEdit?: () => void;
  onEditDraftChange?: (value: string) => void;
  onSubmitEdit?: () => void;
  sessionId?: string;
  /** 仅整轮收尾展示；不传则不显示用量（避免中间工具轮到处刷）。 */
  turnUsage?: TokenUsage | null;
}

/** 被谁调用：ChatView 消息列表。做什么：用户气泡或助手 CodexTurnTimeline；噪声消息直接隐藏。 */
export default function MessageBubble({
  message,
  canInlineEdit,
  isEditing,
  editDraft,
  onStartEdit,
  onCancelEdit,
  onEditDraftChange,
  onSubmitEdit,
  sessionId,
  turnUsage,
}: Props) {
  if (isCenteredTimelineNoise(message)) return null;
  const normalized = normalizeTimelineMessage(message);
  if (!normalized) return null;

  const isUser = normalized.role === 'user';

  if (isUser) {
    return (
      <UserTimelineRow
        content={normalized.content}
        turnId={normalized.turnId}
        timestamp={normalized.timestamp}
        attachments={normalized.attachments}
        canInlineEdit={canInlineEdit}
        isEditing={isEditing}
        editDraft={editDraft}
        onStartEdit={onStartEdit}
        onCancelEdit={onCancelEdit}
        onEditDraftChange={onEditDraftChange}
        onSubmitEdit={onSubmitEdit}
      />
    );
  }

  const segments = normalized.segments || [];
  const timelineSegments =
    segments.length > 0
      ? segments
      : normalized.content.trim()
        ? [{ type: 'text' as const, id: 'plain', content: normalized.content }]
        : [];

  const footerUsage = turnUsage || undefined;

  if (timelineSegments.length === 0 && !footerUsage) {
    return null;
  }

  return (
    <div className="xy-codex-shell xy-assistant-turn">
      {timelineSegments.length > 0 && (
        <CodexTurnTimeline
          segments={timelineSegments}
          completed
          workStartedAt={normalized.workStartedAt}
          workEndedAt={normalized.workEndedAt || normalized.timestamp}
          sessionId={sessionId}
          turnId={normalized.turnId}
        />
      )}
      {footerUsage ? <TokenUsageLine usage={footerUsage} /> : null}
    </div>
  );
}

/** 被谁调用：MessageBubble 整轮收尾。做什么：展示本轮 token 用量。 */
function TokenUsageLine({ usage }: { usage: TokenUsage }) {
  const lt = useLocaleText();
  const contextFinal = contextTokensAfterTurn(usage);
  const output = usage.completionTokens;
  const cache = usage.cacheTokens || 0;
  const billed = usage.billedPromptTokens || 0;
  const cacheBase = usage.promptTokens || 0;
  const cacheHitRate = cacheBase > 0 ? cache / cacheBase : 0;
  const parts: string[] = [];
  if (contextFinal > 0) {
    parts.push(`${lt('上下文', 'Context')} ${formatTokenCount(contextFinal)} token`);
  }
  if (output > 0) parts.push(`${lt('输出', 'Output')} ${formatTokenCount(output)} token`);
  if (billed > 0 && billed !== usage.promptTokens) {
    parts.push(`${lt('本轮计费输入', 'Billed input')} ${formatTokenCount(billed)} token`);
  }
  if (cacheBase > 0 || cache > 0) {
    parts.push(`${lt('缓存命中', 'Cache hit')} ${formatTokenCount(cache)} token`);
    parts.push(`${lt('命中率', 'Hit rate')} ${formatPercent(cacheHitRate)}`);
  }
  if (parts.length === 0) return null;

  return (
    <div className="xy-codex-meta xy-token-usage">
      <div className="xy-token-usage-line">{parts.join(' · ')}</div>
    </div>
  );
}

/** 被谁调用：TokenUsageLine。做什么：千分位格式。 */
function formatTokenCount(value: number): string {
  return Number.isFinite(value) ? Math.floor(value).toLocaleString() : '0';
}

/** 被谁调用：TokenUsageLine。做什么：百分比格式。 */
function formatPercent(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0%';
  const percent = Math.min(value, 1) * 100;
  return `${percent >= 10 ? Math.round(percent) : Math.round(percent * 10) / 10}%`;
}
