/**
 * Markdown 编辑器 — 基于 @uiw/react-md-editor（工具栏 + 实时预览）。
 */

import React from 'react';
import MDEditor from '@uiw/react-md-editor';

import { useTheme } from '../../theme/ThemeContext';

import '@uiw/react-md-editor/markdown-editor.css';
import '@uiw/react-markdown-preview/markdown.css';

export interface MarkdownSplitEditorProps {
  value: string;
  onChange: (value: string) => void;
  /** 可选标签；编辑/预览切换由 MDEditor 自带。 */
  editLabel?: string;
  previewLabel?: string;
  placeholder?: string;
  height?: number | string;
}

/** 被谁调用：MemoryPanel profile。做什么：左右预览 Markdown 编辑。 */
export default function MarkdownSplitEditor({
  value,
  onChange,
  placeholder,
  height = '100%',
}: MarkdownSplitEditorProps) {
  const { resolved } = useTheme();
  const colorMode = resolved.mode === 'dark' ? 'dark' : 'light';

  return (
    <div className="xy-md-editor-shell" data-color-mode={colorMode}>
      <MDEditor
        value={value}
        onChange={(next) => onChange(next ?? '')}
        height={height}
        preview="live"
        visibleDragbar={false}
        highlightEnable
        textareaProps={{
          placeholder,
          spellCheck: false,
        }}
      />
    </div>
  );
}
