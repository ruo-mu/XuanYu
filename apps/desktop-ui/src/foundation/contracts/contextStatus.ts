/**
 * 上下文占用摘要：供 CLI /status、会话查询与网关诊断共用。
 */

import type {
  CompactResult,
  ContextBudgetAllocation,
  TokenBreakdown,
  TurnIntent,
} from './sharedTypes';
import type { ContextLoadPolicy, ContextSectionName } from './contextLoadPolicy';

/** 上下文状态摘要。 */
export interface ContextStatus {
  messageCount: number;
  estimatedTokens: number;
  maxTokens: number;
  usageRatio: number;
  autoCompactEnabled: boolean;
  autoCompactThreshold: number;
  compactCount: number;
  lastCompact?: CompactResult;
  breakdown: TokenBreakdown;
  allocation: Omit<ContextBudgetAllocation, 'messages' | 'breakdown'> & {
    sections: ContextBudgetAllocation['sections'];
  };
  /** 当前 turn 推断的意图。 */
  turnIntent?: TurnIntent;
  /** ContextSurface 已激活的 section 列表。 */
  activeSections?: ContextSectionName[];
  /** 当前 turn 生效的 context load policy 快照。 */
  contextLoadPolicy?: ContextLoadPolicy;
}
