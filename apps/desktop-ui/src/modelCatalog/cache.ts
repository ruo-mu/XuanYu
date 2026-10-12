/**
 * 模型列表内存 + localStorage 缓存。
 * 按 provider / protocol / baseUrl / apiKey 指纹区分；不含明文密钥。
 */

export interface ModelCacheEntry {
  models: string[];
  fetchedAt: number;
  error?: string;
}

const STORAGE_KEY = 'xy-model-catalog-v1';
/** 默认 30 分钟内复用缓存。 */
export const MODEL_CACHE_TTL_MS = 30 * 60 * 1000;

const memory = new Map<string, ModelCacheEntry>();
const inflight = new Map<string, Promise<ModelCacheEntry>>();

/** FNV-1a 指纹，仅用于缓存键。 */
export function credentialFingerprint(value?: string): string {
  if (!value) return '0';
  let hash = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

export function buildModelCacheKey(params: {
  providerType: string;
  protocol: string;
  baseUrl?: string;
  apiKey?: string;
}): string {
  const base = (params.baseUrl || '').replace(/\/+$/, '').toLowerCase();
  return [
    params.providerType.trim().toLowerCase(),
    params.protocol.trim().toLowerCase(),
    base,
    credentialFingerprint(params.apiKey),
  ].join('|');
}

function readStorage(): Record<string, ModelCacheEntry> {
  if (typeof window === 'undefined') return {};
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, ModelCacheEntry>;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

function writeStorage(all: Record<string, ModelCacheEntry>): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(all));
  } catch {
    // quota / private mode — ignore
  }
}

export function getCachedModels(key: string, ttlMs = MODEL_CACHE_TTL_MS): ModelCacheEntry | null {
  const mem = memory.get(key);
  if (mem && Date.now() - mem.fetchedAt <= ttlMs) return mem;

  const stored = readStorage()[key];
  if (stored && Date.now() - stored.fetchedAt <= ttlMs) {
    memory.set(key, stored);
    return stored;
  }
  return null;
}

export function setCachedModels(key: string, entry: ModelCacheEntry): void {
  memory.set(key, entry);
  const all = readStorage();
  all[key] = entry;
  // 只保留最近 40 条，避免无限膨胀
  const keys = Object.keys(all).sort((a, b) => (all[b].fetchedAt || 0) - (all[a].fetchedAt || 0));
  if (keys.length > 40) {
    for (const stale of keys.slice(40)) delete all[stale];
  }
  writeStorage(all);
}

export function getInflight(key: string): Promise<ModelCacheEntry> | undefined {
  return inflight.get(key);
}

export function setInflight(key: string, promise: Promise<ModelCacheEntry>): void {
  inflight.set(key, promise);
  void promise.finally(() => {
    if (inflight.get(key) === promise) inflight.delete(key);
  });
}

export function invalidateModelCache(key?: string): void {
  if (!key) {
    memory.clear();
    if (typeof window !== 'undefined') {
      try {
        window.localStorage.removeItem(STORAGE_KEY);
      } catch {
        // ignore
      }
    }
    return;
  }
  memory.delete(key);
  const all = readStorage();
  delete all[key];
  writeStorage(all);
}
