/**
 * 「搜索聊天」面板：会话列表 + 快捷操作。
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  FileSearchOutlined,
  FolderOpenOutlined,
  FormOutlined,
  MessageOutlined,
} from '@ant-design/icons';
import { Modal, message } from 'antd';
import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import type { SessionInfo } from '../../protocol';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import {
  hasListedConversation,
  projectGroupLabel,
  sessionProjectDir,
} from '../chat/sessionSidebarUtils';

export interface ChatSearchPaletteProps {
  open: boolean;
  onClose: () => void;
  onNavigateChat: () => void;
  onNewChat: () => void;
  onOpenFolder: () => void;
}

type PaletteItem =
  | { kind: 'session'; session: SessionInfo; shortcut?: string }
  | { kind: 'action'; id: 'new-chat' | 'open-folder' | 'search-files'; shortcut: string };

function sessionSubtitle(session: SessionInfo): string {
  const dir = sessionProjectDir(session);
  if (dir) return projectGroupLabel(dir) || dir;
  return '';
}

/** 被谁调用：AppSidebar 搜索按钮。做什么：居中搜索会话并提供快捷操作。 */
export default function ChatSearchPalette({
  open,
  onClose,
  onNavigateChat,
  onNewChat,
  onOpenFolder,
}: ChatSearchPaletteProps) {
  const { t } = useI18n();
  const { send } = useWs();
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const initSession = useChatStore((s) => s.initSession);
  const setActiveSession = useChatStore((s) => s.setActiveSession);

  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) {
      setQuery('');
      setActiveIndex(0);
      return;
    }
    const id = window.setTimeout(() => {
      inputRef.current?.focus({ preventScroll: true });
    }, 0);
    return () => window.clearTimeout(id);
  }, [open]);

  const listedSessions = useMemo(() => {
    const q = query.trim().toLowerCase();
    const base = sessions
      .filter((s) => !s.archivedAt && hasListedConversation(s))
      .sort((a, b) => {
        const at = a.lastMessageAt || a.updatedAt || a.createdAt || 0;
        const bt = b.lastMessageAt || b.updatedAt || b.createdAt || 0;
        return bt - at;
      });
    if (!q) return base;
    return base.filter((s) => {
      const name = (s.name || '').toLowerCase();
      const sub = sessionSubtitle(s).toLowerCase();
      const dir = (sessionProjectDir(s) || '').toLowerCase();
      return name.includes(q) || sub.includes(q) || dir.includes(q);
    });
  }, [query, sessions]);

  const sessionItems: PaletteItem[] = useMemo(
    () =>
      listedSessions.slice(0, 12).map((session, index) => ({
        kind: 'session' as const,
        session,
        shortcut: index < 9 ? `Ctrl+${index + 1}` : undefined,
      })),
    [listedSessions],
  );

  const actionItems: PaletteItem[] = useMemo(
    () => [
      { kind: 'action', id: 'new-chat', shortcut: 'Ctrl+N' },
      { kind: 'action', id: 'open-folder', shortcut: 'Ctrl+O' },
      { kind: 'action', id: 'search-files', shortcut: 'Ctrl+P' },
    ],
    [],
  );

  const flatItems = useMemo(() => [...sessionItems, ...actionItems], [actionItems, sessionItems]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query, open]);

  useEffect(() => {
    if (activeIndex >= flatItems.length) {
      setActiveIndex(Math.max(0, flatItems.length - 1));
    }
  }, [activeIndex, flatItems.length]);

  const selectSession = useCallback(
    (session: SessionInfo) => {
      onNavigateChat();
      useChatStore.getState().setComposeDraft(null);
      initSession(session.id);
      setActiveSession(session.id);
      const dir = sessionProjectDir(session);
      if (dir) {
        send('workspace.open', { path: dir, sessionId: session.id });
      }
      send('session.restore', { sessionId: session.id });
      send('team.status', { sessionId: session.id });
      send('context.status', { sessionId: session.id });
      onClose();
    },
    [initSession, onClose, onNavigateChat, send, setActiveSession],
  );

  const runAction = useCallback(
    (id: 'new-chat' | 'open-folder' | 'search-files') => {
      if (id === 'new-chat') {
        onNavigateChat();
        onNewChat();
        onClose();
        return;
      }
      if (id === 'open-folder') {
        onNavigateChat();
        onOpenFolder();
        onClose();
        return;
      }
      // 文件搜索暂未接入主工作台
      message.info(t('nav.searchFilesSoon', '文件搜索即将支持'));
    },
    [onClose, onNavigateChat, onNewChat, onOpenFolder, t],
  );

  const activateItem = useCallback(
    (item: PaletteItem) => {
      if (item.kind === 'session') selectSession(item.session);
      else runAction(item.id);
    },
    [runAction, selectSession],
  );

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, Math.max(0, flatItems.length - 1)));
        return;
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (event.key === 'Enter') {
        event.preventDefault();
        const item = flatItems[activeIndex];
        if (item) activateItem(item);
        return;
      }
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      const key = event.key.toLowerCase();
      if (key === 'n') {
        event.preventDefault();
        runAction('new-chat');
        return;
      }
      if (key === 'o') {
        event.preventDefault();
        runAction('open-folder');
        return;
      }
      if (key === 'p') {
        event.preventDefault();
        runAction('search-files');
        return;
      }
      if (/^[1-9]$/.test(key)) {
        const idx = Number(key) - 1;
        const item = sessionItems[idx];
        if (item) {
          event.preventDefault();
          activateItem(item);
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [activateItem, activeIndex, flatItems, onClose, open, runAction, sessionItems]);

  const actionLabel = (id: 'new-chat' | 'open-folder' | 'search-files') => {
    if (id === 'new-chat') return t('sessionSidebar.newChat');
    if (id === 'open-folder') return t('chat.projectPicker.useFolder', '打开文件夹');
    return t('nav.searchFiles', '搜索文件');
  };

  const actionIcon = (id: 'new-chat' | 'open-folder' | 'search-files') => {
    if (id === 'new-chat') return <FormOutlined />;
    if (id === 'open-folder') return <FolderOpenOutlined />;
    return <FileSearchOutlined />;
  };

  return (
    <Modal
      open={open}
      onCancel={onClose}
      footer={null}
      closable={false}
      destroyOnClose
      centered
      width={520}
      className="xy-chat-search-modal"
      rootClassName="xy-chat-search-modal-root"
      styles={{
        body: { padding: 0 },
        mask: { background: 'rgba(0, 0, 0, 0.1)' },
        content: {
          padding: 0,
          overflow: 'hidden',
        },
      }}
    >
      <div className="xy-chat-search" role="dialog" aria-label={t('nav.searchChats', '搜索聊天')}>
        <div className="xy-chat-search-head">
          <div className="xy-chat-search-field">
            <input
              ref={inputRef}
              type="text"
              className="xy-chat-search-input"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('nav.searchChatsPlaceholder', '搜索聊天')}
              aria-label={t('nav.searchChats', '搜索聊天')}
              spellCheck={false}
              autoComplete="off"
            />
          </div>
        </div>

        <div className="xy-chat-search-body">
          <section className="xy-chat-search-section">
            <div className="xy-chat-search-section-title">{t('nav.searchChatsSection', '聊天')}</div>
            {sessionItems.length === 0 ? (
              <div className="xy-chat-search-empty">{t('sessionSidebar.empty')}</div>
            ) : (
              sessionItems.map((item, index) => {
                if (item.kind !== 'session') return null;
                const { session, shortcut } = item;
                const meta = sessionSubtitle(session);
                const selected = index === activeIndex;
                const current = session.id === activeSessionId;
                return (
                  <button
                    key={session.id}
                    type="button"
                    className={`xy-chat-search-row${selected ? ' is-active' : ''}${current ? ' is-current' : ''}`}
                    onMouseEnter={() => setActiveIndex(index)}
                    onClick={() => activateItem(item)}
                  >
                    <MessageOutlined className="xy-chat-search-row-ico" aria-hidden />
                    <span className="xy-chat-search-row-title">
                      {session.name || t('sessionSidebar.newChat')}
                    </span>
                    {meta ? <span className="xy-chat-search-row-meta">{meta}</span> : <span className="xy-chat-search-row-meta" />}
                    {shortcut ? <kbd className="xy-chat-search-kbd">{shortcut}</kbd> : null}
                  </button>
                );
              })
            )}
          </section>

          <section className="xy-chat-search-section">
            <div className="xy-chat-search-section-title">{t('nav.quickActions', '快捷操作')}</div>
            {actionItems.map((item, offset) => {
              if (item.kind !== 'action') return null;
              const index = sessionItems.length + offset;
              const selected = index === activeIndex;
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`xy-chat-search-row${selected ? ' is-active' : ''}`}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => activateItem(item)}
                >
                  <span className="xy-chat-search-row-ico" aria-hidden>
                    {actionIcon(item.id)}
                  </span>
                  <span className="xy-chat-search-row-title">{actionLabel(item.id)}</span>
                  <span className="xy-chat-search-row-meta" />
                  <kbd className="xy-chat-search-kbd">{item.shortcut}</kbd>
                </button>
              );
            })}
          </section>
        </div>
      </div>
    </Modal>
  );
}
