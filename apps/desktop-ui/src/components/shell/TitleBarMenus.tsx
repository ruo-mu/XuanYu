/**
 * 标题栏应用菜单：圆角浮层，含文件 / 编辑 / 视图 / 帮助。
 */

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useI18n } from '../../i18n';

export type TitleBarMenuAction =
  | { type: 'navigate'; page: string }
  | { type: 'new-chat' }
  | { type: 'open-folder' }
  | { type: 'open-setup' }
  | { type: 'close-window' }
  | { type: 'quit' }
  | { type: 'toggle-sidebar' }
  | { type: 'toggle-terminal' }
  | { type: 'open-theme' }
  | { type: 'toggle-theme-mode' }
  | { type: 'toggle-locale' }
  | { type: 'about' }
  | { type: 'edit'; command: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'delete' | 'selectAll' }
  | { type: 'zoom'; command: 'in' | 'out' | 'reset' }
  | { type: 'toggle-fullscreen' }
  | { type: 'reload' };

type MenuId = 'file' | 'edit' | 'view' | 'help';

type MenuEntry =
  | { kind: 'item'; id: string; label: string; shortcut?: string; action: TitleBarMenuAction }
  | { kind: 'separator' };

function modKey(): string {
  return typeof window !== 'undefined' && window.xuanyu?.platform === 'darwin' ? '⌘' : 'Ctrl';
}

function buildMenus(t: (key: string, fallback?: string) => string): Record<MenuId, MenuEntry[]> {
  const mod = modKey();
  return {
    file: [
      {
        kind: 'item',
        id: 'new-chat',
        label: t('titlebar.file.newChat', '新建对话'),
        shortcut: `${mod}+N`,
        action: { type: 'new-chat' },
      },
      {
        kind: 'item',
        id: 'open-folder',
        label: t('titlebar.file.openFolder', '打开文件夹'),
        shortcut: `${mod}+O`,
        action: { type: 'open-folder' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'settings',
        label: t('titlebar.file.settings', '系统设置'),
        shortcut: `${mod}+,`,
        action: { type: 'navigate', page: 'system' },
      },
      {
        kind: 'item',
        id: 'config-wizard',
        label: t('titlebar.file.configWizard', '配置向导'),
        action: { type: 'open-setup' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'close',
        label: t('titlebar.file.close', '关闭'),
        shortcut: `${mod}+W`,
        action: { type: 'close-window' },
      },
      {
        kind: 'item',
        id: 'quit',
        label: t('titlebar.file.quit', '退出玄语'),
        shortcut: `${mod}+Q`,
        action: { type: 'quit' },
      },
    ],
    edit: [
      {
        kind: 'item',
        id: 'undo',
        label: t('titlebar.edit.undo', '撤销'),
        shortcut: `${mod}+Z`,
        action: { type: 'edit', command: 'undo' },
      },
      {
        kind: 'item',
        id: 'redo',
        label: t('titlebar.edit.redo', '重做'),
        shortcut: `${mod}+Y`,
        action: { type: 'edit', command: 'redo' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'cut',
        label: t('titlebar.edit.cut', '剪切'),
        shortcut: `${mod}+X`,
        action: { type: 'edit', command: 'cut' },
      },
      {
        kind: 'item',
        id: 'copy',
        label: t('titlebar.edit.copy', '复制'),
        shortcut: `${mod}+C`,
        action: { type: 'edit', command: 'copy' },
      },
      {
        kind: 'item',
        id: 'paste',
        label: t('titlebar.edit.paste', '粘贴'),
        shortcut: `${mod}+V`,
        action: { type: 'edit', command: 'paste' },
      },
      {
        kind: 'item',
        id: 'delete',
        label: t('titlebar.edit.delete', '删除'),
        action: { type: 'edit', command: 'delete' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'select-all',
        label: t('titlebar.edit.selectAll', '全选'),
        shortcut: `${mod}+A`,
        action: { type: 'edit', command: 'selectAll' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'edit-settings',
        label: t('titlebar.edit.settings', '设置'),
        shortcut: `${mod}+,`,
        action: { type: 'navigate', page: 'system' },
      },
    ],
    view: [
      {
        kind: 'item',
        id: 'toggle-sidebar',
        label: t('titlebar.view.toggleSidebar', '切换侧边栏'),
        shortcut: `${mod}+B`,
        action: { type: 'toggle-sidebar' },
      },
      {
        kind: 'item',
        id: 'toggle-terminal',
        label: t('titlebar.view.toggleTerminal', '切换终端'),
        shortcut: `${mod}+\``,
        action: { type: 'toggle-terminal' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'theme-settings',
        label: t('titlebar.view.themeSettings', '主题设置'),
        action: { type: 'open-theme' },
      },
      {
        kind: 'item',
        id: 'toggle-theme',
        label: t('titlebar.view.toggleTheme', '切换浅色/深色'),
        action: { type: 'toggle-theme-mode' },
      },
      {
        kind: 'item',
        id: 'toggle-locale',
        label: t('titlebar.view.toggleLocale', '切换语言'),
        action: { type: 'toggle-locale' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'zoom-in',
        label: t('titlebar.view.zoomIn', '放大'),
        shortcut: `${mod}+Shift+=`,
        action: { type: 'zoom', command: 'in' },
      },
      {
        kind: 'item',
        id: 'zoom-out',
        label: t('titlebar.view.zoomOut', '缩小'),
        shortcut: `${mod}+-`,
        action: { type: 'zoom', command: 'out' },
      },
      {
        kind: 'item',
        id: 'zoom-reset',
        label: t('titlebar.view.actualSize', '实际大小'),
        shortcut: `${mod}+0`,
        action: { type: 'zoom', command: 'reset' },
      },
      { kind: 'separator' },
      {
        kind: 'item',
        id: 'fullscreen',
        label: t('titlebar.view.fullscreen', '切换全屏'),
        shortcut: 'F11',
        action: { type: 'toggle-fullscreen' },
      },
      {
        kind: 'item',
        id: 'reload',
        label: t('titlebar.view.reload', '重新加载'),
        shortcut: `${mod}+R`,
        action: { type: 'reload' },
      },
    ],
    help: [
      {
        kind: 'item',
        id: 'about',
        label: t('titlebar.help.about', '关于玄语'),
        action: { type: 'about' },
      },
    ],
  };
}

const MENU_IDS: MenuId[] = ['file', 'edit', 'view', 'help'];

export interface TitleBarMenusProps {
  onAction: (action: TitleBarMenuAction) => void;
}

export default function TitleBarMenus({ onAction }: TitleBarMenusProps) {
  const { t } = useI18n();
  const [openId, setOpenId] = useState<MenuId | null>(null);
  const rootRef = useRef<HTMLElement>(null);
  const listId = useId();
  const menus = buildMenus(t);

  const labels: Record<MenuId, string> = {
    file: t('titlebar.file', '文件'),
    edit: t('titlebar.edit', '编辑'),
    view: t('titlebar.view', '视图'),
    help: t('titlebar.help', '帮助'),
  };

  const close = useCallback(() => setOpenId(null), []);

  useEffect(() => {
    if (!openId) return;
    const onPointer = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) close();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    document.addEventListener('mousedown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [openId, close]);

  const run = (action: TitleBarMenuAction) => {
    close();
    onAction(action);
  };

  return (
    <nav ref={rootRef} className="xy-titlebar-menus" aria-label={t('titlebar.menus', '应用菜单')}>
      {MENU_IDS.map((menuId) => {
        const open = openId === menuId;
        return (
          <div key={menuId} className={`xy-app-menu${open ? ' is-open' : ''}`}>
            <button
              type="button"
              className={`xy-titlebar-menu-btn${open ? ' is-active' : ''}`}
              aria-haspopup="menu"
              aria-expanded={open}
              aria-controls={open ? `${listId}-${menuId}` : undefined}
              onClick={() => setOpenId((cur) => (cur === menuId ? null : menuId))}
              onMouseEnter={() => {
                if (openId && openId !== menuId) setOpenId(menuId);
              }}
            >
              {labels[menuId]}
            </button>
            {open ? (
              <div
                id={`${listId}-${menuId}`}
                className="xy-app-menu-panel"
                role="menu"
                aria-label={labels[menuId]}
              >
                {menus[menuId].map((entry, index) => {
                  if (entry.kind === 'separator') {
                    return <div key={`sep-${index}`} className="xy-app-menu-sep" role="separator" />;
                  }
                  return (
                    <button
                      key={entry.id}
                      type="button"
                      className="xy-app-menu-item"
                      role="menuitem"
                      onClick={() => run(entry.action)}
                    >
                      <span className="xy-app-menu-item-label">{entry.label}</span>
                      {entry.shortcut ? (
                        <span className="xy-app-menu-item-shortcut">{entry.shortcut}</span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            ) : null}
          </div>
        );
      })}
    </nav>
  );
}
