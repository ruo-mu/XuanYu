/**
 * 聊天附件卡片：展示文件名、类型、大小，可选移除。
 */

import React from 'react';

import { CloseOutlined, CodeOutlined, FileTextOutlined } from '@ant-design/icons';

/** 已选或已发送附件的展示信息。 */
export interface FileAttachmentInfo {
  name: string;
  mimeType: string;
  sizeBytes: number;
}

const CODE_EXTENSIONS = new Set([
  'md', 'markdown', 'txt', 'json', 'jsonl', 'yaml', 'yml', 'xml', 'csv', 'tsv',
  'html', 'htm', 'css', 'js', 'mjs', 'cjs', 'ts', 'tsx', 'jsx', 'py', 'java', 'go',
  'rs', 'c', 'cpp', 'h', 'hpp', 'cs', 'rb', 'php', 'sql', 'sh', 'bash', 'toml',
]);

/** 把字节数格式化为 B / KB / MB。 */
export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0B';
  if (bytes < 1024) return `${bytes}B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(2) : kb.toFixed(1)}KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(2) : mb.toFixed(1)}MB`;
}

/** 用扩展名或 MIME 生成短类型标签（MD / JSON / FILE）。 */
export function formatFileTypeLabel(name: string, mimeType: string): string {
  const ext = name.includes('.') ? name.split('.').pop()?.toLowerCase() : '';
  if (ext) return ext.toUpperCase();
  const mime = mimeType.toLowerCase();
  if (mime.includes('markdown')) return 'MD';
  if (mime.includes('json')) return 'JSON';
  if (mime.includes('pdf')) return 'PDF';
  if (mime.startsWith('text/')) return 'TXT';
  return 'FILE';
}

/** 被谁调用：附件预览。做什么：是否按代码样式展示。 */
function isCodeLikeFile(name: string, mimeType: string): boolean {
  const ext = name.includes('.') ? name.split('.').pop()?.toLowerCase() : '';
  if (ext && CODE_EXTENSIONS.has(ext)) return true;
  const mime = mimeType.toLowerCase();
  return mime.startsWith('text/') || mime.includes('json') || mime.includes('markdown');
}

interface FileAttachmentCardProps extends FileAttachmentInfo {
  className?: string;
  onRemove?: () => void;
  removeLabel?: string;
}

/** 输入框或消息里的单个附件预览卡片。 */
export function FileAttachmentCard({
  name,
  mimeType,
  sizeBytes,
  className,
  onRemove,
  removeLabel,
}: FileAttachmentCardProps) {
  const typeLabel = formatFileTypeLabel(name, mimeType);
  const sizeLabel = formatFileSize(sizeBytes);
  const showCodeIcon = isCodeLikeFile(name, mimeType);

  return (
    <div className={['xy-file-attachment-card', className].filter(Boolean).join(' ')}>
      <div className="xy-file-attachment-icon" aria-hidden="true">
        {showCodeIcon ? <CodeOutlined /> : <FileTextOutlined />}
      </div>
      <div className="xy-file-attachment-info">
        <div className="xy-file-attachment-name" title={name}>{name}</div>
        <div className="xy-file-attachment-meta">{typeLabel} | {sizeLabel}</div>
      </div>
      {onRemove && (
        <button
          type="button"
          className="xy-attachment-remove-btn xy-file-attachment-remove-btn"
          aria-label={removeLabel}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <CloseOutlined />
        </button>
      )}
    </div>
  );
}
