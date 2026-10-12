/**
 * 会话回退浮层 — 双击 Esc 或 /rewind 选择一轮，
 * 预览要还原的文件，选择回退模式，可选编辑后重发。
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';

import { Button, Input, Spin, Tag, Typography } from 'antd';
import {
  ArrowLeftOutlined,
  CodeOutlined,
  CommentOutlined,
  HistoryOutlined,
  SwapOutlined,
} from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import type { RewindMode, RewindTargetPreview } from '../../protocol';
import { useChatStore } from '../../stores/chatStore';

const { Text } = Typography;

type Step = 'pick' | 'confirm';

export interface RewindOverlayProps {
  open: boolean;
  sessionId: string;
  initialTurnId?: string;
  onClose: () => void;
}

/** 被谁调用：ChatView 回滚流程。做什么：选 checkpoint/模式后发 rewind。 */
export default function RewindOverlay({
  open,
  sessionId,
  initialTurnId,
  onClose,
}: RewindOverlayProps) {
  const { send, addHandler } = useWs();
  const { t } = useI18n();
  const [step, setStep] = useState<Step>('pick');
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [targets, setTargets] = useState<RewindTargetPreview[]>([]);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [mode, setMode] = useState<RewindMode>('both');
  const [editContent, setEditContent] = useState('');
  const listRef = useRef<HTMLDivElement>(null);

  const fetchPreview = useCallback(() => {
    setLoading(true);
    const timeout = window.setTimeout(() => setLoading(false), 12_000);
    const unsub = addHandler('session.rewind.preview', (payload) => {
      const data = payload as { sessionId: string; targets: RewindTargetPreview[] };
      if (data.sessionId !== sessionId) return;
      window.clearTimeout(timeout);
      unsub();
      setTargets(data.targets || []);
      setLoading(false);
      if (initialTurnId) {
        const idx = (data.targets || []).findIndex((item) => item.turnId === initialTurnId);
        if (idx >= 0) {
          setSelectedIdx(idx);
          setEditContent(data.targets[idx].userMessagePreview);
          setStep('confirm');
        }
      }
    });
    send('session.rewind.preview', { sessionId });
  }, [addHandler, send, sessionId, initialTurnId]);

  useEffect(() => {
    if (!open) return;
    setStep(initialTurnId ? 'confirm' : 'pick');
    setMode('both');
    setSelectedIdx(0);
    setEditContent('');
    fetchPreview();
  }, [open, sessionId, initialTurnId, fetchPreview]);

  useEffect(() => {
    if (!open || step !== 'pick') return;
    const items = listRef.current?.querySelectorAll('.xy-rewind-item');
    items?.[selectedIdx]?.scrollIntoView({ block: 'nearest' });
  }, [open, step, selectedIdx]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        if (step === 'confirm' && !initialTurnId) {
          setStep('pick');
          return;
        }
        onClose();
        return;
      }
      if (step === 'pick' && targets.length > 0) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          setSelectedIdx((prev) => Math.min(prev + 1, targets.length - 1));
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          setSelectedIdx((prev) => Math.max(prev - 1, 0));
        } else if (e.key === 'Enter') {
          e.preventDefault();
          const target = targets[selectedIdx];
          if (target) {
            setEditContent(target.userMessagePreview);
            setStep('confirm');
          }
        }
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, step, targets, selectedIdx, onClose, initialTurnId]);

  const selected = targets[selectedIdx];

  const handleConfirm = () => {
    if (!selected) return;
    setSubmitting(true);
    if (mode !== 'workspace') {
      useChatStore.getState().prepareInlineRewind(sessionId, selected.turnId);
    }
    const unsubRewound = addHandler('session.rewound', (payload) => {
      const data = payload as { sessionId: string };
      if (data.sessionId !== sessionId) return;
      unsubRewound();
      unsubHistory();
      setSubmitting(false);
      onClose();
    });
    const unsubHistory = addHandler('session.history', (payload) => {
      const data = payload as { sessionId: string };
      if (data.sessionId !== sessionId) return;
      unsubRewound();
      unsubHistory();
      setSubmitting(false);
      onClose();
    });
    const trimmed = editContent.trim();
    send('session.rewind', {
      sessionId,
      turnId: selected.turnId,
      mode,
      ...(trimmed && mode !== 'workspace' ? { newContent: trimmed } : {}),
    });
    window.setTimeout(() => {
      setSubmitting(false);
      onClose();
    }, 15_000);
  };

  if (!open) return null;

  return (
    <div className="xy-rewind-overlay" role="dialog" aria-modal="true" aria-label={t('rewind.title')}>
      <div className="xy-rewind-backdrop" onClick={onClose} />
      <div className="xy-rewind-panel">
        <div className="xy-rewind-header">
          {step === 'confirm' && !initialTurnId ? (
            <Button
              type="text"
              size="small"
              icon={<ArrowLeftOutlined />}
              onClick={() => setStep('pick')}
            />
          ) : (
            <HistoryOutlined className="xy-rewind-header-icon" />
          )}
          <div>
            <div className="xy-rewind-title">{t('rewind.title')}</div>
            <div className="xy-rewind-subtitle">
              {step === 'pick' ? t('rewind.pickHint') : t('rewind.confirmHint')}
            </div>
          </div>
        </div>

        {loading ? (
          <div className="xy-rewind-loading">
            <Spin />
            <Text type="secondary">{t('rewind.loading')}</Text>
          </div>
        ) : targets.length === 0 ? (
          <div className="xy-rewind-empty">{t('rewind.empty')}</div>
        ) : step === 'pick' ? (
          <div className="xy-rewind-list" ref={listRef}>
            {targets.map((target, idx) => (
              <button
                key={target.turnId}
                type="button"
                className={`xy-rewind-item${idx === selectedIdx ? ' is-active' : ''}`}
                onClick={() => {
                  setSelectedIdx(idx);
                  setEditContent(target.userMessagePreview);
                  setStep('confirm');
                }}
                onMouseEnter={() => setSelectedIdx(idx)}
              >
                <div className="xy-rewind-item-preview">{target.userMessagePreview || target.turnId}</div>
                <div className="xy-rewind-item-meta">
                  {target.hasWorkspaceChanges && (
                    <Tag icon={<CodeOutlined />} color="orange">
                      {target.filesToRevert.length} {t('rewind.files')}
                    </Tag>
                  )}
                  <Text type="secondary" className="xy-rewind-item-time">
                    {new Date(target.createdAt).toLocaleString()}
                  </Text>
                </div>
              </button>
            ))}
          </div>
        ) : (
          <div className="xy-rewind-confirm">
            {selected && selected.filesToRevert.length > 0 && (
              <div className="xy-rewind-files">
                <div className="xy-rewind-files-title">{t('rewind.filesToRevert')}</div>
                <ul>
                  {selected.filesToRevert.slice(0, 12).map((file) => (
                    <li key={file}>{file}</li>
                  ))}
                  {selected.filesToRevert.length > 12 && (
                    <li className="xy-rewind-files-more">
                      +{selected.filesToRevert.length - 12} {t('rewind.moreFiles')}
                    </li>
                  )}
                </ul>
              </div>
            )}

            <div className="xy-rewind-modes">
              <ModeButton
                active={mode === 'both'}
                icon={<SwapOutlined />}
                label={t('rewind.modeBoth')}
                hint={t('rewind.modeBothHint')}
                onClick={() => setMode('both')}
              />
              <ModeButton
                active={mode === 'conversation'}
                icon={<CommentOutlined />}
                label={t('rewind.modeConversation')}
                hint={t('rewind.modeConversationHint')}
                onClick={() => setMode('conversation')}
              />
              <ModeButton
                active={mode === 'workspace'}
                icon={<CodeOutlined />}
                label={t('rewind.modeWorkspace')}
                hint={t('rewind.modeWorkspaceHint')}
                onClick={() => setMode('workspace')}
              />
            </div>

            {mode !== 'workspace' && (
              <div className="xy-rewind-edit">
                <div className="xy-rewind-edit-label">{t('rewind.editMessage')}</div>
                <Input.TextArea
                  autoSize={{ minRows: 2, maxRows: 6 }}
                  value={editContent}
                  onChange={(e) => setEditContent(e.target.value)}
                  placeholder={t('rewind.editPlaceholder')}
                />
              </div>
            )}

            <div className="xy-rewind-actions">
              <Button onClick={onClose}>{t('common.cancel')}</Button>
              <Button type="primary" loading={submitting} onClick={handleConfirm}>
                {mode === 'workspace' ? t('rewind.revertCode') : t('rewind.rewindAndResend')}
              </Button>
            </div>
          </div>
        )}

        {step === 'pick' && targets.length > 0 && (
          <div className="xy-rewind-footer-hint">
            <kbd>↑</kbd><kbd>↓</kbd> {t('rewind.navigate')} · <kbd>Enter</kbd> {t('rewind.select')} · <kbd>Esc</kbd> {t('rewind.close')}
          </div>
        )}
      </div>
    </div>
  );
}

/** 被谁调用：RewindOverlay。做什么：回滚模式切换按钮。 */
function ModeButton({
  active,
  icon,
  label,
  hint,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`xy-rewind-mode${active ? ' is-active' : ''}`} onClick={onClick}>
      <span className="xy-rewind-mode-icon">{icon}</span>
      <span className="xy-rewind-mode-label">{label}</span>
      <span className="xy-rewind-mode-hint">{hint}</span>
    </button>
  );
}
