/**
 * 订阅供应商模型列表（缓存 + 自动拉取）。
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { fetchProviderModels } from './fetchModels';
import { canFetchProviderModels } from './resolveProvider';

export interface UseProviderModelsParams {
  providerType?: string;
  baseUrl?: string;
  apiKey?: string;
  protocol?: string;
  planId?: string;
  /** false 时不自动拉取（例如抽屉未打开）。 */
  enabled?: boolean;
  /** 当前已选模型：若不在列表中则前置展示，避免选中值消失。 */
  currentModel?: string;
}

export interface UseProviderModelsResult {
  models: string[];
  loading: boolean;
  error?: string;
  fromCache: boolean;
  refresh: () => void;
}

const API_KEY_DEBOUNCE_MS = 400;

/** 被谁调用：设置 / 对话 / init 选模型。做什么：带缓存的模型列表。 */
export function useProviderModels(params: UseProviderModelsParams): UseProviderModelsResult {
  const {
    providerType,
    baseUrl,
    apiKey,
    protocol,
    planId,
    enabled = true,
    currentModel,
  } = params;

  const [debouncedApiKey, setDebouncedApiKey] = useState(apiKey);
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const [fromCache, setFromCache] = useState(false);
  const seqRef = useRef(0);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedApiKey(apiKey), API_KEY_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [apiKey]);

  const load = useCallback(
    async (force: boolean) => {
      if (!providerType || !canFetchProviderModels({ providerType, apiKey: debouncedApiKey, baseUrl })) {
        setModels([]);
        setError(undefined);
        setFromCache(false);
        setLoading(false);
        return;
      }
      const seq = ++seqRef.current;
      setLoading(true);
      try {
        const result = await fetchProviderModels({
          providerType,
          baseUrl,
          apiKey: debouncedApiKey,
          protocol,
          planId,
          force,
        });
        if (seq !== seqRef.current) return;
        setModels(result.models);
        setError(result.error);
        setFromCache(result.fromCache);
      } catch (err) {
        if (seq !== seqRef.current) return;
        setModels([]);
        setError(err instanceof Error ? err.message : String(err));
        setFromCache(false);
      } finally {
        if (seq === seqRef.current) setLoading(false);
      }
    },
    [baseUrl, debouncedApiKey, planId, protocol, providerType],
  );

  useEffect(() => {
    if (!enabled) return;
    void load(false);
  }, [enabled, load]);

  const displayModels =
    currentModel && !models.includes(currentModel) ? [currentModel, ...models] : models;

  return {
    models: displayModels,
    loading,
    error,
    fromCache,
    refresh: () => {
      void load(true);
    },
  };
}
