/**
 * QuestionDialog — ask_user 结构化问答卡片（聊天流内嵌）。
 */

import React, { useMemo, useState } from 'react';
import { Button, Input, Space, Tag, Typography } from 'antd';
import { QuestionCircleOutlined } from '@ant-design/icons';
import type { PendingQuestion } from '../../stores/chatStore';
import { useI18n } from '../../i18n';

const { Text } = Typography;

interface Props {
  pending: PendingQuestion;
  onRespond: (answers: Array<{ id: string; selected?: string[]; text?: string }>) => void;
}

/** 被谁调用：ChatView 挂 ask_user。做什么：结构化问答，respond 经 WS。 */
export default function QuestionDialog({ pending, onRespond }: Props) {
  const { t } = useI18n();
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [texts, setTexts] = useState<Record<string, string>>({});

  const canSubmit = useMemo(() => {
    return pending.questions.every((q) => {
      const picks = selected[q.id] || [];
      const text = (texts[q.id] || '').trim();
      if (q.options?.length) return picks.length > 0 || text.length > 0;
      return text.length > 0;
    });
  }, [pending.questions, selected, texts]);

  const toggleOption = (questionId: string, option: string, allowMultiple?: boolean) => {
    setSelected((prev) => {
      const current = prev[questionId] || [];
      if (allowMultiple) {
        const next = current.includes(option)
          ? current.filter((item) => item !== option)
          : [...current, option];
        return { ...prev, [questionId]: next };
      }
      return { ...prev, [questionId]: [option] };
    });
  };

  const submit = () => {
    onRespond(
      pending.questions.map((q) => ({
        id: q.id,
        selected: selected[q.id]?.length ? selected[q.id] : undefined,
        text: (texts[q.id] || '').trim() || undefined,
      })),
    );
  };

  return (
    <div className="xy-permission-card">
      <div className="xy-permission-head">
        <div className="xy-permission-icon">
          <QuestionCircleOutlined />
        </div>
        <div className="xy-permission-title">
          <Text strong>{t('question.requestTitle')}</Text>
          <Text type="secondary">{t('question.requestSubtitle')}</Text>
        </div>
        <Tag color="blue">ask_user</Tag>
      </div>

      <Space direction="vertical" size="middle" style={{ width: '100%' }}>
        {pending.questions.map((q) => (
          <div key={q.id}>
            <Text strong>{q.prompt}</Text>
            {q.options?.length ? (
              <Space wrap style={{ marginTop: 8 }}>
                {q.options.map((option) => {
                  const active = (selected[q.id] || []).includes(option);
                  return (
                    <Button
                      key={option}
                      type={active ? 'primary' : 'default'}
                      size="small"
                      onClick={() => toggleOption(q.id, option, q.allow_multiple)}
                    >
                      {option}
                    </Button>
                  );
                })}
              </Space>
            ) : null}
            <Input.TextArea
              style={{ marginTop: 8 }}
              rows={2}
              placeholder={t('question.freeTextPlaceholder')}
              value={texts[q.id] || ''}
              onChange={(e) => setTexts((prev) => ({ ...prev, [q.id]: e.target.value }))}
            />
          </div>
        ))}
      </Space>

      <Space wrap className="xy-permission-actions" style={{ marginTop: 12 }}>
        <Button type="primary" disabled={!canSubmit} onClick={submit}>
          {t('question.submit')}
        </Button>
      </Space>
    </div>
  );
}
