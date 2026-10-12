/**
 * SystemStatusPanel — 运行状态（控制台风格：摘要条 + 环境 + 日志）
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Input, Segmented, Switch, Typography } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import type { GatewayLogEntry, GatewayLogLevel, SystemStatusPayload } from '../../protocol';
import { useConfigStore } from '../../stores/configStore';
import { DataBlock, PageSurface } from '../shell/PanelShell';

const { Text } = Typography;

type LevelFilter = GatewayLogLevel | 'all';

/** 被谁调用：SystemStatusPanel。做什么：压缩 details 对象成短串。 */
function compactDetails(details: Record<string, unknown>): string {
  const text = JSON.stringify(details);
  return text.length > 160 ? `${text.slice(0, 157)}...` : text;
}

/** 被谁调用：状态/统计 UI。做什么：毫秒 → 可读时长。 */
function formatDuration(ms: number): string {
  if (!ms) return '—';
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

/** Strip Windows `\\?\` extended path prefix for display. */
function displayPath(path?: string): string {
  if (!path) return '—';
  const trimmed = path.trim();
  if (trimmed.startsWith('\\\\?\\UNC\\')) return `\\\\${trimmed.slice(8)}`;
  if (trimmed.startsWith('\\\\?\\')) return trimmed.slice(4);
  if (trimmed.startsWith('//?/')) return trimmed.slice(4).replace(/\//g, '\\');
  // Avoid file-URL style leftovers like `/C:/Users/...`
  if (/^\/[A-Za-z]:\//.test(trimmed)) return trimmed.slice(1).replace(/\//g, '\\');
  return trimmed;
}

/** Compact postgres URL (host:port/db); pass through local paths cleaned. */
function summarizeDbLocation(location?: string): string {
  if (!location) return '—';
  const trimmed = location.trim();
  if (!/^postgres(ql)?:\/\//i.test(trimmed)) {
    return displayPath(trimmed);
  }
  try {
    const normalized = trimmed.replace(/^postgres(ql)?:\/\//i, 'http://');
    const url = new URL(normalized);
    const db = url.pathname.replace(/^\//, '') || '';
    return db ? `${url.host}/${db}` : url.host || '—';
  } catch {
    return trimmed.length > 48 ? `${trimmed.slice(0, 45)}…` : trimmed;
  }
}

/** Backend chip label for conversation / memory stores. */
function backendLabel(
  backend: string | undefined,
  lt: (zh: string, en: string, ...args: Array<string | number>) => string,
): string {
  const value = (backend || '').toLowerCase();
  if (value === 'file') return lt('本地文件', 'Local file');
  if (value === 'postgres' || value === 'postgresql') return 'PostgreSQL';
  if (value === 'sqlite' || value === 'local') return 'SQLite';
  return backend || '—';
}

/** 被谁调用：系统页。做什么：拉 system.status 展示健康摘要。 */
export default function SystemStatusPanel() {
  const { send, addHandler, connected } = useWs();
  const { locale } = useI18n();
  const lt = useLocaleText();
  const config = useConfigStore((s) => s.config);
  const [status, setStatus] = useState<SystemStatusPayload | null>(null);
  const [logs, setLogs] = useState<GatewayLogEntry[]>([]);
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('info');
  const [search, setSearch] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(
    () =>
      addHandler((msg) => {
        if (msg.type === 'system.status') {
          setStatus(msg.payload as SystemStatusPayload);
        }
        if (msg.type === 'gateway.logs') {
          const payload = msg.payload as { entries?: GatewayLogEntry[] };
          setLogs(payload.entries || []);
        }
        if (msg.type === 'gateway.log') {
          const payload = msg.payload as { entry?: GatewayLogEntry };
          if (!payload.entry) return;
          setLogs((items) => [...items, payload.entry as GatewayLogEntry].slice(-300));
        }
      }),
    [addHandler],
  );

  useEffect(() => {
    send('system.status', {});
    send('gateway.logs', { limit: 200 });
  }, [send]);

  const filteredLogs = useMemo(() => {
    const q = search.trim().toLowerCase();
    const minRank =
      levelFilter === 'all' ? -1 : { debug: 0, info: 1, warn: 2, error: 3 }[levelFilter];
    return logs.filter((item) => {
      const rank = { debug: 0, info: 1, warn: 2, error: 3 }[item.level];
      if (minRank > rank) return false;
      if (!q) return true;
      const haystack = `${item.scope} ${item.message} ${
        item.details ? JSON.stringify(item.details) : ''
      }`.toLowerCase();
      return haystack.includes(q);
    });
  }, [logs, levelFilter, search]);

  const errorCount = useMemo(
    () => logs.filter((item) => item.level === 'error').length,
    [logs],
  );
  const warnCount = useMemo(() => logs.filter((item) => item.level === 'warn').length, [logs]);

  useEffect(() => {
    if (!autoScroll || !listRef.current) return;
    listRef.current.scrollTop = 0;
  }, [filteredLogs, autoScroll]);

  const refresh = () => {
    send('system.status', {});
    send('gateway.logs', { limit: 200 });
  };

  const toggleDetails = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // 桌面端不开 TCP 监听（port 0），通信全部走 Electron IPC。
  const gatewayCfg = status?.gateway ?? config?.gateway;
  const gatewayBind = gatewayCfg
    ? gatewayCfg.port > 0
      ? `${gatewayCfg.bind}:${gatewayCfg.port}`
      : 'IPC'
    : '—';

  const storageOk =
    status?.conversation.ok !== false && (status?.memory.ok !== false || status?.memory.ok == null);
  const alertTone = errorCount ? 'is-danger' : warnCount ? 'is-warn' : 'is-ok';
  const memoryPg = status?.memory.pgvector;
  const memoryBackend = status?.memory.backend || config?.memory?.backend || 'postgres';

  return (
    <div className="xy-settings-stack xy-runtime-status">
      <PageSurface>
        <div className="xy-runtime-strip">
          <div className="xy-runtime-strip-metrics">
            <div className="xy-runtime-metric">
              <span className="xy-runtime-metric-label">{lt('引擎', 'Engine')}</span>
              <span className={`xy-runtime-metric-value${connected ? ' is-ok' : ' is-danger'}`}>
                <span className="xy-runtime-dot" aria-hidden />
                {connected ? lt('在线', 'Online') : lt('离线', 'Offline')}
              </span>
            </div>
            <div className="xy-runtime-metric">
              <span className="xy-runtime-metric-label">{lt('运行时间', 'Uptime')}</span>
              <span className="xy-runtime-metric-value">
                {formatDuration((status?.runtime?.uptimeSec || 0) * 1000)}
              </span>
            </div>
            <div className="xy-runtime-metric">
              <span className="xy-runtime-metric-label">{lt('连接数', 'Clients')}</span>
              <span className="xy-runtime-metric-value">
                {status?.gateway?.wsClients ?? (connected ? 1 : 0)}
              </span>
            </div>
            <div className="xy-runtime-metric">
              <span className="xy-runtime-metric-label">{lt('告警', 'Alerts')}</span>
              <span className={`xy-runtime-metric-value ${alertTone}`}>
                {warnCount}/{errorCount}
              </span>
            </div>
            <div className="xy-runtime-metric">
              <span className="xy-runtime-metric-label">{lt('存储', 'Storage')}</span>
              <span className={`xy-runtime-metric-value${storageOk ? ' is-ok' : ' is-danger'}`}>
                {storageOk ? lt('正常', 'Healthy') : lt('异常', 'Unhealthy')}
              </span>
            </div>
          </div>
          <Button size="small" icon={<ReloadOutlined />} onClick={refresh}>
            {lt('刷新', 'Refresh')}
          </Button>
        </div>
      </PageSurface>

      <PageSurface>
        <DataBlock title={lt('环境', 'Environment')}>
          <dl className="xy-runtime-facts">
            <div>
              <dt>{lt('工作区', 'Workspace')}</dt>
              <dd>
                <code>{displayPath(status?.projectDir || status?.runtime?.cwd)}</code>
              </dd>
            </div>
            <div>
              <dt>{lt('通信方式', 'Transport')}</dt>
              <dd>
                <code>{gatewayBind}</code>
              </dd>
            </div>
            <div>
              <dt>{lt('会话存储', 'Sessions')}</dt>
              <dd>
                <code>{summarizeDbLocation(status?.conversation.location)}</code>
                <span className="xy-runtime-fact-tags">
                  <span className="xy-runtime-chip">
                    {backendLabel(status?.conversation.backend, lt)}
                  </span>
                  {typeof status?.conversation.pgvector === 'boolean' ? (
                    <span
                      className={`xy-runtime-chip${
                        status.conversation.pgvector ? ' is-ok' : ' is-warn'
                      }`}
                    >
                      pgvector{' '}
                      {status.conversation.pgvector ? lt('可用', 'on') : lt('未就绪', 'off')}
                    </span>
                  ) : null}
                </span>
              </dd>
            </div>
            <div>
              <dt>{lt('记忆库', 'Memory DB')}</dt>
              <dd>
                <code>{summarizeDbLocation(status?.memory.location)}</code>
                <span className="xy-runtime-fact-tags">
                  <span className="xy-runtime-chip">{backendLabel(memoryBackend, lt)}</span>
                  {typeof memoryPg === 'boolean' ? (
                    <span className={`xy-runtime-chip${memoryPg ? ' is-ok' : ' is-warn'}`}>
                      pgvector {memoryPg ? lt('可用', 'on') : lt('未就绪', 'off')}
                    </span>
                  ) : null}
                  {status?.memory.postgresConfigured && status?.memory.ok === false ? (
                    <span className="xy-runtime-chip is-warn">
                      {lt('未接入', 'not attached')}
                    </span>
                  ) : null}
                </span>
              </dd>
            </div>
            <div>
              <dt>{lt('运行时', 'Runtime')}</dt>
              <dd>
                {status?.runtime
                  ? `Node ${status.runtime.node} · PID ${status.runtime.pid} · ${status.runtime.platform}`
                  : '—'}
              </dd>
            </div>
          </dl>
          {status?.conversation.error ? (
            <Text type="danger" className="xy-settings-hint">
              {status.conversation.error}
            </Text>
          ) : null}
        </DataBlock>
      </PageSurface>

      <PageSurface>
        <DataBlock
          title={lt('实时日志', 'Live logs')}
          extra={
            <span className="xy-runtime-log-count">
              {filteredLogs.length} / {logs.length}
            </span>
          }
        >
          <div className="xy-runtime-log-toolbar">
            <Segmented<LevelFilter>
              size="small"
              value={levelFilter}
              onChange={(v) => setLevelFilter(v)}
              options={[
                { label: 'ALL', value: 'all' },
                { label: 'INFO', value: 'info' },
                { label: 'WARN', value: 'warn' },
                { label: 'ERR', value: 'error' },
              ]}
            />
            <Input.Search
              allowClear
              size="small"
              placeholder={lt('搜索 scope / 消息', 'Search scope / message')}
              className="xy-runtime-log-search"
              onChange={(e) => setSearch(e.target.value)}
            />
            <label className="xy-runtime-log-autoscroll">
              <Switch size="small" checked={autoScroll} onChange={setAutoScroll} />
              <span>{lt('跟随', 'Follow')}</span>
            </label>
          </div>

          {filteredLogs.length === 0 ? (
            <div className="xy-runtime-log-empty">
              <Text type="secondary">
                {lt(
                  '暂无匹配日志。调整过滤或等待新事件。',
                  'No matching logs. Adjust filters or wait for events.',
                )}
              </Text>
            </div>
          ) : (
            <div ref={listRef} className="xy-runtime-log-list">
              {[...filteredLogs].reverse().map((item) => {
                const expanded = expandedIds.has(item.id);
                const hasDetails = Boolean(item.details && Object.keys(item.details).length > 0);
                return (
                  <div
                    key={item.id}
                    className={`xy-runtime-log-line is-${item.level}`}
                  >
                    <time className="xy-runtime-log-time">
                      {new Date(item.timestamp).toLocaleTimeString(locale, {
                        hour12: false,
                      })}
                    </time>
                    <span className={`xy-runtime-log-level is-${item.level}`}>
                      {item.level.toUpperCase()}
                    </span>
                    <span className="xy-runtime-log-scope">{item.scope}</span>
                    <div className="xy-runtime-log-body">
                      <span className="xy-runtime-log-message">{item.message}</span>
                      {hasDetails && (
                        <div className="xy-runtime-log-details-wrap">
                          {expanded ? (
                            <pre className="xy-runtime-log-details">
                              {JSON.stringify(item.details, null, 2)}
                            </pre>
                          ) : (
                            <code className="xy-runtime-log-details-inline">
                              {compactDetails(item.details!)}
                            </code>
                          )}
                          <button
                            type="button"
                            className="xy-runtime-log-toggle"
                            onClick={() => toggleDetails(item.id)}
                          >
                            {expanded ? lt('收起', 'Collapse') : lt('展开', 'Expand')}
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </DataBlock>
      </PageSurface>
    </div>
  );
}
