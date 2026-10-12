/**
 * 解析供应商拉取参数：protocol / baseUrl 来自目录 plans，不写死模型列表。
 */

import {
  providerDefs,
  type InitLocale,
  type InitProviderDef,
} from '../init/providerCatalog';

export type ModelFetchProtocol = 'openai' | 'anthropic';

export interface ProviderFetchParams {
  providerType: string;
  baseUrl?: string;
  apiKey?: string;
  protocol: ModelFetchProtocol;
}

/** 按 type 找供应商定义（plans / needsApiKey）。 */
export function findProviderDef(
  providerType: string | undefined,
  locale: InitLocale = 'zh-CN',
): InitProviderDef | undefined {
  if (!providerType) return undefined;
  return providerDefs(locale).find((item) => item.type === providerType);
}

/**
 * 根据 type + 可选 planId/baseUrl 解析拉取参数。
 * protocol 优先显式传入，否则匹配 plan，再退回首个 plan。
 */
export function resolveProviderFetchParams(input: {
  providerType: string;
  baseUrl?: string;
  apiKey?: string;
  protocol?: string;
  planId?: string;
  locale?: InitLocale;
}): ProviderFetchParams {
  const def = findProviderDef(input.providerType, input.locale || 'zh-CN');
  const byPlanId = input.planId
    ? def?.plans.find((plan) => plan.id === input.planId)
    : undefined;
  const normalizedBase = (input.baseUrl || '').replace(/\/+$/, '').toLowerCase();
  const byBase = normalizedBase
    ? def?.plans.find((plan) => plan.baseUrl.replace(/\/+$/, '').toLowerCase() === normalizedBase)
    : undefined;
  const plan = byPlanId || byBase || def?.plans[0];
  const protocolRaw = (input.protocol || plan?.protocol || 'openai').toLowerCase();
  const protocol: ModelFetchProtocol = protocolRaw === 'anthropic' ? 'anthropic' : 'openai';

  return {
    providerType: input.providerType,
    baseUrl: input.baseUrl || plan?.baseUrl,
    apiKey: input.apiKey,
    protocol,
  };
}

/** 是否具备发起探索的最低条件（密钥可由引擎本地配置回填）。 */
export function canFetchProviderModels(params: {
  providerType?: string;
  apiKey?: string;
  baseUrl?: string;
}): boolean {
  return Boolean(params.providerType?.trim());
}
