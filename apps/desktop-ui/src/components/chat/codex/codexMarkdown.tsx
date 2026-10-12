/**
 * Shared assistant markdown pipeline for chat timelines.
 * GFM on; components use CSS classes (see .xy-md-* / .xy-cx-prose).
 */

import React from 'react';
import type { Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';

export const remarkPlugins = [remarkGfm];

/**
 * Light fixes so CommonMark does not swallow structure models often emit.
 * Does not invent tables — only spacing/fence hygiene.
 */
export function normalizeAssistantMarkdown(md: string): string {
  let text = md.replace(/\r\n/g, '\n');
  // Bold-only line immediately followed by numbered list → need blank line
  text = text.replace(/(^|\n)(\*\*[^*\n]+\*\*)\s*\n(\d+\.\s)/gm, '$1$2\n\n$3');
  // Heading-like **Title** then pipe-table on next line
  text = text.replace(/(^|\n)(\*\*[^*\n]+\*\*)\s*\n(\|)/gm, '$1$2\n\n$3');
  return text;
}

/**
 * react-markdown 元素映射：标题/列表/表格走 `.xy-md-*`；
 * 链接新开标签；围栏代码用 Prism oneDark，行内代码走 `.xy-md-code-inline`。
 */
export const assistantMarkdownComponents: Components = {
  p({ children }) {
    return <p className="xy-md-p">{children}</p>;
  },
  h1({ children }) {
    return <h1 className="xy-md-h1">{children}</h1>;
  },
  h2({ children }) {
    return <h2 className="xy-md-h2">{children}</h2>;
  },
  h3({ children }) {
    return <h3 className="xy-md-h3">{children}</h3>;
  },
  h4({ children }) {
    return <h4 className="xy-md-h4">{children}</h4>;
  },
  ul({ children }) {
    return <ul className="xy-md-ul">{children}</ul>;
  },
  ol({ children }) {
    return <ol className="xy-md-ol">{children}</ol>;
  },
  li({ children }) {
    return <li className="xy-md-li">{children}</li>;
  },
  blockquote({ children }) {
    return <blockquote className="xy-md-quote">{children}</blockquote>;
  },
  hr() {
    return <hr className="xy-md-hr" />;
  },
  a({ href, children }) {
    return (
      <a className="xy-md-a" href={href} target="_blank" rel="noopener noreferrer">
        {children}
      </a>
    );
  },
  strong({ children }) {
    return <strong className="xy-md-strong">{children}</strong>;
  },
  em({ children }) {
    return <em className="xy-md-em">{children}</em>;
  },
  table({ children }) {
    return (
      <div className="xy-md-table-wrap">
        <table className="xy-md-table">{children}</table>
      </div>
    );
  },
  thead({ children }) {
    return <thead className="xy-md-thead">{children}</thead>;
  },
  tbody({ children }) {
    return <tbody>{children}</tbody>;
  },
  tr({ children }) {
    return <tr className="xy-md-tr">{children}</tr>;
  },
  th({ children, style }) {
    return (
      <th className="xy-md-th" style={style}>
        {children}
      </th>
    );
  },
  td({ children, style }) {
    return (
      <td className="xy-md-td" style={style}>
        {children}
      </td>
    );
  },
  pre({ children }) {
    return <div className="xy-md-pre-wrap">{children}</div>;
  },
  code({ className, children, ...props }) {
    const match = /language-(\w+)/.exec(className || '');
    const codeStr = String(children).replace(/\n$/, '');
    // Fenced block (multi-line or language class)
    const isBlock = Boolean(match) || codeStr.includes('\n');
    if (isBlock) {
      const lang = match?.[1] || 'text';
      return (
        <SyntaxHighlighter
          style={oneDark as Record<string, React.CSSProperties>}
          language={lang}
          PreTag="div"
          className="xy-md-code-block"
          customStyle={{
            margin: 0,
            borderRadius: 10,
            fontSize: 13,
            lineHeight: 1.55,
            fontFamily: 'var(--xy-mono)',
          }}
        >
          {codeStr}
        </SyntaxHighlighter>
      );
    }
    return (
      <code className="xy-md-code-inline" {...props}>
        {children}
      </code>
    );
  },
};
