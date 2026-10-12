/**
 * StatsPanel — Token 用量监控
 *
 * 指标卡 + 模型分布 / Token 趋势 / 工具调用 / 会话消耗（均来自真实 analytics）
 */

import React, { useEffect, useMemo } from 'react';
import { Empty, Table } from 'antd';
import {
  BarChartOutlined,
  ClockCircleOutlined,
  DatabaseOutlined,
  DollarOutlined,
  FieldTimeOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import ReactECharts from 'echarts-for-react';
import type { EChartsOption } from 'echarts';

import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import { useTheme } from '../../theme/ThemeContext';
import { useSessionStore } from '../../stores/sessionStore';
import { toolDisplayName } from '../../utils/toolCatalog';
import type { AnalyticsRow, ToolAnalyticsRow, TokenTrendPoint } from '../../protocol';
import {
  AdminContent,
  AdminPage,
  PageSurface,
} from '../shell/PanelShell';

/** 低饱和色板：克制、偏 slate/teal，避免霓虹彩虹 */
const MODEL_COLORS_DARK = ['#5eead4', '#94a3b8', '#7dd3fc', '#a5b4fc', '#86efac', '#c4b5fd', '#67e8f9', '#cbd5e1'];
const MODEL_COLORS_LIGHT = ['#0f766e', '#64748b', '#0369a1', '#4f46e5', '#15803d', '#6d28d9', '#0e7490', '#475569'];

type LocaleText = (zh: string, en: string) => string;

/** 被谁调用：StatsPanel。做什么：模型色板。 */
function modelPalette(isDark: boolean): string[] {
  return isDark ? MODEL_COLORS_DARK : MODEL_COLORS_LIGHT;
}

/** 同色系按排名衰减透明度，形成层次而非彩虹条 */
function toolRankColor(rankIndex: number, count: number, isDark: boolean): string {
  const t = count <= 1 ? 0 : rankIndex / Math.max(1, count - 1);
  const alpha = Math.round((0.92 - t * 0.52) * 100) / 100;
  return isDark ? `rgba(45, 212, 191, ${alpha})` : `rgba(15, 118, 110, ${alpha})`;
}

/** 被谁调用：资源导航。做什么：会话/模型/工具用量图表。 */
export default function StatsPanel() {
  const analytics = useSessionStore((s) => s.analytics);
  const { send } = useWs();
  const { locale } = useI18n();
  const lt = useLocaleText();
  const { resolved } = useTheme();
  const isDark = resolved.mode === 'dark';

  useEffect(() => {
    send('stats.get', {});
    const interval = setInterval(() => send('stats.get', {}), 10000);
    return () => clearInterval(interval);
  }, [send]);

  const today = analytics?.today;
  const totals = analytics?.totals;
  const performance = analytics?.performance;
  const quality = analytics?.quality;
  const trend = analytics?.trend || [];

  const palette = useMemo(() => modelPalette(isDark), [isDark]);

  const models = useMemo(() => {
    const rows = [...(analytics?.byModel || [])].sort((a, b) => b.totalTokens - a.totalTokens);
    return rows.map((row, index) => ({
      ...row,
      color: palette[index % palette.length],
      short: shortModel(row.label),
    }));
  }, [analytics?.byModel, palette]);

  const tools = useMemo(() => {
    const ranked = [...(analytics?.byTool || [])]
      .sort((a, b) => b.toolCallCount - a.toolCallCount)
      .slice(0, 10);
    return ranked.map((row, index) => ({
      ...row,
      display: toolDisplayName(row.key || row.label, locale),
      color: toolRankColor(index, ranked.length, isDark),
    }));
  }, [analytics?.byTool, locale, isDark]);

  const sessions = useMemo(() => {
    return [...(analytics?.bySession || [])]
      .sort((a, b) => b.totalTokens - a.totalTokens)
      .slice(0, 8)
      .map((row) => ({
        ...row,
        short: shortSession(row.label),
      }));
  }, [analytics?.bySession]);

  const modelTokenSum = models.reduce((sum, row) => sum + row.totalTokens, 0);
  const chartTheme = useMemo(() => buildChartTheme(isDark), [isDark]);

  const donutOption = useMemo(
    () => buildDonutOption(models, modelTokenSum, locale, lt, chartTheme, isDark),
    [models, modelTokenSum, locale, lt, chartTheme, isDark],
  );

  const trendOption = useMemo(
    () => buildTrendOption(trend, locale, lt, chartTheme, isDark),
    [trend, locale, lt, chartTheme, isDark],
  );

  const toolOption = useMemo(
    () => buildToolOption(tools, locale, lt, chartTheme),
    [tools, locale, lt, chartTheme],
  );

  const empty = !analytics || ((totals?.totalTokens || 0) === 0 && (totals?.requestCount || 0) === 0);

  return (
    <AdminPage className="xy-usage-page">
      <AdminContent wide>
        <div className="xy-usage-hero is-six">
          <MetricCard
            icon={<BarChartOutlined />}
            tone="emerald"
            label={lt('今日请求', 'Today Requests')}
            value={(today?.requestCount || 0).toLocaleString(locale)}
            sub={`${lt('总计', 'Total')}: ${(totals?.requestCount || 0).toLocaleString(locale)}`}
          />
          <MetricCard
            icon={<DollarOutlined />}
            tone="violet"
            label={lt('今日消费', 'Today Spend')}
            value={`$${(today?.costUsd || 0).toFixed(4)}`}
            sub={`${lt('标准', 'List')} $${(today?.standardCostUsd || 0).toFixed(4)} · ${lt('累计', 'Total')} $${(totals?.costUsd || 0).toFixed(4)}`}
          />
          <MetricCard
            icon={<ThunderboltOutlined />}
            tone="amber"
            label={lt('今日 Token', 'Today Tokens')}
            value={formatTokens(today?.totalTokens || 0, locale)}
            sub={`${lt('输入', 'In')} ${formatTokens(today?.inputTokens || 0, locale)} / ${lt('输出', 'Out')} ${formatTokens(today?.outputTokens || 0, locale)}`}
          />
          <MetricCard
            icon={<DatabaseOutlined />}
            tone="cyan"
            label={lt('累计 Token', 'Total Tokens')}
            value={formatTokens(totals?.totalTokens || 0, locale)}
            sub={`${lt('输入', 'In')} ${formatTokens(totals?.inputTokens || 0, locale)} / ${lt('输出', 'Out')} ${formatTokens(totals?.outputTokens || 0, locale)}`}
          />
          <MetricCard
            icon={<FieldTimeOutlined />}
            tone="sky"
            label={lt('性能指标', 'Performance')}
            value={`${performance?.rpm || 0}`}
            unit="RPM"
            sub={`${(performance?.tpm || 0).toLocaleString(locale)} TPM`}
          />
          <MetricCard
            icon={<ClockCircleOutlined />}
            tone="rose"
            label={lt('平均响应', 'Avg Latency')}
            value={formatDuration(quality?.averageDurationMs)}
            sub={lt('平均耗时', 'Average duration')}
          />
        </div>

        {empty ? (
          <PageSurface className="xy-usage-panel">
            <Empty description={lt('暂无用量数据，跑几轮对话后再看', 'No usage yet — run a few chats first')} />
          </PageSurface>
        ) : (
          <div className="xy-usage-grid is-quad">
            <PageSurface className="xy-usage-panel">
              <div className="xy-usage-panel-head">
                <div className="xy-usage-panel-title">
                  <span>{lt('模型分布', 'Model Distribution')}</span>
                </div>
              </div>
              {models.length === 0 ? (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={lt('暂无模型数据', 'No model data')} />
              ) : (
                <div className="xy-usage-model-block">
                  <div className="xy-usage-chart xy-usage-chart--fill">
                    <ReactECharts option={donutOption} style={{ height: '100%', width: '100%' }} opts={{ renderer: 'canvas' }} notMerge lazyUpdate />
                  </div>
                  <Table<AnalyticsRow & { color: string; short: string }>
                    className="xy-usage-table"
                    size="small"
                    pagination={false}
                    rowKey="key"
                    dataSource={models}
                    columns={[
                      {
                        title: lt('模型', 'Model'),
                        dataIndex: 'short',
                        key: 'model',
                        align: 'center',
                        ellipsis: true,
                        render: (value: string, row) => (
                          <span className="xy-usage-model-cell">
                            <span className="xy-usage-rank-dot" style={{ background: row.color }} />
                            {value}
                          </span>
                        ),
                      },
                      {
                        title: lt('请求', 'Requests'),
                        dataIndex: 'runCount',
                        key: 'req',
                        width: 64,
                        align: 'center',
                      },
                      {
                        title: 'Token',
                        dataIndex: 'totalTokens',
                        key: 'tok',
                        width: 78,
                        align: 'center',
                        render: (v: number) => formatTokens(v, locale),
                      },
                      {
                        title: lt('实际', 'Actual'),
                        dataIndex: 'costUsd',
                        key: 'actual',
                        width: 88,
                        align: 'center',
                        render: (v: number) => <span className="xy-usage-cost-actual">${v.toFixed(4)}</span>,
                      },
                      {
                        title: lt('标准', 'List'),
                        dataIndex: 'standardCostUsd',
                        key: 'std',
                        width: 88,
                        align: 'center',
                        render: (v?: number) => `$${(v || 0).toFixed(4)}`,
                      },
                    ]}
                  />
                </div>
              )}
            </PageSurface>

            <PageSurface className="xy-usage-panel">
              <div className="xy-usage-panel-head">
                <div className="xy-usage-panel-title">
                  <span>{lt('Token 使用趋势', 'Token Usage Trend')}</span>
                </div>
              </div>
              {trend.length === 0 ? (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={lt('暂无趋势', 'No trend yet')} />
              ) : (
                <div className="xy-usage-chart xy-usage-chart--fill">
                  <ReactECharts option={trendOption} style={{ height: '100%', width: '100%' }} opts={{ renderer: 'canvas' }} notMerge lazyUpdate />
                </div>
              )}
            </PageSurface>

            <PageSurface className="xy-usage-panel">
              <div className="xy-usage-panel-head">
                <div className="xy-usage-panel-title">
                  <span>{lt('工具调用排行', 'Tool Usage')}</span>
                </div>
                {tools.length > 0 ? (
                  <span className="xy-usage-panel-meta">
                    {lt('Top', 'Top')} {tools.length} · {lt('失败率', 'Fail')}{' '}
                    {((quality?.toolFailureRate || 0) * 100).toFixed(1)}%
                  </span>
                ) : null}
              </div>
              {tools.length === 0 ? (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={lt('暂无工具调用', 'No tool calls yet')} />
              ) : (
                <div className="xy-usage-chart xy-usage-chart--fill">
                  <ReactECharts option={toolOption} style={{ height: '100%', width: '100%' }} opts={{ renderer: 'canvas' }} notMerge lazyUpdate />
                </div>
              )}
            </PageSurface>

            <PageSurface className="xy-usage-panel">
              <div className="xy-usage-panel-head">
                <div className="xy-usage-panel-title">
                  <span>{lt('会话消耗', 'Session Usage')}</span>
                </div>
                {sessions.length > 0 ? (
                  <span className="xy-usage-panel-meta">
                    {lt('按 Token 占比', 'Share by tokens')}
                  </span>
                ) : null}
              </div>
              {sessions.length === 0 ? (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={lt('暂无会话数据', 'No session data')} />
              ) : (
                <SessionShareList sessions={sessions} locale={locale} lt={lt} />
              )}
            </PageSurface>
          </div>
        )}
      </AdminContent>
    </AdminPage>
  );
}

/** 被谁调用：StatsPanel。做什么：单个指标卡。 */
function MetricCard({
  icon,
  label,
  value,
  sub,
  tone,
  unit,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  sub: string;
  tone: 'emerald' | 'violet' | 'amber' | 'cyan' | 'sky' | 'rose';
  unit?: string;
}) {
  return (
    <div className={`xy-usage-card is-${tone}`}>
      <div className="xy-usage-card-icon" aria-hidden>
        {icon}
      </div>
      <div className="xy-usage-card-body">
        <span className="xy-usage-card-label">{label}</span>
        <div className="xy-usage-card-value-row">
          <strong className="xy-usage-card-value">{value}</strong>
          {unit ? <span className="xy-usage-card-unit">{unit}</span> : null}
        </div>
        <span className="xy-usage-card-sub">{sub}</span>
      </div>
    </div>
  );
}

/** 被谁调用：StatsPanel。做什么：会话占用列表。 */
function SessionShareList({
  sessions,
  locale,
  lt,
}: {
  sessions: Array<AnalyticsRow & { short: string }>;
  locale: string;
  lt: LocaleText;
}) {
  const maxTokens = Math.max(1, sessions[0]?.totalTokens || 0);
  const sumTokens = Math.max(1, sessions.reduce((s, row) => s + row.totalTokens, 0));

  return (
    <ul className="xy-usage-session-rank">
      {sessions.map((row, index) => {
        const share = (row.totalTokens / sumTokens) * 100;
        const width = Math.max(4, (row.totalTokens / maxTokens) * 100);
        return (
          <li key={row.key} className="xy-usage-session-rank-item">
            <span className="xy-usage-session-rank-no">{String(index + 1).padStart(2, '0')}</span>
            <div className="xy-usage-session-rank-main">
              <div className="xy-usage-session-rank-head">
                <span className="xy-usage-session-rank-name" title={row.label}>
                  {row.short}
                </span>
                <strong className="xy-usage-session-rank-tokens">{formatTokens(row.totalTokens, locale)}</strong>
              </div>
              <div className="xy-usage-session-rank-track" aria-hidden>
                <span style={{ width: `${width}%` }} />
              </div>
              <div className="xy-usage-session-rank-meta">
                <span>{share.toFixed(1)}%</span>
                <span>
                  {lt('运行', 'Runs')} {row.runCount.toLocaleString(locale)}
                </span>
                <span>${row.costUsd.toFixed(4)}</span>
                <span>
                  {lt('工具', 'Tools')} {row.toolCallCount.toLocaleString(locale)}
                </span>
              </div>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

interface ChartTheme {
  text: string;
  muted: string;
  faint: string;
  border: string;
  split: string;
  tooltipBg: string;
  tooltipBorder: string;
}

/** 被谁调用：StatsPanel 图表。做什么：按亮暗色生成 ECharts 主题。 */
function buildChartTheme(isDark: boolean): ChartTheme {
  return {
    text: isDark ? 'rgba(248,250,252,0.92)' : 'rgba(15,23,42,0.9)',
    muted: isDark ? 'rgba(148,163,184,0.9)' : 'rgba(100,116,139,0.95)',
    faint: isDark ? 'rgba(148,163,184,0.55)' : 'rgba(148,163,184,0.85)',
    border: isDark ? 'rgba(148,163,184,0.18)' : 'rgba(15,23,42,0.08)',
    split: isDark ? 'rgba(148,163,184,0.12)' : 'rgba(15,23,42,0.06)',
    tooltipBg: isDark ? 'rgba(15,23,42,0.94)' : 'rgba(255,255,255,0.96)',
    tooltipBorder: isDark ? 'rgba(148,163,184,0.22)' : 'rgba(15,23,42,0.08)',
  };
}

/** 被谁调用：StatsPanel。做什么：环形图 option。 */
function buildDonutOption(
  models: Array<AnalyticsRow & { color: string; short: string }>,
  tokenSum: number,
  locale: string,
  lt: LocaleText,
  theme: ChartTheme,
  isDark: boolean,
): EChartsOption {
  const gap = isDark ? 'rgba(12, 12, 16, 0.92)' : 'rgba(255, 255, 255, 0.96)';
  const track = isDark ? 'rgba(148, 163, 184, 0.12)' : 'rgba(15, 23, 42, 0.06)';
  return {
    color: models.map((m) => m.color),
    tooltip: {
      trigger: 'item',
      backgroundColor: theme.tooltipBg,
      borderColor: theme.tooltipBorder,
      textStyle: { color: theme.text, fontSize: 12 },
      formatter: (params: unknown) => {
        const p = params as { name: string; value: number; percent: number };
        return `${p.name}<br/>${formatTokens(p.value, locale)} · ${p.percent.toFixed(1)}%`;
      },
    },
    series: [
      {
        type: 'pie',
        radius: ['62%', '76%'],
        center: ['50%', '50%'],
        silent: true,
        z: 1,
        label: { show: false },
        data: [{ value: 1, itemStyle: { color: track } }],
      },
      {
        type: 'pie',
        radius: ['62%', '76%'],
        center: ['50%', '50%'],
        z: 2,
        avoidLabelOverlap: true,
        padAngle: models.length > 1 ? 2 : 0,
        itemStyle: {
          borderRadius: 5,
          borderColor: gap,
          borderWidth: models.length > 1 ? 3 : 0,
        },
        label: { show: false },
        labelLine: { show: false },
        emphasis: {
          scale: true,
          scaleSize: 3,
          itemStyle: { shadowBlur: 8, shadowColor: 'rgba(0,0,0,0.12)' },
        },
        data: models.map((m) => ({
          name: m.short,
          value: m.totalTokens,
          itemStyle: { color: m.color },
        })),
      },
    ],
    graphic: [
      {
        type: 'text',
        left: 'center',
        top: '43%',
        style: {
          text: formatTokens(tokenSum, locale),
          fill: theme.text,
          fontSize: 20,
          fontWeight: 600,
          fontFamily: 'ui-sans-serif, system-ui, sans-serif',
          align: 'center',
        },
      },
      {
        type: 'text',
        left: 'center',
        top: '56%',
        style: {
          text: lt('总 Token', 'Tokens'),
          fill: theme.faint,
          fontSize: 11,
          align: 'center',
        },
      },
    ],
  };
}

/** 被谁调用：StatsPanel。做什么：趋势折线 option。 */
function buildTrendOption(
  points: TokenTrendPoint[],
  locale: string,
  lt: LocaleText,
  theme: ChartTheme,
  isDark: boolean,
): EChartsOption {
  const labels = points.map((p) => p.label);
  const colors = isDark
    ? ['#7dd3fc', '#5eead4', '#cbd5e1', '#a5b4fc', '#94a3b8']
    : ['#0284c7', '#0f766e', '#64748b', '#4f46e5', '#94a3b8'];
  return {
    color: colors,
    tooltip: {
      trigger: 'axis',
      backgroundColor: theme.tooltipBg,
      borderColor: theme.tooltipBorder,
      textStyle: { color: theme.text, fontSize: 12 },
      axisPointer: { type: 'line', lineStyle: { color: theme.border, width: 1 } },
    },
    legend: {
      bottom: 0,
      left: 'center',
      icon: 'circle',
      itemWidth: 8,
      itemHeight: 8,
      itemGap: 16,
      textStyle: { color: theme.muted, fontSize: 11 },
      data: [
        lt('输入', 'Input'),
        lt('输出', 'Output'),
        lt('缓存创建', 'Cache Create'),
        lt('缓存读取', 'Cache Read'),
        lt('命中率', 'Hit Rate'),
      ],
    },
    grid: {
      left: 8,
      right: 12,
      top: 12,
      bottom: 36,
      containLabel: true,
    },
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: labels,
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: { color: theme.faint, fontSize: 11 },
    },
    yAxis: [
      {
        type: 'value',
        axisLabel: {
          color: theme.faint,
          fontSize: 10,
          formatter: (v: number) => formatTokens(v, locale),
        },
        splitLine: { lineStyle: { color: theme.split, type: 'dashed' } },
        axisLine: { show: false },
        axisTick: { show: false },
      },
      {
        type: 'value',
        min: 0,
        max: 100,
        axisLabel: {
          color: theme.faint,
          fontSize: 10,
          formatter: '{value}%',
        },
        splitLine: { show: false },
        axisLine: { show: false },
        axisTick: { show: false },
      },
    ],
    series: [
      {
        name: lt('输入', 'Input'),
        type: 'line',
        smooth: 0.35,
        showSymbol: points.length <= 2,
        symbolSize: 5,
        lineStyle: { width: 2 },
        areaStyle: {
          color: {
            type: 'linear',
            x: 0,
            y: 0,
            x2: 0,
            y2: 1,
            colorStops: [
              { offset: 0, color: isDark ? 'rgba(125,211,252,0.18)' : 'rgba(2,132,199,0.12)' },
              { offset: 1, color: 'transparent' },
            ],
          },
        },
        data: points.map((p) => p.inputTokens),
      },
      {
        name: lt('输出', 'Output'),
        type: 'line',
        smooth: 0.35,
        showSymbol: points.length <= 2,
        symbolSize: 5,
        lineStyle: { width: 2 },
        data: points.map((p) => p.outputTokens),
      },
      {
        name: lt('缓存创建', 'Cache Create'),
        type: 'line',
        smooth: 0.35,
        showSymbol: points.length <= 2,
        symbolSize: 5,
        lineStyle: { width: 1.5 },
        data: points.map((p) => p.cacheCreateTokens),
      },
      {
        name: lt('缓存读取', 'Cache Read'),
        type: 'line',
        smooth: 0.35,
        showSymbol: points.length <= 2,
        symbolSize: 5,
        lineStyle: { width: 1.5 },
        data: points.map((p) => p.cacheReadTokens),
      },
      {
        name: lt('命中率', 'Hit Rate'),
        type: 'line',
        yAxisIndex: 1,
        smooth: 0.35,
        showSymbol: points.length <= 2,
        symbolSize: 5,
        lineStyle: { width: 1.5, type: [4, 4] },
        data: points.map((p) => Math.round(p.cacheHitRate * 1000) / 10),
      },
    ],
  };
}

/** 被谁调用：StatsPanel。做什么：工具排行 option。 */
function buildToolOption(
  tools: Array<ToolAnalyticsRow & { display: string; color: string }>,
  locale: string,
  lt: LocaleText,
  theme: ChartTheme,
): EChartsOption {
  const rows = [...tools].reverse();
  return {
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow', shadowStyle: { color: 'rgba(148,163,184,0.08)' } },
      backgroundColor: theme.tooltipBg,
      borderColor: theme.tooltipBorder,
      textStyle: { color: theme.text, fontSize: 12 },
      formatter: (params: unknown) => {
        const list = params as Array<{ name: string; value: number; dataIndex: number }>;
        const p = list[0];
        if (!p) return '';
        const row = rows[p.dataIndex];
        const fail = ((row?.failureRate || 0) * 100).toFixed(1);
        const raw = row?.key || row?.label || '';
        return [
          `<b>${p.name}</b>`,
          raw && raw !== p.name ? `<span style="opacity:.65">${raw}</span>` : '',
          `${lt('调用', 'Calls')}: ${p.value.toLocaleString(locale)}`,
          `${lt('失败率', 'Fail')}: ${fail}%`,
        ]
          .filter(Boolean)
          .join('<br/>');
      },
    },
    grid: {
      left: 4,
      right: 28,
      top: 4,
      bottom: 4,
      containLabel: true,
    },
    xAxis: {
      type: 'value',
      axisLabel: { color: theme.faint, fontSize: 10 },
      splitLine: { lineStyle: { color: theme.split, type: 'dashed' } },
      axisLine: { show: false },
      axisTick: { show: false },
    },
    yAxis: {
      type: 'category',
      data: rows.map((t) => t.display),
      axisLabel: {
        color: theme.muted,
        fontSize: 12,
        width: 92,
        overflow: 'truncate',
      },
      axisLine: { show: false },
      axisTick: { show: false },
    },
    series: [
      {
        type: 'bar',
        data: rows.map((t) => ({
          value: t.toolCallCount,
          itemStyle: {
            color: t.color,
            borderRadius: [0, 4, 4, 0],
          },
        })),
        barMaxWidth: 12,
        label: {
          show: true,
          position: 'right',
          color: theme.faint,
          fontSize: 10,
          fontWeight: 500,
        },
      },
    ],
  };
}

/** 被谁调用：图表标签。做什么：缩短模型名。 */
function shortModel(label: string): string {
  if (!label) return '—';
  const parts = label.split('/');
  return parts[parts.length - 1] || label;
}

/** 被谁调用：图表标签。做什么：缩短会话 id。 */
function shortSession(label: string): string {
  if (!label) return '—';
  if (label.length <= 18) return label;
  return `${label.slice(0, 8)}…${label.slice(-6)}`;
}

/** 被谁调用：GoalBar/Stats。做什么：token 数本地化格式。 */
function formatTokens(value: number, locale: string): string {
  if (!Number.isFinite(value)) return '0';
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 10_000) return `${(value / 1000).toFixed(1)}K`;
  return Math.round(value).toLocaleString(locale);
}

/** 被谁调用：状态/统计 UI。做什么：毫秒 → 可读时长。 */
function formatDuration(value?: number): string {
  if (value === undefined || !Number.isFinite(value) || value <= 0) return '—';
  if (value < 1000) return `${Math.round(value)}ms`;
  if (value < 60_000) return `${(value / 1000).toFixed(2)}s`;
  return `${Math.floor(value / 60_000)}m ${Math.round((value % 60_000) / 1000)}s`;
}
