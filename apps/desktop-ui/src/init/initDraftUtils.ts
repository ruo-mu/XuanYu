/**
 * 初始化向导草稿工具：规范化 watchdog、locale，以及草稿元数据读写。
 */

import type { InitConfigDraft, InitDraft, InitLocale, InitMeta } from './initApi';

/** 向导里子 Agent watchdog 的缺省：静默超时、无进展轮次、最大重试。 */
export const INIT_WATCHDOG_DEFAULTS = {
  silentMs: 120_000,
  noProgressIterations: 8,
  maxRetries: 3,
} as const;

/** 初始化向导里的 watchdog 默认值。 */
export function normalizeWatchdogForInit(subAgents?: Record<string, unknown>): Record<string, unknown> {
  const base = {
    configDir: 'capabilities/agents',
    ...(subAgents || {}),
  };
  const watchdog = (subAgents?.watchdog as Record<string, unknown> | undefined) || {};
  const rawRetries = watchdog.maxRetries;
  const maxRetries =
    rawRetries === undefined ? INIT_WATCHDOG_DEFAULTS.maxRetries : Number(rawRetries);

  return {
    ...base,
    watchdog: {
      silentMs: Number(watchdog.silentMs ?? INIT_WATCHDOG_DEFAULTS.silentMs),
      noProgressIterations: Number(
        watchdog.noProgressIterations ?? INIT_WATCHDOG_DEFAULTS.noProgressIterations,
      ),
      maxRetries,
    },
  };
}

/** 把草稿里的 subAgents.watchdog 补成完整默认值。 */
export function normalizeInitDraftWatchdog(draft: InitDraft): InitDraft {
  return {
    ...draft,
    config: {
      ...draft.config,
      subAgents: normalizeWatchdogForInit(draft.config.subAgents as Record<string, unknown> | undefined),
    },
  };
}

/** 被谁调用：createDraftFromSaved。做什么：从配置反推套餐 id。 */
function resolveProviderPlanId(meta: InitMeta, config: InitConfigDraft): string | undefined {
  const provider = config.provider;
  if (!provider?.type) return undefined;
  const def = meta.providers.find((item) => item.type === provider.type);
  if (!def) return undefined;
  const plan =
    def.plans.find((item) => item.baseUrl && item.baseUrl === provider.baseUrl) || def.plans[0];
  return plan?.id;
}

/** 用磁盘上的已保存配置构建 init 草稿（配置字段以文件为准）。 */
export function createDraftFromSaved(meta: InitMeta, saved: InitConfigDraft): InitDraft {
  const locale = (saved.locale === 'en' ? 'en' : 'zh-CN') as InitLocale;

  return {
    locale,
    providerPlanId: resolveProviderPlanId(meta, saved),
    config: {
      ...meta.defaults,
      ...saved,
      locale,
      subAgents: normalizeWatchdogForInit({
        ...(meta.defaults.subAgents as Record<string, unknown> | undefined),
        ...(saved.subAgents as Record<string, unknown> | undefined),
      }),
    },
  };
}

/** 合并 session 里仅存在于向导 UI 的字段（步骤、套餐选择等）。 */
export function mergeWizardSessionFields(base: InitDraft, session?: InitDraft | null): InitDraft {
  if (!session) return base;
  return {
    ...base,
    providerPlanId: session.providerPlanId ?? base.providerPlanId,
  };
}
