/**
 * 集成标题栏：侧栏切换、应用菜单、右侧主题/亮暗/语言；Windows 自绘窗口按钮。
 */

import React, { useEffect, useState } from 'react';
import { Popover } from 'antd';
import { Languages, Moon, Palette, PanelLeft, PanelLeftClose, Sun } from 'lucide-react';
import { useI18n } from '../../i18n';
import type { ThemeMode } from '../../theme/types';
import ThemeSettingsPanel from './ThemeSettings';
import TitleBarMenus, { type TitleBarMenuAction } from './TitleBarMenus';

export interface TitleBarProps {
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  locale: string;
  themeMode: ThemeMode;
  themeOpen: boolean;
  onThemeOpenChange: (open: boolean) => void;
  onToggleThemeMode: () => void;
  onToggleLocale: () => void;
  onMenuAction: (action: TitleBarMenuAction) => void;
}

function isWinDesktop(): boolean {
  return typeof window !== 'undefined' && window.xuanyu?.platform === 'win32';
}

async function runCaptionCommand(
  command: 'minimize' | 'toggleMaximize' | 'close',
): Promise<{ ok?: boolean; maximized?: boolean } | void> {
  return window.xuanyu?.windowCommand?.(command);
}

/** Windows 无边框窗口的最小化 / 最大化 / 关闭。 */
function WindowCaptionButtons() {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const onResize = () => {
      const approx =
        Math.abs(window.outerWidth - window.screen.availWidth) <= 4 &&
        Math.abs(window.outerHeight - window.screen.availHeight) <= 4;
      setMaximized(approx);
    };
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  return (
    <div className="xy-titlebar-caption" role="group" aria-label={t('titlebar.windowControls', '窗口控制')}>
      <button
        type="button"
        className="xy-titlebar-caption-btn"
        title={t('titlebar.minimize', '最小化')}
        aria-label={t('titlebar.minimize', '最小化')}
        onClick={() => {
          void runCaptionCommand('minimize');
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M1 5h8" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      </button>
      <button
        type="button"
        className="xy-titlebar-caption-btn"
        title={maximized ? t('titlebar.restore', '还原') : t('titlebar.maximize', '最大化')}
        aria-label={maximized ? t('titlebar.restore', '还原') : t('titlebar.maximize', '最大化')}
        onClick={() => {
          void runCaptionCommand('toggleMaximize').then((result) => {
            if (result && typeof result.maximized === 'boolean') {
              setMaximized(result.maximized);
            }
          });
        }}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path
              d="M3 1.5h5.5V7M1.5 3h5.5v5.5H1.5V3z"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.1"
            />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <rect
              x="1.5"
              y="1.5"
              width="7"
              height="7"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
            />
          </svg>
        )}
      </button>
      <button
        type="button"
        className="xy-titlebar-caption-btn is-close"
        title={t('titlebar.close', '关闭')}
        aria-label={t('titlebar.close', '关闭')}
        onClick={() => {
          void runCaptionCommand('close');
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M2 2l6 6M8 2L2 8" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      </button>
    </div>
  );
}

export default function TitleBar({
  sidebarCollapsed,
  onToggleSidebar,
  locale,
  themeMode,
  themeOpen,
  onThemeOpenChange,
  onToggleThemeMode,
  onToggleLocale,
  onMenuAction,
}: TitleBarProps) {
  const { t } = useI18n();
  const isLight = themeMode === 'light';
  const showCaption = isWinDesktop();

  return (
    <header className="xy-titlebar" aria-label={t('titlebar.aria', '窗口标题栏')}>
      <div className="xy-titlebar-leading">
        <button
          type="button"
          className="xy-titlebar-sidebar-btn"
          onClick={onToggleSidebar}
          aria-label={
            sidebarCollapsed
              ? t('titlebar.view.showSidebar', '显示侧栏')
              : t('titlebar.view.hideSidebar', '隐藏侧栏')
          }
          aria-pressed={!sidebarCollapsed}
        >
          {sidebarCollapsed ? (
            <PanelLeft size={16} strokeWidth={1.75} />
          ) : (
            <PanelLeftClose size={16} strokeWidth={1.75} />
          )}
        </button>
      </div>

      <TitleBarMenus onAction={onMenuAction} />

      <div
        className="xy-titlebar-drag"
        aria-hidden
        onDoubleClick={() => {
          if (showCaption) void runCaptionCommand('toggleMaximize');
        }}
      />

      <div className="xy-titlebar-trailing">
        <Popover
          open={themeOpen}
          onOpenChange={onThemeOpenChange}
          trigger="click"
          placement="bottomRight"
          arrow={{ pointAtCenter: true }}
          overlayClassName="xy-theme-popover"
          destroyTooltipOnHide
          content={<ThemeSettingsPanel open={themeOpen} />}
        >
          <button
            type="button"
            className={`xy-titlebar-action-btn${themeOpen ? ' is-active' : ''}`}
            aria-label={t('theme.title', '主题')}
            title={t('theme.title', '主题')}
            aria-expanded={themeOpen}
          >
            <Palette size={16} strokeWidth={1.75} />
          </button>
        </Popover>
        <button
          type="button"
          className="xy-titlebar-action-btn"
          onClick={onToggleThemeMode}
          aria-label={isLight ? t('header.theme.dark', '深色模式') : t('header.theme.light', '浅色模式')}
          title={isLight ? t('header.theme.dark', '深色模式') : t('header.theme.light', '浅色模式')}
        >
          {isLight ? <Moon size={16} strokeWidth={1.75} /> : <Sun size={16} strokeWidth={1.75} />}
        </button>
        <button
          type="button"
          className="xy-titlebar-action-btn"
          onClick={onToggleLocale}
          aria-label={t('header.language', '切换语言')}
          title={locale === 'zh-CN' ? 'English' : '中文'}
        >
          <Languages size={16} strokeWidth={1.75} />
        </button>
      </div>

      {showCaption ? <WindowCaptionButtons /> : null}
    </header>
  );
}
