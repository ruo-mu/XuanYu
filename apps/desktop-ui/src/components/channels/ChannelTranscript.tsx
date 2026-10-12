/**
 * 频道会话记录：按角色渲染来回气泡，并展示输入中 / 流式草稿。
 */

import React, { useEffect, useRef } from 'react';
import { useI18n, useLocaleText } from '../../i18n';

/** 频道时间线里的一条消息。 */
export interface ChannelChatMessage {
  role: string;
  content: string;
  timestamp?: number;
  turnId?: string;
}

/** 去掉 [Channel:…] / [User:…] 前缀再显示气泡。 */
function displayContent(content: string): string {
  return content
    .replace(/^\[Channel:[^\]]*\]\s*/i, '')
    .replace(/^\[User:[^\]]*\]\s*/i, '')
    .trim();
}

/** 一来一回的会话气泡，不是投递日志。 */
export function ChannelTranscript({
  messages,
  loading,
  emptyHint,
  typing,
  streamingText,
}: {
  messages: ChannelChatMessage[];
  loading?: boolean;
  emptyHint?: string;
  /** 机器人正在处理 / 输入中 */
  typing?: boolean;
  /** 流式草稿（可选） */
  streamingText?: string;
}) {
  const { locale } = useI18n();
  const lt = useLocaleText();
  const endRef = useRef<HTMLDivElement>(null);
  const draft = (streamingText || '').trim();
  const showTyping = Boolean(typing || draft);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [messages.length, loading, showTyping, draft]);

  if (loading) {
    return (
      <div className="xy-ch-transcript is-empty">
        {lt('正在拉取会话…', 'Loading conversation…')}
      </div>
    );
  }

  if (messages.length === 0 && !showTyping) {
    return (
      <div className="xy-ch-transcript is-empty">
        {emptyHint || lt('还没有聊天记录', 'No chat history yet')}
      </div>
    );
  }

  return (
    <div className="xy-ch-transcript">
      <div className="xy-ch-transcript-inner">
        {messages.map((msg, index) => {
          const role = msg.role === 'assistant' ? 'assistant' : 'user';
          const text = displayContent(msg.content);
          if (!text) return null;
          return (
            <div key={`${role}-${index}-${msg.timestamp || 0}`} className={`xy-ch-msg role-${role}`}>
              <div className="xy-ch-msg-meta">
                <span>{role === 'user' ? lt('对方', 'Peer') : lt('玄语', 'XuanYu')}</span>
                {msg.timestamp ? (
                  <time dateTime={new Date(msg.timestamp).toISOString()}>
                    {new Date(msg.timestamp).toLocaleString(locale, {
                      year: 'numeric',
                      month: 'numeric',
                      day: 'numeric',
                      hour: '2-digit',
                      minute: '2-digit',
                      second: '2-digit',
                      hour12: false,
                    })}
                  </time>
                ) : null}
              </div>
              <div className="xy-ch-msg-body">{text}</div>
            </div>
          );
        })}
        {showTyping ? (
          <div className="xy-ch-msg role-assistant is-typing">
            <div className="xy-ch-msg-meta">
              <span>{lt('玄语', 'XuanYu')}</span>
              <em>{lt('输入中', 'Typing')}</em>
            </div>
            {draft ? (
              <div className="xy-ch-msg-body is-draft">{displayContent(draft)}</div>
            ) : (
              <div className="xy-ch-typing" aria-label={lt('正在输入', 'Typing')}>
                <i />
                <i />
                <i />
              </div>
            )}
          </div>
        ) : null}
        <div ref={endRef} />
      </div>
    </div>
  );
}
