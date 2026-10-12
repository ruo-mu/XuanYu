/**
 * chat-timeline.tsx - 聊天时间线组件集
 *
 * 用途：单列时间线 — 正文直出、工具/命令/编辑可折叠单行展示。
 * 提供 ThinkingTimelineRow、AssistantSayRow、各类工具行、DiffViewer、StreamingTimeline 等组件。
 */

import React, { useMemo, useRef, useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import {
  Alert,
  Button,
  Input,
  Modal,
  Space,
  Tag,
  message,
} from 'antd';
import {
  DownOutlined,
  CopyOutlined,
  CheckOutlined,
  EditOutlined,
  LoadingOutlined,
  LockOutlined,
  PaperClipOutlined,
  PartitionOutlined,
  RightOutlined,
  SafetyCertificateOutlined,
} from '@ant-design/icons';
import { parsePatch, structuredPatch } from 'diff';
import type { StructuredPatch } from 'diff';

import type {
  CheckpointSegment,
  Segment,
  SessionState,
  TextSegment,
  ToolCallSegment,
} from '../../stores/chatStore';
import type {
  RunPlan,
  SubAgentTaskLedger,
  SubAgentTimelineEvent,
} from '../../protocol';
import {
  shouldHideToolResult,
  summariseArgs,
} from '../../stores/chatStore';
import { useChatStore } from '../../stores/chatStore';
import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import { CodexTurnTimeline } from './codex/CodexTimeline';
import {
  assistantMarkdownComponents,
  normalizeAssistantMarkdown,
  remarkPlugins,
} from './codex/codexMarkdown';
import { FileAttachmentCard } from './FileAttachmentCard';
import { formatSubAgentStatus, subAgentStatusColor } from '../../utils/subagentStatus';

// ---------------------------------------------------------------------------
// Design tokens (GitHub-like)
// ---------------------------------------------------------------------------

export const AGENT_DISPLAY_NAME = 'XuanYu';

const C = {
  pageBg: 'var(--xy-bg)',
  border: 'var(--xy-border)',
  codeBg: 'var(--xy-bg-soft)',
  diffDelBg: 'color-mix(in srgb, var(--xy-danger) 12%, var(--xy-bg-elevated))',
  diffDelText: 'var(--xy-danger)',
  diffAddBg: 'color-mix(in srgb, var(--xy-success) 14%, var(--xy-bg-elevated))',
  diffAddText: 'var(--xy-success)',
  linkBlue: 'var(--xy-primary)',
  text: 'var(--xy-text)',
  muted: 'var(--xy-text-muted)',
  icon: 'var(--xy-text-soft)',
};

// ---------------------------------------------------------------------------
// Layout: no icon gutter, no “Agent says” headers
// ---------------------------------------------------------------------------

function TimelineShell({
  icon: _icon,
  children,
}: {
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return <>{children}</>;
}

/** 折叠头：open 时 DownOutlined，否则 RightOutlined。 */
function CodexChevron({ open }: { open: boolean }) {
  return open
    ? <DownOutlined className="xy-codex-chevron" />
    : <RightOutlined className="xy-codex-chevron" />;
}

/** 工具/思考进行中：给文案加 shimmer；active 时加 is-active。 */
function CodexShimmerText({ active, children }: { active?: boolean; children: React.ReactNode }) {
  return (
    <span className={`xy-codex-shimmer${active ? ' is-active' : ''}`}>
      {children}
    </span>
  );
}

/**
 * 被谁调用：各活动行头部。
 * 做什么：按 status 计运行秒数；calling 时每秒刷新，结束后定格。
 */
function useToolDuration(status: ToolCallSegment['status']): string {
  const startRef = useRef(Date.now());
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    startRef.current = Date.now();
    if (status !== 'calling') {
      setSeconds(Math.max(1, Math.round((Date.now() - startRef.current) / 1000)));
      return;
    }
    const tick = () => setSeconds(Math.max(1, Math.round((Date.now() - startRef.current) / 1000)));
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [status]);

  return `${seconds}s`;
}

/**
 * 被谁调用：exec/通用活动行标题。
 * 做什么：按工具名拼一行命令摘要（exec 取 command、search 拟 rg、list_files 拟 Get-ChildItem）。
 */
function activityCommandLabel(tc: ToolCallSegment): string {
  if (tc.name === 'exec') {
    return String(tc.args.command ?? tc.args.cmd ?? summariseArgs(tc) ?? '').trim();
  }
  if (tc.name === 'search') {
    const kw = extractSearchKeyword(tc);
    const path = String(tc.args.path ?? '.');
    if (kw) return `rg -n "${kw}" -S ${path}`;
    return summariseArgs(tc) || tc.name;
  }
  if (tc.name === 'list_files') {
    const path = String(tc.args.path ?? tc.args.directory ?? '.');
    return `Get-ChildItem ${path}${tc.args.recursive ? ' -Recurse' : ''}`.trim();
  }
  const summary = summariseArgs(tc);
  return summary || tc.name;
}

/**
 * 被谁调用：助手文本段旁。
 * 做什么：可折叠思考区；流式时强制展开，结束后按 defaultOpen；正文 Markdown 渲染 reasoning。
 */
function CodexThinkingSection({
  streaming,
  reasoning,
  defaultOpen = false,
  children,
}: {
  streaming?: boolean;
  reasoning?: string;
  defaultOpen?: boolean;
  children?: React.ReactNode;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(streaming ? true : defaultOpen);
  const hasReasoning = !!reasoning?.trim();
  const hasChildren = React.Children.count(children) > 0;
  const hasBody = hasReasoning || hasChildren;

  useEffect(() => {
    // 流式展开，结束后默认折叠
    setOpen(!!streaming);
  }, [streaming]);

  return (
    <div className={`xy-codex-thinking${streaming ? ' is-streaming' : ''}`}>
      <button
        type="button"
        className="xy-codex-thinking-head"
        onClick={() => hasBody && setOpen(!open)}
        disabled={!hasBody}
      >
        {hasBody ? <CodexChevron open={open} /> : <span className="xy-codex-chevron-spacer" />}
        <CodexShimmerText active={streaming}>
          {streaming ? t('timeline.thinkingActive') : t('timeline.thinkingProcess')}
        </CodexShimmerText>
      </button>
      {open && hasBody ? (
        <div className="xy-codex-thinking-body">
          {hasReasoning ? (
            <div className="xy-codex-reasoning xy-markdown-body">
              <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
                {normalizeAssistantMarkdown(reasoning!)}
              </ReactMarkdown>
            </div>
          ) : null}
          {children}
        </div>
      ) : null}
    </div>
  );
}

/** 工具输出展示：超过 limit（默认 8000）截断并加省略号。 */
function truncateToolResult(text: string, limit = 8000): string {
  const trimmed = text.trim();
  if (trimmed.length <= limit) return trimmed;
  return `${trimmed.slice(0, limit)}\n…`;
}

/**
 * 被谁调用：CodexToolDisclosure 标题。
 * 做什么：按工具名生成 i18n title + 路径/命令/查询等 subtitle。
 */
function describeToolCall(
  tc: ToolCallSegment,
  t: (key: string, ...args: unknown[]) => string,
): { title: string; subtitle?: string } {
  const args = tc.args;
  const path = String(args.path ?? args.file_path ?? '');

  if (tc.name === 'file_edit' || tc.name === 'file_write') {
    return { title: t('timeline.codexEdit'), subtitle: path || t('timeline.noPath') };
  }
  if (tc.name === 'file_read') {
    const start = args.start_line;
    const end = args.end_line;
    let range = '';
    if (typeof start === 'number' && typeof end === 'number') range = ` · ${start}-${end}`;
    else if (typeof start === 'number') range = ` · ${start}+`;
    else if (typeof end === 'number') range = ` · ≤${end}`;
    return { title: t('timeline.codexRead'), subtitle: `${path || t('timeline.noPath')}${range}` };
  }
  if (tc.name === 'search') {
    return { title: tc.name, subtitle: extractSearchKeyword(tc) || summariseArgs(tc) };
  }
  if (tc.name === 'exec') {
    return {
      title: t('timeline.codexShell'),
      subtitle: String(args.command ?? args.cmd ?? summariseArgs(tc) ?? '').slice(0, 160),
    };
  }
  if (tc.name === 'list_files') {
    return { title: tc.name, subtitle: String(args.path ?? args.directory ?? '.') };
  }
  if (tc.name === 'glob') {
    return { title: tc.name, subtitle: String(args.pattern ?? args.glob_pattern ?? '') };
  }
  if (tc.name === 'web_search') {
    return { title: tc.name, subtitle: String(args.query ?? args.q ?? '') };
  }
  return { title: tc.name, subtitle: summariseArgs(tc) };
}

/**
 * 被谁调用：通用工具详情（非活动行样式）。
 * 做什么：可展开工具头；展示参数/结果，running/error 加样式。
 */
function CodexToolDisclosure({
  tc,
  defaultOpen = false,
  children,
}: {
  tc: ToolCallSegment;
  defaultOpen?: boolean;
  children?: React.ReactNode;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(defaultOpen);
  const running = tc.status === 'calling';
  const error = tc.status === 'error';
  const meta = describeToolCall(tc, t as (key: string, ...args: unknown[]) => string);
  const hasExpandable = !!(children || (tc.result?.trim() && !shouldHideToolResult(tc.name)));

  return (
    <div className={`xy-codex-tool${running ? ' is-running' : ''}${error ? ' is-error' : ''}`}>
      <button
        type="button"
        className="xy-codex-tool-head"
        onClick={() => hasExpandable && setOpen(!open)}
        disabled={!hasExpandable}
      >
        {running ? (
          <LoadingOutlined spin className="xy-codex-chevron" />
        ) : hasExpandable ? (
          <CodexChevron open={open} />
        ) : (
          <span className="xy-codex-tool-dot" />
        )}
        <span className="xy-codex-tool-name">{meta.title}</span>
        {meta.subtitle ? <span className="xy-codex-tool-args">{meta.subtitle}</span> : null}
      </button>
      {open && hasExpandable ? (
        <div className="xy-codex-tool-body">
          {children ?? (
            <pre className="xy-codex-tool-pre">{truncateToolResult(tc.result || '')}</pre>
          )}
        </div>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Thinking
// ---------------------------------------------------------------------------

export function ThinkingTimelineRow({
  text,
  streaming,
  title: _title,
  showContent = true,
}: {
  text: string;
  streaming?: boolean;
  title?: string;
  showContent?: boolean;
}) {
  const safe = (text || '').trim();

  if (!showContent || !safe) {
    return <CodexThinkingSection streaming={streaming} defaultOpen={false} />;
  }

  return (
    <CodexThinkingSection streaming={streaming} reasoning={safe} defaultOpen={false} />
  );
}

// ---------------------------------------------------------------------------
// Activity rows (commands, edits, reads)
// ---------------------------------------------------------------------------

function CodexShellCard({
  tc,
  cmd,
  open,
}: {
  tc: ToolCallSegment;
  cmd: string;
  open: boolean;
}) {
  const { t } = useI18n();
  if (!open) return null;
  const output = tc.result?.trim() || '';
  const failed = tc.status === 'error' || output.toLowerCase().includes('error');
  const running = tc.status === 'calling';
  const body = output
    || (running ? '…' : t('timeline.noCommandOutput', '（无输出）'));

  return (
    <div className="xy-codex-shell-card">
      <div className="xy-codex-shell-card-meta">
        <span>{t('timeline.shell')}</span>
        {running ? (
          <span className="xy-codex-shell-running">{t('timeline.running')}</span>
        ) : failed ? (
          <span className="xy-codex-shell-fail">✗</span>
        ) : (
          <span className="xy-codex-shell-ok">✓ {t('timeline.success')}</span>
        )}
      </div>
      <pre className="xy-codex-shell-cmd">{cmd.startsWith('$') ? cmd : `$ ${cmd}`}</pre>
      <pre className={`xy-codex-shell-out${failed ? ' is-error' : ''}${!output && !running ? ' is-empty' : ''}`}>
        {running && !output ? '…' : truncateToolResult(body)}
      </pre>
    </div>
  );
}

/**
 * 被谁调用：CodexActivityRow / CodexCommandBatch。
 * 做什么：exec/search 活动行：折叠头显示「跑了命令 + 时长」，展开 CodexShellCard。
 */
function CodexExecActivity({ tc, nested }: { tc: ToolCallSegment; nested?: boolean }) {
  const { t } = useI18n();
  const cmd = activityCommandLabel(tc);
  const running = tc.status === 'calling';
  const duration = useToolDuration(tc.status);
  const [open, setOpen] = useState(running);

  return (
    <div className={`xy-codex-activity${nested ? ' is-nested' : ''}`}>
      <button
        type="button"
        className="xy-codex-activity-head"
        onClick={() => setOpen(!open)}
      >
        <CodexChevron open={open} />
        <span className="xy-codex-activity-label">
          {t('timeline.ranCommand', undefined, cmd || '—', duration)}
        </span>
      </button>
      <CodexShellCard tc={tc} cmd={cmd} open={open} />
    </div>
  );
}

/**
 * 被谁调用：CodexActivityRow（edit）。
 * 做什么：file_edit/write 行：文件名、+/- 统计，展开 DiffViewer 或原始 result。
 */
function CodexEditActivity({ tc }: { tc: ToolCallSegment }) {
  const { t } = useI18n();
  const path = String(tc.args.path ?? tc.args.file_path ?? '');
  const fileName = path.split(/[/\\]/).pop() || path || t('timeline.noPath');
  const patch = buildStructuredPatchFromTool(tc);
  const stats = editLineStatsFromTool(tc, patch);
  const showDiff = !!patch && patch.hunks.length > 0;
  const running = tc.status === 'calling';
  const duration = useToolDuration(tc.status);
  const [open, setOpen] = useState(running || showDiff);
  const hasBody = showDiff || !!(tc.result?.trim()) || running;

  return (
    <div className="xy-codex-activity xy-codex-activity-edit">
      <button type="button" className="xy-codex-activity-head" onClick={() => hasBody && setOpen(!open)} disabled={!hasBody}>
        <PaperClipOutlined className="xy-codex-activity-icon" />
        {hasBody ? <CodexChevron open={open} /> : <span className="xy-codex-chevron-spacer" />}
        <span className="xy-codex-activity-label">
          {path ? t('timeline.editedFileNamed', undefined, fileName) : t('timeline.editedFile')}
        </span>
        {(stats.add > 0 || stats.del > 0) && (
          <span className="xy-codex-diff-stats">
            <span className="is-add">+{stats.add}</span>
            <span className="is-del">−{stats.del}</span>
          </span>
        )}
        {!running && stats.add === 0 && stats.del === 0 ? (
          <span className="xy-codex-activity-duration">{duration}</span>
        ) : null}
      </button>
      {open && hasBody ? (
        <div className="xy-codex-edit-card">
          <div className="xy-codex-edit-card-head">
            <span className="xy-codex-edit-filename">{fileName}</span>
            {(stats.add > 0 || stats.del > 0) && (
              <span className="xy-codex-diff-stats">
                <span className="is-add">+{stats.add}</span>
                <span className="is-del">−{stats.del}</span>
              </span>
            )}
          </div>
          {showDiff ? (
            <DiffViewer patch={patch!} sourcePath={path} />
          ) : (
            <pre className="xy-codex-shell-out">
              {tc.result?.trim() || (running ? t('timeline.applyingEdit') : '—')}
            </pre>
          )}
        </div>
      ) : null}
    </div>
  );
}

/**
 * 被谁调用：CodexActivityRow（read）。
 * 做什么：file_read 行：路径+时长；有结果时可展开截断正文。
 */
function CodexReadActivity({ tc }: { tc: ToolCallSegment }) {
  const { t } = useI18n();
  const path = String(tc.args.path ?? tc.args.file_path ?? t('timeline.noPath'));
  const running = tc.status === 'calling';
  const duration = useToolDuration(tc.status);
  const hasOutput = !!(tc.result?.trim() && !shouldHideToolResult(tc.name));
  const [open, setOpen] = useState(false);

  return (
    <div className="xy-codex-activity">
      <button type="button" className="xy-codex-activity-head" onClick={() => hasOutput && setOpen(!open)} disabled={!hasOutput}>
        {hasOutput ? <CodexChevron open={open} /> : <span className="xy-codex-chevron-spacer" />}
        <span className="xy-codex-activity-label">
          {t('timeline.ranRead', undefined, path, duration)}
        </span>
      </button>
      {open && hasOutput ? (
        <pre className="xy-codex-shell-out">{truncateToolResult(tc.result || '')}</pre>
      ) : null}
    </div>
  );
}

/**
 * 被谁调用：CodexActivityRow（其它工具）。
 * 做什么：通用活动行；标签用 activityCommandLabel，展开截断 result。
 */
function CodexGenericActivity({ tc }: { tc: ToolCallSegment }) {
  const { t } = useI18n();
  const label = activityCommandLabel(tc) || tc.name;
  const running = tc.status === 'calling';
  const duration = useToolDuration(tc.status);
  const hideResult = shouldHideToolResult(tc.name);
  const output = tc.result?.trim() || '';
  const body = hideResult
    ? ''
    : (output || (running ? '…' : t('timeline.noCommandOutput', '（无输出）')));
  const [open, setOpen] = useState(running);

  return (
    <div className="xy-codex-activity">
      <button
        type="button"
        className="xy-codex-activity-head"
        onClick={() => !hideResult && setOpen(!open)}
        disabled={hideResult}
      >
        {!hideResult ? <CodexChevron open={open} /> : <span className="xy-codex-chevron-spacer" />}
        <span className="xy-codex-activity-label">
          {t('timeline.ranCommand', undefined, label, duration)}
        </span>
      </button>
      {open && !hideResult ? (
        <pre className={`xy-codex-shell-out${!output && !running ? ' is-empty' : ''}`}>
          {truncateToolResult(body)}
        </pre>
      ) : null}
    </div>
  );
}

/**
 * 被谁调用：groupActivitySegments 产出的 command_batch。
 * 做什么：折叠显示「跑了 N 条命令」，展开后逐条嵌套 CodexExecActivity。
 */
function CodexCommandBatch({ items }: { items: ToolCallSegment[] }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);

  return (
    <div className="xy-codex-activity xy-codex-command-batch">
      <button type="button" className="xy-codex-activity-head xy-codex-batch-head" onClick={() => setOpen(!open)}>
        <span className="xy-codex-terminal-icon" aria-hidden />
        <CodexChevron open={open} />
        <span className="xy-codex-activity-label">
          {t('timeline.ranCommands', undefined, items.length)}
        </span>
      </button>
      {open ? (
        <div className="xy-codex-batch-body">
          {items.map((item) => (
            <CodexExecActivity key={item.id} tc={item} nested />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/**
 * 被谁调用：ToolCallTimelineRow / 时间线分发。
 * 做什么：按 toolKind 路由到 edit/read/exec/search/generic 活动组件。
 */
function CodexActivityRow({ tc }: { tc: ToolCallSegment }) {
  const k = toolKind(tc.name);
  if (k === 'edit') return <CodexEditActivity tc={tc} />;
  if (k === 'read') return <CodexReadActivity tc={tc} />;
  if (k === 'exec' || k === 'search') return <CodexExecActivity tc={tc} />;
  return <CodexGenericActivity tc={tc} />;
}

/** exec/search/list_files 可合成命令批。 */
function isBatchableTool(name: string): boolean {
  return name === 'exec' || name === 'search' || name === 'list_files';
}

type GroupedSegment = Segment | { type: 'command_batch'; id: string; items: ToolCallSegment[] };

/**
 * 被谁调用：CodexTurnTimeline 渲染前。
 * 做什么：连续≥2 条已完成的 exec/search/list_files 收成 command_batch；进行中的不合并。
 */
function groupActivitySegments(segments: Segment[]): GroupedSegment[] {
  const out: GroupedSegment[] = [];
  let batch: ToolCallSegment[] = [];

  const flush = () => {
    if (batch.length >= 2) {
      out.push({ type: 'command_batch', id: `batch_${batch[0].id}`, items: [...batch] });
    } else {
      batch.forEach((item) => out.push(item));
    }
    batch = [];
  };

  for (const seg of segments) {
    if (seg.type === 'tool_call' && isBatchableTool(seg.name) && seg.status !== 'calling') {
      batch.push(seg);
    } else {
      flush();
      out.push(seg);
    }
  }
  flush();
  return out;
}

// ---------------------------------------------------------------------------
// Assistant “说”
// ---------------------------------------------------------------------------

export function AssistantSayRow({ content }: { content: string }) {
  if (!content.trim()) return null;
  return (
    <TimelineShell>
      <div className="xy-codex-prose xy-markdown-body">
        <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
          {normalizeAssistantMarkdown(content)}
        </ReactMarkdown>
      </div>
    </TimelineShell>
  );
}

/** 子 Agent 文本段：带 agentName 标签的 Markdown 气泡。 */
function SubAgentSayRow({ content, agentName = '子 Agent' }: { content: string; agentName?: string }) {
  if (!content.trim()) return null;
  return (
    <TimelineShell>
      <div className="xy-codex-prose xy-markdown-body">
        <div className="xy-codex-subagent-label">{agentName}</div>
        <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
          {normalizeAssistantMarkdown(content)}
        </ReactMarkdown>
      </div>
    </TimelineShell>
  );
}

/** 信息类事件通用壳：标题 + 状态文案 + 可选 children（沙箱/权限等复用）。 */
function InfoEventRow({
  icon: _icon,
  title,
  status,
  children,
}: {
  icon?: React.ReactNode;
  title: string;
  status?: string;
  children?: React.ReactNode;
}) {
  const [open, setOpen] = useState(!!children);
  return (
    <TimelineShell>
      <div className="xy-codex-tool">
        <button
          type="button"
          className="xy-codex-tool-head"
          onClick={() => children && setOpen(!open)}
          disabled={!children}
        >
          {children ? <CodexChevron open={open} /> : <span className="xy-codex-tool-dot" />}
          <span className="xy-codex-tool-name">{title}</span>
          {status ? <span className="xy-codex-tool-status">{status}</span> : null}
        </button>
        {open && children ? <div className="xy-codex-tool-detail">{children}</div> : null}
      </div>
    </TimelineShell>
  );
}

/**
 * 被谁调用：SegmentTimelineItem（sandbox_event）。
 * 做什么：沙箱事件 Info 行；展示 content、args 摘要与截断 result。
 */
function SandboxTimelineRow({ seg }: { seg: Extract<Segment, { type: 'sandbox_event' }> }) {
  const { t, locale } = useI18n();
  const summary = summariseGenericArgs(seg.args || {});
  return (
    <InfoEventRow icon={<SafetyCertificateOutlined />} title={seg.title || seg.name || t('timeline.sandbox')} status={formatSubAgentStatus(seg.status, locale)}>
      <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>
        {seg.content && <div>{seg.content}</div>}
        {summary && <div>{t('timeline.target')}{summary}</div>}
        {seg.result && (
          <div style={{ marginTop: 6, whiteSpace: 'pre-wrap', maxHeight: 160, overflow: 'auto' }}>
            {seg.result.length > 800 ? `${seg.result.slice(0, 800)}...` : seg.result}
          </div>
        )}
      </div>
    </InfoEventRow>
  );
}

/**
 * 被谁调用：沙箱/权限行摘要。
 * 做什么：优先取 task/prompt/path/command 等字符串字段，超 100 字截断。
 */
function summariseGenericArgs(args: Record<string, unknown>): string {
  const preferred = ['task', 'prompt', 'goal', 'sandboxId', 'name', 'path', 'command'];
  for (const key of preferred) {
    const value = args[key];
    if (typeof value === 'string' && value.trim()) {
      return value.length > 100 ? `${value.slice(0, 97)}...` : value;
    }
  }
  const first = Object.values(args).find((value) => typeof value === 'string' && value.trim());
  if (typeof first === 'string') return first.length > 100 ? `${first.slice(0, 97)}...` : first;
  return '';
}

/** 单行展示：空白压成空格，超 max 加省略号。 */
function trimText(value: string, max: number): string {
  const clean = value.replace(/\s+/g, ' ').trim();
  return clean.length > max ? `${clean.slice(0, max - 3)}...` : clean;
}

/** JSON 解析为普通对象；失败或非 object 返回 {}。 */
function safeParseRecord(raw: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(raw || '{}');
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed) ? parsed as Record<string, unknown> : {};
  } catch {
    return {};
  }
}

/**
 * 被谁调用：SegmentTimelineItem（permission）。
 * 做什么：权限检查点行；展示工具名、允许/拒绝/等待、reason 与 args 摘要。
 */
function PermissionTimelineRow({ seg }: { seg: CheckpointSegment }) {
  const { t } = useI18n();
  const payload = seg.payload || {};
  const toolName = typeof payload.toolName === 'string' && payload.toolName.trim()
    ? payload.toolName
    : t('timeline.toolCall');
  const reason = typeof payload.reason === 'string' ? payload.reason : '';
  const allowed = typeof payload.allowed === 'boolean' ? payload.allowed : undefined;
  const statusText = allowed === true
    ? t('timeline.allowed')
    : allowed === false
      ? t('timeline.denied')
      : (seg.status === 'pending' ? t('timeline.waitingConfirm') : seg.status);
  const args = payload.args && typeof payload.args === 'object' && !Array.isArray(payload.args)
    ? payload.args as Record<string, unknown>
    : {};
  const summary = summariseGenericArgs(args);
  return (
    <InfoEventRow icon={<LockOutlined />} title={seg.title || `${t('timeline.permissionRequest')}${toolName}`} status={statusText}>
      {(reason || summary) && (
        <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>
          {reason && <div>{t('timeline.reason')}{reason}</div>}
          {summary && <div>{t('timeline.target')}{summary}</div>}
        </div>
      )}
    </InfoEventRow>
  );
}

// ---------------------------------------------------------------------------
// User
// ---------------------------------------------------------------------------

export function UserTimelineRow({
  content,
  turnId,
  timestamp,
  canInlineEdit,
  isEditing,
  isRewinding,
  editDraft,
  onStartEdit,
  onCancelEdit,
  onEditDraftChange,
  onSubmitEdit,
  attachments,
}: {
  content: string;
  turnId?: string;
  timestamp?: number;
  onRewind?: () => void;
  canInlineEdit?: boolean;
  isEditing?: boolean;
  isRewinding?: boolean;
  editDraft?: string;
  onStartEdit?: () => void;
  onCancelEdit?: () => void;
  onEditDraftChange?: (value: string) => void;
  onSubmitEdit?: () => void;
  attachments?: Array<{ kind: 'image' | 'file'; name: string; mimeType: string; sizeBytes: number; dataUrl: string }>;
}) {
  const { t, locale } = useI18n();
  const [copied, setCopied] = useState(false);
  const imageAttachments = attachments?.filter((a) => a.kind === 'image') || [];
  const fileAttachments = attachments?.filter((a) => a.kind === 'file') || [];
  const MAX_VISIBLE = 3;
  const visibleImages = imageAttachments.slice(0, MAX_VISIBLE);
  const hiddenCount = imageAttachments.length - MAX_VISIBLE;

  const handlePreview = (src: string) => {
    window.dispatchEvent(new CustomEvent('xy-image-preview', { detail: { src } }));
  };

  const hasAttachments = fileAttachments.length > 0 || visibleImages.length > 0 || hiddenCount > 0;
  const showEdit = Boolean(canInlineEdit && turnId && !hasAttachments);

  const handleCopy = async () => {
    const text = content?.trim();
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      message.success(t('chat.copied', '已复制'));
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      message.error(t('chat.copyFailed', '复制失败'));
    }
  };

  return (
    <div className={`xy-user-bubble${isEditing ? ' is-editing' : ''}`}>
      {hasAttachments && (
        <div className="xy-user-attachments">
          {fileAttachments.map((att, i) => (
            <FileAttachmentCard
              key={`file-${i}`}
              name={att.name}
              mimeType={att.mimeType}
              sizeBytes={att.sizeBytes}
              className="xy-file-attachment-card-message"
            />
          ))}
          {visibleImages.map((att, i) => (
            <div key={`img-${i}`} className="xy-user-image-wrap" onClick={() => handlePreview(att.dataUrl)}>
              <img src={att.dataUrl} alt="" className="xy-user-image" />
            </div>
          ))}
          {hiddenCount > 0 && (
            <div className="xy-user-images-more">+{hiddenCount}</div>
          )}
        </div>
      )}
      {isEditing ? (
        <div className="xy-user-bubble-edit">
          <Input.TextArea
            value={editDraft}
            onChange={(e) => onEditDraftChange?.(e.target.value)}
            autoSize={{ minRows: 3, maxRows: 16 }}
            className="xy-user-bubble-edit-input"
            autoFocus
            disabled={isRewinding}
            onKeyDown={(e) => {
              if (isRewinding) return;
              if (e.key === 'Escape') {
                e.preventDefault();
                onCancelEdit?.();
              }
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                onSubmitEdit?.();
              }
            }}
          />
          <div className="xy-user-bubble-edit-actions">
            <button
              type="button"
              className="xy-user-bubble-edit-cancel"
              disabled={isRewinding}
              onClick={onCancelEdit}
            >
              {t('common.cancel', '取消')}
            </button>
            <button
              type="button"
              className="xy-user-bubble-edit-send"
              disabled={!editDraft?.trim() || isRewinding}
              onClick={onSubmitEdit}
            >
              {isRewinding ? t('chat.rewinding', '正在回退…') : t('chat.send', '发送')}
            </button>
          </div>
        </div>
      ) : (
        <div className="xy-user-bubble-shell">
          {content ? <div className="xy-user-bubble-inner">{content}</div> : null}
          <div className="xy-user-bubble-meta">
            {timestamp ? (
              <span className="xy-user-bubble-time">{formatUserMessageTime(timestamp, locale)}</span>
            ) : null}
            <button
              type="button"
              className={`xy-user-bubble-icon-btn${copied ? ' is-copied' : ''}`}
              aria-label={copied ? t('chat.copied', '已复制') : t('chat.copyMessage', '复制')}
              title={copied ? t('chat.copied', '已复制') : t('chat.copyMessage', '复制')}
              onClick={handleCopy}
            >
              {copied ? <CheckOutlined /> : <CopyOutlined />}
            </button>
            {showEdit ? (
              <button
                type="button"
                className="xy-user-bubble-icon-btn"
                aria-label={t('chat.editMessage', '编辑')}
                title={t('chat.editMessageHint', '编辑并回退到此消息')}
                onClick={onStartEdit}
              >
                <EditOutlined />
              </button>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}

/** 用户消息时间：中文「M月D日 HH:mm」，其它 locale short month。 */
function formatUserMessageTime(timestamp: number, locale: string): string {
  const d = new Date(timestamp);
  if (!Number.isFinite(d.getTime())) return '';
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  if (locale === 'zh-CN' || locale.startsWith('zh')) {
    return `${d.getMonth() + 1}月${d.getDate()}日 ${hh}:${mm}`;
  }
  return d.toLocaleString(locale || undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

type DiffLineRow =
  | { kind: 'ctx'; oldNum: number; newNum: number; text: string }
  | { kind: 'del'; oldNum: number; text: string }
  | { kind: 'add'; newNum: number; text: string };

type DiffRow =
  | { kind: 'hunk'; text: string }
  | DiffLineRow
  | { kind: 'fold'; hidden: DiffLineRow[] };

const FOLD_THRESHOLD = 6;
const FOLD_KEEP = 3;

/**
 * 被谁调用：DiffViewer。
 * 做什么：StructuredPatch → 行列表；长上下文折叠成 fold（首尾各留 FOLD_KEEP）。
 */
function patchToRows(patch: StructuredPatch): DiffRow[] {
  const out: DiffRow[] = [];
  for (const hunk of patch.hunks) {
    out.push({ kind: 'hunk', text: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@` });
    let oldNum = hunk.oldStart;
    let newNum = hunk.newStart;
    let ctxBuf: DiffLineRow[] = [];

    const flushCtx = () => {
      if (ctxBuf.length === 0) return;
      if (ctxBuf.length <= FOLD_THRESHOLD) {
        out.push(...ctxBuf);
      } else {
        const hidden = ctxBuf.slice(FOLD_KEEP, -FOLD_KEEP);
        out.push(...ctxBuf.slice(0, FOLD_KEEP));
        if (hidden.length > 0) out.push({ kind: 'fold', hidden });
        out.push(...ctxBuf.slice(-FOLD_KEEP));
      }
      ctxBuf = [];
    };

    for (const line of hunk.lines) {
      const op = line[0];
      const content = line.endsWith('\n') ? line.slice(1, -1) : line.slice(1);
      if (op === ' ') {
        ctxBuf.push({ kind: 'ctx', oldNum, newNum, text: content });
        oldNum++;
        newNum++;
      } else {
        flushCtx();
        if (op === '-') {
          out.push({ kind: 'del', oldNum, text: content });
          oldNum++;
        } else if (op === '+') {
          out.push({ kind: 'add', newNum, text: content });
          newNum++;
        }
      }
    }
    flushCtx();
  }
  return out;
}

/** 路径末段扩展名小写，供 diff 轻量高亮选语言。 */
function extFromPath(p: string): string {
  const m = p.match(/\.([^.\\/]+)$/);
  return (m?.[1] || '').toLowerCase();
}

const KW =
  /\b(?:const|let|var|function|return|async|await|import|export|from|default|interface|type|class|extends|implements|enum|namespace|public|private|protected|readonly|static|new|void|null|undefined|true|false|if|else|switch|case|break|continue|for|while|do|try|catch|finally|throw|yield|super)\b/g;

/** highlightDiffLine：给 JS/TS 关键字片段上色。 */
function keywordTint(fragment: string): React.ReactNode {
  const nodes: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  const re = new RegExp(KW.source, 'g');
  while ((m = re.exec(fragment))) {
    if (m.index > last) nodes.push(fragment.slice(last, m.index));
    nodes.push(
      <span key={`kw-${m.index}`} style={{ color: C.linkBlue, fontWeight: 500 }}>
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < fragment.length) nodes.push(fragment.slice(last));
  return nodes.length === 0 ? fragment : nodes.length === 1 ? nodes[0] : <>{nodes}</>;
}

/** Light syntax tint for diff rows (comments, strings, keywords). */
function highlightDiffLine(line: string, ext: string): React.ReactNode {
  const codeLike = ['ts', 'tsx', 'js', 'jsx', 'json', 'css', 'py', 'rs', 'go', 'java', 'c', 'cpp', 'h', 'cs'].includes(ext);
  if (!codeLike) return line;

  if ((ext === 'py' || ext === 'sh' || ext === 'yaml' || ext === 'yml') && line.trimStart().startsWith('#')) {
    return <span style={{ color: C.muted }}>{line}</span>;
  }

  let codePart = line;
  let commentPart = '';
  if (ext !== 'json' && !['py', 'sh'].includes(ext)) {
    const ci = line.indexOf('//');
    if (ci >= 0) {
      codePart = line.slice(0, ci);
      commentPart = line.slice(ci);
    }
  }

  const strRe = /('(?:\\.|[^'])*'|"(?:\\.|[^"])*"|`(?:\\.|[^`])*`)/g;
  const nodes: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = strRe.exec(codePart))) {
    if (m.index > last) nodes.push(<span key={`kw-${k}`}>{keywordTint(codePart.slice(last, m.index))}</span>);
    nodes.push(
      <span key={`st-${k}`} style={{ color: ext === 'css' ? C.diffAddText : C.linkBlue }}>
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
    k++;
  }
  if (last < codePart.length) nodes.push(<span key="kw-end">{keywordTint(codePart.slice(last))}</span>);
  const body = nodes.length > 0 ? <>{nodes}</> : keywordTint(codePart);
  if (!commentPart) return body;
  return (
    <>
      {body}
      <span style={{ color: C.muted }}>{commentPart}</span>
    </>
  );
}

/**
 * 被谁调用：DiffViewer 每一行。
 * 做什么：旧/新行号 + +/- 标记 + 高亮正文；按 add/del/ctx 上色。
 */
function DiffLineCells({ row, langExt }: { row: DiffLineRow; langExt: string }) {
  let bg = 'transparent';
  let fg = C.text;
  let borderLeft = '3px solid transparent';
  if (row.kind === 'del') {
    bg = C.diffDelBg;
    fg = C.diffDelText;
    borderLeft = `3px solid ${C.diffDelText}`;
  } else if (row.kind === 'add') {
    bg = C.diffAddBg;
    fg = C.diffAddText;
    borderLeft = `3px solid ${C.diffAddText}`;
  }
  const mark = row.kind === 'del' ? '−' : row.kind === 'add' ? '+' : ' ';
  const oldCol =
    row.kind === 'ctx' ? String(row.oldNum) : row.kind === 'del' ? String(row.oldNum) : '';
  const newCol =
    row.kind === 'ctx' ? String(row.newNum) : row.kind === 'add' ? String(row.newNum) : '';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'stretch',
        borderBottom: `1px solid ${C.border}`,
        background: bg,
        color: fg,
        borderLeft,
      }}
    >
      <div
        style={{
          flexShrink: 0,
          width: 36,
          padding: '2px 4px',
          color: C.muted,
          textAlign: 'right',
          userSelect: 'none',
          borderRight: `1px solid ${C.border}`,
          background: row.kind === 'ctx' ? 'var(--xy-bg-subtle)' : bg,
        }}
      >
        {oldCol}
      </div>
      <div
        style={{
          flexShrink: 0,
          width: 36,
          padding: '2px 4px',
          color: C.muted,
          textAlign: 'right',
          userSelect: 'none',
          borderRight: `1px solid ${C.border}`,
          background: row.kind === 'ctx' ? 'var(--xy-bg-subtle)' : bg,
        }}
      >
        {newCol}
      </div>
      <div style={{ flexShrink: 0, width: 18, padding: '2px 4px', color: fg, userSelect: 'none' }}>{mark}</div>
      <div style={{ flex: 1, padding: '2px 8px', minWidth: 0, overflowX: 'auto', color: row.kind === 'ctx' ? C.text : fg }}>
        {highlightDiffLine(row.text || ' ', langExt)}
      </div>
    </div>
  );
}

/**
 * 被谁调用：CodexEditActivity。
 * 做什么：把 StructuredPatch 渲染成可折叠 diff 表（hunk/行/fold）。
 */
function DiffViewer({ patch, sourcePath }: { patch: StructuredPatch; sourcePath?: string }) {
  const { t } = useI18n();
  const rows = useMemo(() => patchToRows(patch), [patch]);
  const langExt = sourcePath ? extFromPath(sourcePath) : '';
  const [foldOpen, setFoldOpen] = useState<Record<number, boolean>>({});

  return (
    <div className="xy-codex-diff">
      {rows.map((row, i) => {
        if (row.kind === 'hunk') {
          return (
            <div key={`h-${i}`} className="xy-codex-diff-hunk">
              {row.text}
            </div>
          );
        }
        if (row.kind === 'fold') {
          const open = !!foldOpen[i];
          return (
            <div key={`f-${i}`}>
              <div
                className="xy-codex-diff-fold"
                onClick={() => setFoldOpen((s) => ({ ...s, [i]: !open }))}
              >
                {open ? '▼' : '▶'} {t('timeline.hiddenLinesCount', undefined, row.hidden.length)}
              </div>
              {open && row.hidden.map((ln, j) => (
                <DiffLineCells key={`fh-${i}-${j}`} row={ln} langExt={langExt} />
              ))}
            </div>
          );
        }
        return <DiffLineCells key={`r-${i}`} row={row} langExt={langExt} />;
      })}
    </div>
  );
}

/** CodexEditActivity 头：统计 patch 中 +/− 行数。 */
function countPatchStats(patch: StructuredPatch): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const h of patch.hunks) {
    for (const line of h.lines) {
      if (line[0] === '+') add++;
      else if (line[0] === '-') del++;
    }
  }
  return { add, del };
}

/**
 * 被谁调用：CodexEditActivity。
 * 做什么：file_edit 用 old/new_text、file_write 用 content 造 patch；否则尝试 parsePatch(result)。
 */
function buildStructuredPatchFromTool(tc: ToolCallSegment): StructuredPatch | null {
  const name = tc.name;
  const args = tc.args;
  const path = String(args.path ?? args.file_path ?? 'file');

  try {
    if (name === 'file_edit') {
      const oldT = String(args.old_text ?? '');
      const newT = String(args.new_text ?? '');
      if (oldT || newT) {
        return structuredPatch(path, path, oldT, newT, '', '', { context: 4 });
      }
      return patchFromToolResultText(String(tc.result ?? ''), path);
    }
    if (name === 'file_write') {
      const newT = String(args.content ?? '');
      if (tc.status === 'done' || tc.status === 'error') {
        const before =
          typeof tc.streamBefore === 'string'
            ? tc.streamBefore
            : extractBeforeFromNaiveResultText(String(tc.result ?? ''));
        if (before !== null && (newT.length > 0 || before.length > 0)) {
          return structuredPatch(path, path, before, newT, '', '', { context: 4 });
        }
        return patchFromToolResultText(String(tc.result ?? ''), path);
      }
      if (typeof tc.streamBefore === 'string') {
        return structuredPatch(path, path, tc.streamBefore, newT, '', '', { context: 4 });
      }
      return null;
    }
    const r = (tc.result || '').trim();
    if (r && (r.includes('@@') || r.includes('diff --git'))) {
      return patchFromToolResultText(r, path);
    }
  } catch {
    return null;
  }
  return null;
}

function extractBeforeFromNaiveResultText(result: string): string | null {
  if (!result.trim()) return null;
  if (/^---\s+\/dev\/null/m.test(result)) return '';
  const diffStart = result.search(/^diff --git|^--- /m);
  if (diffStart < 0) return null;
  const lines = result.slice(diffStart).replace(/\r\n/g, '\n').split('\n');
  let i = 0;
  while (i < lines.length && !lines[i].startsWith('@@')) i++;
  if (i >= lines.length) return null;
  i++;
  const body: string[] = [];
  while (i < lines.length) {
    const line = lines[i];
    if (
      line.startsWith('@@') ||
      line.startsWith('diff --git') ||
      line.startsWith('--- ') ||
      line.startsWith('+++ ')
    ) {
      break;
    }
    const op = line[0];
    if (op === '+' || op === '-' || op === ' ') {
      body.push(line);
      i++;
      continue;
    }
    if (body.length > 0) break;
    i++;
  }
  if (body.length === 0) return null;
  let phase: 'del' | 'add' = 'del';
  for (const line of body) {
    const op = line[0];
    if (op === ' ') return null;
    if (op === '-' && phase === 'add') return null;
    if (op === '+') phase = 'add';
  }
  const beforeLines = body.filter((l) => l[0] === '-').map((l) => l.slice(1));
  if (beforeLines.length === 0) return '';
  return `${beforeLines.join('\n')}\n`;
}

function patchFromToolResultText(result: string, filePath: string): StructuredPatch | null {
  if (!result?.trim()) return null;
  const diffStart = result.search(/^diff --git|^--- /m);
  const diffText = diffStart >= 0 ? result.slice(diffStart) : result;
  if (!diffText.includes('@@') && !/^[-+]/m.test(diffText)) return null;
  try {
    const patches = parsePatch(diffText);
    const base = filePath.split(/[/\\]/).pop() || filePath;
    const matched =
      patches.find((p) =>
        [p.oldFileName, p.newFileName].some(
          (f) =>
            f === base ||
            f === filePath ||
            (f && (f.endsWith(`/${base}`) || f.endsWith(`\\${base}`))),
        ),
      ) ?? (patches.length === 1 ? patches[0] : null);
    if (matched?.hunks?.length) return matched;
  } catch {
    /* fall through */
  }
  return parseLooseUnifiedDiffText(diffText, filePath);
}

function parseLooseUnifiedDiffText(diffText: string, filePath: string): StructuredPatch | null {
  const lines = diffText.replace(/\r\n/g, '\n').split('\n');
  const rel = filePath.replace(/\\/g, '/') || 'file';
  let oldName = `a/${rel}`;
  let newName = `b/${rel}`;
  const hunks: StructuredPatch['hunks'] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith('--- ')) {
      const name = line.slice(4).trim().split('\t')[0];
      if (name && name !== '/dev/null') oldName = name;
      i++;
      continue;
    }
    if (line.startsWith('+++ ')) {
      const name = line.slice(4).trim().split('\t')[0];
      if (name && name !== '/dev/null') newName = name;
      i++;
      continue;
    }
    if (!line.startsWith('@@')) {
      i++;
      continue;
    }
    const header = line.match(/^@@\s+-(\d+)(?:,(\d+))?\s+\+(\d+)(?:,(\d+))?\s*@@/);
    i++;
    const hunkLines: string[] = [];
    while (i < lines.length) {
      const body = lines[i];
      if (
        body.startsWith('@@') ||
        body.startsWith('diff --git') ||
        body.startsWith('--- ') ||
        body.startsWith('+++ ')
      ) {
        break;
      }
      if (body.startsWith('\\ No newline')) {
        hunkLines.push(body);
        i++;
        continue;
      }
      const op = body[0];
      if (op === '+' || op === '-' || op === ' ') {
        hunkLines.push(body);
        i++;
        continue;
      }
      if (hunkLines.length > 0) break;
      i++;
    }
    if (hunkLines.length === 0) continue;
    let oldCount = 0;
    let newCount = 0;
    for (const hl of hunkLines) {
      if (hl[0] === '-') oldCount++;
      else if (hl[0] === '+') newCount++;
      else if (hl[0] === ' ') {
        oldCount++;
        newCount++;
      }
    }
    hunks.push({
      oldStart: header ? Number(header[1]) || 1 : 1,
      oldLines: header && header[2] != null ? Number(header[2]) : oldCount,
      newStart: header ? Number(header[3]) || 1 : 1,
      newLines: header && header[4] != null ? Number(header[4]) : newCount,
      lines: hunkLines,
    });
  }
  if (hunks.length === 0) return null;
  return {
    oldFileName: oldName,
    newFileName: newName,
    oldHeader: '',
    newHeader: '',
    hunks,
  };
}

function editLineStatsFromTool(
  tc: ToolCallSegment,
  patch: StructuredPatch | null,
): { add: number; del: number } {
  if (patch) {
    const stats = countPatchStats(patch);
    if (stats.add > 0 || stats.del > 0) return stats;
  }
  const m = String(tc.result ?? '').match(/Lines changed:\s*\+(\d+)\s*\/\s*-(\d+)/i);
  if (m) return { add: Number(m[1]) || 0, del: Number(m[2]) || 0 };
  if (typeof tc.streamAdd === 'number' || typeof tc.streamDel === 'number') {
    return {
      add: Math.max(0, Math.floor(tc.streamAdd ?? 0)),
      del: Math.max(0, Math.floor(tc.streamDel ?? 0)),
    };
  }
  return { add: 0, del: 0 };
}

// ---------------------------------------------------------------------------
// Search paths
// ---------------------------------------------------------------------------

function extractSearchKeyword(tc: ToolCallSegment): string {
  const a = tc.args;
  return String(a.pattern ?? a.regex ?? summariseArgs(tc) ?? '');
}

/** 从 search 工具 result 抽 `path:line:` 形式的命中路径列表。 */
function extractSearchPaths(result: string): string[] {
  const paths = new Set<string>();
  for (const line of result.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('Found ') || t.startsWith('No matches')) continue;
    const m = t.match(/^([^:]+?):\d+:/);
    if (m) paths.add(m[1].trim());
  }
  return [...paths];
}

// ---------------------------------------------------------------------------
// Tool rows
// ---------------------------------------------------------------------------

function toolKind(name: string): 'edit' | 'read' | 'search' | 'exec' | 'generic' {
  if (name === 'file_edit' || name === 'file_write') return 'edit';
  if (name === 'file_read') return 'read';
  if (name === 'search') return 'search';
  if (name === 'exec') return 'exec';
  return 'generic';
}

/** 对外工具行入口：包一层 TimelineShell 再走 CodexActivityRow。 */
export function ToolCallTimelineRow({ tc }: { tc: ToolCallSegment }) {
  return (
    <TimelineShell>
      <CodexActivityRow tc={tc} />
    </TimelineShell>
  );
}

// ---------------------------------------------------------------------------
// Segment lists
// ---------------------------------------------------------------------------

export function TextSegmentBlock({ segment }: { segment: TextSegment }) {
  const { t } = useI18n();
  const hasThinking = !!segment.thinking?.trim();
  const hasContent = !!segment.content?.trim();
  if (!hasThinking && !hasContent) return null;
  return (
    <>
      {hasThinking && <ThinkingTimelineRow text={segment.thinking!} title={t('timeline.thinking')} />}
      {hasContent && <AssistantSayRow content={segment.content} />}
    </>
  );
}

/**
 * 被谁调用：CodexTurnTimeline 逐段。
 * 做什么：按 segment.type 分发 text/tool/sandbox/permission；委派与 task 事件由别处渲染。
 */
function SegmentTimelineItem({ seg }: { seg: Segment }) {
  if (seg.type === 'text') return <TextSegmentBlock segment={seg} />;
  if (seg.type === 'tool_call') return <ToolCallTimelineRow tc={seg} />;
  if (seg.type === 'agent_delegate' || seg.type === 'agent_result') return null;
  if (seg.type === 'sandbox_event') return <SandboxTimelineRow seg={seg} />;
  if (seg.type === 'permission') return <PermissionTimelineRow seg={seg} />;
  if (seg.type === 'task_event' || seg.type === 'task_result') return null;
  return null;
}

/**
 * 被谁调用：时间线（若单独挂载 task 段）。
 * 做什么：task_event/result → Info 行；展示结论/风险或 stepTitle/summary。
 */
function TaskEventTimelineRow({ seg }: { seg: Extract<Segment, { type: 'task_event' | 'task_result' }> }) {
  const { t } = useI18n();
  const event = seg.payload?.event as { actor?: { name?: string }; payload?: Record<string, unknown>; type?: string } | undefined;
  const result = seg.payload?.result as { status?: string; output?: { conclusion?: string; risks?: string[] } } | undefined;
  const title = seg.title || event?.actor?.name || t('timeline.taskEvent');
  const body = seg.type === 'task_result'
    ? [
        result?.output?.conclusion || '',
        result?.output?.risks?.length ? `${t('timeline.risk')}${result.output.risks.join('; ')}` : '',
      ].filter(Boolean).join('\n')
    : [
        typeof event?.payload?.stepTitle === 'string' ? event.payload.stepTitle : '',
        typeof event?.payload?.summary === 'string' ? event.payload.summary : '',
      ].filter(Boolean).join(' - ');
  return (
    <InfoEventRow
      icon={<PartitionOutlined />}
      title={title}
      status={seg.status}
    >
      <div style={{ fontSize: 12, color: C.muted, whiteSpace: 'pre-wrap', lineHeight: 1.6 }}>
        {body || t('timeline.taskEventUpdate')}
      </div>
    </InfoEventRow>
  );
}

// ---------------------------------------------------------------------------
// Task checklist (RunPlan todos)
// ---------------------------------------------------------------------------

export function RunPlanChecklist({ plan }: { plan: RunPlan }) {
  const { t } = useI18n();
  const todoIcon = (status: string) => {
    if (status === 'completed') return <span style={{ color: 'var(--xy-success)' }}>✓</span>;
    if (status === 'failed') return <span style={{ color: 'var(--xy-danger)' }}>✗</span>;
    if (status === 'running') return <LoadingOutlined style={{ fontSize: 11, color: 'var(--xy-primary)' }} />;
    return <span style={{ color: C.muted }}>○</span>;
  };
  return (
    <div style={{ padding: '8px 16px', borderBottom: `1px solid ${C.border}`, background: C.codeBg, fontSize: 12 }}>
      <div style={{ color: C.muted, marginBottom: 4, fontWeight: 500 }}>{t('timeline.taskList')}</div>
      {plan.todos.map((todo) => (
        <div key={todo.id} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '2px 0', color: todo.status === 'completed' ? C.muted : C.text }}>
          {todoIcon(todo.status)}
          <span style={{ textDecoration: todo.status === 'completed' ? 'line-through' : 'none' }}>{todo.title}</span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Streaming block (in-flight turn)
// ---------------------------------------------------------------------------

export function StreamingTimeline({
  session,
  sessionId,
}: {
  session: SessionState;
  sessionId?: string;
}) {
  const turnId = [...session.messages].reverse().find((m) => m.role === 'user' && m.turnId)?.turnId;
  return (
    <div className="xy-codex-shell xy-streaming-timeline">
      <CodexTurnTimeline
        segments={session.turnSegments}
        streamingText={session.streamingText}
        reasoningText={session.reasoningText}
        isProcessing
        workStartedAt={session.turnWorkStartedAt}
        sessionId={sessionId}
        turnId={turnId}
      />
    </div>
  );
}

export {
  C as TIMELINE_COLORS,
  formatSubAgentStatus,
  subAgentStatusColor,
};
