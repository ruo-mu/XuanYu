/**
 * 拉取供应商模型列表：走 init_models_fetch，带缓存与 in-flight 去重。
 * 无静态占位列表；失败返回空数组 + error。
 */

import { initModelsFetch } from '../ipc/desktopIpc';
import {
  buildModelCacheKey,
  getCachedModels,
  getInflight,
  setCachedModels,
  setInflight,
  type ModelCacheEntry,
} from './cache';
import {
  canFetchProviderModels,
  resolveProviderFetchParams,
  type ModelFetchProtocol,
} from './resolveProvider';

export interface FetchProviderModelsInput {
  providerType: string;
  baseUrl?: string;
  apiKey?: string;
  protocol?: string;
  planId?: string;
  /** 忽略 TTL，强制重新请求。 */
  force?: boolean;
  timeoutMs?: number;
}

export interface FetchProviderModelsResult {
  models: string[];
  fromCache: boolean;
  error?: string;
  cacheKey: string;
  protocol: ModelFetchProtocol;
  baseUrl?: string;
}

function filterChatModels(ids: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of ids) {
    const id = typeof raw === 'string' ? raw.trim() : '';
    if (!id) continue;
    if (/embedding|moderation|tts|asr|image|whisper/i.test(id)) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

async function requestModels(params: {
  providerType: string;
  baseUrl?: string;
  apiKey?: string;
  protocol: ModelFetchProtocol;
  timeoutMs?: number;
}): Promise<ModelCacheEntry> {
  try {
    const result = await initModelsFetch({
      providerType: params.providerType,
      baseUrl: params.baseUrl,
      apiKey: params.apiKey,
      protocol: params.protocol,
      timeoutMs: params.timeoutMs ?? 15_000,
    });
    const models = filterChatModels(result.models || []);
    return {
      models,
      fetchedAt: Date.now(),
      error: models.length === 0 ? result.error || undefined : undefined,
    };
  } catch (err) {
    return {
      models: [],
      fetchedAt: Date.now(),
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/** 拉取模型列表；命中缓存则直接返回。 */
export async function fetchProviderModels(
  input: FetchProviderModelsInput,
): Promise<FetchProviderModelsResult> {
  const resolved = resolveProviderFetchParams(input);
  const cacheKey = buildModelCacheKey(resolved);

  if (!canFetchProviderModels(resolved)) {
    return {
      models: [],
      fromCache: false,
      error: undefined,
      cacheKey,
      protocol: resolved.protocol,
      baseUrl: resolved.baseUrl,
    };
  }

  if (!input.force) {
    const cached = getCachedModels(cacheKey);
    if (cached) {
      return {
        models: cached.models,
        fromCache: true,
        error: cached.error,
        cacheKey,
        protocol: resolved.protocol,
        baseUrl: resolved.baseUrl,
      };
    }

    const pending = getInflight(cacheKey);
    if (pending) {
      const shared = await pending;
      return {
        models: shared.models,
        fromCache: true,
        error: shared.error,
        cacheKey,
        protocol: resolved.protocol,
        baseUrl: resolved.baseUrl,
      };
    }
  }

  const promise = requestModels({
    ...resolved,
    timeoutMs: input.timeoutMs,
  }).then((entry) => {
    // 只缓存成功列表，避免空结果占住 TTL
    if (entry.models.length > 0) {
      setCachedModels(cacheKey, entry);
    }
    return entry;
  });
  setInflight(cacheKey, promise);

  const entry = await promise;
  return {
    models: entry.models,
    fromCache: false,
    error: entry.error,
    cacheKey,
    protocol: resolved.protocol,
    baseUrl: resolved.baseUrl,
  };
}
