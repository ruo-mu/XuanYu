/**
 * 市场页前端缓存：跨重启保留列表；列表走 WS，后端磁盘缓存秒回。
 */

import { create } from 'zustand';

/** 市场目录条目类型。 */
export type MarketplaceCatalogKind = 'plugin' | 'mcp' | 'skill' | 'channel';

/** 市场列表里的一条可安装条目。 */
export interface MarketEntry {
  id: string;
  kind?: string;
  name: string;
  version: string;
  description: string;
  author: string;
  category: string;
  install: {
    type: string;
    package?: string;
    command?: string;
    args?: string[];
    url?: string;
    repository?: string;
    path?: string;
  };
  tags: string[];
  requestedEffects: string[];
  sourceId?: string;
  externalId?: string;
  homepage?: string;
  installable?: boolean;
  installReason?: string;
  provenance: {
    registry: string;
    digest: string;
    signature?: string;
  };
}

/** 安装前审批计划（planId + digest + 风险说明）。 */
export interface MarketplaceInstallPlan {
  planId: string;
  entry: MarketEntry;
  scope: 'user' | 'project' | 'org';
  approvalDigest: string;
  requestedEffects: string[];
  provenance: MarketEntry['provenance'];
  risks: string[];
  changes: string[];
  rollback: string[];
  createdAt: number;
  expiresAt: number;
}

/** 市场来源（原生 / MCP registry / ClawHub 等）。 */
export interface MarketplaceSource {
  id: string;
  name: string;
  official: boolean;
  type?: string;
  status?: 'ready' | 'cached' | 'unavailable' | 'loading' | 'error';
  error?: string;
  location?: string;
}

/** 已安装能力行（含内置只读项）。 */
export interface InstalledCapability {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  kind: string;
  /** 产品内置：只读，不可启用/禁用/卸载 */
  builtin?: boolean;
  entry?: Partial<MarketEntry>;
  /** lock 文件中的历史版本栈，供回滚 */
  previous?: Array<{ version?: string; digest?: string; path?: string; enabled?: boolean }>;
}

interface ScopeCache {
  entries: MarketEntry[];
  sources: MarketplaceSource[];
  /** 目录真实总量（不是当前页条数） */
  total?: number;
  updatedAt: number;
}

interface MarketplaceUiState {
  byScope: Record<string, ScopeCache>;
  installed: InstalledCapability[];
  setScopeResult: (
    scope: string,
    entries: MarketEntry[],
    sources: MarketplaceSource[],
    total?: number,
  ) => void;
  setInstalled: (installed: InstalledCapability[]) => void;
}

const STORAGE_KEY = 'xy-marketplace-ui-v2';
const LEGACY_SESSION_KEY = 'xy-marketplace-ui-v1';
/** localStorage 配额紧张时每个 scope 最多保留条数，保证重启仍能秒出首屏。 */
const MAX_PERSIST_ENTRIES = 600;

/** 读取 localStorage（并迁移旧 sessionStorage）。 */
function readPersisted(): Pick<MarketplaceUiState, 'byScope' | 'installed'> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY) || sessionStorage.getItem(LEGACY_SESSION_KEY);
    if (!raw) return { byScope: {}, installed: [] };
    const parsed = JSON.parse(raw) as Partial<Pick<MarketplaceUiState, 'byScope' | 'installed'>>;
    const byScope = parsed.byScope && typeof parsed.byScope === 'object' ? parsed.byScope : {};
    const installed = Array.isArray(parsed.installed) ? parsed.installed : [];
    if (!localStorage.getItem(STORAGE_KEY) && (Object.keys(byScope).length || installed.length)) {
      writePersisted(byScope, installed);
    }
    try {
      sessionStorage.removeItem(LEGACY_SESSION_KEY);
    } catch {
      // ignore
    }
    return { byScope, installed };
  } catch {
    return { byScope: {}, installed: [] };
  }
}

/** 配额不够时截断每个 scope 的条目，仍保留 sources 与时间戳。 */
function compactByScope(byScope: Record<string, ScopeCache>): Record<string, ScopeCache> {
  const next: Record<string, ScopeCache> = {};
  for (const [scope, cache] of Object.entries(byScope)) {
    next[scope] = {
      ...cache,
      entries: cache.entries.slice(0, MAX_PERSIST_ENTRIES),
    };
  }
  return next;
}

/** 把 byScope / installed 写回 localStorage；满则截断重试。 */
function writePersisted(byScope: Record<string, ScopeCache>, installed: InstalledCapability[]): void {
  const payload = { byScope, installed };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
    return;
  } catch {
    // fall through
  }
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ byScope: compactByScope(byScope), installed }),
    );
  } catch {
    // ignore quota
  }
}

/** kinds 排序后拼成缓存键；空则 `all`。 */
export function marketplaceScopeKey(kinds: MarketplaceCatalogKind[]): string {
  return [...kinds].sort().join('+') || 'all';
}

/** 各菜单允许出现的市场来源类型 */
export function allowedSourceTypesForKinds(kinds: MarketplaceCatalogKind[]): Set<string> {
  const types = new Set<string>(['native']);
  for (const kind of kinds) {
    if (kind === 'mcp') types.add('mcp-registry');
    if (kind === 'skill') types.add('clawhub');
    if (kind === 'plugin' || kind === 'channel') {
      types.add('claude');
      types.add('codex');
    }
  }
  return types;
}

/**
 * 按菜单 kinds 过滤来源：类型不在白名单则丢；
 * 有条目时仅保留被引用或仍 loading 的源。
 */
export function filterSourcesForKinds(
  sources: MarketplaceSource[],
  kinds: MarketplaceCatalogKind[],
  entries: MarketEntry[],
): MarketplaceSource[] {
  const allowed = allowedSourceTypesForKinds(kinds);
  const usedIds = new Set(entries.map((item) => item.sourceId).filter(Boolean) as string[]);
  return sources.filter((source) => {
    if (source.type && !allowed.has(source.type)) return false;
    if (!source.type) {
      return usedIds.has(source.id) || source.status === 'loading';
    }
    if (entries.length > 0) return usedIds.has(source.id) || source.status === 'loading';
    return true;
  });
}

const persisted = readPersisted();

/** 市场页 Zustand store：切菜单 / 重启不丢列表。 */
export const useMarketplaceStore = create<MarketplaceUiState>((set, get) => ({
  byScope: persisted.byScope,
  installed: persisted.installed,
  setScopeResult: (scope, entries, sources, total) => {
    set((state) => {
      const prev = state.byScope[scope];
      // 后台同步中途空包不覆盖已有列表；但仍可刷新总量
      if (entries.length === 0 && prev?.entries?.length) {
        if (typeof total === 'number' && total > 0 && total !== prev.total) {
          const byScope = {
            ...state.byScope,
            [scope]: { ...prev, total, sources: sources.length ? sources : prev.sources, updatedAt: Date.now() },
          };
          writePersisted(byScope, state.installed);
          return { byScope };
        }
        return state;
      }
      const nextTotal = typeof total === 'number' && total > 0
        ? total
        : (prev?.total && prev.total > entries.length ? prev.total : undefined);
      // 数量没涨且已有数据：只刷新 sources/total，避免无意义重写超大缓存
      if (
        prev?.entries?.length
        && entries.length === prev.entries.length
        && entries[0]?.id === prev.entries[0]?.id
        && entries[entries.length - 1]?.id === prev.entries[prev.entries.length - 1]?.id
      ) {
        const byScope = {
          ...state.byScope,
          [scope]: {
            ...prev,
            sources,
            total: nextTotal ?? prev.total,
            updatedAt: Date.now(),
          },
        };
        writePersisted(byScope, state.installed);
        return { byScope };
      }
      const byScope = {
        ...state.byScope,
        [scope]: {
          entries,
          sources,
          total: nextTotal,
          updatedAt: Date.now(),
        },
      };
      writePersisted(byScope, state.installed);
      return { byScope };
    });
  },
  setInstalled: (installed) => {
    set({ installed });
    writePersisted(get().byScope, installed);
  },
}));
