/**
 * GoalBar — 贴在输入区上方的主目标条（控制条在 composer 侧，不占对话顶）。
 */

import React, { useCallback, useEffect, useState } from 'react';
import { Button, Input, Modal, Tooltip, message } from 'antd';
import {
  DeleteOutlined,
  EditOutlined,
  PauseCircleOutlined,
  PlayCircleOutlined,
} from '@ant-design/icons';
import { useWs } from '../../App';
import { useI18n } from '../../i18n';

export interface SessionGoalSnapshot {
  id: string;
  objective: string;
  status: 'active' | 'paused' | 'budget_limited';
  successCriteria: string[];
  tokenBudget?: number;
  tokensUsed: number;
  startedAt: number;
  updatedAt: number;
  continuationCount: number;
  maxContinuations: number;
  recentLedger: Array<{ at: number; kind: string; summary: string }>;
}

interface GoalBarProps {
  sessionId: string;
}

/** 被谁调用：ChatView composer 上方。做什么：订阅 goal.updated；暂停/继续/清空/改目标经 WS goal.*。 */
export default function GoalBar({ sessionId }: GoalBarProps) {
  const { send, addHandler } = useWs();
  const { t } = useI18n();
  const [goal, setGoal] = useState<SessionGoalSnapshot | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);

  const applyPayload = useCallback(
    (payload: unknown) => {
      const p = payload as { sessionId?: string; goal?: SessionGoalSnapshot | null };
      if (!p || p.sessionId !== sessionId) return;
      setGoal(p.goal ?? null);
      if (!p.goal) setEditing(false);
    },
    [sessionId],
  );

  useEffect(() => {
    setGoal(null);
    setEditing(false);
    send('goal.get', { sessionId });
    return addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type === 'goal.updated') applyPayload(msg.payload);
    });
  }, [sessionId, send, addHandler, applyPayload]);

  const runAction = useCallback(
    (action: 'pause' | 'resume' | 'clear' | 'edit', options?: { objective?: string }) => {
      if (busy) return;
      setBusy(true);
      send('goal.action', { sessionId, action, ...options });
      window.setTimeout(() => setBusy(false), 400);
    },
    [busy, send, sessionId],
  );

  const handlePauseResume = () => {
    runAction(goal?.status === 'paused' ? 'resume' : 'pause');
  };

  const openEdit = () => {
    if (!goal) return;
    setDraft(goal.objective);
    setEditing(true);
  };

  const saveEdit = () => {
    const next = draft.trim();
    if (!next) {
      message.warning(t('goal.editEmpty', '主目标内容不能为空'));
      return;
    }
    setEditing(false);
    runAction('edit', { objective: next });
  };

  const handleClear = () => {
    Modal.confirm({
      title: t('goal.clearConfirmTitle', '清除主目标？'),
      content: t('goal.clearConfirmBody', '清除后将停止自动续跑；步骤清单会一并清空。可用 /goal 重新设定。'),
      okText: t('goal.clear', '清除'),
      okButtonProps: { danger: true },
      cancelText: t('common.cancel', '取消'),
      onOk: () => runAction('clear'),
    });
  };

  if (!goal) return null;

  const paused = goal.status === 'paused';
  const budgetLimited = goal.status === 'budget_limited';
  const budget =
    typeof goal.tokenBudget === 'number' && goal.tokenBudget > 0
      ? `${formatTokens(goal.tokensUsed ?? 0)} / ${formatTokens(goal.tokenBudget)}`
      : null;
  const continuation = `${goal.continuationCount}/${goal.maxContinuations}`;
  const latestLedger = goal.recentLedger.length
    ? goal.recentLedger[goal.recentLedger.length - 1]
    : undefined;

  return (
    <div className={`xy-goal-bar${paused ? ' is-paused' : ''}`} role="status">
      <div className="xy-goal-bar-main">
        <span className="xy-goal-bar-label">{t('goal.label', '主目标')}</span>
        <span className={`xy-goal-bar-status${paused || budgetLimited ? ' is-paused' : ''}`}>
          {budgetLimited
            ? t('goal.statusBudgetLimited', '预算已用尽')
            : paused
              ? t('goal.statusPaused', '已暂停')
              : t('goal.statusActive', '进行中')}
        </span>
        <span className="xy-goal-bar-objective" title={goal.objective}>
          {goal.objective}
        </span>
        {budget ? <span className="xy-goal-bar-budget">{budget}</span> : null}
        {continuation ? (
          <span className="xy-goal-bar-budget">
            {t('goal.continuation', '续跑')} {continuation}
          </span>
        ) : null}
        {latestLedger ? (
          <span className="xy-goal-bar-ledger" title={latestLedger.summary}>
            {latestLedger.kind}: {latestLedger.summary}
          </span>
        ) : null}
      </div>
      <div className="xy-goal-bar-actions">
        <Tooltip title={paused ? t('goal.resume', '继续') : t('goal.pause', '暂停')}>
          <Button
            type="text"
            size="small"
            className="xy-goal-bar-btn"
            icon={paused ? <PlayCircleOutlined /> : <PauseCircleOutlined />}
            disabled={busy}
            onClick={handlePauseResume}
            aria-label={paused ? t('goal.resume', '继续') : t('goal.pause', '暂停')}
          />
        </Tooltip>
        <Tooltip title={t('goal.edit', '编辑')}>
          <Button
            type="text"
            size="small"
            className="xy-goal-bar-btn"
            icon={<EditOutlined />}
            disabled={busy}
            onClick={openEdit}
            aria-label={t('goal.edit', '编辑')}
          />
        </Tooltip>
        <Tooltip title={t('goal.clear', '清除')}>
          <Button
            type="text"
            size="small"
            className="xy-goal-bar-btn is-danger"
            icon={<DeleteOutlined />}
            disabled={busy}
            onClick={handleClear}
            aria-label={t('goal.clear', '清除')}
          />
        </Tooltip>
      </div>

      <Modal
        title={t('goal.editTitle', '编辑主目标')}
        open={editing}
        onCancel={() => setEditing(false)}
        onOk={saveEdit}
        okText={t('goal.save', '保存')}
        cancelText={t('common.cancel', '取消')}
        destroyOnClose
      >
        <Input.TextArea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          autoSize={{ minRows: 3, maxRows: 8 }}
          placeholder={t('goal.editPlaceholder', '描述要持续推进的主目标…')}
          autoFocus
        />
      </Modal>
    </div>
  );
}

/** 被谁调用：GoalBar/Stats。做什么：token 数本地化格式。 */
function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}
