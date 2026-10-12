/**
 * 能力市场通用壳：已安装 + 市场双 Tab，布局与交互对齐原插件中心。
 * 通过 kinds / i18nPrefix 区分插件、MCP、技能。
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Alert,
  App,
  Button,
  Empty,
  Input,
  Modal,
  Pagination,
  Progress,
  Select,
  Space,
  Spin,
  Switch,
  Tabs,
  Tag,
  Tooltip,
  Typography,
} from 'antd';
import {
  CheckCircleFilled,
  DeleteOutlined,
  DownloadOutlined,
  LinkOutlined,
  RollbackOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import { formatMarketplaceInstallError } from '../../utils/marketplaceInstallError';
import { useConfigStore } from '../../stores/configStore';
import {
  marketplaceScopeKey,
  useMarketplaceStore,
  filterSourcesForKinds,
  type InstalledCapability,
  type MarketEntry,
  type MarketplaceCatalogKind,
  type MarketplaceInstallPlan,
  type MarketplaceSource,
} from '../../stores/marketplaceStore';
import {
  AdminContent,
  AdminPage,
  PageSurface,
} from '../shell/PanelShell';

const { Text, Paragraph, Title } = Typography;

const PAGE_SIZE_OPTIONS = [10, 15, 20, 30, 50, 100];
const DEFAULT_PAGE_SIZE = 20;

export type { MarketplaceCatalogKind };

export interface MarketplaceHubProps {
  /** 市场条目允许的 kind */
  kinds: MarketplaceCatalogKind[];
  /** i18n 键前缀，如 plugins / mcp / skills */
  i18nPrefix: 'plugins' | 'mcp' | 'skills';
  /** 技能页：已安装 Tab 同时展示运行时已加载的 skills */
  includeRuntimeSkills?: boolean;
  /** keep-alive 时仅当前可见页处理安装事件 */
  active?: boolean;
}

type MainTab = 'installed' | 'market';

/** 全页刷新前，每个市场菜单只向 WS 拉一次列表 */
const fetchedMarketplaceScopes = new Set<string>();

/** 被谁调用：MarketplaceHub。做什么：插件列表滚动容器。 */
function PluginScrollArea({
  children,
  className,
  deps = [],
}: {
  children: React.ReactNode;
  className?: string;
  deps?: unknown[];
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [fadeTop, setFadeTop] = useState(false);
  const [fadeBottom, setFadeBottom] = useState(false);

  const updateFades = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const overflow = scrollHeight > clientHeight + 1;
    setFadeTop(overflow && scrollTop > 2);
    setFadeBottom(overflow && scrollTop + clientHeight < scrollHeight - 2);
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = 0;
    updateFades();
    if (!el || typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(() => updateFades());
    observer.observe(el);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [updateFades, ...deps]);

  return (
    <div
      className={`xy-plugin-scroll-wrap${fadeTop ? ' is-fade-top' : ''}${fadeBottom ? ' is-fade-bottom' : ''}${className ? ` ${className}` : ''}`}
    >
      <div ref={ref} className="xy-plugin-scroll" onScroll={updateFades}>
        {children}
      </div>
    </div>
  );
}

/** 被谁调用：市场卡片。做什么：缩短版本号。 */
function shortVersion(version: string): string {
  const v = (version || '').trim();
  if (!v) return '-';
  if (/^[0-9a-f]{7,}$/i.test(v)) return `${v.slice(0, 7)}…`;
  if (v.length > 18) return `${v.slice(0, 16)}…`;
  return v;
}

/** 被谁调用：市场卡片。做什么：单行截断描述。 */
function oneLine(text: string, max = 72): string {
  const cleaned = (text || '').replace(/\s+/g, ' ').trim();
  if (!cleaned) return '';
  return cleaned.length > max ? `${cleaned.slice(0, max - 1)}…` : cleaned;
}

/** 被谁调用：市场卡片。做什么：头像字母。 */
function avatarLetter(name: string): string {
  const trimmed = (name || '').trim();
  if (!trimmed) return 'P';
  const cjk = trimmed.match(/[\u4e00-\u9fff]/);
  if (cjk) return cjk[0];
  return trimmed.charAt(0).toUpperCase();
}

/** 被谁调用：市场卡片。做什么：头像色调索引。 */
function avatarTone(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  return hash % 6;
}

/** 被谁调用：市场详情。做什么：缩短 digest。 */
function shortDigest(value?: string): string {
  const digest = (value || '').trim();
  if (!digest) return '-';
  return digest.length > 28 ? `${digest.slice(0, 24)}…${digest.slice(-8)}` : digest;
}

/** 市场文案：先试 prefix.key，再 plugins.key，最后 fallback。 */
function tx(
  t: (key: string, fallback?: string) => string,
  prefix: string,
  key: string,
  fallback: string,
): string {
  return t(`${prefix}.${key}`, t(`plugins.${key}`, fallback));
}

/** 被谁调用：安装计划弹层。做什么：风险/变更列表。 */
function PlanList({ title, items }: { title: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div className="xy-market-install-plan-list">
      <Text strong>{title}</Text>
      <ul>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </div>
  );
}

/** 被谁调用：市场页。做什么：搜索/计划安装/确认/已安装管理。 */
export default function MarketplaceHub({
  kinds,
  i18nPrefix,
  includeRuntimeSkills = false,
  active = true,
}: MarketplaceHubProps) {
  const { send, addHandler, connected } = useWs();
  const { t } = useI18n();
  const { message, modal } = App.useApp();
  const extensions = useConfigStore((s) => s.extensions);
  const scope = useMemo(() => marketplaceScopeKey(kinds), [kinds]);
  const kindSet = useMemo(() => new Set(kinds), [kinds]);
  const cachedScope = useMarketplaceStore((s) => s.byScope[scope]);
  const storeInstalled = useMarketplaceStore((s) => s.installed);
  const setScopeResult = useMarketplaceStore((s) => s.setScopeResult);
  const setStoreInstalled = useMarketplaceStore((s) => s.setInstalled);

  const [marketplace, setMarketplace] = useState<MarketEntry[]>(() => cachedScope?.entries || []);
  const [marketplaceSources, setMarketplaceSources] = useState<MarketplaceSource[]>(
    () => cachedScope?.sources || [],
  );
  const [installed, setInstalled] = useState<InstalledCapability[]>(() => (
    storeInstalled.filter((item) => kindSet.has(item.kind as MarketplaceCatalogKind))
  ));
  const [tab, setTab] = useState<MainTab>('market');
  const [search, setSearch] = useState('');
  const [category, setCategory] = useState('all');
  const [sourceId, setSourceId] = useState('all');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [installProgress, setInstallProgress] = useState(0);
  const [mutatingId, setMutatingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<MarketEntry | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [catalogIncomplete, setCatalogIncomplete] = useState(false);
  const [marketTotal, setMarketTotal] = useState(() => cachedScope?.total || 0);
  const [catalogLoaded, setCatalogLoaded] = useState(() => cachedScope?.total || 0);
  const [serverCategories, setServerCategories] = useState<string[]>([]);
  const marketplaceRef = useRef<MarketEntry[]>(cachedScope?.entries || []);
  // 内容请求与摘要轮询各自用独立槽位，避免 5s 轮询覆盖正在发出的翻页请求，
  // 导致翻页响应被 responseId 守卫丢弃、列表停留旧页。
  const pendingRequestId = useRef<string | null>(null);
  const pendingPollId = useRef<string | null>(null);
  const lastCatalogCountRef = useRef(0);
  const searchDebounceRef = useRef<number | null>(null);
  const filterRef = useRef({ search, category, sourceId });
  filterRef.current = { search, category, sourceId };

  useEffect(() => {
    if (!installingId) {
      setInstallProgress(0);
      return;
    }
    setInstallProgress(12);
    const timer = window.setInterval(() => {
      setInstallProgress((prev) => {
        if (prev >= 90) return prev;
        return Math.min(90, prev + 4 + Math.random() * 8);
      });
    }, 350);
    return () => window.clearInterval(timer);
  }, [installingId]);

  useEffect(() => {
    marketplaceRef.current = marketplace;
  }, [marketplace]);

  const applySearchResult = useCallback((
    entriesRaw: MarketEntry[],
    sourcesRaw: MarketplaceSource[],
    total?: number,
  ) => {
    const entries = entriesRaw.filter(
      (p) => !p.kind || kindSet.has(p.kind as MarketplaceCatalogKind),
    );
    const sources = filterSourcesForKinds(sourcesRaw, kinds, entries);
    const stillLoading = sourcesRaw.some((source) => source.status === 'loading');
    if (entries.length === 0 && marketplaceRef.current.length > 0 && stillLoading) {
      setMarketplaceSources((prev) => filterSourcesForKinds(
        sourcesRaw.length ? sourcesRaw : prev,
        kinds,
        marketplaceRef.current,
      ));
      if (typeof total === 'number' && total > 0) {
        setMarketTotal(total);
        setScopeResult(scope, marketplaceRef.current, sourcesRaw.length ? sourcesRaw : [], total);
      }
      return;
    }
    setMarketplace(entries);
    setMarketplaceSources(sources);
    setScopeResult(scope, entries, sources, total);
  }, [kindSet, kinds, scope, setScopeResult]);

  const showInstallPlan = useCallback(
    (plan: MarketplaceInstallPlan) => {
      const entryId = plan.entry?.id || '';
      if (entryId) setInstallingId(entryId);
      setInstallProgress(24);
      const effects = plan.requestedEffects;
      const provenance = plan.provenance;
      modal.confirm({
        title: tx(t, i18nPrefix, 'installReviewTitle', '确认安装能力'),
        width: 760,
        okText: tx(t, i18nPrefix, 'installConfirm', '确认安装'),
        cancelText: t('common.cancel', '取消'),
        onOk: () => {
          send('marketplace.install.confirm', {
            planId: plan.planId,
            approvalDigest: plan.approvalDigest,
          });
        },
        onCancel: () => {
          setInstallingId(null);
          setInstallProgress(0);
        },
        content: (
          <Space direction="vertical" size={12} style={{ width: '100%' }}>
            <Alert
              type={provenance?.signature ? 'info' : 'warning'}
              showIcon
              message={
                provenance?.signature
                  ? tx(t, i18nPrefix, 'signedPlan', '发布者签名存在，请核对摘要后安装')
                  : tx(t, i18nPrefix, 'unsignedPlan', '该能力没有发布者签名，安装前必须核对来源和权限')
              }
            />
            <div className="xy-market-install-review">
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'installTarget', '安装对象')}</Text>
                <Title level={5} style={{ margin: '2px 0 0' }}>
                  {plan.entry?.name || plan.entry?.id} <Text type="secondary">@{plan.entry?.version}</Text>
                </Title>
              </div>
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'installScope', '安装范围')}</Text>
                <div><Tag>{plan.scope}</Tag></div>
              </div>
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'requestedEffects', '请求能力')}</Text>
                <div>
                  {effects.length ? effects.map((effect) => <Tag key={effect} color="volcano">{effect}</Tag>) : <Text>-</Text>}
                </div>
              </div>
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'digest', '审批摘要')}</Text>
                <Paragraph copyable={{ text: plan.approvalDigest }} style={{ marginBottom: 0 }}>
                  <Text code>{shortDigest(plan.approvalDigest)}</Text>
                </Paragraph>
              </div>
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'packageDigest', '包摘要')}</Text>
                <Paragraph copyable={provenance?.digest ? { text: provenance.digest } : false} style={{ marginBottom: 0 }}>
                  <Text code>{shortDigest(provenance?.digest)}</Text>
                </Paragraph>
              </div>
              <div>
                <Text type="secondary">{tx(t, i18nPrefix, 'signature', '签名')}</Text>
                <div>{provenance?.signature ? <Text code>{shortDigest(provenance.signature)}</Text> : <Tag color="warning">missing</Tag>}</div>
              </div>
            </div>
            <PlanList title={tx(t, i18nPrefix, 'changes', '将执行')} items={plan.changes} />
            <PlanList title={tx(t, i18nPrefix, 'risks', '风险')} items={plan.risks} />
            <PlanList title={tx(t, i18nPrefix, 'rollback', '回滚')} items={plan.rollback} />
          </Space>
        ),
      });
    },
    [i18nPrefix, modal, send, t],
  );

  // 切回菜单先用前端缓存秒出（总量用缓存的 total，绝不用当前页条数冒充）
  useEffect(() => {
    const cached = useMarketplaceStore.getState().byScope[scope];
    if (cached?.entries?.length) {
      setMarketplace(cached.entries);
      setMarketplaceSources(filterSourcesForKinds(cached.sources, kinds, cached.entries));
      marketplaceRef.current = cached.entries;
    }
    if (typeof cached?.total === 'number' && cached.total > 0) {
      setMarketTotal(cached.total);
      setCatalogLoaded(cached.total);
    }
    setInstalled(
      useMarketplaceStore.getState().installed
        .filter((item) => kindSet.has(item.kind as MarketplaceCatalogKind)),
    );
  }, [scope, kinds, kindSet]);

  const buildSearchPayload = useCallback((opts?: {
    requestId?: string;
    summaryOnly?: boolean;
    page?: number;
    pageSize?: number;
    query?: string;
    category?: string;
    sourceId?: string;
  }) => {
    const requestId = opts?.requestId || `mkt-${scope}-${Date.now()}`;
    const body: Record<string, unknown> = {
      kinds,
      requestId,
      page: opts?.page ?? page,
      pageSize: opts?.pageSize ?? pageSize,
      query: (opts?.query ?? search).trim() || undefined,
      category: (opts?.category ?? category) !== 'all' ? (opts?.category ?? category) : undefined,
      sourceId: (opts?.sourceId ?? sourceId) !== 'all' ? (opts?.sourceId ?? sourceId) : undefined,
      summaryOnly: opts?.summaryOnly === true ? true : undefined,
    };
    if (kinds.length === 1) body.kind = kinds[0];
    return body;
  }, [scope, kinds, page, pageSize, search, category, sourceId]);

  const requestMarketPage = useCallback((opts?: {
    summaryOnly?: boolean;
    page?: number;
    pageSize?: number;
    query?: string;
    category?: string;
    sourceId?: string;
    showSpinner?: boolean;
  }) => {
    const requestId = `mkt-${scope}-${Date.now()}`;
    pendingRequestId.current = requestId;
    if (opts?.showSpinner && marketplaceRef.current.length === 0) setRefreshing(true);
    send('marketplace.search', buildSearchPayload({ ...opts, requestId }));
  }, [scope, send, buildSearchPayload]);

  // 先挂 handler 再按需发一次 WS search（走后端缓存，不打远程）
  useEffect(() => {
    if (!connected || !active) return;

    const unsubs = [
      addHandler('marketplace.search', (payload) => {
        const body = payload as {
          requestId?: string;
          entries?: MarketEntry[];
          sources?: MarketplaceSource[];
          incomplete?: boolean;
          total?: number;
          categories?: string[];
          catalogCounts?: {
            mcp?: number;
            skills?: number;
            plugins?: number;
          };
        };
        const responseId = body.requestId;
        // 摘要轮询只走总量分支，用独立槽位匹配；翻页/筛选内容请求用 pendingRequestId。
        const isPoll = typeof responseId === 'string' && responseId.includes('poll-');
        const guardRef = isPoll ? pendingPollId : pendingRequestId;
        if (responseId && guardRef.current && responseId !== guardRef.current) {
          return;
        }
        if (!Array.isArray(body.entries) && typeof body.total !== 'number') return;

        const stillLoading = Boolean(body.incomplete)
          || (body.sources || []).some((source) => source.status === 'loading');
        const total = typeof body.total === 'number' ? body.total : body.entries?.length || 0;
        if (typeof body.total === 'number') setMarketTotal(body.total);
        if (Array.isArray(body.categories)) setServerCategories(body.categories.filter(Boolean));

        const catalogCount = kinds.includes('mcp') && kinds.length === 1
          ? (body.catalogCounts?.mcp ?? total)
          : kinds.includes('skill') && kinds.length === 1
            ? (body.catalogCounts?.skills ?? total)
            : kinds.includes('plugin') && kinds.length === 1
              ? (body.catalogCounts?.plugins ?? total)
              : total;
        if (catalogCount > 0) {
          setCatalogLoaded(catalogCount);
          const filter = filterRef.current;
          // 无筛选时页签/底部分页总量跟远程目录一致。
          if (!filter.search.trim() && filter.category === 'all' && filter.sourceId === 'all') {
            setMarketTotal(catalogCount);
          }
        }

        // 摘要轮询：只更新总量；数量涨了再拉当前页。
        if (body.entries?.length === 0 && stillLoading) {
          setCatalogIncomplete(true);
          setRefreshing(false);
          if (catalogCount > 0) {
            setScopeResult(
              scope,
              marketplaceRef.current,
              Array.isArray(body.sources) ? body.sources : [],
              catalogCount,
            );
          }
          if (catalogCount > lastCatalogCountRef.current + 50) {
            lastCatalogCountRef.current = catalogCount;
            requestMarketPage({ showSpinner: false });
          } else {
            lastCatalogCountRef.current = Math.max(lastCatalogCountRef.current, catalogCount);
          }
          return;
        }

        if (!Array.isArray(body.entries)) {
          setCatalogIncomplete(stillLoading);
          setRefreshing(false);
          return;
        }

        lastCatalogCountRef.current = Math.max(lastCatalogCountRef.current, catalogCount);
        applySearchResult(
          body.entries,
          Array.isArray(body.sources) ? body.sources : [],
          catalogCount > 0 ? catalogCount : (typeof body.total === 'number' ? body.total : undefined),
        );
        setCatalogIncomplete(stillLoading);
        setRefreshing(false);
      }),
      addHandler('capability.installed.list', (payload) => {
        const body = payload as { capabilities?: InstalledCapability[] };
        if (!Array.isArray(body.capabilities)) return;
        const all = body.capabilities;
        setStoreInstalled(all);
        setInstalled(all.filter((item) => kindSet.has(item.kind as MarketplaceCatalogKind)));
      }),
      addHandler('marketplace.install.plan', (plan) => {
        showInstallPlan(plan as MarketplaceInstallPlan);
      }),
      addHandler('marketplace.install.operation', (operation) => {
        const op = operation as {
          status?: string;
          action?: string;
          error?: unknown;
        };
        if (op.status === 'completed') {
          const action = String(op.action || 'install');
          setMutatingId(null);
          if (action === 'install' || action === 'update') {
            setInstallProgress(100);
            window.setTimeout(() => setInstallingId(null), 280);
            message.success(tx(t, i18nPrefix, 'installSuccess', '安装成功'));
            setDetail(null);
          } else if (action === 'enable') {
            message.success(tx(t, i18nPrefix, 'enableSuccess', '已启用'));
          } else if (action === 'disable') {
            message.success(tx(t, i18nPrefix, 'disableSuccess', '已禁用'));
          } else if (action === 'uninstall') {
            message.success(tx(t, i18nPrefix, 'uninstallSuccess', '已卸载'));
            setDetail(null);
          } else if (action === 'rollback') {
            message.success(tx(t, i18nPrefix, 'rollbackSuccess', '已回滚到上一版本'));
          } else {
            message.success(tx(t, i18nPrefix, 'operationSuccess', '操作成功'));
          }
          send('capability.installed.list', {});
          send('extensions.list', {});
          requestMarketPage({ showSpinner: false });
        } else {
          setInstallingId(null);
          setMutatingId(null);
          message.error(formatMarketplaceInstallError(
            typeof op.error === 'string' ? op.error : undefined,
            t,
          ));
        }
      }),
    ];

    if (!fetchedMarketplaceScopes.has(scope)) {
      fetchedMarketplaceScopes.add(scope);
      requestMarketPage({ showSpinner: true });
      send('capability.installed.list', {});
      if (includeRuntimeSkills) send('extensions.list', {});
    }

    return () => {
      unsubs.forEach((unsubscribe) => unsubscribe());
    };
  }, [
    connected,
    active,
    scope,
    kinds,
    kindSet,
    includeRuntimeSkills,
    send,
    addHandler,
    applySearchResult,
    setStoreInstalled,
    setScopeResult,
    t,
    message,
    i18nPrefix,
    showInstallPlan,
    requestMarketPage,
  ]);

  useEffect(() => {
    if (!connected || !active || !catalogIncomplete) return;
    const timer = window.setInterval(() => {
      const requestId = `mkt-${scope}-poll-${Date.now()}`;
      pendingPollId.current = requestId;
      send('marketplace.search', buildSearchPayload({ requestId, summaryOnly: true }));
    }, 5000);
    return () => window.clearInterval(timer);
  }, [connected, active, catalogIncomplete, scope, send, buildSearchPayload]);

  // 搜索 / 筛选 / 翻页走服务端分页，避免一次拉几万条。
  useEffect(() => {
    if (!connected || !active || !fetchedMarketplaceScopes.has(scope)) return;
    if (searchDebounceRef.current) window.clearTimeout(searchDebounceRef.current);
    searchDebounceRef.current = window.setTimeout(() => {
      requestMarketPage({ showSpinner: false });
    }, search ? 220 : 0);
    return () => {
      if (searchDebounceRef.current) window.clearTimeout(searchDebounceRef.current);
    };
  }, [connected, active, scope, page, pageSize, search, category, sourceId, requestMarketPage]);

  const runtimeSkillRows = useMemo((): InstalledCapability[] => {
    if (!includeRuntimeSkills) return [];
    const skills = extensions?.skills || [];
    return skills.map((skill) => ({
      id: skill.id,
      name: skill.name,
      version: skill.version || '',
      enabled: true,
      kind: 'skill',
    }));
  }, [includeRuntimeSkills, extensions?.skills]);

  const installedMerged = useMemo(() => {
    if (!includeRuntimeSkills) return installed;
    const byId = new Map<string, InstalledCapability>();
    for (const row of runtimeSkillRows) byId.set(row.id, row);
    for (const row of installed) byId.set(row.id, row);
    return Array.from(byId.values());
  }, [installed, includeRuntimeSkills, runtimeSkillRows]);

  const entryById = useMemo(() => {
    const map = new Map<string, MarketEntry>();
    for (const item of marketplace) {
      map.set(item.id, item);
      if (item.externalId) map.set(item.externalId, item);
    }
    return map;
  }, [marketplace]);

  const installedIds = useMemo(() => {
    const ids = new Set<string>();
    for (const item of installedMerged) ids.add(item.id);
    return ids;
  }, [installedMerged]);

  const categories = useMemo(
    () => (serverCategories.length
      ? serverCategories
      : [...new Set(marketplace.map((p) => p.category).filter(Boolean))].sort()),
    [serverCategories, marketplace],
  );

  useEffect(() => {
    if (sourceId !== 'all' && !marketplaceSources.some((source) => source.id === sourceId)) {
      setSourceId('all');
    }
  }, [marketplaceSources, sourceId]);

  useEffect(() => {
    if (category !== 'all' && !categories.includes(category)) {
      setCategory('all');
    }
  }, [categories, category]);

  const sourceNames = useMemo(
    () => new Map(marketplaceSources.map((item) => [item.id, item.name])),
    [marketplaceSources],
  );

  const sourceLabel = (id?: string) => {
    if (id === 'builtin') return t('plugins.builtinSource', '内置');
    return sourceNames.get(id || '') || id || t('common.unknown', 'Unknown');
  };

  const findLockRecord = (id: string) => installed.find((p) => p.id === id);

  const isInstalled = (id: string) => installedIds.has(id);
  const isEnabled = (id: string) => {
    const row = installedMerged.find((p) => p.id === id);
    return !!row?.enabled;
  };
  const isBuiltin = (id: string) => Boolean(installedMerged.find((p) => p.id === id)?.builtin);
  const canToggle = (id: string) => {
    const row = findLockRecord(id);
    return Boolean(row && !row.builtin);
  };
  const canRollback = (id: string) => {
    const row = findLockRecord(id);
    return Boolean(row && !row.builtin && Array.isArray(row.previous) && row.previous.length > 0);
  };
  const resolveMutateId = (id: string) => findLockRecord(id)?.id || id;

  const marketAvailable = useMemo(
    () => marketplace.filter((p) => !isInstalled(p.id)),
    [marketplace, installedIds],
  );

  const marketSourceFailed = useMemo(
    () => marketplaceSources.some((source) => source.status === 'error' || source.status === 'unavailable'),
    [marketplaceSources],
  );

  const reloadMarket = useCallback(() => {
    fetchedMarketplaceScopes.delete(scope);
    fetchedMarketplaceScopes.add(scope);
    lastCatalogCountRef.current = 0;
    setRefreshing(true);
    requestMarketPage({ showSpinner: true, page: 1 });
  }, [scope, requestMarketPage]);

  // 服务端已按 search/category/source/page 过滤；这里只去掉已安装项。
  const marketEntries = marketAvailable;
  const pageItems = marketAvailable;

  const installedViews = useMemo(() => {
    const q = search.toLowerCase();
    return installedMerged
      .map((item) => {
        const entry = entryById.get(item.id);
        const runtime = includeRuntimeSkills
          ? (extensions?.skills || []).find((skill) => skill.id === item.id)
          : undefined;
        return {
          record: item,
          entry: (entry ||
            item.entry || {
              id: item.id,
              name: item.name,
              version: item.version,
              description: runtime?.description || '',
              author: item.builtin ? '玄语' : '',
              category: item.kind === 'channel' ? 'channel' : '',
              install: { type: item.builtin ? 'builtin' : '' },
              tags: item.builtin ? ['builtin'] : [],
              sourceId: item.builtin ? 'builtin' : undefined,
            }) as MarketEntry,
          runtime,
        };
      })
      .filter(({ record, entry }) => {
        if (!q) return true;
        return record.name.toLowerCase().includes(q)
          || record.id.toLowerCase().includes(q)
          || (entry.description || '').toLowerCase().includes(q);
      })
      .sort((a, b) => a.record.name.localeCompare(b.record.name));
  }, [installedMerged, entryById, search, includeRuntimeSkills, extensions?.skills]);

  useEffect(() => {
    setPage(1);
  }, [search, category, sourceId, tab, pageSize]);

  const handleInstall = (plugin: MarketEntry) => {
    setInstallingId(plugin.id);
    send('marketplace.install.plan', { entryId: plugin.id, scope: 'user' });
  };

  const label = (key: string, fallback: string) => tx(t, i18nPrefix, key, fallback);

  const toggleEnabled = (id: string, enabled: boolean) => {
    const entryId = resolveMutateId(id);
    setMutatingId(entryId);
    send('capability.installed.mutate', {
      entryId,
      action: enabled ? 'enable' : 'disable',
    });
  };

  const handleUninstall = (id: string, name: string) => {
    const entryId = resolveMutateId(id);
    modal.confirm({
      title: label('uninstallConfirmTitle', '确认卸载'),
      content: label('uninstallConfirm', '将从本机删除「{0}」及其安装文件，此操作不可恢复。').replace('{0}', name),
      okText: label('uninstall', '卸载'),
      okType: 'danger',
      cancelText: t('common.cancel', '取消'),
      onOk: () => {
        setMutatingId(entryId);
        send('capability.installed.mutate', {
          entryId,
          action: 'uninstall',
        });
      },
    });
  };

  const handleRollback = (id: string, name: string) => {
    const entryId = resolveMutateId(id);
    const row = findLockRecord(id);
    const previousVersion = row?.previous?.[row.previous.length - 1]?.version;
    modal.confirm({
      title: label('rollbackConfirmTitle', '确认回滚'),
      content: label('rollbackConfirm', '将「{0}」回滚到上一版本{1}').replace('{0}', name).replace(
        '{1}',
        previousVersion ? ` (${previousVersion})` : '',
      ),
      okText: label('rollbackAction', '回滚'),
      cancelText: t('common.cancel', '取消'),
      onOk: () => {
        setMutatingId(entryId);
        send('capability.installed.mutate', {
          entryId,
          action: 'rollback',
        });
      },
    });
  };

  const openDetail = (plugin: MarketEntry) => setDetail(plugin);

  const renderMarketCard = (plugin: MarketEntry) => {
    const installing = installingId === plugin.id;
    return (
      <article
        key={plugin.id}
        className={`xy-plugin-tile${installing ? ' is-installing' : ''}`}
        onClick={() => openDetail(plugin)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            openDetail(plugin);
          }
        }}
        role="button"
        tabIndex={0}
      >
        <div className="xy-plugin-tile-top">
          <div className={`xy-plugin-avatar tone-${avatarTone(plugin.id)}`} aria-hidden>
            {avatarLetter(plugin.name)}
          </div>
          <div className="xy-plugin-tile-copy">
            <div className="xy-plugin-tile-title">{plugin.name}</div>
            <div className="xy-plugin-tile-meta">
              <span>{plugin.author || label('unknownAuthor', '未知作者')}</span>
              <span className="xy-plugin-dot" />
              <span>{shortVersion(plugin.version)}</span>
            </div>
          </div>
          {!installing ? (
            <Tooltip title={plugin.installable === false ? plugin.installReason : label('install', '安装')}>
              <Button
                type="text"
                size="small"
                className="xy-plugin-tile-install"
                icon={<DownloadOutlined />}
                disabled={plugin.installable === false}
                aria-label={label('install', '安装')}
                onClick={(event) => {
                  event.stopPropagation();
                  handleInstall(plugin);
                }}
              />
            </Tooltip>
          ) : null}
        </div>
        <p className="xy-plugin-tile-desc">
          {oneLine(plugin.description, 88) || plugin.id}
        </p>
        <div className="xy-plugin-tile-foot">
          <div className="xy-plugin-tile-tags">
            {plugin.category ? <span className="xy-plugin-chip">{plugin.category}</span> : null}
            <span className="xy-plugin-chip is-soft">{sourceLabel(plugin.sourceId)}</span>
          </div>
        </div>
        {installing ? (
          <div
            className="xy-plugin-tile-overlay"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <div className="xy-plugin-tile-overlay-panel">
              <Progress
                percent={Math.round(installProgress)}
                showInfo={false}
                strokeColor="#fafafa"
                trailColor="rgba(255,255,255,0.18)"
                size={[180, 6]}
              />
              <span className="xy-plugin-tile-overlay-text">
                {label('installing', '安装中…')}
                {' '}
                {Math.round(installProgress)}
                %
              </span>
            </div>
          </div>
        ) : null}
      </article>
    );
  };

  const renderInstalledCard = (item: {
    record: InstalledCapability;
    entry: MarketEntry;
  }) => {
    const { record, entry } = item;
    return (
      <article
        key={record.id}
        className="xy-plugin-tile is-installed"
        onClick={() => openDetail(entry)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            openDetail(entry);
          }
        }}
        role="button"
        tabIndex={0}
      >
        <div className="xy-plugin-tile-top">
          <div className={`xy-plugin-avatar tone-${avatarTone(record.id)}`} aria-hidden>
            {avatarLetter(record.name)}
          </div>
          <div className="xy-plugin-tile-copy">
            <div className="xy-plugin-tile-title">{record.name}</div>
            <div className="xy-plugin-tile-meta">
              <span>{entry.author || label('unknownAuthor', '未知作者')}</span>
              <span className="xy-plugin-dot" />
              <span>{shortVersion(record.version || entry.version)}</span>
            </div>
          </div>
          <span className={`xy-plugin-badge${record.builtin ? ' is-builtin' : record.enabled ? ' is-on' : ''}`}>
            {record.builtin ? (
              label('builtin', '内置')
            ) : record.enabled ? (
              <>
                <CheckCircleFilled /> {label('enabled', '已启用')}
              </>
            ) : (
              t('common.disabled', '已禁用')
            )}
          </span>
        </div>
        <p className="xy-plugin-tile-desc">
          {oneLine(entry.description, 88) || record.id}
        </p>
        <div className="xy-plugin-tile-foot">
          <div className="xy-plugin-tile-tags">
            {entry.category ? <span className="xy-plugin-chip">{entry.category}</span> : null}
            {record.builtin ? (
              <span className="xy-plugin-chip is-soft">{label('builtinSource', '内置')}</span>
            ) : (
              <span className="xy-plugin-chip is-soft">{sourceLabel(entry.sourceId)}</span>
            )}
          </div>
        </div>
      </article>
    );
  };

  return (
    <AdminPage>
      <AdminContent wide>
        <div className="xy-plugin-page">
          <PageSurface className="xy-plugin-surface">
            <Tabs
              className="xy-plugin-tabs"
              activeKey={tab}
              onChange={(key) => setTab(key as MainTab)}
              items={[
                {
                  key: 'installed',
                  label: (
                    <span className="xy-plugin-tab-label">
                      {label('tabInstalled', '已安装')}
                      <em>{installedMerged.length}</em>
                    </span>
                  ),
                  children: (
                    <div className="xy-plugin-panel">
                      {installedMerged.length > 0 ? (
                        <div className="xy-plugin-toolbar">
                          <Input
                            allowClear
                            prefix={<SearchOutlined />}
                            placeholder={label('searchInstalled', '搜索已安装...')}
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="xy-plugin-search"
                          />
                        </div>
                      ) : null}
                      {installedMerged.length === 0 ? (
                        <Empty
                          className="xy-plugin-empty"
                          description={label('emptyInstalled', '还没有安装任何条目')}
                          image={Empty.PRESENTED_IMAGE_SIMPLE}
                        >
                          <Button type="primary" onClick={() => setTab('market')}>
                            {label('goMarket', '去市场看看')}
                          </Button>
                        </Empty>
                      ) : (
                        <PluginScrollArea
                          className={installedViews.length === 0 ? 'is-empty' : undefined}
                          deps={[installedViews.length, search, tab]}
                        >
                          {installedViews.length === 0 ? (
                            <Empty
                              className="xy-plugin-empty is-filtered"
                              description={label('noResults', '没有匹配结果')}
                              image={Empty.PRESENTED_IMAGE_SIMPLE}
                            />
                          ) : (
                            <div className="xy-plugin-tile-grid">
                              {installedViews.map(renderInstalledCard)}
                            </div>
                          )}
                        </PluginScrollArea>
                      )}
                    </div>
                  ),
                },
                {
                  key: 'market',
                  label: (
                    <span className="xy-plugin-tab-label">
                      {label('tabMarket', '市场')}
                      {marketTotal > 0 ? <em>{marketTotal}</em> : null}
                    </span>
                  ),
                  children: (
                    <div className="xy-plugin-panel">
                      {marketTotal > 0 || marketAvailable.length > 0 ? (
                        <div className="xy-plugin-toolbar">
                          <Input
                            allowClear
                            prefix={<SearchOutlined />}
                            placeholder={label('searchPlaceholder', '搜索...')}
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="xy-plugin-search"
                          />
                          <Select
                            value={category}
                            onChange={setCategory}
                            className="xy-plugin-select"
                            options={[
                              { value: 'all', label: label('allCategories', '全部分类') },
                              ...categories.map((c) => ({ value: c, label: c })),
                            ]}
                          />
                          <Select
                            value={sourceId}
                            onChange={setSourceId}
                            className="xy-plugin-select is-wide"
                            options={[
                              { value: 'all', label: label('allSources', '全部来源') },
                              ...marketplaceSources.map((source) => ({
                                value: source.id,
                                label: source.name,
                              })),
                            ]}
                          />
                          {catalogIncomplete ? (
                            <span className="xy-plugin-syncing">
                              {label('syncingCatalog', '后台补全中，已加载 {0} 个…')
                                .replace('{0}', String(Math.max(catalogLoaded, marketTotal) || 0))}
                            </span>
                          ) : null}
                        </div>
                      ) : null}

                      {marketAvailable.length === 0 && marketTotal === 0 ? (
                        refreshing ? (
                          <div className="xy-plugin-empty" style={{ padding: 48, textAlign: 'center' }}>
                            <Spin tip={t('common.loading', '加载中...')} />
                          </div>
                        ) : (
                          <Empty
                            className="xy-plugin-empty"
                            description={
                              marketSourceFailed
                                ? label('marketLoadFailed', '插件目录暂时拉不到，请检查网络后重试')
                                : label('noResults', '没有匹配结果')
                            }
                            image={Empty.PRESENTED_IMAGE_SIMPLE}
                          >
                            {marketSourceFailed ? (
                              <Button type="primary" onClick={reloadMarket}>
                                {label('retryLoad', '重新加载')}
                              </Button>
                            ) : null}
                          </Empty>
                        )
                      ) : (
                        <>
                          <PluginScrollArea
                            className={pageItems.length === 0 ? 'is-empty' : undefined}
                            deps={[pageItems.length, page, pageSize, search, category, sourceId, tab]}
                          >
                            {pageItems.length === 0 ? (
                              <Empty
                                className="xy-plugin-empty is-filtered"
                                description={label('noResults', '没有匹配结果')}
                                image={Empty.PRESENTED_IMAGE_SIMPLE}
                              />
                            ) : (
                              <div className="xy-plugin-tile-grid">
                                {pageItems.map(renderMarketCard)}
                              </div>
                            )}
                          </PluginScrollArea>

                          {marketTotal > 0 || marketEntries.length > 0 ? (
                            <div className="xy-plugin-pager">
                              <Pagination
                                current={page}
                                pageSize={pageSize}
                                total={Math.max(marketTotal, marketEntries.length)}
                                showSizeChanger
                                pageSizeOptions={PAGE_SIZE_OPTIONS.map(String)}
                                showTotal={(total) =>
                                  label('pageTotal', '共 {0} 个').replace('{0}', String(total))}
                                onChange={(nextPage, nextSize) => {
                                  setPage(nextPage);
                                  if (nextSize && nextSize !== pageSize) setPageSize(nextSize);
                                }}
                                onShowSizeChange={(_current, size) => {
                                  setPage(1);
                                  setPageSize(size);
                                }}
                              />
                            </div>
                          ) : null}
                        </>
                      )}
                    </div>
                  ),
                },
              ]}
            />
          </PageSurface>
        </div>

        <Modal
          open={!!detail}
          title={null}
          onCancel={() => setDetail(null)}
          footer={null}
          width={680}
          destroyOnHidden
          className="xy-plugin-detail-modal"
          rootClassName="xy-plugin-detail-modal-root"
        >
          {detail && (
            <div className="xy-plugin-detail">
              <div className="xy-plugin-detail-head">
                <div className="xy-plugin-detail-identity">
                  <div className={`xy-plugin-avatar is-lg tone-${avatarTone(detail.id)}`} aria-hidden>
                    {avatarLetter(detail.name)}
                  </div>
                  <div className="xy-plugin-detail-title-block">
                    <div className="xy-plugin-detail-title-row">
                      <Title level={4} className="xy-plugin-detail-name">{detail.name}</Title>
                      {detail.homepage ? (
                        <a
                          className="xy-plugin-detail-home"
                          href={detail.homepage}
                          target="_blank"
                          rel="noreferrer"
                        >
                          <LinkOutlined />
                          <span>{label('homepage', '主页')}</span>
                        </a>
                      ) : null}
                    </div>
                    <Text type="secondary" className="xy-plugin-detail-id">{detail.id}</Text>
                  </div>
                </div>
                {isInstalled(detail.id) && canToggle(detail.id) ? (
                  <div className="xy-plugin-installed-actions">
                    <span className="xy-plugin-switch-label">
                      {isEnabled(detail.id) ? label('enabled', '已启用') : t('common.disabled', '已禁用')}
                    </span>
                    <Switch
                      checked={isEnabled(detail.id)}
                      loading={mutatingId === detail.id}
                      disabled={mutatingId === detail.id}
                      onChange={(enabled) => toggleEnabled(detail.id, enabled)}
                    />
                    <Button
                      danger
                      icon={<DeleteOutlined />}
                      loading={mutatingId === detail.id}
                      disabled={mutatingId === detail.id}
                      onClick={() => handleUninstall(detail.id, detail.name)}
                    >
                      {label('uninstall', '卸载')}
                    </Button>
                    {canRollback(detail.id) ? (
                      <Button
                        icon={<RollbackOutlined />}
                        loading={mutatingId === detail.id}
                        disabled={mutatingId === detail.id}
                        onClick={() => handleRollback(detail.id, detail.name)}
                      >
                        {label('rollbackAction', '回滚')}
                      </Button>
                    ) : null}
                  </div>
                ) : isInstalled(detail.id) && isBuiltin(detail.id) ? (
                  <div className="xy-plugin-installed-actions">
                    <span className="xy-plugin-badge is-builtin">{label('builtin', '内置')}</span>
                    <Text type="secondary">{label('builtinReadonly', '产品内置，不可修改')}</Text>
                  </div>
                ) : !isInstalled(detail.id) ? (
                  <Tooltip title={detail.installable === false ? detail.installReason : undefined}>
                    <Button
                      type="primary"
                      className="xy-plugin-detail-install"
                      icon={<DownloadOutlined />}
                      loading={installingId === detail.id}
                      disabled={detail.installable === false || installingId === detail.id}
                      onClick={() => handleInstall(detail)}
                    >
                      {installingId === detail.id
                        ? `${label('installing', '安装中…')} ${Math.round(installProgress)}%`
                        : label('install', '安装')}
                    </Button>
                  </Tooltip>
                ) : null}
              </div>

              <div className="xy-plugin-detail-tags">
                {detail.category ? <span className="xy-plugin-chip">{detail.category}</span> : null}
                <span className="xy-plugin-chip is-soft">
                  {isBuiltin(detail.id) ? label('builtinSource', '内置') : sourceLabel(detail.sourceId)}
                </span>
                <span
                  className={`xy-plugin-badge${
                    isBuiltin(detail.id)
                      ? ' is-builtin'
                      : isInstalled(detail.id) && isEnabled(detail.id)
                        ? ' is-on'
                        : ''
                  }`}
                >
                  {isBuiltin(detail.id)
                    ? label('builtin', '内置')
                    : isInstalled(detail.id)
                      ? (isEnabled(detail.id) ? label('enabled', '已启用') : t('common.disabled', '已禁用'))
                      : label('notInstalled', '未安装')}
                </span>
              </div>

              <Paragraph className="xy-plugin-detail-desc">
                {detail.description || label('noDescription', '暂无描述')}
              </Paragraph>

              <dl className="xy-plugin-detail-grid">
                <div>
                  <dt>{label('author', '作者')}</dt>
                  <dd>{detail.author || label('unknownAuthor', '未知作者')}</dd>
                </div>
                <div>
                  <dt>{label('version', '版本')}</dt>
                  <dd title={detail.version}>{shortVersion(detail.version)}</dd>
                </div>
                <div>
                  <dt>{label('installType', '安装方式')}</dt>
                  <dd>{detail.install?.type || '-'}</dd>
                </div>
                <div>
                  <dt>{label('source', '市场来源')}</dt>
                  <dd>{sourceLabel(detail.sourceId)}</dd>
                </div>
              </dl>

              {(detail.tags || []).length > 0 && (
                <div className="xy-plugin-detail-section">
                  <div className="xy-plugin-detail-label">{label('tags', '标签')}</div>
                  <Space size={[6, 6]} wrap>
                    {detail.tags.map((tag) => <Tag key={tag}>{tag}</Tag>)}
                  </Space>
                </div>
              )}

              {(detail.install?.package || detail.install?.command || detail.install?.url || detail.install?.repository) && (
                <div className="xy-plugin-detail-section">
                  <div className="xy-plugin-detail-label">{label('installCommand', '安装命令')}</div>
                  <pre className="xy-plugin-detail-code">
                    {detail.install.package
                      || [detail.install.command, ...(detail.install.args || [])].filter(Boolean).join(' ')
                      || detail.install.url
                      || detail.install.repository}
                  </pre>
                </div>
              )}
            </div>
          )}
        </Modal>
      </AdminContent>
    </AdminPage>
  );
}
