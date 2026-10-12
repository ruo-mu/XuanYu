/**
 * Diff 预览
 * - 文件头栏：文件名 + +/- 统计 + 复制
 * - 行级红/绿底 + 左侧色条（删除斜线、新增实线）
 * - 相邻删改行做词级 intra-line 高亮
 * - 轻量语法着色
 */
import React, { useMemo, useState } from 'react';
import type { StructuredPatch } from 'diff';
import { diffWordsWithSpace } from 'diff';
import { CopyOutlined, CheckOutlined } from '@ant-design/icons';
import { useI18n } from '../../../i18n';

type DiffLineRow =
  | { kind: 'ctx'; oldNum: number; newNum: number; text: string }
  | { kind: 'del'; oldNum: number; text: string; pairText?: string }
  | { kind: 'add'; newNum: number; text: string; pairText?: string };

type DiffRow =
  | { kind: 'sep' }
  | DiffLineRow
  | { kind: 'fold'; hidden: DiffLineRow[] };

const FOLD_THRESHOLD = 8;
const FOLD_KEEP = 2;

/** 结构化 patch 转可折叠行：hunk 分隔、上下文折叠。 */
function patchToRows(patch: StructuredPatch): DiffRow[] {
  const out: DiffRow[] = [];
  let firstHunk = true;
  for (const hunk of patch.hunks) {
    if (!firstHunk) out.push({ kind: 'sep' });
    firstHunk = false;

    let oldNum = hunk.oldStart;
    let newNum = hunk.newStart;
    let ctxBuf: DiffLineRow[] = [];
    const flushCtx = () => {
      if (ctxBuf.length === 0) return;
      if (ctxBuf.length <= FOLD_THRESHOLD) out.push(...ctxBuf);
      else {
        const hidden = ctxBuf.slice(FOLD_KEEP, -FOLD_KEEP);
        out.push(...ctxBuf.slice(0, FOLD_KEEP));
        if (hidden.length > 0) out.push({ kind: 'fold', hidden });
        out.push(...ctxBuf.slice(-FOLD_KEEP));
      }
      ctxBuf = [];
    };

    let dels: Array<{ oldNum: number; text: string }> = [];
    let adds: Array<{ newNum: number; text: string }> = [];
    const flushChange = () => {
      // Unified style: emit the whole delete block, then the whole add block.
      // Only pair for word-level highlight on a 1:1 line replace.
      if (dels.length === 1 && adds.length === 1) {
        out.push({
          kind: 'del',
          oldNum: dels[0].oldNum,
          text: dels[0].text,
          pairText: adds[0].text,
        });
        out.push({
          kind: 'add',
          newNum: adds[0].newNum,
          text: adds[0].text,
          pairText: dels[0].text,
        });
      } else {
        for (const d of dels) {
          out.push({ kind: 'del', oldNum: d.oldNum, text: d.text });
        }
        for (const a of adds) {
          out.push({ kind: 'add', newNum: a.newNum, text: a.text });
        }
      }
      dels = [];
      adds = [];
    };

    for (const line of hunk.lines) {
      const op = line[0];
      const content = line.endsWith('\n') ? line.slice(1, -1) : line.slice(1);
      if (op === ' ') {
        flushChange();
        ctxBuf.push({ kind: 'ctx', oldNum, newNum, text: content });
        oldNum++;
        newNum++;
      } else if (op === '-') {
        flushCtx();
        dels.push({ oldNum, text: content });
        oldNum++;
      } else if (op === '+') {
        flushCtx();
        adds.push({ newNum, text: content });
        newNum++;
      }
    }
    flushChange();
    flushCtx();
  }
  return out;
}

/** 路径扩展名小写，给行内语法着色。 */
function extOf(path?: string): string {
  if (!path) return '';
  const base = path.split(/[/\\]/).pop() || '';
  const i = base.lastIndexOf('.');
  return i >= 0 ? base.slice(i + 1).toLowerCase() : '';
}

/** 轻量 token 着色（不依赖整行 highlighter，便于叠词级高亮） */
function highlightCode(text: string, ext: string): React.ReactNode {
  if (!text) return ' ';
  const rules: Array<{ re: RegExp; cls: string }> = [];
  if (ext === 'css' || ext === 'scss' || ext === 'less') {
    rules.push(
      { re: /(\/\*[\s\S]*?\*\/|\/\/[^\n]*)/g, cls: 'is-comment' },
      { re: /([.#][\w-]+)/g, cls: 'is-selector' },
      { re: /(--[\w-]+)/g, cls: 'is-prop' },
      { re: /\b([\w-]+)(?=\s*:)/g, cls: 'is-prop' },
      { re: /\b(\d+(?:\.\d+)?(?:px|em|rem|%|vh|vw|s|ms)?)\b/g, cls: 'is-number' },
    );
  } else {
    rules.push(
      { re: /(\/\/[^\n]*|\/\*[\s\S]*?\*\/|#\s[^\n]*)/g, cls: 'is-comment' },
      { re: /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`)/g, cls: 'is-string' },
      {
        re: /\b(const|let|var|function|return|if|else|for|while|class|interface|type|import|from|export|default|async|await|new|typeof|keyof|extends|implements|public|private|protected|readonly|enum|namespace|module|switch|case|break|continue|try|catch|throw|null|undefined|true|false)\b/g,
        cls: 'is-keyword',
      },
      { re: /\b([A-Z][A-Za-z0-9_]*)\b/g, cls: 'is-type' },
      { re: /\b(\d+(?:\.\d+)?)\b/g, cls: 'is-number' },
      { re: /\b([a-zA-Z_][\w]*)(?=\s*:)/g, cls: 'is-key' },
    );
  }

  // 单遍：按优先级切分
  type Piece = { start: number; end: number; cls: string };
  const pieces: Piece[] = [];
  for (const { re, cls } of rules) {
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text))) {
      const start = m.index;
      const end = start + m[0].length;
      if (pieces.some((p) => start < p.end && end > p.start)) continue;
      pieces.push({ start, end, cls });
    }
  }
  pieces.sort((a, b) => a.start - b.start);

  if (pieces.length === 0) return text;
  const nodes: React.ReactNode[] = [];
  let cursor = 0;
  pieces.forEach((p, i) => {
    if (p.start > cursor) nodes.push(text.slice(cursor, p.start));
    nodes.push(
      <span key={`t-${i}`} className={`xy-cx-syn ${p.cls}`}>
        {text.slice(p.start, p.end)}
      </span>,
    );
    cursor = p.end;
  });
  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

/** 单行 diff：词级高亮 + 轻量 token 色，不跑整文件 highlighter。 */
function InlineCode({
  text,
  pairText,
  side,
  ext,
}: {
  text: string;
  pairText?: string;
  side: 'add' | 'del' | 'ctx';
  ext: string;
}) {
  const className = `xy-cx-diff-code is-${side}`;
  if (side === 'ctx' || !pairText) {
    return <code className={className}>{highlightCode(text || ' ', ext)}</code>;
  }
  const before = side === 'del' ? text : pairText;
  const after = side === 'add' ? text : pairText;
  const parts = diffWordsWithSpace(before, after);
  return (
    <code className={className}>
      {parts.map((part, i) => {
        if (side === 'del' && part.added) return null;
        if (side === 'add' && part.removed) return null;
        const changed = (side === 'del' && part.removed) || (side === 'add' && part.added);
        if (changed) {
          return (
            <span key={i} className={`xy-cx-diff-inline is-${side}`}>
              {part.value}
            </span>
          );
        }
        return <React.Fragment key={i}>{highlightCode(part.value, ext)}</React.Fragment>;
      })}
    </code>
  );
}

/** 拆成 gutter + code 两个 grid 单元格，保证整列同宽、背景拉满 */
function DiffLineCells({ row, ext }: { row: DiffLineRow; ext: string }) {
  const lineNo =
    row.kind === 'del' ? row.oldNum : row.kind === 'add' ? row.newNum : row.newNum;
  return (
    <>
      <div className={`xy-cx-diff-gutter is-${row.kind}`}>
        <span className={`xy-cx-diff-bar is-${row.kind}`} aria-hidden />
        <span className={`xy-cx-diff-ln is-${row.kind}`}>{lineNo}</span>
      </div>
      <InlineCode
        text={row.text}
        pairText={row.kind === 'ctx' ? undefined : row.pairText}
        side={row.kind}
        ext={ext}
      />
    </>
  );
}

/** 取出 patch 新文件全文（+ 与上下文行），给「新内容」预览。 */
function patchNewText(patch: StructuredPatch): string {
  const lines: string[] = [];
  for (const hunk of patch.hunks) {
    for (const line of hunk.lines) {
      const op = line[0];
      if (op === '+' || op === ' ') {
        const content = line.endsWith('\n') ? line.slice(1, -1) : line.slice(1);
        lines.push(content);
      }
    }
  }
  return lines.join('\n');
}

/** 被谁调用：编辑活动。做什么：StructuredPatch 可视化。 */
export function DiffViewer({
  patch,
  sourcePath,
  stats,
  fileName,
  headerExtra,
}: {
  patch: StructuredPatch;
  sourcePath?: string;
  stats?: { add: number; del: number };
  fileName?: string;
  headerExtra?: React.ReactNode;
}) {
  const { t } = useI18n();
  const rows = useMemo(() => patchToRows(patch), [patch]);
  const ext = extOf(sourcePath || fileName);
  const [foldOpen, setFoldOpen] = useState<Record<number, boolean>>({});
  const [copied, setCopied] = useState(false);
  const displayName =
    fileName ||
    (sourcePath ? sourcePath.split(/[/\\]/).pop() : '') ||
    patch.newFileName?.replace(/^[ab]\//, '') ||
    'file';

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(patchNewText(patch));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  };

  return (
    <div className="xy-cx-diff-card">
      <div className="xy-cx-diff-card-head">
        <div className="xy-cx-diff-card-head-left">
          <span className="xy-cx-diff-fname">{displayName}</span>
          {stats && (stats.add > 0 || stats.del > 0) ? (
            <span className="xy-cx-diff-stats">
              {stats.add > 0 ? <span className="is-add">+{stats.add}</span> : null}
              <span className="is-del">−{stats.del}</span>
            </span>
          ) : null}
        </div>
        <div className="xy-cx-diff-card-head-right">
          {headerExtra}
          <button
            type="button"
            className="xy-cx-diff-copy"
            onClick={() => void onCopy()}
            aria-label={t('timeline.copyDiff', '复制')}
            title={t('timeline.copyDiff', '复制')}
          >
            {copied ? <CheckOutlined /> : <CopyOutlined />}
          </button>
        </div>
      </div>
      <div className="xy-cx-diff">
        <div className="xy-cx-diff-body">
          {rows.map((row, i) => {
            if (row.kind === 'sep') {
              return <div key={`s-${i}`} className="xy-cx-diff-sep" />;
            }
            if (row.kind === 'fold') {
              const open = !!foldOpen[i];
              return (
                <React.Fragment key={`f-${i}`}>
                  <button
                    type="button"
                    className="xy-cx-diff-fold"
                    onClick={() => setFoldOpen((s) => ({ ...s, [i]: !open }))}
                  >
                    {open ? '▾' : '▸'} {t('timeline.hiddenLinesCount', undefined, row.hidden.length)}
                  </button>
                  {open
                    ? row.hidden.map((ln, j) => (
                        <DiffLineCells key={`fh-${i}-${j}`} row={ln} ext={ext} />
                      ))
                    : null}
                </React.Fragment>
              );
            }
            return <DiffLineCells key={`r-${i}`} row={row} ext={ext} />;
          })}
        </div>
      </div>
    </div>
  );
}
