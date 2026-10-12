/**
 * CodexTimeline — 可复用的助手时间线。
 *
 * 层级：思考过程 → 命令/编辑 → 正文；深度思考正文挂在思考块内展示
 */

import React, { useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import {
  CheckOutlined,
  CloseOutlined,
  DownOutlined,
  EditOutlined,
  PaperClipOutlined,
  RightOutlined,
} from '@ant-design/icons';
import { Button, Modal, message as antdMessage } from 'antd';
import { parsePatch, structuredPatch } from 'diff';
import type { StructuredPatch } from 'diff';

import type {
  Segment,
  TextSegment,
  ToolCallSegment,
  AgentRunSegment,
} from '../../../stores/chatStore';
import { summariseArgs } from '../../../stores/chatStore';
import { useI18n } from '../../../i18n';
import { useWs } from '../../../App';
import {
  assistantMarkdownComponents,
  normalizeAssistantMarkdown,
  remarkPlugins,
} from './codexMarkdown';
import { DiffViewer } from './diffViewer';

const DiffReviewContext = React.createContext<{ sessionId?: string; turnId?: string }>({});

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CodexEditSummary {
  path: string;
  fileName: string;
  add: number;
  del: number;
}

export type CodexBlock =
  | { kind: 'prose'; id: string; content: string; streaming?: boolean }
  | { kind: 'thinking'; id: string; active: boolean; commands: ToolCallSegment[]; reasoning?: string }
  | { kind: 'commands'; id: string; commands: ToolCallSegment[] }
  | { kind: 'edit'; id: string; tc: ToolCallSegment }
  | { kind: 'files_batch'; id: string; mode: 'create' | 'edit'; items: ToolCallSegment[] }
  | { kind: 'agent_run'; id: string; run: AgentRunSegment }
  | {
      kind: 'work_fold';
      id: string;
      startedAt: number;
      endedAt: number;
      inner: CodexBlock[];
      edits: CodexEditSummary[];
    };

export interface CodexTurnTimelineProps {
  segments?: Segment[];
  streamingText?: string;
  reasoningText?: string;
  isProcessing?: boolean;
  completed?: boolean;
  workStartedAt?: number;
  workEndedAt?: number;
  className?: string;
  sessionId?: string;
  turnId?: string;
}

// ---------------------------------------------------------------------------
// Block builder
// ---------------------------------------------------------------------------

function isCommandTool(name: string): boolean {
  return (
    name === 'exec' ||
    name === 'search' ||
    name === 'list_files' ||
    name === 'glob' ||
    name === 'web_search'
  );
}

/** 被谁调用：块构建。做什么：是否读类工具（折叠样式）。 */
function isReadTool(name: string): boolean {
  return name === 'file_read';
}

/** file_write 可能是新建也可能是覆盖；以工具结果里的真实 diff 为准，禁止只看工具名。 */
function isWriteCreate(tc: ToolCallSegment): boolean {
  if (tc.name !== 'file_write') return false;
  const result = String(tc.result || '');
  if (result.trim()) {
    const statsMatch = result.match(/Lines changed:\s*\+(\d+)\s*\/\s*-(\d+)/i);
    if (statsMatch) return Number(statsMatch[2]) === 0;
    const patch = buildPatch(tc);
    if (patch) return countPatchStats(patch).del === 0;
    // 有结果但解析不出删行：整文件写入且无旧内容痕迹 → 按创建
    return !/Lines changed:\s*\+\d+\s*\/\s*-[1-9]/i.test(result);
  }
  // 流式中：已知旧文件内容则按覆盖/编辑；空字符串表示确认新建。
  if (typeof tc.streamBefore === 'string') return tc.streamBefore.length === 0;
  // 尚不知是否存在：默认「创建」（局部修改走 file_edit）。
  return true;
}

/** 被谁调用：块构建。做什么：是否写/改文件工具。 */
function isWriteModify(tc: ToolCallSegment): boolean {
  return tc.name === 'file_edit' || (tc.name === 'file_write' && !isWriteCreate(tc));
}

/** 被谁调用：diff 构建。做什么：从 tool result 解析 StructuredPatch。 */
function patchFromToolResult(result: string, filePath: string): StructuredPatch | null {
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
          (f) => f === base || f === filePath || (f && (f.endsWith(`/${base}`) || f.endsWith(`\\${base}`))),
        ),
      ) ?? (patches.length === 1 ? patches[0] : null);
    if (matched?.hunks?.length) return matched;
  } catch {
    /* fall through to loose parser */
  }
  return parseLooseUnifiedDiff(diffText, filePath);
}

/**
 * jsdiff 对裸 `@@`（无 -n,m +n,m）会得到空 hunks。
 * 手工收集合法 +/-/空格行，拼成可给 DiffViewer 用的 StructuredPatch。
 */
function parseLooseUnifiedDiff(diffText: string, filePath: string): StructuredPatch | null {
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
      // Trailing prose / spill notice — stop this hunk.
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

/** Prefer live stream counts while calling; otherwise patch / Lines changed. */
function editLineStats(tc: ToolCallSegment): { add: number; del: number } {
  // Streaming FlipNumber must follow engine preview.add/del as args grow — not wait for done.
  if (
    tc.status === 'calling' &&
    (typeof tc.streamAdd === 'number' || typeof tc.streamDel === 'number')
  ) {
    return {
      add: Math.max(0, Math.floor(tc.streamAdd ?? 0)),
      del: Math.max(0, Math.floor(tc.streamDel ?? 0)),
    };
  }
  const patch = buildPatch(tc);
  if (patch) {
    const stats = countPatchStats(patch);
    if (stats.add > 0 || stats.del > 0 || patch.hunks.some((h) => h.lines.some((l) => l[0] === ' '))) {
      return stats;
    }
  }
  if (typeof tc.streamAdd === 'number' || typeof tc.streamDel === 'number') {
    return {
      add: Math.max(0, Math.floor(tc.streamAdd ?? 0)),
      del: Math.max(0, Math.floor(tc.streamDel ?? 0)),
    };
  }
  const fromResult = parseLinesChanged(String(tc.result ?? ''));
  if (fromResult) return fromResult;
  return { add: 0, del: 0 };
}

function parseLinesChanged(result: string): { add: number; del: number } | null {
  const m = result.match(/Lines changed:\s*\+(\d+)\s*\/\s*-(\d+)/i);
  if (!m) return null;
  return { add: Number(m[1]) || 0, del: Number(m[2]) || 0 };
}

/** 被谁调用：时间线命令行。做什么：从 tool 参数抽出命令摘要。 */
export function commandLabel(tc: ToolCallSegment): string {
  if (tc.name === 'exec') {
    return String(tc.args.command ?? tc.args.cmd ?? summariseArgs(tc) ?? '').trim();
  }
  if (tc.name === 'search') {
    const kw = String(tc.args.pattern ?? tc.args.regex ?? tc.args.query ?? '').trim();
    const path = String(tc.args.path ?? '.');
    return kw ? `rg -n "${kw}" -S ${path}` : summariseArgs(tc) || tc.name;
  }
  if (tc.name === 'list_files') {
    const path = String(tc.args.path ?? tc.args.directory ?? '.');
    return `Get-ChildItem -Force${path !== '.' ? ` ${path}` : ''}`;
  }
  if (tc.name === 'glob') return String(tc.args.pattern ?? tc.args.glob_pattern ?? tc.name);
  const summary = summariseArgs(tc);
  if (summary && !summary.startsWith('task_')) return summary;
  return tc.name;
}

/** 被谁调用：编辑行。做什么：路径与增删行数摘要。 */
function editSummaryFromTool(tc: ToolCallSegment): CodexEditSummary {
  const path = String(tc.args.path ?? tc.args.file_path ?? '');
  const fileName = path.split(/[/\\]/).pop() || path || 'file';
  const stats = editLineStats(tc);
  return { path, fileName, add: stats.add, del: stats.del };
}

/** 被谁调用：编辑合并。做什么：路径归一化键。 */
function normalizePathKey(path: string): string {
  return path.replace(/\\/g, '/').replace(/\/+/g, '/').toLowerCase();
}

/** Collapse repeated edits of the same path (keep latest tool call). */
function collapseFileToolsByPath(items: ToolCallSegment[]): ToolCallSegment[] {
  const order: string[] = [];
  const map = new Map<string, ToolCallSegment>();
  for (const tc of items) {
    const path = String(tc.args.path ?? tc.args.file_path ?? tc.id);
    const key = normalizePathKey(path) || tc.id;
    if (!map.has(key)) order.push(key);
    map.set(key, tc);
  }
  return order.map((k) => map.get(k)!);
}

/** Merge same-path edit summaries for the work-fold card (sum add/del). */
function mergeEditSummariesByPath(edits: CodexEditSummary[]): CodexEditSummary[] {
  const order: string[] = [];
  const map = new Map<string, CodexEditSummary>();
  for (const e of edits) {
    const key = normalizePathKey(e.path) || e.fileName;
    const prev = map.get(key);
    if (!prev) {
      order.push(key);
      map.set(key, { ...e });
    } else {
      prev.add += e.add;
      prev.del += e.del;
    }
  }
  return order.map((k) => map.get(k)!);
}

/** 命令块提示符：$ 或 Windows PS；按 UA 与命令形态判断。 */
function shellPromptPrefix(cmd: string): string {
  const winUa =
    typeof navigator !== 'undefined' && /Win/i.test(navigator.userAgent || '');
  const looksWin =
    winUa ||
    /^[A-Za-z]:[\\/]/.test(cmd) ||
    /\b(Get-ChildItem|Remove-Item|Set-Location|Write-Host)\b/i.test(cmd);
  return looksWin ? 'PS>' : '$';
}

/** 被谁调用：编辑活动。做什么：为 tool_call 生成 StructuredPatch。 */
function buildPatch(tc: ToolCallSegment): StructuredPatch | null {
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
      return patchFromToolResult(String(tc.result ?? ''), path);
    }
    if (name === 'file_write') {
      const content = String(args.content ?? '');
      const result = String(tc.result ?? '');

      // Done: Myers(before→after) so identical lines stay context (also fixes legacy naive dumps).
      if (tc.status === 'done' || tc.status === 'error') {
        const before = resolveWriteBefore(tc, content);
        if (before !== null && (content.length > 0 || before.length > 0)) {
          return structuredPatch(path, path, before, content, '', '', { context: 4 });
        }
        const fromResult = patchFromToolResult(result, path);
        if (fromResult && !isNaiveFullRewritePatch(fromResult)) return fromResult;
        return fromResult;
      }

      // Streaming: same as reference — only after streamBefore is known; never fake empty→full.
      if (typeof tc.streamBefore === 'string') {
        return structuredPatch(path, path, tc.streamBefore, content, '', '', { context: 4 });
      }
      return null;
    }
    const r = (tc.result || '').trim();
    if (r && (r.includes('@@') || r.includes('diff --git'))) {
      return parsePatch(r)[0] ?? null;
    }
  } catch {
    return null;
  }
  return null;
}

/** before for file_write: stream probe, else reconstruct from naive all-delete dump in tool result. */
function resolveWriteBefore(tc: ToolCallSegment, afterContent: string): string | null {
  if (typeof tc.streamBefore === 'string') return tc.streamBefore;
  if (!afterContent && tc.status === 'calling') return null;
  return extractBeforeFromNaiveWriteResult(String(tc.result ?? ''));
}

/**
 * Old engine dumped: @@ then every old line as `-`, every new line as `+`.
 * Recover the old file text so UI can recompute a real Myers patch.
 * Returns null when the result already looks like a proper contextual hunk.
 */
function extractBeforeFromNaiveWriteResult(result: string): string | null {
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
  // Proper hunks include context (' ') or interleaved -/+; don't reconstruct those.
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

/** True when patch is one hunk of pure delete-block then add-block (no context). */
function isNaiveFullRewritePatch(patch: StructuredPatch): boolean {
  if (!patch.hunks.length) return true;
  for (const hunk of patch.hunks) {
    let phase: 'del' | 'add' = 'del';
    let hasDel = false;
    let hasAdd = false;
    for (const line of hunk.lines) {
      const op = line[0];
      if (op === ' ') return false;
      if (op === '-') {
        if (phase === 'add') return false;
        hasDel = true;
      } else if (op === '+') {
        phase = 'add';
        hasAdd = true;
      }
    }
    if (hasDel && hasAdd && hunk.lines.length > 8) {
      // Large delete-then-add with no context → almost certainly the old engine dump.
      return true;
    }
  }
  return false;
}

/** 被谁调用：编辑摘要。做什么：统计 patch 增删行。 */
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

/** 会话 segments → 时间线块：文本、工具、diff、命令、子 Agent。 */
export function buildCodexBlocks(input: {
  segments: Segment[];
  streamingText?: string;
  reasoningText?: string;
  isProcessing?: boolean;
  completed?: boolean;
  workStartedAt?: number;
  workEndedAt?: number;
}): CodexBlock[] {
  const blocks: CodexBlock[] = [];
  let pending: ToolCallSegment[] = [];
  let pendingCreates: ToolCallSegment[] = [];
  let pendingModifies: ToolCallSegment[] = [];

  const flushCommands = () => {
    if (pending.length === 0) return;
    blocks.push({ kind: 'commands', id: `cmds_${pending[0].id}`, commands: [...pending] });
    pending = [];
  };

  const flushCreates = () => {
    if (pendingCreates.length === 0) return;
    const items = collapseFileToolsByPath(pendingCreates);
    if (items.length === 1) {
      blocks.push({ kind: 'edit', id: items[0].id, tc: items[0] });
    } else {
      blocks.push({
        kind: 'files_batch',
        id: `create_${items[0].id}`,
        mode: 'create',
        items,
      });
    }
    pendingCreates = [];
  };

  const flushModifies = () => {
    if (pendingModifies.length === 0) return;
    const items = collapseFileToolsByPath(pendingModifies);
    if (items.length === 1) {
      blocks.push({ kind: 'edit', id: items[0].id, tc: items[0] });
    } else {
      blocks.push({
        kind: 'files_batch',
        id: `edit_${items[0].id}`,
        mode: 'edit',
        items,
      });
    }
    pendingModifies = [];
  };

  const flushFiles = () => {
    flushCreates();
    flushModifies();
  };

  for (const seg of input.segments) {
    if (seg.type === 'text') {
      const text = seg as TextSegment;
      if (text.thinking?.trim()) {
        flushCommands();
        flushFiles();
        blocks.push({
          kind: 'thinking',
          id: `think_${text.id}`,
          active: false,
          commands: [],
          reasoning: text.thinking,
        });
      }
      if (text.content?.trim()) {
        flushCommands();
        flushFiles();
        blocks.push({ kind: 'prose', id: text.id, content: text.content });
      }
    } else if (seg.type === 'tool_call') {
      const tc = seg as ToolCallSegment;
      // 父级 agent 工具调用只作为容器，细节由 agent_run 块展示
      if (tc.name === 'agent') {
        flushFiles();
        flushCommands();
        continue;
      }
      if (isWriteCreate(tc)) {
        flushCommands();
        flushModifies();
        pendingCreates.push(tc);
      } else if (isWriteModify(tc)) {
        flushCommands();
        flushCreates();
        pendingModifies.push(tc);
      } else if (isCommandTool(tc.name)) {
        flushFiles();
        pending.push(tc);
      } else {
        flushFiles();
        pending.push(tc);
      }
    } else if (seg.type === 'agent_run') {
      flushCommands();
      flushFiles();
      blocks.push({ kind: 'agent_run', id: seg.id, run: seg });
    } else if (seg.type === 'agent_delegate' || seg.type === 'agent_result') {
      // 旧侧栏路径：主时间线不插卡片（嵌套改走 agent_run）
      continue;
    }
  }

  const reasoning = input.reasoningText?.trim() || '';
  const processing = !!input.isProcessing;

  // 正文已在 segments 里按到达顺序落盘；不再把 streamingText 另插一条以免重复/乱序。

  if (processing) {
    // 命令/文件工具单独成行，不要塞进「正在思考」折叠里。
    flushFiles();
    flushCommands();

    const last = blocks[blocks.length - 1];
    if (last?.kind === 'thinking') {
      // 思考正文已在 segment.thinking 里；勿再用 reasoningText 整段重复渲染。
      blocks[blocks.length - 1] = { ...last, active: true };
    } else if (blocks.length === 0) {
      blocks.push({
        kind: 'thinking',
        id: 'think_live',
        active: true,
        commands: [],
        // 仅在尚无 text 段时兜底（极短窗口）；有段后以 segment.thinking 为准
        reasoning: reasoning || undefined,
      });
    } else if (
      last?.kind === 'commands' ||
      last?.kind === 'edit' ||
      last?.kind === 'files_batch' ||
      last?.kind === 'prose'
    ) {
      // 工具/正文之后等待下一轮模型输出
      blocks.push({
        kind: 'thinking',
        id: 'think_live',
        active: true,
        commands: [],
      });
    }
  } else {
    flushCommands();
    flushFiles();
  }

  const ordered = dedupeAdjacentProse(blocks);

  // 完成态与流式共用同一套块序（思考/命令/编辑/正文穿插）。
  // 时长标头由 CodexTurnTimeline 统一渲染。
  return ordered;
}

/** 去掉紧挨着的重复叙述（常见于 message 与已流式 text 重复落段）。 */
function dedupeAdjacentProse(blocks: CodexBlock[]): CodexBlock[] {
  const out: CodexBlock[] = [];
  for (const b of blocks) {
    const prev = out[out.length - 1];
    if (
      b.kind === 'prose' &&
      prev?.kind === 'prose' &&
      normalizeProseKey(prev.content) === normalizeProseKey(b.content)
    ) {
      continue;
    }
    out.push(b);
  }
  return out;
}

/** 被谁调用：去重正文。做什么：正文归一化键。 */
function normalizeProseKey(content: string): string {
  return content.replace(/\s+/g, ' ').trim();
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

function Chevron({ open, className }: { open: boolean; className?: string }) {
  return open
    ? <DownOutlined className={className || 'xy-cx-chevron'} />
    : <RightOutlined className={className || 'xy-cx-chevron'} />;
}

/** 统一的小字折叠行：默认无箭头，hover 高亮并显示 chevron，展开时常显 chevron */
function CodexMetaRow({
  open,
  onClick,
  disabled,
  icon,
  children,
  trailing,
  className,
}: {
  open?: boolean;
  onClick?: () => void;
  disabled?: boolean;
  icon?: React.ReactNode;
  children: React.ReactNode;
  trailing?: React.ReactNode;
  className?: string;
}) {
  const collapsible = !disabled && !!onClick;
  return (
    <button
      type="button"
      className={`xy-cx-meta-row${open ? ' is-open' : ''}${className ? ` ${className}` : ''}`}
      onClick={onClick}
      disabled={disabled}
    >
      <span className="xy-cx-meta-row-content">
        {icon ? <span className="xy-cx-meta-row-icon">{icon}</span> : null}
        <span className="xy-cx-meta-row-label">{children}</span>
        {trailing ? <span className="xy-cx-meta-row-trailing">{trailing}</span> : null}
      </span>
      {collapsible ? (
        <span className="xy-cx-meta-row-chevron" aria-hidden>
          <Chevron open={!!open} />
        </span>
      ) : null}
    </button>
  );
}

/** 加载中给子节点加闪烁 class。 */
function Shimmer({ active, children }: { active?: boolean; children: React.ReactNode }) {
  return <span className={`xy-cx-shimmer${active ? ' is-active' : ''}`}>{children}</span>;
}

/**
 * 被谁调用：工具调用行。
 * 做什么：calling 时每秒刷新已耗时；结束后冻结为当时秒数。
 */
function useDuration(status: ToolCallSegment['status']): string {
  const start = useRef(Date.now());
  const [sec, setSec] = useState(1);
  useEffect(() => {
    start.current = Date.now();
    if (status !== 'calling') {
      setSec(Math.max(1, Math.round((Date.now() - start.current) / 1000)));
      return;
    }
    const id = window.setInterval(() => {
      setSec(Math.max(1, Math.round((Date.now() - start.current) / 1000)));
    }, 1000);
    return () => window.clearInterval(id);
  }, [status]);
  return `${sec}s`;
}

/** 工作耗时：中文「m 分 s 秒」，其它 `Xm Ys`；至少 1 秒。 */
function formatWorkDuration(ms: number, locale: string): string {
  const totalSec = Math.max(1, Math.floor(ms / 1000));
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (locale === 'zh-CN') return m > 0 ? `${m} 分 ${s} 秒` : `${s} 秒`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

/**
 * 被谁调用：WorkFold。
 * 做什么：running 时每秒刷新 now；停则冻结。返回当前时钟 ms。
 */
function useWorkClock(startedAt?: number, running?: boolean): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!startedAt || !running) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [startedAt, running]);
  if (!startedAt) return 0;
  return running ? now - startedAt : 0;
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

function CommandShellPanel({ tc, cmd, open }: { tc: ToolCallSegment; cmd: string; open: boolean }) {
  const { t } = useI18n();
  if (!open) return null;
  const out = tc.result?.trim() || '';
  const fail = tc.status === 'error';
  const isExec = tc.name === 'exec';
  const prompt = isExec ? shellPromptPrefix(cmd) : '';
  return (
    <div className="xy-cx-shell">
      <div className="xy-cx-shell-bar">
        <span>{isExec ? t('timeline.shell') : tc.name}</span>
        {tc.status === 'calling' ? (
          <span className="xy-cx-shell-meta">{t('timeline.running')}</span>
        ) : fail ? (
          <span className="xy-cx-shell-fail">✗</span>
        ) : (
          <span className="xy-cx-shell-ok">✓ {t('timeline.success')}</span>
        )}
      </div>
      <pre className="xy-cx-shell-cmd">{prompt ? `${prompt} ${cmd}` : cmd}</pre>
      <pre className={`xy-cx-shell-out${fail ? ' is-error' : ''}${!out && tc.status !== 'calling' ? ' is-empty' : ''}`}>
        {out || (tc.status === 'calling' ? '…' : t('timeline.noCommandOutput', '（无输出）'))}
      </pre>
    </div>
  );
}

/** 单条 exec：默认折叠，展开才显示 shell 面板。 */
function CommandLine({ tc, nested }: { tc: ToolCallSegment; nested?: boolean }) {
  const { t } = useI18n();
  const cmd = commandLabel(tc);
  const dur = useDuration(tc.status);
  // 默认折叠：运行中/失败也不自动展开，避免占满时间线。
  const [open, setOpen] = useState(false);
  const failed = tc.status === 'error';
  const label = isReadTool(tc.name)
    ? failed
      ? t('timeline.readFailed', undefined, cmd || '—', dur)
      : t('timeline.ranRead', undefined, cmd || '—', dur)
    : failed
      ? t('timeline.commandFailed', undefined, cmd || tc.name || '—', dur)
      : t('timeline.ranCommand', undefined, cmd || tc.name || '—', dur);

  return (
    <div className={`xy-cx-cmdline${nested ? ' is-nested' : ''}`}>
      <CodexMetaRow
        open={open}
        onClick={() => setOpen(!open)}
        className="xy-cx-cmdline-row"
      >
        {label}
      </CodexMetaRow>
      <CommandShellPanel
        tc={tc}
        cmd={cmd}
        open={open}
      />
    </div>
  );
}

/** 思考块：摘要行可展开正文 Markdown。 */
function ThinkingBlock({ block }: { block: Extract<CodexBlock, { kind: 'thinking' }> }) {
  const { t } = useI18n();
  const hasReasoning = !!block.reasoning?.trim();
  // 流式中展开；结束后默认折叠（可手动再打开）
  const [open, setOpen] = useState(!!block.active);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setOpen(!!block.active);
  }, [block.active]);

  useEffect(() => {
    if (!block.active || !open || !bodyRef.current) return;
    bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
  }, [block.reasoning, block.active, open]);

  const hasBody = hasReasoning || block.commands.length > 0;
  const label = block.active
    ? t('timeline.thinkingActive')
    : hasReasoning
      ? t('timeline.thinkingProcess')
      : t('timeline.thinkingActive');

  return (
    <div className={`xy-cx-thinking${block.active ? ' is-active' : ''}`}>
      <CodexMetaRow
        open={open && hasBody}
        onClick={hasBody ? () => setOpen(!open) : undefined}
        disabled={!hasBody}
      >
        <Shimmer active={block.active}>{label}</Shimmer>
      </CodexMetaRow>
      {open && hasBody ? (
        <div className="xy-cx-fold-body">
          {hasReasoning ? (
            <div ref={bodyRef} className="xy-cx-reasoning xy-markdown-body">
              <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
                {normalizeAssistantMarkdown(block.reasoning!)}
              </ReactMarkdown>
            </div>
          ) : null}
          {block.commands.map((tc) => (
            <CommandLine key={tc.id} tc={tc} nested />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** 连续 exec 合成一批，折叠展示。 */
function CommandsBatch({ block }: { block: Extract<CodexBlock, { kind: 'commands' }> }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  if (block.commands.length === 1) {
    return <CommandLine tc={block.commands[0]} />;
  }
  const failedCount = block.commands.filter((c) => c.status === 'error').length;
  return (
    <div className="xy-cx-batch">
      <CodexMetaRow
        open={open}
        onClick={() => setOpen(!open)}
        icon={<span className="xy-cx-terminal" aria-hidden />}
      >
        {failedCount > 0
          ? t('timeline.commandsWithFailures', undefined, block.commands.length, failedCount)
          : t('timeline.ranCommands', undefined, block.commands.length)}
      </CodexMetaRow>
      {open ? (
        <div className="xy-cx-fold-body">
          {block.commands.map((tc) => (
            <CommandLine key={tc.id} tc={tc} nested />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** `+n / -n` 翻牌数字。 */
function DiffStats({ add, del }: { add: number; del: number }) {
  if (add === 0 && del === 0) return null;
  return (
    <span className="xy-cx-diff-stats">
      {add > 0 ? <FlipNumber className="is-add" prefix="+" value={add} /> : null}
      {del > 0 ? <FlipNumber className="is-del" prefix="−" value={del} /> : null}
    </span>
  );
}

/** 圆柱旋钮计数：圆心在显示平面后方，数字贴在圆周上转动（非绕字心翻牌）。 */
function FlipNumber({
  value,
  prefix,
  className,
}: {
  value: number;
  prefix: string;
  className?: string;
}) {
  const digits = String(Math.max(0, Math.floor(value))).split('');
  return (
    <span className={`xy-cx-flip-number${className ? ` ${className}` : ''}`}>
      <span className="xy-cx-flip-prefix">{prefix}</span>
      {digits.map((digit, index) => (
        <FlipDigit key={`place-${digits.length - index}`} digit={digit} />
      ))}
    </span>
  );
}

/** 单数字翻牌动画。 */
function FlipDigit({ digit }: { digit: string }) {
  const prevRef = useRef(digit);
  const faceRef = useRef<HTMLSpanElement>(null);
  const [shown, setShown] = useState(digit);
  const [outgoing, setOutgoing] = useState<string | null>(null);
  const [animKey, setAnimKey] = useState(0);

  useEffect(() => {
    if (digit === prevRef.current) return;
    setOutgoing(prevRef.current);
    setShown(digit);
    prevRef.current = digit;
    setAnimKey((key) => key + 1);
  }, [digit]);

  // 同一节点上重触发滚入，结束时不拆 DOM，避免左右顿挫
  useLayoutEffect(() => {
    if (animKey === 0) return;
    const el = faceRef.current;
    if (!el) return;
    el.style.animation = 'none';
    void el.offsetWidth;
    el.style.animation = 'xy-cx-reel-in 0.38s cubic-bezier(0.25, 0.1, 0.2, 1) forwards';
    const timer = window.setTimeout(() => {
      setOutgoing(null);
      if (faceRef.current) faceRef.current.style.animation = '';
    }, 400);
    return () => window.clearTimeout(timer);
  }, [animKey]);

  return (
    <span className="xy-cx-reel-digit">
      <span className="xy-cx-reel-cylinder">
        {outgoing !== null ? (
          <span key={`out-${animKey}`} className="xy-cx-reel-face is-out" aria-hidden>
            {outgoing}
          </span>
        ) : null}
        <span ref={faceRef} className="xy-cx-reel-face is-rest">
          {shown}
        </span>
      </span>
    </span>
  );
}

/** 文件 diff 预览卡：文件名 + DiffStats + 补丁正文。 */
function FilePreviewPanel({
  tc,
  path,
  fileName,
  stats,
  running,
}: {
  tc: ToolCallSegment;
  path: string;
  fileName: string;
  stats: { add: number; del: number };
  running: boolean;
}) {
  const { t } = useI18n();
  const { send } = useWs();
  const review = useContext(DiffReviewContext);
  const [decision, setDecision] = useState<'kept' | 'rejected' | null>(null);
  const [busy, setBusy] = useState(false);
  const isCreate = isWriteCreate(tc);
  const patch = buildPatch(tc);
  const canReview =
    Boolean(review.sessionId && review.turnId && path && tc.status === 'done' && !running);

  const reject = async () => {
    if (!review.sessionId || !review.turnId || !path || busy) return;
    setBusy(true);
    try {
      send('workspace.file.reject', {
        sessionId: review.sessionId,
        turnId: review.turnId,
        path,
      });
      setDecision('rejected');
      antdMessage.success(t('timeline.diffRejected', '已拒绝并还原该文件'));
    } finally {
      setBusy(false);
    }
  };

  const reviewExtra =
    canReview && !decision ? (
      <span className="xy-cx-diff-actions">
        <Button size="small" type="text" icon={<CheckOutlined />} onClick={() => setDecision('kept')}>
          {t('timeline.diffKeep', '保留')}
        </Button>
        <Button
          size="small"
          type="text"
          danger
          icon={<CloseOutlined />}
          loading={busy}
          onClick={() => void reject()}
        >
          {t('timeline.diffReject', '拒绝')}
        </Button>
      </span>
    ) : decision === 'kept' ? (
      <span className="xy-cx-diff-decision is-keep">{t('timeline.diffKept', '已保留')}</span>
    ) : decision === 'rejected' ? (
      <span className="xy-cx-diff-decision is-reject">{t('timeline.diffRejectedShort', '已拒绝')}</span>
    ) : null;

  if (patch && patch.hunks.length > 0) {
    return (
      <DiffViewer
        patch={patch}
        sourcePath={path}
        fileName={fileName}
        stats={stats}
        headerExtra={reviewExtra}
      />
    );
  }

  return (
    <div className={`xy-cx-diff-card${isCreate ? ' is-create' : ''}`}>
      <div className="xy-cx-diff-card-head">
        <div className="xy-cx-diff-card-head-left">
          <span className="xy-cx-diff-fname">{fileName}</span>
          <DiffStats add={stats.add} del={stats.del} />
        </div>
        <div className="xy-cx-diff-card-head-right">{reviewExtra}</div>
      </div>
      <pre className="xy-cx-shell-out">
        {tc.result?.trim() || (running ? t('timeline.applyingEdit') : '—')}
      </pre>
    </div>
  );
}

/** 文件列表一行：点开才出预览。 */
function FileListRow({ tc }: { tc: ToolCallSegment }) {
  const path = String(tc.args.path ?? tc.args.file_path ?? '');
  const fileName = path.split(/[/\\]/).pop() || path || 'file';
  const patch = buildPatch(tc);
  const stats = editLineStats(tc);
  const running = tc.status === 'calling';
  const [previewOpen, setPreviewOpen] = useState(false);

  return (
    <div className="xy-cx-file-item">
      <CodexMetaRow
        open={previewOpen}
        onClick={() => setPreviewOpen(!previewOpen)}
      >
        <span className="xy-cx-file-name">{fileName}</span>
        <DiffStats add={stats.add} del={stats.del} />
      </CodexMetaRow>
      {previewOpen ? (
        <FilePreviewPanel tc={tc} path={path} fileName={fileName} stats={stats} running={running} />
      ) : null}
    </div>
  );
}

/** 多文件读/列目录合成一批。 */
function FilesBatchBlock({ block }: { block: Extract<CodexBlock, { kind: 'files_batch' }> }) {
  const { t } = useI18n();
  const running = block.items.some((tc) => tc.status === 'calling');
  const [open, setOpen] = useState(running);
  const isCreate = block.mode === 'create';
  const label = running
    ? isCreate
      ? t('timeline.creatingFiles', undefined, block.items.length)
      : t('timeline.editingFiles', undefined, block.items.length)
    : isCreate
      ? t('timeline.createdFiles', undefined, block.items.length)
      : t('timeline.editedFiles', undefined, block.items.length);

  return (
    <div className="xy-cx-files-batch">
      <CodexMetaRow
        open={open}
        onClick={() => setOpen(!open)}
        icon={<PaperClipOutlined className="xy-cx-clip" />}
      >
        {label}
      </CodexMetaRow>
      {open ? (
        <div className="xy-cx-file-list">
          {block.items.map((tc) => (
            <FileListRow key={tc.id} tc={tc} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** 写/改文件块：路径 + 增删 + 可展开预览。 */
function EditBlock({ block }: { block: Extract<CodexBlock, { kind: 'edit' }> }) {
  const { t } = useI18n();
  const tc = block.tc;
  const isCreate = isWriteCreate(tc);
  const path = String(tc.args.path ?? tc.args.file_path ?? '');
  const fileName = path.split(/[/\\]/).pop() || path;
  // Streaming animation lives on the collapsed row (FlipNumber). Keep closed by default.
  const stats = editLineStats(tc);
  const running = tc.status === 'calling';
  const failed = tc.status === 'error';
  const [sectionOpen, setSectionOpen] = useState(false);

  const sectionLabel = failed
    ? isCreate
      ? t('timeline.createFileFailed', '创建文件失败')
      : t('timeline.editFileFailed', '编辑文件失败')
    : running
      ? isCreate
        ? t('timeline.creatingFile', '正在创建文件')
        : t('timeline.editingFile', '正在编辑文件')
      : isCreate
        ? t('timeline.createdFile')
        : t('timeline.editedFile');

  // Keep the row visible while streaming so FlipNumber can tick before path arrives.
  // Never show "未提供路径" during calling — omit the name until known.
  const showName = Boolean(fileName) || !running;

  return (
    <div className={`xy-cx-edit${running ? ' is-running' : ''}${failed ? ' is-error' : ''}`}>
      <CodexMetaRow
        open={sectionOpen}
        onClick={() => setSectionOpen(!sectionOpen)}
        icon={<EditOutlined className="xy-cx-clip" />}
      >
        <span className="xy-cx-edit-label">{sectionLabel}</span>
        {!sectionOpen ? (
          <>
            {showName ? (
              <span className="xy-cx-file-name">{fileName || t('timeline.noPath')}</span>
            ) : null}
            {failed ? null : <DiffStats add={stats.add} del={stats.del} />}
          </>
        ) : null}
      </CodexMetaRow>
      {sectionOpen ? (
        <div className="xy-cx-edit-body">
          <FilePreviewPanel tc={tc} path={path} fileName={fileName} stats={stats} running={running} />
        </div>
      ) : null}
    </div>
  );
}

/** 助手正文 Markdown。 */
function ProseBlock({ block }: { block: Extract<CodexBlock, { kind: 'prose' }> }) {
  return (
    <div className="xy-cx-prose xy-markdown-body">
      <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
        {normalizeAssistantMarkdown(block.content)}
      </ReactMarkdown>
    </div>
  );
}

/** 子 Agent 一次 run：标题、耗时、嵌套事件。 */
function AgentRunBlock({ block }: { block: Extract<CodexBlock, { kind: 'agent_run' }> }) {
  const run = block.run;
  const review = useContext(DiffReviewContext);
  const { send } = useWs();
  const { t, locale } = useI18n();
  const [detailOpen, setDetailOpen] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const isRunning = run.status === 'running';
  const isDone =
    run.status === 'completed' || run.status === 'failed' || run.status === 'cancelled';

  // Nested agent preview: follow output only while user is at the bottom of this panel.
  const stickNestedRef = useRef(true);
  useLayoutEffect(() => {
    const el = viewportRef.current;
    if (!el || !stickNestedRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [run.children, run.summary, run.status, detailOpen]);

  // Live refresh only while running; after restart, children come from persisted agent_run.
  useEffect(() => {
    if (!detailOpen || !review.sessionId || !run.taskId || !isRunning) return;
    const pull = () => send('subagent.timeline', { sessionId: review.sessionId, taskId: run.taskId });
    pull();
    const timer = window.setInterval(pull, 1200);
    return () => window.clearInterval(timer);
  }, [detailOpen, review.sessionId, run.taskId, isRunning, send]);

  const statusLabel =
    run.status === 'running'
      ? run.background
        ? locale === 'zh-CN'
          ? '后台'
          : 'Background'
        : locale === 'zh-CN'
          ? '运行中'
          : 'Running'
      : run.status === 'completed'
        ? locale === 'zh-CN'
          ? '完成'
          : 'Done'
        : run.status === 'failed'
          ? locale === 'zh-CN'
            ? '失败'
            : 'Failed'
          : run.status === 'cancelled'
            ? locale === 'zh-CN'
              ? '已取消'
              : 'Cancelled'
            : run.status;

  const scope =
    run.readScope?.length || run.writeScope?.length
      ? [
          run.readScope?.length ? `读 ${run.readScope.join(', ')}` : '',
          run.writeScope?.length ? `写 ${run.writeScope.join(', ')}` : '',
        ]
          .filter(Boolean)
          .join(' · ')
      : '';

  const titleText = `${run.agentName || run.agentId}${
    run.task ? ` · ${run.task.length > 64 ? `${run.task.slice(0, 63)}…` : run.task}` : ''
  }`;

  const openDetail = () => setDetailOpen(true);

  return (
    <>
      <div
        className={`xy-cx-agent-run${isRunning ? ' is-active' : ''}${isDone ? ' is-done' : ''}`}
        role="button"
        tabIndex={0}
        onClick={openDetail}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            openDetail();
          }
        }}
        aria-label={t('timeline.viewDetail', '查看详情')}
      >
        <div className="xy-cx-agent-run-head-row">
          <div className="xy-cx-agent-run-head">
            <span className="xy-cx-agent-run-title">{titleText}</span>
            <span className={`xy-cx-agent-run-status is-${run.status}`}>{statusLabel}</span>
          </div>
        </div>
        {scope ? <div className="xy-cx-agent-run-scope">{scope}</div> : null}
        <div className="xy-cx-agent-run-viewport-wrap">
          <div
            ref={viewportRef}
            className="xy-cx-agent-run-viewport"
            onScroll={() => {
              const el = viewportRef.current;
              if (!el) return;
              stickNestedRef.current =
                el.scrollHeight - el.scrollTop - el.clientHeight <= 80;
            }}
          >
            {run.children?.length ? (
              <CodexTurnTimeline
                segments={run.children}
                isProcessing={isRunning}
                completed={isDone}
                className="xy-cx-agent-run-preview-timeline"
              />
            ) : isRunning ? (
              <div className="xy-cx-agent-run-empty">子 Agent 启动中…</div>
            ) : (
              <div className="xy-cx-agent-run-empty">暂无事件</div>
            )}
            {run.summary ? (
              <div className="xy-cx-agent-run-summary xy-markdown-body">
                <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
                  {normalizeAssistantMarkdown(run.summary)}
                </ReactMarkdown>
              </div>
            ) : null}
          </div>
          <div className="xy-cx-agent-run-fade xy-cx-agent-run-fade-top" aria-hidden />
          <div className="xy-cx-agent-run-fade xy-cx-agent-run-fade-bottom" aria-hidden />
        </div>
        <div className="xy-cx-agent-run-hint">
          {locale === 'zh-CN' ? '点击查看完整时间线' : 'Click for full timeline'}
        </div>
      </div>

      <Modal
        title={titleText}
        open={detailOpen}
        onCancel={() => setDetailOpen(false)}
        footer={null}
        width={Math.min(1180, typeof window !== 'undefined' ? Math.floor(window.innerWidth * 0.92) : 1180)}
        destroyOnClose
        className="xy-cx-agent-run-modal"
        styles={{ body: { maxHeight: '78vh', overflow: 'auto', paddingTop: 8 } }}
      >
        <div className="xy-cx-agent-run-modal-meta">
          <span className={`xy-cx-agent-run-status is-${run.status}`}>{statusLabel}</span>
          {scope ? <span className="xy-cx-agent-run-scope-inline">{scope}</span> : null}
        </div>
        {run.children?.length || run.summary ? (
          <>
            <CodexTurnTimeline
              segments={run.children || []}
              isProcessing={isRunning}
              completed={isDone}
              sessionId={review.sessionId}
              workStartedAt={run.startedAt}
              workEndedAt={run.endedAt}
            />
            {run.summary ? (
              <div className="xy-cx-agent-run-summary xy-markdown-body" style={{ marginTop: 12 }}>
                <ReactMarkdown remarkPlugins={remarkPlugins} components={assistantMarkdownComponents}>
                  {normalizeAssistantMarkdown(run.summary)}
                </ReactMarkdown>
              </div>
            ) : null}
          </>
        ) : (
          <div className="xy-cx-agent-run-empty">
            {isRunning ? '子 Agent 启动中…' : t('timeline.noEvents', '暂无事件')}
          </div>
        )}
      </Modal>
    </>
  );
}

/** 本轮工作折叠：命令/文件改动摘要，可展开。 */
function WorkFold({ block }: { block: Extract<CodexBlock, { kind: 'work_fold' }> }) {
  const { t, locale } = useI18n();
  const [open, setOpen] = useState(false);
  const dur = formatWorkDuration(block.endedAt - block.startedAt, locale);
  const totalAdd = block.edits.reduce((s, e) => s + e.add, 0);
  const totalDel = block.edits.reduce((s, e) => s + e.del, 0);

  return (
    <div className="xy-cx-work">
      <CodexMetaRow
        open={open}
        onClick={() => setOpen(!open)}
        className="xy-cx-work-head"
      >
        {t('timeline.workedFor', undefined, dur)}
      </CodexMetaRow>
      {!open && block.edits.length > 0 ? (
        <div className="xy-cx-work-card">
          <div className="xy-cx-work-card-head">
            <PaperClipOutlined className="xy-cx-clip" />
            <div>
              <div className="xy-cx-work-card-title">
                {t('timeline.editedFiles', undefined, block.edits.length)}
              </div>
              <DiffStats add={totalAdd} del={totalDel} />
            </div>
          </div>
          <ul className="xy-cx-work-filelist">
            {block.edits.map((e) => (
              <li key={e.path}>
                <span className="xy-cx-work-path">{e.path}</span>
                <DiffStats add={e.add} del={e.del} />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {open ? (
        <div className="xy-cx-work-inner">
          {block.inner.map((b) => (
            <CodexBlockView key={b.id} block={b} />
          ))}
          {!block.inner.some((b) => b.kind === 'edit' || b.kind === 'files_batch') &&
          block.edits.length > 0 ? (
            <div className="xy-cx-work-card">
              <div className="xy-cx-work-card-head">
                <PaperClipOutlined className="xy-cx-clip" />
                <div>
                  <div className="xy-cx-work-card-title">
                    {t('timeline.editedFiles', undefined, block.edits.length)}
                  </div>
                  <DiffStats add={totalAdd} del={totalDel} />
                </div>
              </div>
              <ul className="xy-cx-work-filelist">
                {block.edits.map((e) => (
                  <li key={e.path}>
                    <span className="xy-cx-work-path">{e.path}</span>
                    <DiffStats add={e.add} del={e.del} />
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** 按 kind 分发给具体块组件。 */
function CodexBlockView({ block }: { block: CodexBlock }) {
  switch (block.kind) {
    case 'prose':
      return <ProseBlock block={block} />;
    case 'thinking':
      return <ThinkingBlock block={block} />;
    case 'commands':
      return <CommandsBatch block={block} />;
    case 'edit':
      return <EditBlock block={block} />;
    case 'files_batch':
      return <FilesBatchBlock block={block} />;
    case 'agent_run':
      return <AgentRunBlock block={block} />;
    case 'work_fold':
      return <WorkFold block={block} />;
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Public timeline
// ---------------------------------------------------------------------------

export function CodexTurnTimeline({
  segments = [],
  streamingText,
  reasoningText,
  isProcessing,
  completed,
  workStartedAt,
  workEndedAt,
  className,
  sessionId,
  turnId,
}: CodexTurnTimelineProps) {
  const { t, locale } = useI18n();
  const liveMs = useWorkClock(workStartedAt, isProcessing && !completed);
  const blocks = useMemo(
    () =>
      buildCodexBlocks({
        segments,
        streamingText,
        reasoningText,
        isProcessing,
        completed,
        workStartedAt: workStartedAt || (liveMs > 0 ? Date.now() - liveMs : undefined),
        workEndedAt,
      }),
    [
      segments,
      streamingText,
      reasoningText,
      isProcessing,
      completed,
      workStartedAt,
      workEndedAt,
      liveMs,
    ],
  );

  const hasWorkBlocks = blocks.some((block) => block.kind !== 'prose');
  const durationMs = (() => {
    if (!workStartedAt) return liveMs;
    if (completed) {
      const ended = workEndedAt && workEndedAt >= workStartedAt ? workEndedAt : workStartedAt;
      return Math.max(0, ended - workStartedAt);
    }
    return liveMs || Math.max(0, Date.now() - workStartedAt);
  })();
  const showWorkHeader = Boolean(workStartedAt && hasWorkBlocks && (isProcessing || completed));

  return (
    <DiffReviewContext.Provider value={{ sessionId, turnId }}>
      <div className={`xy-cx-timeline${className ? ` ${className}` : ''}`}>
        {showWorkHeader ? (
          <div className={`xy-cx-work-live${completed ? ' is-done' : ''}`}>
            {t('timeline.workedFor', undefined, formatWorkDuration(durationMs, locale))}
          </div>
        ) : null}
        {blocks.map((b) => (
          <CodexBlockView key={b.id} block={b} />
        ))}
      </div>
    </DiffReviewContext.Provider>
  );
}

/** 子 Agent 时间线事件 → 可渲染 CodexBlock 列表。 */
export function segmentsFromSubAgentEvents(
  events: Array<{ id: string; type: string; payload: Record<string, unknown>; status?: string }>,
): Segment[] {
  const out: Segment[] = [];
  for (const ev of events) {
    if (ev.type === 'text') {
      const content = String(ev.payload.content || '');
      if (content.trim()) out.push({ type: 'text', id: ev.id, content });
    } else if (ev.type === 'reasoning') {
      const content = String(ev.payload.content || '');
      if (content.trim()) out.push({ type: 'text', id: ev.id, content: '', thinking: content });
    } else if (ev.type === 'progress') {
      const title = String((ev as { title?: string }).title || ev.payload.stepTitle || '').trim();
      const summary = String(ev.payload.summary || ev.payload.stepTitle || title || '').trim();
      if (summary) out.push({ type: 'text', id: ev.id, content: summary });
    } else if (ev.type === 'tool_call') {
      const call = ev.payload.call as { id?: string; function?: { name?: string; arguments?: string } } | undefined;
      const result = ev.payload.result as { content?: string; isError?: boolean } | undefined;
      let args: Record<string, unknown> = {};
      const raw = call?.function?.arguments || '';
      if (raw) {
        try {
          const p = JSON.parse(raw);
          if (typeof p === 'object' && p) args = p;
        } catch {
          /* ok */
        }
      }
      out.push({
        type: 'tool_call',
        id: call?.id || ev.id,
        name: call?.function?.name || 'tool',
        args:
          ev.payload.streamArgs && typeof ev.payload.streamArgs === 'object'
            ? (ev.payload.streamArgs as Record<string, unknown>)
            : args,
        rawArgs: raw,
        status: result?.isError ? 'error' : ev.status === 'done' ? 'done' : 'calling',
        result: result?.content,
        streamBefore:
          typeof ev.payload.streamBefore === 'string' ? ev.payload.streamBefore : undefined,
      });
    }
  }
  return out;
}
