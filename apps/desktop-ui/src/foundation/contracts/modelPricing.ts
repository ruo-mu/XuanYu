/**
 * 模型定价契约（每 1M token 的 USD）。
 * observability / execution 共用，避免观测层依赖执行层。
 */

export interface ModelPricing {
  input: number;
  output: number;
  cachedInput?: number;
}
