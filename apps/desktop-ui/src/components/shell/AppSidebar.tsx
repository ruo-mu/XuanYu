/**
 * App sidebar: main chat rail, or contextual panels (settings / extensions / channels)
 * with a top back control.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  ApiOutlined,
  AppstoreOutlined,
  ArrowLeftOutlined,
  DownOutlined,
  FormOutlined,
  RadarChartOutlined,
  SearchOutlined,
  SettingOutlined,
  ToolOutlined,
} from '@ant-design/icons';
import { Icon } from '@iconify/react';
import { message } from 'antd';
import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import {
  channelTypeFromLocation,
  extensionsTabFromLocation,
  navigateToChannelType,
  navigateToExtensions,
  type ExtensionsTab,
  type PageKey,
} from '../../navigation/pageRegistry';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import { CHANNEL_CATALOG } from '../channels/channelCatalog';
import { channelBrandIcon } from '../channels/channelIcons';
import SessionSidebar from '../chat/SessionSidebar';
import { sessionProjectDir } from '../chat/sessionSidebarUtils';
import {
  navigateToSettingsSection,
  SETTINGS_NAV_GROUPS,
  settingsSectionFromLocation,
  type SettingsSection,
} from '../system/settingsSections';
import ChatSearchPalette from './ChatSearchPalette';

export interface AppSidebarNavItem {
  key: PageKey;
  label: string;
}

const NAV_ICONS: Partial<Record<PageKey, React.ReactNode>> = {
  extensions: <AppstoreOutlined />,
  channels: <RadarChartOutlined />,
  system: <SettingOutlined />,
};

const EXTENSION_ITEMS: Array<{
  key: ExtensionsTab;
  labelKey: string;
  fallback: string;
  icon: React.ReactNode;
}> = [
  { key: 'plugins', labelKey: 'nav.plugins', fallback: '插件', icon: <AppstoreOutlined /> },
  { key: 'mcp', labelKey: 'nav.mcp', fallback: 'MCP', icon: <ApiOutlined /> },
  { key: 'skills', labelKey: 'nav.skills', fallback: '技能', icon: <ToolOutlined /> },
];

type SidebarPanel = 'main' | 'system' | 'extensions' | 'channels';

function panelFromPage(page: PageKey): SidebarPanel {
  if (page === 'system') return 'system';
  if (page === 'extensions') return 'extensions';
  if (page === 'channels') return 'channels';
  return 'main';
}

interface AppSidebarProps {
  items: AppSidebarNavItem[];
  activeKey: PageKey;
  onNavigate: (page: PageKey) => void;
  showSessions?: boolean;
  collapsed?: boolean;
}

export default function AppSidebar({
  items,
  activeKey,
  onNavigate,
  showSessions = false,
  collapsed = false,
}: AppSidebarProps) {
  const { t } = useI18n();
  const lt = useLocaleText();
  const { send, addHandler } = useWs();
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const composeDraft = useChatStore((s) => s.composeDraft);
  const beginComposeDraft = useChatStore((s) => s.beginComposeDraft);

  const [searchOpen, setSearchOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>(settingsSectionFromLocation);
  const [extensionsTab, setExtensionsTab] = useState<ExtensionsTab>(extensionsTabFromLocation);
  const [channelType, setChannelType] = useState(() => channelTypeFromLocation());

  const panel = panelFromPage(activeKey);

  useEffect(() => {
    const sync = () => {
      setSettingsSection(settingsSectionFromLocation());
      setExtensionsTab(extensionsTabFromLocation());
      setChannelType(channelTypeFromLocation());
    };
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }, []);

  const handleNewChat = useCallback(() => {
    onNavigate('chat');
    if (composeDraft?.kind === 'project' && composeDraft.projectDir.trim()) {
      const dir = composeDraft.projectDir.trim();
      beginComposeDraft({ kind: 'project', projectDir: dir });
      send('workspace.open', { path: dir });
      return;
    }
    const active = sessions.find((item) => item.id === activeSessionId);
    const dir = active ? sessionProjectDir(active) : undefined;
    if (dir) {
      beginComposeDraft({ kind: 'project', projectDir: dir });
      send('workspace.open', { path: dir });
      return;
    }
    beginComposeDraft({ kind: 'recent' });
  }, [
    activeSessionId,
    beginComposeDraft,
    composeDraft,
    onNavigate,
    send,
    sessions,
  ]);

  const handleOpenFolder = useCallback(() => {
    onNavigate('chat');
    const unsub = addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type !== 'workspace.folderPicked') return;
      unsub();
      const payload = msg.payload as { path?: string | null; cancelled?: boolean; error?: string };
      if (payload.path) {
        beginComposeDraft({ kind: 'project', projectDir: payload.path });
        send('workspace.open', { path: payload.path });
        return;
      }
      if (payload.error) {
        message.warning(t('chat.projectPicker.browseFailed', '无法打开系统文件夹对话框'));
      }
    });
    window.setTimeout(() => unsub(), 60000);
    send('workspace.pickFolder', {
      title: t('chat.projectPicker.useFolder', '打开文件夹'),
    });
  }, [addHandler, beginComposeDraft, onNavigate, send, t]);

  const openSearch = useCallback(() => {
    onNavigate('chat');
    setSearchOpen(true);
  }, [onNavigate]);

  const goBack = useCallback(() => {
    onNavigate('chat');
  }, [onNavigate]);

  useEffect(() => {
    if (collapsed) setSearchOpen(false);
  }, [collapsed]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) return;
      if (event.key.toLowerCase() !== 'k') return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (tag === 'input' || tag === 'textarea' || target?.isContentEditable) return;
      event.preventDefault();
      openSearch();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [openSearch]);

  return (
    <aside
      className={`xy-app-sidebar${collapsed ? ' is-collapsed' : ''}${panel !== 'main' ? ' is-panel' : ''}`}
      aria-label={
        panel === 'main'
          ? t('nav.main', '主导航')
          : panel === 'system'
            ? t('page.system', '系统配置')
            : panel === 'extensions'
              ? t('page.extensions', '扩展')
              : t('page.channels', '频道')
      }
      aria-hidden={collapsed}
    >
      {panel === 'main' ? (
        <>
          <div className="xy-app-sidebar-brand">
            <button type="button" className="xy-app-sidebar-brand-btn" aria-label={t('app.title')}>
              <span className="xy-app-sidebar-logo">{t('app.title')}</span>
              <DownOutlined className="xy-app-sidebar-brand-caret" aria-hidden />
            </button>
            <button
              type="button"
              className={`xy-app-sidebar-search-btn${searchOpen ? ' is-active' : ''}`}
              onClick={openSearch}
              aria-label={t('nav.searchChats', '搜索聊天')}
            >
              <SearchOutlined />
            </button>
          </div>

          <nav className="xy-app-sidebar-nav" aria-label={t('nav.main', '主导航')}>
            <button
              type="button"
              className="xy-app-sidebar-nav-item"
              onClick={handleNewChat}
              aria-label={t('sessionSidebar.newChat')}
            >
              <span className="xy-app-sidebar-nav-icon" aria-hidden>
                <FormOutlined />
              </span>
              <span className="xy-app-sidebar-nav-label">{t('sessionSidebar.newChat')}</span>
            </button>

            {items.map((item) => (
              <button
                key={item.key}
                type="button"
                className={`xy-app-sidebar-nav-item${activeKey === item.key ? ' is-active' : ''}`}
                onClick={() => onNavigate(item.key)}
                aria-current={activeKey === item.key ? 'page' : undefined}
              >
                <span className="xy-app-sidebar-nav-icon" aria-hidden>
                  {NAV_ICONS[item.key]}
                </span>
                <span className="xy-app-sidebar-nav-label">{item.label}</span>
              </button>
            ))}
          </nav>

          {showSessions ? (
            <div className="xy-app-sidebar-sessions">
              <SessionSidebar embedded hideNewChat />
            </div>
          ) : (
            <div className="xy-app-sidebar-spacer" aria-hidden />
          )}

          <div className="xy-app-sidebar-footer">
            <button
              type="button"
              className={`xy-app-sidebar-footer-btn${activeKey === 'system' ? ' is-active' : ''}`}
              onClick={() => onNavigate('system')}
              aria-label={t('nav.system')}
            >
              <SettingOutlined />
            </button>
          </div>
        </>
      ) : (
        <>
          <div className="xy-app-sidebar-panel-head">
            <button
              type="button"
              className="xy-app-sidebar-back-btn"
              onClick={goBack}
              aria-label={t('common.back', '返回')}
            >
              <ArrowLeftOutlined />
              <span>{t('common.back', '返回')}</span>
            </button>
          </div>

          {panel === 'system' && (
            <nav className="xy-app-sidebar-panel-body" aria-label={t('page.system', '系统配置')}>
              {SETTINGS_NAV_GROUPS.map((group) => (
                <div key={group.id} className="xy-app-sidebar-group">
                  <div className="xy-app-sidebar-group-label">{t(group.labelKey, group.fallback)}</div>
                  {group.items.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`xy-app-sidebar-nav-item${settingsSection === item.id ? ' is-active' : ''}`}
                      onClick={() => navigateToSettingsSection(item.id)}
                      aria-current={settingsSection === item.id ? 'page' : undefined}
                    >
                      <span className="xy-app-sidebar-nav-label">{t(item.labelKey, item.fallback)}</span>
                    </button>
                  ))}
                </div>
              ))}
            </nav>
          )}

          {panel === 'extensions' && (
            <nav className="xy-app-sidebar-panel-body" aria-label={t('page.extensions', '扩展')}>
              {EXTENSION_ITEMS.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  className={`xy-app-sidebar-nav-item${extensionsTab === item.key ? ' is-active' : ''}`}
                  onClick={() => navigateToExtensions(item.key)}
                  aria-current={extensionsTab === item.key ? 'page' : undefined}
                >
                  <span className="xy-app-sidebar-nav-icon" aria-hidden>
                    {item.icon}
                  </span>
                  <span className="xy-app-sidebar-nav-label">{t(item.labelKey, item.fallback)}</span>
                </button>
              ))}
            </nav>
          )}

          {panel === 'channels' && (
            <nav className="xy-app-sidebar-panel-body xy-app-sidebar-channel-list" aria-label={t('page.channels', '频道')}>
              {CHANNEL_CATALOG.map((item) => {
                const active = channelType === item.type;
                return (
                  <button
                    key={item.type}
                    type="button"
                    className={`xy-app-sidebar-channel-item tone-${item.tone}${active ? ' is-active' : ''}`}
                    onClick={() => navigateToChannelType(item.type)}
                    aria-current={active ? 'page' : undefined}
                  >
                    <span className="xy-app-sidebar-channel-icon" aria-hidden>
                      <Icon icon={channelBrandIcon(item.type)} width={18} height={18} />
                    </span>
                    <span className="xy-app-sidebar-channel-copy">
                      <strong>{lt(item.label, item.labelEn)}</strong>
                      <em>{lt(item.summary, item.summaryEn)}</em>
                    </span>
                  </button>
                );
              })}
            </nav>
          )}
        </>
      )}

      <ChatSearchPalette
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onNavigateChat={() => onNavigate('chat')}
        onNewChat={handleNewChat}
        onOpenFolder={handleOpenFolder}
      />
    </aside>
  );
}
