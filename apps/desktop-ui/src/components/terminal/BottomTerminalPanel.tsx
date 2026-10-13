/**
 * 主区底部终端：多标签、拖拽高度；加号下拉可选 PowerShell / CMD。
 */

import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Plus, SquareTerminal, X } from 'lucide-react';
import { useI18n } from '../../i18n';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import {
  TERMINAL_PANEL_MAX,
  TERMINAL_PANEL_MIN,
  useTerminalStore,
  type TerminalShellKind,
} from '../../stores/terminalStore';
import TerminalSessionView from './TerminalSessionView';

function resolvePreferredCwd(): string | undefined {
  const chat = useChatStore.getState();
  const activeId = chat.activeSessionId;
  if (activeId) {
    const fromList = useSessionStore.getState().sessions.find((item) => item.id === activeId);
    if (fromList?.projectDir?.trim()) return fromList.projectDir.trim();
    const draft = chat.composeDraft;
    if (draft && 'projectDir' in draft && draft.projectDir?.trim()) {
      return draft.projectDir.trim();
    }
  }
  return undefined;
}

function defaultShell(): TerminalShellKind {
  return window.xuanyu?.platform === 'win32' ? 'powershell' : 'shell';
}

export default function BottomTerminalPanel() {
  const { t } = useI18n();
  const open = useTerminalStore((s) => s.open);
  const height = useTerminalStore((s) => s.height);
  const tabs = useTerminalStore((s) => s.tabs);
  const activeId = useTerminalStore((s) => s.activeId);
  const setOpen = useTerminalStore((s) => s.setOpen);
  const setHeight = useTerminalStore((s) => s.setHeight);
  const setActiveId = useTerminalStore((s) => s.setActiveId);
  const upsertTab = useTerminalStore((s) => s.upsertTab);
  const removeTab = useTerminalStore((s) => s.removeTab);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const addBtnRef = useRef<HTMLButtonElement>(null);
  const addMenuRef = useRef<HTMLDivElement>(null);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<{ left: number; top: number } | null>(null);
  const isWin = (window.xuanyu?.platform || navigator.platform).toLowerCase().includes('win');

  const createTab = useCallback(
    async (shell: TerminalShellKind = defaultShell()) => {
      if (!window.xuanyu?.terminalCreate) return;
      setAddMenuOpen(false);
      try {
        const created = await window.xuanyu.terminalCreate({
          cwd: resolvePreferredCwd(),
          cols: 80,
          rows: 14,
          shell,
        });
        upsertTab({
          id: created.id,
          title: created.title || t('terminal.tabFallback', '终端'),
          cwd: created.cwd,
          shell: created.shell || shell,
        });
      } catch (err) {
        window.alert(
          err instanceof Error
            ? err.message
            : t('terminal.createFailed', '无法启动终端'),
        );
      }
    },
    [t, upsertTab],
  );

  useEffect(() => {
    if (!open) return;
    if (tabs.length > 0) return;
    void createTab(defaultShell());
  }, [open, tabs.length, createTab]);

  useLayoutEffect(() => {
    if (!addMenuOpen || !addBtnRef.current) {
      setMenuPos(null);
      return;
    }
    const rect = addBtnRef.current.getBoundingClientRect();
    const menuWidth = 220;
    const maxLeft = Math.max(8, window.innerWidth - menuWidth - 8);
    setMenuPos({
      left: Math.min(maxLeft, Math.max(8, rect.left)),
      top: rect.bottom + 6,
    });
  }, [addMenuOpen]);

  useEffect(() => {
    if (!addMenuOpen) return;
    const onPointer = (event: MouseEvent) => {
      const target = event.target as Node;
      if (addBtnRef.current?.contains(target)) return;
      if (addMenuRef.current?.contains(target)) return;
      setAddMenuOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setAddMenuOpen(false);
    };
    const timer = window.setTimeout(() => {
      document.addEventListener('mousedown', onPointer);
    }, 0);
    document.addEventListener('keydown', onKey);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener('mousedown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [addMenuOpen]);

  const closeTab = useCallback(
    async (id: string) => {
      try {
        await window.xuanyu?.terminalDispose?.({ id });
      } catch {
        /* ignore */
      }
      removeTab(id);
    },
    [removeTab],
  );

  const closePanel = useCallback(() => {
    setAddMenuOpen(false);
    setOpen(false);
  }, [setOpen]);

  const onDragStart = (event: React.MouseEvent) => {
    event.preventDefault();
    dragRef.current = { startY: event.clientY, startHeight: height };
    const onMove = (ev: MouseEvent) => {
      const origin = dragRef.current;
      if (!origin) return;
      const next = origin.startHeight + (origin.startY - ev.clientY);
      setHeight(Math.min(TERMINAL_PANEL_MAX, Math.max(TERMINAL_PANEL_MIN, next)));
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  };

  if (!open) return null;

  const shellMenu =
    addMenuOpen && menuPos
      ? createPortal(
          <div
            ref={addMenuRef}
            className="xy-terminal-add-menu"
            role="menu"
            style={{ left: menuPos.left, top: menuPos.top }}
          >
            {isWin ? (
              <>
                <button
                  type="button"
                  className="xy-terminal-add-item"
                  role="menuitem"
                  onClick={() => void createTab('powershell')}
                >
                  <SquareTerminal size={14} strokeWidth={1.75} />
                  <span className="xy-terminal-add-label">
                    {t('terminal.shell.powershell', 'PowerShell')}
                  </span>
                  <span className="xy-terminal-add-hint">ps</span>
                </button>
                <button
                  type="button"
                  className="xy-terminal-add-item"
                  role="menuitem"
                  onClick={() => void createTab('cmd')}
                >
                  <SquareTerminal size={14} strokeWidth={1.75} />
                  <span className="xy-terminal-add-label">
                    {t('terminal.shell.cmd', '命令提示符')}
                  </span>
                  <span className="xy-terminal-add-hint">cmd</span>
                </button>
              </>
            ) : (
              <button
                type="button"
                className="xy-terminal-add-item"
                role="menuitem"
                onClick={() => void createTab('shell')}
              >
                <SquareTerminal size={14} strokeWidth={1.75} />
                <span className="xy-terminal-add-label">{t('terminal.shell.default', 'Shell')}</span>
              </button>
            )}
          </div>,
          document.body,
        )
      : null;

  return (
    <section
      className="xy-terminal-panel"
      style={{ height }}
      aria-label={t('terminal.title', '终端')}
    >
      <div className="xy-terminal-resize" onMouseDown={onDragStart} role="separator" aria-orientation="horizontal" />
      <header className="xy-terminal-toolbar">
        <div className="xy-terminal-tabs" role="tablist">
          {tabs.map((tab) => {
            const active = tab.id === activeId;
            return (
              <div
                key={tab.id}
                className={`xy-terminal-tab${active ? ' is-active' : ''}`}
                role="tab"
                aria-selected={active}
              >
                <button
                  type="button"
                  className="xy-terminal-tab-label"
                  title={tab.cwd}
                  onClick={() => setActiveId(tab.id)}
                >
                  <SquareTerminal size={13} strokeWidth={1.75} className="xy-terminal-tab-icon" />
                  <span className="xy-terminal-tab-text">{tab.title}</span>
                </button>
                <button
                  type="button"
                  className="xy-terminal-tab-close"
                  aria-label={t('terminal.closeTab', '关闭标签')}
                  onClick={() => void closeTab(tab.id)}
                >
                  <X size={12} strokeWidth={2} />
                </button>
              </div>
            );
          })}
          <button
            ref={addBtnRef}
            type="button"
            className={`xy-terminal-tab-add${addMenuOpen ? ' is-open' : ''}`}
            aria-label={t('terminal.newTab', '新建终端')}
            title={t('terminal.newTab', '新建终端')}
            aria-haspopup="menu"
            aria-expanded={addMenuOpen}
            onClick={(event) => {
              event.stopPropagation();
              setAddMenuOpen((v) => !v);
            }}
          >
            <Plus size={14} strokeWidth={2} />
          </button>
        </div>
        <button
          type="button"
          className="xy-terminal-panel-close"
          aria-label={t('terminal.hide', '隐藏终端')}
          title={t('terminal.hide', '隐藏终端')}
          onClick={closePanel}
        >
          <X size={14} strokeWidth={2} />
        </button>
      </header>
      {shellMenu}
      <div className="xy-terminal-body">
        {tabs.map((tab) => (
          <TerminalSessionView key={tab.id} id={tab.id} active={tab.id === activeId} />
        ))}
      </div>
    </section>
  );
}
