/**
 * Token 用量：核心算法复用 foundation/contracts；此处仅保留 Web UI 展示辅助。
 */

import type { TokenUsage } from '../protocol';
import { mergeTurnUsage } from '@xuanyu/contracts/tokenUsage';

export {
  contextTokensAfterTurn,
  hasTokenUsage,
  mergeTurnUsage,
  normalizeTokenUsage,
} from '@xuanyu/contracts/tokenUsage';

/** 用量只挂在「连续 assistant 段」的最后一条（下一轮用户发言前 / 列表末尾）。 */
export function isAssistantTurnUsageAnchor(
  messages: Array<{ role: string }>,
  index: number,
): boolean {
  if (index < 0 || index >= messages.length) return false;
  if (messages[index].role !== 'assistant') return false;
  const next = messages[index + 1];
  return !next || next.role === 'user';
}

/**
 * 本轮收尾应展示的用量：合并该 assistant 连续段上各条消息的 usage（历史按次落库时拼回整轮）。
 * 非收尾锚点返回 undefined。
 */
export function resolveTurnFooterUsage(
  messages: Array<{ role: string; usage?: TokenUsage }>,
  index: number,
): TokenUsage | undefined {
  if (!isAssistantTurnUsageAnchor(messages, index)) return undefined;

  let start = index;
  while (start > 0 && messages[start - 1].role === 'assistant') start -= 1;

  let merged: TokenUsage | undefined;
  for (let i = start; i <= index; i += 1) {
    const usage = messages[i].usage;
    if (!usage) continue;
    merged = mergeTurnUsage(merged, usage) ?? merged;
  }
  return merged;
}
