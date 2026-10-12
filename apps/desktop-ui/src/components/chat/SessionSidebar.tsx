/**
 * 聊天页会话侧栏：项目树 + 最近会话列表。
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Dropdown, Input, Modal, Popover, message } from 'antd';
import type { MenuProps } from 'antd';
import {
  CheckOutlined,
  CloseOutlined,
  EditOutlined,
  EllipsisOutlined,
  FolderAddOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  FormOutlined,
  MessageOutlined,
  PlusOutlined,
  PushpinFilled,
  PushpinOutlined,
  SettingOutlined,
} from '@ant-design/icons';
import { Archive, Pin, Trash2 } from 'lucide-react';
import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import { displayFsPath } from '../../foundation/config/paths';
import type { SessionInfo } from '../../protocol';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import {
  formatSessionAge,
  groupSessionsForSidebar,
  projectGroupLabel,
  PROJECT_VISIBLE_LIMIT,
  sessionProjectDir,
  type ProjectSessionGroup,
  type SidebarOrganizeMode,
  type SidebarSortMode,
} from './sessionSidebarUtils';

const COLLAPSED_PROJECTS_KEY = 'xy.sessionProjectsCollapsed';
const PROJECTS_SECTION_KEY = 'xy.sessionProjectsSectionCollapsed';
const RECENT_SECTION_KEY = 'xy.sessionRecentCollapsed';
const ORGANIZE_KEY = 'xy.sessionSidebarOrganize';
const SORT_KEY = 'xy.sessionSidebarSort';
const HIDDEN_PROJECTS_KEY = 'xy.sessionHiddenProjects';
const PINNED_PROJECTS_KEY = 'xy.sessionPinnedProjects';
const ALIASES_KEY = 'xy.sessionProjectAliases';
const PROJECT_FOLDERS_KEY = 'xy.sessionProjectFolders';

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // ignore
  }
}

function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === '1';
  } catch {
    return false;
  }
}

function writeFlag(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? '1' : '0');
  } catch {
    // ignore
  }
}

function projectPathLabel(key: string): string {
  if (/[/\\]/.test(key) || /^[A-Za-z]:/.test(key)) return key;
  return '';
}

function ProjectDetailCard({
  label,
  path,
  pinned,
  onTogglePin,
  onEdit,
  pinLabel,
  unpinLabel,
  taskCountLabel,
  editLabel,
}: {
  label: string;
  path: string;
  pinned: boolean;
  onTogglePin: () => void;
  onEdit: () => void;
  pinLabel: string;
  unpinLabel: string;
  taskCountLabel: string;
  editLabel: string;
}) {
  return (
    <div className="xy-session-project-card">
      <div className="xy-session-project-card-head">
        <span className="xy-session-project-card-title">
          <FolderOutlined className="xy-session-ico" />
          <strong>{label}</strong>
        </span>
        <button
          type="button"
          className={`xy-session-icon-btn${pinned ? ' is-on' : ''}`}
          aria-label={pinned ? unpinLabel : pinLabel}
          title={pinned ? unpinLabel : pinLabel}
          onClick={(e) => {
            e.stopPropagation();
            onTogglePin();
          }}
        >
          {pinned ? <PushpinFilled /> : <PushpinOutlined />}
        </button>
      </div>
      <div className="xy-session-project-card-row">
        <MessageOutlined className="xy-session-ico" />
        <span>{taskCountLabel}</span>
      </div>
      {path ? (
        <>
          <div className="xy-session-project-card-divider" />
          <div className="xy-session-project-card-row" title={displayFsPath(path)}>
            <FolderOutlined className="xy-session-ico" />
            <span className="xy-session-project-card-path">{displayFsPath(path)}</span>
          </div>
        </>
      ) : null}
      <div className="xy-session-project-card-divider" />
      <button
        type="button"
        className="xy-session-project-card-edit"
        onClick={(e) => {
          e.stopPropagation();
          onEdit();
        }}
      >
        <SettingOutlined className="xy-session-ico" />
        <span>{editLabel}</span>
      </button>
    </div>
  );
}

function SessionChatCard({
  title,
  ageLabel,
  projectLabel,
}: {
  title: string;
  ageLabel: string;
  projectLabel?: string;
}) {
  return (
    <div className="xy-session-chat-card">
      <div className="xy-session-chat-card-head">
        <strong className="xy-session-chat-card-title">{title}</strong>
        {ageLabel ? <span className="xy-session-chat-card-age">{ageLabel}</span> : null}
      </div>
      {projectLabel ? (
        <div className="xy-session-chat-card-row">
          <FolderOutlined className="xy-session-ico" />
          <span className="xy-session-chat-card-project">{projectLabel}</span>
        </div>
      ) : null}
    </div>
  );
}

function SessionRow({
  session,
  active,
  nested,
  inPinnedSection,
  projectLabel,
  ageLabel,
  onSelect,
  onPin,
  onArchive,
  pinLabel,
  unpinLabel,
  archiveLabel,
}: {
  session: SessionInfo;
  active: boolean;
  nested?: boolean;
  /** 置顶区内始终露出图钉/归档，并提供悬浮详情。 */
  inPinnedSection?: boolean;
  projectLabel?: string;
  ageLabel?: string;
  onSelect: (session: SessionInfo) => void;
  onPin: (session: SessionInfo) => void;
  onArchive: (session: SessionInfo) => void;
  pinLabel: string;
  unpinLabel: string;
  archiveLabel: string;
}) {
  const row = (
    <div
      className={`xy-session-item${nested ? ' is-nested' : ''}${active ? ' is-active' : ''}${session.pinned ? ' is-pinned' : ''}${inPinnedSection ? ' is-in-pinned' : ''}`}
      onClick={() => onSelect(session)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onSelect(session);
        }
      }}
    >
      <span className="xy-session-title">{session.name}</span>
      <span className="xy-session-item-actions">
        <button
          type="button"
          className={`xy-session-action-btn${session.pinned ? ' is-on is-pin' : ' is-pin'}`}
          title={session.pinned ? unpinLabel : pinLabel}
          aria-label={session.pinned ? unpinLabel : pinLabel}
          onClick={(e) => {
            e.stopPropagation();
            onPin(session);
          }}
        >
          <Pin
            size={14}
            strokeWidth={1.75}
            absoluteStrokeWidth
            fill={session.pinned ? 'currentColor' : 'none'}
            aria-hidden
          />
        </button>
        <button
          type="button"
          className="xy-session-action-btn is-danger"
          title={archiveLabel}
          aria-label={archiveLabel}
          onClick={(e) => {
            e.stopPropagation();
            onArchive(session);
          }}
        >
          <Trash2 size={14} strokeWidth={1.75} absoluteStrokeWidth aria-hidden />
        </button>
      </span>
    </div>
  );

  if (!inPinnedSection) return row;

  return (
    <Popover
      placement="rightTop"
      trigger="hover"
      mouseEnterDelay={0.35}
      mouseLeaveDelay={0.12}
      overlayClassName="xy-session-chat-popover"
      content={
        <SessionChatCard
          title={session.name}
          ageLabel={ageLabel || ''}
          projectLabel={projectLabel}
        />
      }
    >
      {row}
    </Popover>
  );
}

/** 准备态不占侧栏：尚未发消息前不插入任何行。 */

function ProjectFolder({
  group,
  collapsed,
  expandedLimit,
  activeSessionId,
  pinned,
  onToggle,
  onExpandMore,
  onSelect,
  onPinSession,
  onArchiveSession,
  onNewSession,
  onTogglePinProject,
  onEditProject,
  projectMenu,
  pinLabel,
  unpinLabel,
  archiveLabel,
  showMoreLabel,
  newSessionLabel,
  moreLabel,
  taskCountLabel,
  editProjectLabel,
  emptyChatsLabel,
}: {
  group: ProjectSessionGroup;
  collapsed: boolean;
  expandedLimit: boolean;
  activeSessionId: string | null;
  pinned: boolean;
  onToggle: () => void;
  onExpandMore: () => void;
  onSelect: (session: SessionInfo) => void;
  onPinSession: (session: SessionInfo) => void;
  onArchiveSession: (session: SessionInfo) => void;
  onNewSession: () => void;
  onTogglePinProject: () => void;
  onEditProject: () => void;
  projectMenu: MenuProps;
  pinLabel: string;
  unpinLabel: string;
  archiveLabel: string;
  showMoreLabel: string;
  newSessionLabel: string;
  moreLabel: string;
  taskCountLabel: string;
  editProjectLabel: string;
  emptyChatsLabel: string;
}) {
  const visible = expandedLimit ? group.sessions : group.sessions.slice(0, PROJECT_VISIBLE_LIMIT);
  const hiddenCount = group.sessions.length - visible.length;
  const path = projectPathLabel(group.key);

  const row = (
    <div className="xy-session-folder-row">
      <button
        type="button"
        className="xy-session-folder-head"
        onClick={onToggle}
        aria-expanded={!collapsed}
      >
        <span className="xy-session-folder-icon" aria-hidden>
          {collapsed ? <FolderOutlined /> : <FolderOpenOutlined />}
        </span>
        <span className="xy-session-folder-label">{group.label}</span>
      </button>
      <div className="xy-session-folder-actions">
        <Dropdown menu={projectMenu} trigger={['click']}>
          <button
            type="button"
            className="xy-session-icon-btn"
            aria-label={moreLabel}
            onClick={(e) => e.stopPropagation()}
          >
            <EllipsisOutlined />
          </button>
        </Dropdown>
        <button
          type="button"
          className="xy-session-icon-btn"
          aria-label={newSessionLabel}
          title={newSessionLabel}
          onClick={(e) => {
            e.stopPropagation();
            onNewSession();
          }}
        >
          <EditOutlined />
        </button>
      </div>
    </div>
  );

  return (
    <div className={`xy-session-folder${pinned ? ' is-pinned' : ''}`}>
      <Popover
        placement="rightTop"
        trigger="hover"
        mouseEnterDelay={0.35}
        mouseLeaveDelay={0.12}
        overlayClassName="xy-session-project-popover"
        content={
          <ProjectDetailCard
            label={group.label}
            path={path}
            pinned={pinned}
            onTogglePin={onTogglePinProject}
            onEdit={onEditProject}
            pinLabel={pinLabel}
            unpinLabel={unpinLabel}
            taskCountLabel={taskCountLabel}
            editLabel={editProjectLabel}
          />
        }
      >
        {row}
      </Popover>
      {!collapsed && (
        <div className="xy-session-folder-body">
          {visible.length === 0 ? (
            <div className="xy-session-empty">{emptyChatsLabel}</div>
          ) : null}
          {visible.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              nested
              active={activeSessionId === session.id}
              onSelect={onSelect}
              onPin={onPinSession}
              onArchive={onArchiveSession}
              pinLabel={pinLabel}
              unpinLabel={unpinLabel}
              archiveLabel={archiveLabel}
            />
          ))}
          {hiddenCount > 0 ? (
            <button type="button" className="xy-session-show-more" onClick={onExpandMore}>
              {showMoreLabel}
            </button>
          ) : null}
        </div>
      )}
    </div>
  );
}

/** 被谁调用：ChatView / AppSidebar。做什么：项目分组会话树 + 最近会话列表。 */
export default function SessionSidebar({
  embedded = false,
  hideNewChat = false,
  searchQuery = '',
}: {
  embedded?: boolean;
  /** 新对话入口已上移到 AppSidebar 时隐藏。 */
  hideNewChat?: boolean;
  /** 侧栏顶栏搜索关键字（过滤会话名）。 */
  searchQuery?: string;
}) {
  const { t, locale } = useI18n();
  const { send, addHandler } = useWs();
  const sessions = useSessionStore((s) => s.sessions);
  const updateSession = useSessionStore((s) => s.updateSession);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const setActiveSession = useChatStore((s) => s.setActiveSession);
  const initSession = useChatStore((s) => s.initSession);
  const composeDraft = useChatStore((s) => s.composeDraft);
  const beginComposeDraft = useChatStore((s) => s.beginComposeDraft);

  const [collapsedProjects, setCollapsedProjects] = useState<Record<string, boolean>>(
    () => readJson(COLLAPSED_PROJECTS_KEY, {}),
  );
  const [projectsCollapsed, setProjectsCollapsed] = useState(() => readFlag(PROJECTS_SECTION_KEY));
  const [recentCollapsed, setRecentCollapsed] = useState(() => readFlag(RECENT_SECTION_KEY));
  const [expandedProjectLimits, setExpandedProjectLimits] = useState<Record<string, boolean>>({});
  const [organize, setOrganize] = useState<SidebarOrganizeMode>(
    () => readJson(ORGANIZE_KEY, 'byProject'),
  );
  const [sortMode, setSortMode] = useState<SidebarSortMode>(() => readJson(SORT_KEY, 'recent'));
  const [hiddenProjects, setHiddenProjects] = useState<string[]>(
    () => readJson(HIDDEN_PROJECTS_KEY, []),
  );
  const [pinnedProjects, setPinnedProjects] = useState<string[]>(
    () => readJson(PINNED_PROJECTS_KEY, []),
  );
  const [aliases, setAliases] = useState<Record<string, string>>(() => readJson(ALIASES_KEY, {}));
  const [projectFolders, setProjectFolders] = useState<Record<string, string[]>>(
    () => readJson(PROJECT_FOLDERS_KEY, {}),
  );
  const [editOpen, setEditOpen] = useState(false);
  const [editKey, setEditKey] = useState('');
  const [editLabel, setEditLabel] = useState('');
  const [editFolders, setEditFolders] = useState<string[]>([]);

  const visibleSessions = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter((session) => session.name.toLowerCase().includes(q));
  }, [searchQuery, sessions]);

  const groups = useMemo(() => {
    return groupSessionsForSidebar(visibleSessions, {
      organize,
      sort: sortMode,
      hiddenProjectKeys: hiddenProjects,
      pinnedProjectKeys: pinnedProjects,
      projectAliases: aliases,
    });
  }, [visibleSessions, organize, sortMode, hiddenProjects, pinnedProjects, aliases]);

  useEffect(() => {
    send('session.list', { limit: 200 });
  }, [send]);

  const switchSession = useCallback(
    (session: SessionInfo) => {
      // 切会话时结束准备态，composer 由 ChatView 按会话 projectDir 同步
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
    },
    [initSession, send, setActiveSession],
  );

  const handlePinSession = useCallback(
    (session: SessionInfo) => {
      const next = !session.pinned;
      updateSession(session.id, { pinned: next });
      send('session.pin', { sessionId: session.id, pinned: next });
    },
    [send, updateSession],
  );

  const handleArchiveSession = useCallback(
    (session: SessionInfo) => {
      updateSession(session.id, { archivedAt: Date.now(), status: 'archived' });
      send('session.archive', { sessionId: session.id, archived: true });
    },
    [send, updateSession],
  );

  const handleNewRecentSession = useCallback(() => {
    beginComposeDraft({ kind: 'recent' });
    setRecentCollapsed(false);
    writeFlag(RECENT_SECTION_KEY, false);
  }, [beginComposeDraft]);

  const handleNewProjectSession = useCallback(
    (projectDir: string) => {
      beginComposeDraft({ kind: 'project', projectDir });
      setCollapsedProjects((prev) => {
        const next = { ...prev, [projectDir]: false };
        writeJson(COLLAPSED_PROJECTS_KEY, next);
        return next;
      });
      setProjectsCollapsed(false);
      writeFlag(PROJECTS_SECTION_KEY, false);
      send('workspace.open', { path: projectDir });
    },
    [beginComposeDraft, send],
  );

  /** 顶部「新对话」：有项目归属则进该项目，否则进「最近」。 */
  const handleNewChat = useCallback(() => {
    if (composeDraft?.kind === 'project' && composeDraft.projectDir.trim()) {
      handleNewProjectSession(composeDraft.projectDir.trim());
      return;
    }
    const active = sessions.find((item) => item.id === activeSessionId);
    const dir = active ? sessionProjectDir(active) : undefined;
    if (dir) {
      handleNewProjectSession(dir);
      return;
    }
    handleNewRecentSession();
  }, [
    activeSessionId,
    composeDraft,
    handleNewProjectSession,
    handleNewRecentSession,
    sessions,
  ]);

  const handleNewProject = useCallback(() => {
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
        message.warning(t('sessionSidebar.browseFailed'));
      }
    });
    send('workspace.pickFolder', {
      title: t('sessionSidebar.newProject'),
    });
  }, [addHandler, beginComposeDraft, send, t]);

  const setOrganizeMode = (mode: SidebarOrganizeMode) => {
    setOrganize(mode);
    writeJson(ORGANIZE_KEY, mode);
  };

  const setSort = (mode: SidebarSortMode) => {
    setSortMode(mode);
    writeJson(SORT_KEY, mode);
  };

  const organizeMenuItems: MenuProps['items'] = [
    {
      type: 'group',
      label: t('sessionSidebar.organizeTitle'),
      children: [
        {
          key: 'byProject',
          label: t('sessionSidebar.organizeByProject'),
          icon: organize === 'byProject' ? <CheckOutlined /> : <span className="xy-session-menu-gap" />,
          onClick: () => setOrganizeMode('byProject'),
        },
        {
          key: 'flat',
          label: t('sessionSidebar.organizeFlat'),
          icon: organize === 'flat' ? <CheckOutlined /> : <span className="xy-session-menu-gap" />,
          onClick: () => setOrganizeMode('flat'),
        },
      ],
    },
    {
      type: 'group',
      label: t('sessionSidebar.sortTitle'),
      children: [
        {
          key: 'priority',
          label: t('sessionSidebar.sortPriority'),
          icon: sortMode === 'priority' ? <CheckOutlined /> : <span className="xy-session-menu-gap" />,
          onClick: () => setSort('priority'),
        },
        {
          key: 'recent',
          label: t('sessionSidebar.sortRecent'),
          icon: sortMode === 'recent' ? <CheckOutlined /> : <span className="xy-session-menu-gap" />,
          onClick: () => setSort('recent'),
        },
        {
          key: 'manual',
          label: t('sessionSidebar.sortManual'),
          icon: sortMode === 'manual' ? <CheckOutlined /> : <span className="xy-session-menu-gap" />,
          onClick: () => setSort('manual'),
        },
      ],
    },
  ];

  const openEditProject = useCallback(
    (group: ProjectSessionGroup) => {
      setEditKey(group.key);
      setEditLabel(aliases[group.key] || group.label);
      const stored = projectFolders[group.key];
      if (stored && stored.length > 0) {
        setEditFolders(stored);
      } else if (projectPathLabel(group.key)) {
        setEditFolders([group.key]);
      } else {
        setEditFolders([]);
      }
      setEditOpen(true);
    },
    [aliases, projectFolders],
  );

  const handleSaveEditProject = useCallback(() => {
    const name = editLabel.trim();
    setAliases((prev) => {
      const next = { ...prev };
      if (name) next[editKey] = name;
      else delete next[editKey];
      writeJson(ALIASES_KEY, next);
      return next;
    });
    setProjectFolders((prev) => {
      const next = { ...prev };
      if (editFolders.length > 0) next[editKey] = editFolders;
      else delete next[editKey];
      writeJson(PROJECT_FOLDERS_KEY, next);
      return next;
    });
    setEditOpen(false);
  }, [editFolders, editKey, editLabel]);

  const handleRemoveEditProject = useCallback(() => {
    if (!editKey) return;
    setHiddenProjects((prev) => {
      const next = prev.includes(editKey) ? prev : [...prev, editKey];
      writeJson(HIDDEN_PROJECTS_KEY, next);
      return next;
    });
    setPinnedProjects((prev) => {
      const next = prev.filter((k) => k !== editKey);
      writeJson(PINNED_PROJECTS_KEY, next);
      return next;
    });
    setAliases((prev) => {
      const next = { ...prev };
      delete next[editKey];
      writeJson(ALIASES_KEY, next);
      return next;
    });
    setProjectFolders((prev) => {
      const next = { ...prev };
      delete next[editKey];
      writeJson(PROJECT_FOLDERS_KEY, next);
      return next;
    });
    setEditOpen(false);
  }, [editKey]);

  const handleAddEditFolder = useCallback(() => {
    const unsub = addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type !== 'workspace.folderPicked') return;
      unsub();
      const payload = msg.payload as { path?: string | null; cancelled?: boolean; error?: string };
      if (payload.path) {
        const path = payload.path.trim();
        setEditFolders((prev) => (prev.includes(path) ? prev : [...prev, path]));
        return;
      }
      if (payload.error) {
        message.warning(t('sessionSidebar.browseFailed'));
      }
    });
    send('workspace.pickFolder', {
      title: t('sessionSidebar.addFolder'),
      initialDir: editFolders[0] || editKey || undefined,
    });
  }, [addHandler, editFolders, editKey, send, t]);

  const buildProjectMenu = (group: ProjectSessionGroup): MenuProps => {
    const canReveal = /[/\\]/.test(group.key) || /^[A-Za-z]:/.test(group.key);
    return {
      items: [
        {
          key: 'pin',
          icon: <PushpinOutlined />,
          label: pinnedProjects.includes(group.key)
            ? t('sessionSidebar.unpinProject')
            : t('sessionSidebar.pinProject'),
          onClick: () => {
            setPinnedProjects((prev) => {
              const next = prev.includes(group.key)
                ? prev.filter((k) => k !== group.key)
                : [group.key, ...prev.filter((k) => k !== group.key)];
              writeJson(PINNED_PROJECTS_KEY, next);
              return next;
            });
          },
        },
        {
          key: 'edit',
          icon: <EditOutlined />,
          label: t('sessionSidebar.editProject'),
          onClick: () => openEditProject(group),
        },
        { type: 'divider' },
        ...(canReveal
          ? [
              {
                key: 'reveal',
                icon: <FolderOpenOutlined />,
                label: t('sessionSidebar.openInExplorer'),
                onClick: () => send('workspace.reveal', { path: group.key }),
              } as const,
              {
                key: 'worktree',
                icon: <FolderOutlined />,
                label: t('sessionSidebar.createWorktree'),
                onClick: () => message.info(t('sessionSidebar.worktreeUnavailable')),
              } as const,
              { type: 'divider' as const },
            ]
          : []),
        {
          key: 'archive',
          icon: <Archive size={14} strokeWidth={1.75} absoluteStrokeWidth />,
          label: t('sessionSidebar.archiveChats'),
          onClick: () => {
            for (const session of group.sessions) {
              updateSession(session.id, { archivedAt: Date.now(), status: 'archived' });
              send('session.archive', { sessionId: session.id, archived: true });
            }
          },
        },
        { type: 'divider' },
        {
          key: 'remove',
          icon: <CloseOutlined />,
          label: t('sessionSidebar.removeProject'),
          danger: true,
          onClick: () => {
            setHiddenProjects((prev) => {
              const next = prev.includes(group.key) ? prev : [...prev, group.key];
              writeJson(HIDDEN_PROJECTS_KEY, next);
              return next;
            });
            setPinnedProjects((prev) => {
              const next = prev.filter((k) => k !== group.key);
              writeJson(PINNED_PROJECTS_KEY, next);
              return next;
            });
          },
        },
      ],
    };
  };

  const projectCollapsed = (group: ProjectSessionGroup): boolean => {
    if (collapsedProjects[group.key] !== undefined) return collapsedProjects[group.key];
    return !group.sessions.some((s) => s.id === activeSessionId);
  };

  const pinLabel = t('sessionSidebar.pin');
  const unpinLabel = t('sessionSidebar.unpin');
  const archiveLabel = t('sessionSidebar.archive');
  const showMoreLabel = t('sessionSidebar.showMore');
  const newSessionLabel = t('sessionSidebar.newSession');
  const newChatLabel = t('sessionSidebar.newChat');
  const moreLabel = t('sessionSidebar.more');

  const resolveSessionProjectLabel = useCallback(
    (session: SessionInfo): string | undefined => {
      const dir = sessionProjectDir(session);
      if (!dir) return undefined;
      return aliases[dir]?.trim() || projectGroupLabel(dir) || undefined;
    },
    [aliases],
  );

  const renderSessionList = (list: SessionInfo[], nested?: boolean) => {
    if (list.length === 0) {
      return <div className="xy-session-empty">{t('sessionSidebar.empty')}</div>;
    }
    return (
      <>
        {list.map((session) => (
          <SessionRow
            key={session.id}
            session={session}
            nested={nested}
            active={activeSessionId === session.id}
            onSelect={switchSession}
            onPin={handlePinSession}
            onArchive={handleArchiveSession}
            pinLabel={pinLabel}
            unpinLabel={unpinLabel}
            archiveLabel={archiveLabel}
          />
        ))}
      </>
    );
  };

  const Root = embedded ? 'div' : 'aside';

  return (
    <Root
      className={`xy-session-panel${embedded ? ' is-embedded' : ''}`}
      aria-label={t('sessionSidebar.title')}
    >
      <div className="xy-session-scroll">
        {!hideNewChat ? (
          <button
            type="button"
            className="xy-session-new-chat"
            onClick={handleNewChat}
            aria-label={newChatLabel}
          >
            <FormOutlined className="xy-session-new-chat-ico" aria-hidden />
            <span>{newChatLabel}</span>
          </button>
        ) : null}

        {groups.pinned.length > 0 ? (
          <section className="xy-session-block xy-session-block-pinned">
            <div className="xy-session-section-head is-static">
              <span className="xy-session-section-title">{t('sessionSidebar.pinned')}</span>
            </div>
            <div className="xy-session-pinned-list">
              {groups.pinned.map((session) => (
                <SessionRow
                  key={session.id}
                  session={session}
                  inPinnedSection
                  active={activeSessionId === session.id}
                  projectLabel={resolveSessionProjectLabel(session)}
                  ageLabel={formatSessionAge(
                    session.lastMessageAt || session.updatedAt || session.createdAt,
                    locale === 'zh-CN' ? 'zh-CN' : 'en',
                  )}
                  onSelect={switchSession}
                  onPin={handlePinSession}
                  onArchive={handleArchiveSession}
                  pinLabel={pinLabel}
                  unpinLabel={unpinLabel}
                  archiveLabel={archiveLabel}
                />
              ))}
            </div>
          </section>
        ) : null}

        {organize === 'byProject' ? (
          <section className="xy-session-block xy-session-block-projects">
            <div
              className="xy-session-section-head"
              role="button"
              tabIndex={0}
              aria-expanded={!projectsCollapsed}
              onClick={() => {
                const next = !projectsCollapsed;
                setProjectsCollapsed(next);
                writeFlag(PROJECTS_SECTION_KEY, next);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  const next = !projectsCollapsed;
                  setProjectsCollapsed(next);
                  writeFlag(PROJECTS_SECTION_KEY, next);
                }
              }}
            >
              <span className="xy-session-section-title">{t('sessionSidebar.projects')}</span>
              <div
                className="xy-session-section-actions"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.stopPropagation()}
              >
                <Dropdown menu={{ items: organizeMenuItems }} trigger={['click']}>
                  <button type="button" className="xy-session-icon-btn" aria-label={moreLabel}>
                    <EllipsisOutlined />
                  </button>
                </Dropdown>
                <button
                  type="button"
                  className="xy-session-icon-btn"
                  aria-label={t('sessionSidebar.newProject')}
                  title={t('sessionSidebar.newProject')}
                  onClick={handleNewProject}
                >
                  <PlusOutlined />
                </button>
              </div>
            </div>
            {!projectsCollapsed && (
              <div className="xy-session-folder-list">
                {groups.projects.length === 0 ? (
                  <div className="xy-session-empty">{t('sessionSidebar.noProjects')}</div>
                ) : (
                  groups.projects.map((group) => (
                    <ProjectFolder
                      key={group.key}
                      group={group}
                      collapsed={projectCollapsed(group)}
                      expandedLimit={expandedProjectLimits[group.key] ?? false}
                      activeSessionId={activeSessionId}
                      pinned={pinnedProjects.includes(group.key)}
                      onToggle={() => {
                        const nextCollapsed = !projectCollapsed(group);
                        const next = { ...collapsedProjects, [group.key]: nextCollapsed };
                        setCollapsedProjects(next);
                        writeJson(COLLAPSED_PROJECTS_KEY, next);
                      }}
                      onExpandMore={() =>
                        setExpandedProjectLimits((prev) => ({ ...prev, [group.key]: true }))
                      }
                      onSelect={switchSession}
                      onPinSession={handlePinSession}
                      onArchiveSession={handleArchiveSession}
                      onNewSession={() => handleNewProjectSession(group.key)}
                      onTogglePinProject={() => {
                        setPinnedProjects((prev) => {
                          const next = prev.includes(group.key)
                            ? prev.filter((k) => k !== group.key)
                            : [group.key, ...prev.filter((k) => k !== group.key)];
                          writeJson(PINNED_PROJECTS_KEY, next);
                          return next;
                        });
                      }}
                      onEditProject={() => openEditProject(group)}
                      projectMenu={buildProjectMenu(group)}
                      pinLabel={pinLabel}
                      unpinLabel={unpinLabel}
                      archiveLabel={archiveLabel}
                      showMoreLabel={showMoreLabel}
                      newSessionLabel={newSessionLabel}
                      moreLabel={moreLabel}
                      taskCountLabel={t(
                        'sessionSidebar.taskCount',
                        '{0} 个任务',
                        group.sessions.length,
                      )}
                      editProjectLabel={t('sessionSidebar.editProject')}
                      emptyChatsLabel={t('sessionSidebar.noChats')}
                    />
                  ))
                )}
              </div>
            )}
          </section>
        ) : null}

        <section className="xy-session-block xy-session-block-recent">
          <div
            className="xy-session-section-head"
            role="button"
            tabIndex={0}
            aria-expanded={!recentCollapsed}
            onClick={() => {
              const next = !recentCollapsed;
              setRecentCollapsed(next);
              writeFlag(RECENT_SECTION_KEY, next);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                const next = !recentCollapsed;
                setRecentCollapsed(next);
                writeFlag(RECENT_SECTION_KEY, next);
              }
            }}
          >
            <span className="xy-session-section-title">
              {organize === 'flat' ? t('sessionSidebar.allChats') : t('sessionSidebar.recent')}
            </span>
            <div
              className="xy-session-section-actions"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => e.stopPropagation()}
            >
              <Dropdown menu={{ items: organizeMenuItems }} trigger={['click']}>
                <button type="button" className="xy-session-icon-btn" aria-label={moreLabel}>
                  <EllipsisOutlined />
                </button>
              </Dropdown>
              <button
                type="button"
                className="xy-session-icon-btn"
                onClick={handleNewRecentSession}
                aria-label={newSessionLabel}
                title={newSessionLabel}
              >
                <EditOutlined />
              </button>
            </div>
          </div>
          {!recentCollapsed && (
            <div className="xy-session-recent-list">
              {renderSessionList(organize === 'flat' ? groups.flat : groups.recent)}
            </div>
          )}
        </section>
      </div>

      <Modal
        className="xy-session-edit-project-modal"
        rootClassName="xy-session-edit-project-modal-root"
        title={t('sessionSidebar.editProject')}
        open={editOpen}
        onCancel={() => setEditOpen(false)}
        footer={null}
        width={440}
        centered
        destroyOnClose
      >
        <div className="xy-session-edit-project">
          <div className="xy-session-edit-name">
            <FolderOutlined className="xy-session-edit-name-ico" aria-hidden />
            <Input
              className="xy-session-edit-name-input"
              variant="borderless"
              value={editLabel}
              onChange={(e) => setEditLabel(e.target.value)}
              placeholder={t('sessionSidebar.editProjectPlaceholder')}
              onPressEnter={handleSaveEditProject}
            />
          </div>

          <div className="xy-session-edit-sources-label">{t('sessionSidebar.sourceFolders')}</div>
          <div className="xy-session-edit-sources">
            {editFolders.map((folder) => (
              <div key={folder} className="xy-session-edit-source-row" title={displayFsPath(folder)}>
                <FolderOutlined className="xy-session-edit-source-ico" aria-hidden />
                <span className="xy-session-edit-source-name">
                  {projectGroupLabel(folder) || displayFsPath(folder)}
                </span>
                <button
                  type="button"
                  className="xy-session-edit-source-remove"
                  aria-label={t('common.cancel', '取消')}
                  onClick={() => setEditFolders((prev) => prev.filter((item) => item !== folder))}
                >
                  <CloseOutlined />
                </button>
              </div>
            ))}
            <button
              type="button"
              className="xy-session-edit-source-add"
              onClick={handleAddEditFolder}
            >
              <FolderAddOutlined className="xy-session-edit-source-ico" aria-hidden />
              <span>{t('sessionSidebar.addFolder')}</span>
            </button>
          </div>

          <div className="xy-session-edit-footer">
            <button
              type="button"
              className="xy-session-edit-btn is-danger"
              onClick={handleRemoveEditProject}
            >
              {t('sessionSidebar.removeLocalProject')}
            </button>
            <div className="xy-session-edit-footer-right">
              <button
                type="button"
                className="xy-session-edit-btn is-ghost"
                onClick={() => setEditOpen(false)}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                type="button"
                className="xy-session-edit-btn is-primary"
                onClick={handleSaveEditProject}
              >
                {t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      </Modal>
    </Root>
  );
}
