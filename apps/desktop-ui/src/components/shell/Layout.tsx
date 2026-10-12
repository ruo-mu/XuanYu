/**
 * 主工作台壳：顶栏、底栏 Dock、按 hash 切换聊天 / 市场 / 系统等页面。
 */

import React, { useCallback, useEffect, useState } from 'react';
import { Layout as AntLayout } from 'antd';
import { useWs } from '../../App';
import { useTheme } from '../../theme/ThemeContext';
import { useI18n } from '../../i18n';
import { useConfigStore } from '../../stores/configStore';
import { useChatStore } from '../../stores/chatStore';
import ChatView from '../chat/ChatView';
import AmbientBackground from './AmbientBackground';
import AboutDialog from './AboutDialog';
import AppSidebar from './AppSidebar';
import TitleBar from './TitleBar';
import type { TitleBarMenuAction } from './TitleBarMenus';
import BottomTerminalPanel from '../terminal/BottomTerminalPanel';
import TerminalToggleFab from './TerminalToggleFab';
import EngineStatusBanner from './EngineStatusBanner';
import { WorkspaceLoading } from './PanelShell';
import {
  navigateToPage,
  normalizeLegacyRoute,
  PAGE_REGISTRY,
  pageFromLocation,
  type PageKey,
} from '../../navigation/pageRegistry';
import { syncElectronTitleBarChrome } from '../../utils/electronChrome';
import { workspacePickFolder } from '../../ipc/desktopIpcInvoke';
import { useTerminalStore } from '../../stores/terminalStore';
import ExtensionsLayout from '../marketplace/ExtensionsLayout';

const { Content } = AntLayout;

const SystemSettingsLayout = React.lazy(() => import('../system/SystemSettingsLayout'));
const ChannelPanel = React.lazy(() => import('../channels/ChannelPanel'));
const InitWizard = React.lazy(() => import('../../init/InitWizard'));

const EDIT_COMMANDS: Record<
  Extract<TitleBarMenuAction, { type: 'edit' }>['command'],
  string
> = {
  undo: 'undo',
  redo: 'redo',
  cut: 'cut',
  copy: 'copy',
  paste: 'paste',
  delete: 'delete',
  selectAll: 'selectAll',
};

function runEditCommand(command: Extract<TitleBarMenuAction, { type: 'edit' }>['command']) {
  const mapped = EDIT_COMMANDS[command];
  if (!mapped) return;
  try {
    document.execCommand(mapped);
  } catch {
    /* ignore */
  }
}

async function runWindowCommand(
  command:
    | 'quit'
    | 'close'
    | 'minimize'
    | 'maximize'
    | 'unmaximize'
    | 'toggleMaximize'
    | 'zoomIn'
    | 'zoomOut'
    | 'resetZoom'
    | 'toggleFullscreen'
    | 'reload',
) {
  if (window.xuanyu?.windowCommand) {
    await window.xuanyu.windowCommand(command);
    return;
  }
  if (command === 'close' || command === 'quit') {
    window.close();
  }
}

/** 已连接网关后的主界面布局。 */
export default function Layout() {
  const [tab, setTab] = useState<PageKey>(pageFromLocation);
  const [themeOpen, setThemeOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const { send } = useWs();
  const { toggleMode, mode } = useTheme();
  const { t, locale, setLocale } = useI18n();
  const updateConfig = useConfigStore((s) => s.updateConfig);
  const terminalOpen = useTerminalStore((s) => s.open);
  const terminalHeight = useTerminalStore((s) => s.height);
  const toggleTerminal = useTerminalStore((s) => s.toggleOpen);

  const toggleLocale = useCallback(() => {
    const next = locale === 'zh-CN' ? 'en' : 'zh-CN';
    setLocale(next);
    updateConfig({ locale: next });
    send('config.update', { config: { locale: next } });
  }, [locale, send, setLocale, updateConfig]);

  useEffect(() => {
    syncElectronTitleBarChrome();
  }, []);

  useEffect(() => {
    const sync = () => {
      document.documentElement.classList.toggle(
        'xy-page-hidden',
        document.visibilityState === 'hidden',
      );
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => {
      document.removeEventListener('visibilitychange', sync);
      document.documentElement.classList.remove('xy-page-hidden');
    };
  }, []);

  useEffect(() => {
    const syncRoute = () => {
      normalizeLegacyRoute();
      setTab(pageFromLocation());
    };
    normalizeLegacyRoute();
    window.addEventListener('hashchange', syncRoute);
    if (!window.location.hash) navigateToPage('chat');
    return () => window.removeEventListener('hashchange', syncRoute);
  }, []);

  const navigate = useCallback((page: PageKey) => {
    navigateToPage(page);
    setTab(page);
  }, []);

  const handleMenuAction = useCallback(
    async (payload: TitleBarMenuAction | { type: string; page?: string }) => {
      switch (payload.type) {
        case 'navigate':
          if (payload.page) navigate(payload.page as PageKey);
          break;
        case 'new-chat':
          navigate('chat');
          useChatStore.getState().beginComposeDraft({ kind: 'recent' });
          break;
        case 'open-folder': {
          navigate('chat');
          try {
            const picked = await workspacePickFolder({
              title: t('titlebar.file.openFolder', '打开文件夹'),
            });
            if (!picked.cancelled && picked.path) {
              useChatStore.getState().beginComposeDraft({
                kind: 'project',
                projectDir: picked.path,
              });
              send('workspace.open', { path: picked.path });
            }
          } catch {
            /* ignore */
          }
          break;
        }
        case 'open-setup':
          navigate('setup');
          break;
        case 'close-window':
          await runWindowCommand('close');
          break;
        case 'quit':
          await runWindowCommand('quit');
          break;
        case 'toggle-sidebar':
          setSidebarCollapsed((v) => !v);
          break;
        case 'toggle-terminal':
          if (tab === 'chat') toggleTerminal();
          break;
        case 'open-theme':
          setThemeOpen(true);
          break;
        case 'toggle-theme-mode':
          toggleMode();
          break;
        case 'toggle-locale':
          toggleLocale();
          break;
        case 'about':
          setAboutOpen(true);
          break;
        case 'edit':
          if ('command' in payload) runEditCommand(payload.command);
          break;
        case 'zoom':
          if ('command' in payload) {
            const map = { in: 'zoomIn', out: 'zoomOut', reset: 'resetZoom' } as const;
            await runWindowCommand(map[payload.command]);
          }
          break;
        case 'toggle-fullscreen':
          await runWindowCommand('toggleFullscreen');
          break;
        case 'reload':
          await runWindowCommand('reload');
          break;
        default:
          break;
      }
    },
    [navigate, send, t, tab, toggleLocale, toggleMode, toggleTerminal],
  );

  useEffect(() => {
    const unsub = window.xuanyu?.onMenuAction?.((payload) => {
      void handleMenuAction(payload as TitleBarMenuAction);
    });
    return () => {
      unsub?.();
    };
  }, [handleMenuAction]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (tab !== 'chat') return;
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      if (event.key === '`' || event.code === 'Backquote') {
        event.preventDefault();
        toggleTerminal();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [tab, toggleTerminal]);

  const menuItems: { key: PageKey; label: string }[] = [
    { key: 'channels', label: t('nav.channels', '频道') },
    { key: 'extensions', label: t('nav.extensions', '扩展') },
  ];

  const tabTitles = Object.fromEntries(PAGE_REGISTRY.map((page) => [page.key, t(page.titleKey, page.fallback)])) as Record<PageKey, string>;

  const isWorkspace = tab !== 'chat';
  const isSetup = tab === 'setup';
  const hasAmbientShell = tab === 'chat';
  const useWorkspaceChrome = isWorkspace || hasAmbientShell;
  const suspenseFallback = <WorkspaceLoading label={t('common.loading')} />;
  const terminalOnSession = tab === 'chat';
  const terminalVisible = terminalOnSession && terminalOpen;

  return (
    <AntLayout className="xy-app-shell">
      <TitleBar
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={() => setSidebarCollapsed((v) => !v)}
        locale={locale}
        themeMode={mode}
        themeOpen={themeOpen}
        onThemeOpenChange={setThemeOpen}
        onToggleThemeMode={toggleMode}
        onToggleLocale={toggleLocale}
        onMenuAction={(action) => {
          void handleMenuAction(action);
        }}
      />

      <EngineStatusBanner />

      <AboutDialog open={aboutOpen} onClose={() => setAboutOpen(false)} />

      <div className="xy-app-body">
        {!isSetup && (
          <AppSidebar
            items={menuItems}
            activeKey={tab}
            onNavigate={navigate}
            showSessions={tab === 'chat'}
            collapsed={sidebarCollapsed}
          />
        )}

        <AntLayout className={`xy-main-shell${sidebarCollapsed || isSetup ? ' is-flush-left' : ''}`}>
        <Content className={`xy-main-content${useWorkspaceChrome ? ' is-workspace' : ''}`} aria-label={tabTitles[tab]}>
          {isSetup ? (
            <React.Suspense fallback={suspenseFallback}>
              <InitWizard
                onExit={() => {
                  // 向导写盘后必须让主界面重新拉一次配置，否则仍显示启动时的默认模型。
                  send('config.get', {});
                  navigate('chat');
                }}
              />
            </React.Suspense>
          ) : (
          <div
            className={`xy-main-stack${terminalVisible ? ' has-terminal' : ''}`}
            style={
              {
                ['--xy-terminal-h' as string]: terminalVisible ? `${terminalHeight}px` : '0px',
              } as React.CSSProperties
            }
          >
            <div className="xy-workspace xy-main-workspace">
              <AmbientBackground />
              <div className="xy-main-stack-primary">
                {terminalOnSession ? <TerminalToggleFab /> : null}
                {tab === 'chat' && (
                  <div className="xy-workspace-frame xy-workspace-scroll is-flush">
                    <ChatView />
                  </div>
                )}
                {isWorkspace && (
                  <div className="xy-workspace-frame xy-workspace-content">
                    {tab === 'extensions' && <ExtensionsLayout />}
                    {tab === 'channels' && (
                      <div className="xy-workspace-scroll is-flush">
                        <React.Suspense fallback={suspenseFallback}>
                          <ChannelPanel />
                        </React.Suspense>
                      </div>
                    )}
                    {tab === 'system' && <SystemPanel fallback={suspenseFallback} />}
                  </div>
                )}
              </div>
              {terminalOnSession ? <BottomTerminalPanel /> : null}
            </div>
          </div>
          )}
        </Content>
        </AntLayout>
      </div>
    </AntLayout>
  );
}

/** 被谁调用：Layout。做什么：系统区懒加载壳。 */
function SystemPanel({ fallback }: { fallback: React.ReactNode }) {
  return (
    <div className="xy-workspace-scroll is-flush">
      <React.Suspense fallback={fallback}>
        <SystemSettingsLayout fallback={fallback} />
      </React.Suspense>
    </div>
  );
}
