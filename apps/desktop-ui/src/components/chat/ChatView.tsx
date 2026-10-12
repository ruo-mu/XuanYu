/**
 * ChatView — 主聊天页（多会话 + 侧栏）。
 *
 * 被谁调用：Layout 聊天路由。
 * 做什么：SessionSidebar + 消息列表 + composer；经 useWs 发 chat.send / abort。
 */

import React, { useEffect, useRef, useState, useMemo, useCallback } from 'react';

import {
  Button,
  Dropdown,
  Input,
  Modal,
  message,
  Tag,
  Tooltip,
  Typography,
} from 'antd';

import {
  BranchesOutlined,
  BugOutlined,
  CloseOutlined,
  CodeOutlined,
  DownOutlined,
  FileOutlined,
  FolderAddOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  HistoryOutlined,
  LoadingOutlined,
  LockOutlined,
  PictureOutlined,
  ReadOutlined,
  RobotOutlined,
  StopFilled,
  ToolOutlined,
  DashboardOutlined,
} from '@ant-design/icons';
import { ArrowUp, Plus, Settings2 } from 'lucide-react';

import { useWs } from '../../App';
import GoalBar from './GoalBar';
import { useI18n } from '../../i18n';
import type { ChatAttachment, GitStatusSnapshot, SessionInfo, SystemStatusPayload } from '../../protocol';
import { useChatStore, type Segment, coalesceAssistantTurns, COMPOSE_DRAFT_SESSION_ID } from '../../stores/chatStore';
import { useConfigStore, type AppConfig } from '../../stores/configStore';
import { useSessionStore } from '../../stores/sessionStore';
import { sessionProjectDir, hasListedConversation } from './sessionSidebarUtils';
import { resolveVisiblePermissionMode, VISIBLE_PERMISSION_MODES, type VisiblePermissionMode } from '../../utils/permissionModes';
import ErrorBoundary from '../shell/ErrorBoundary';
import { FileAttachmentCard } from './FileAttachmentCard';
import { useProviderModels } from '../../modelCatalog';
import ComposerModelEffort, {
  type ReasoningEffort,
} from './ComposerModelEffort';
import { useStickToBottomScroll } from './useStickToBottomScroll';
import ComposerProjectPicker, { basenamePath } from './ComposerProjectPicker';
import {
  fsPathKey,
  normalizeRecentProjectPath,
} from '../../foundation/config/paths';
import RewindOverlay from './RewindOverlay';
import { StreamingTimeline } from './chatTimeline';
import MessageBubble from './MessageBubble';
import PermissionDialog from './PermissionDialog';
import QuestionDialog from './QuestionDialog';
import { Progress } from 'antd';
import { resolveTurnFooterUsage } from '../../utils/tokenUsage';

const RECENT_PROJECTS_KEY = 'xy.recentProjects';

type GitBranchListItem = { name: string; current: boolean };

type GitBranchCacheEntry = {
  branch: string;
  branches: GitBranchListItem[];
  isRepo: boolean;
};

/** 按项目目录缓存分支，避免 projectDir 先到、git.status 后到时闪「无分支」。 */
const gitBranchCacheByDir = new Map<string, GitBranchCacheEntry>();

/** 被谁调用：ChatView git 条。做什么：规范化分支名。 */
function isGitRepoBranchSentinel(value: string): boolean {
  return !value || value === 'not-a-git-repository' || value === 'not-a-git-worktree' || value.startsWith('not-a-git-');
}

function normalizeGitBranchName(raw: string | undefined): string {
  const value = (raw || '').trim();
  if (isGitRepoBranchSentinel(value)) return '';
  return value;
}

/** 被谁调用：ChatView。做什么：读本地分支缓存。 */
function readGitBranchCache(dir: string): GitBranchCacheEntry | undefined {
  const key = dir.trim();
  if (!key) return undefined;
  const entry = gitBranchCacheByDir.get(key);
  if (!entry) return undefined;
  if (isGitRepoBranchSentinel(entry.branch)) {
    return { ...entry, branch: '', isRepo: false };
  }
  return entry;
}

/** 被谁调用：ChatView。做什么：写本地分支缓存。 */
function writeGitBranchCache(dir: string, entry: GitBranchCacheEntry): void {
  const key = dir.trim();
  if (!key) return;
  gitBranchCacheByDir.set(key, entry);
}

/** 被谁调用：ChatView/项目选择。做什么：读最近项目列表（按路径键去重）。 */
function loadRecentProjects(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const item of parsed) {
      if (typeof item !== 'string' || !item.trim()) continue;
      const norm = normalizeRecentProjectPath(item);
      const key = fsPathKey(norm);
      if (!norm || !key || seen.has(key)) continue;
      seen.add(key);
      out.push(norm);
    }
    return out;
  } catch {
    return [];
  }
}

/** 合并服务端 xuanyu.json 与本地缓存的最近项目列表。 */
function mergeRecentProjects(serverDirs?: string[]): string[] {
  const local = loadRecentProjects();
  const seen = new Set<string>();
  const merged: string[] = [];
  for (const item of [...(serverDirs || []), ...local]) {
    const norm = normalizeRecentProjectPath(item);
    const key = fsPathKey(norm);
    if (!norm || !key || seen.has(key)) continue;
    seen.add(key);
    merged.push(norm);
    if (merged.length >= 12) break;
  }
  try {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(merged));
  } catch {
    // ignore
  }
  return merged;
}

/** 被谁调用：打开工作区后。做什么：写入最近项目。 */
function rememberProject(dir: string): void {
  const trimmed = normalizeRecentProjectPath(dir);
  const key = fsPathKey(trimmed);
  if (!trimmed || !key) return;
  const next = [
    trimmed,
    ...loadRecentProjects().filter((item) => fsPathKey(item) !== key),
  ].slice(0, 12);
  try {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next));
  } catch {
    // ignore quota / private mode
  }
}

/** 被谁调用：项目菜单。做什么：从最近列表移除。 */
function forgetProject(dir: string): void {
  const key = fsPathKey(dir);
  if (!key) return;
  const next = loadRecentProjects().filter((item) => fsPathKey(item) !== key);
  try {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next));
  } catch {
    // ignore
  }
}

const { Text } = Typography;
const { TextArea } = Input;

// Gateway-routed slash commands for composer auto-complete.
const COMMANDS = [
  { name: 'help', descKey: 'commands.help' },
  { name: 'compact', descKey: 'commands.compact' },
  { name: 'context', descKey: 'commands.context' },
  { name: 'goal', descKey: 'commands.goal' },
  { name: 'cost', descKey: 'commands.cost' },
] as const;

/** 被谁调用：ChatView 空态。做什么：展示欢迎引导与可点建议。 */
type HeroSuggestion = {
  id: string;
  icon: React.ReactNode;
  title: string;
  prompt: string;
};

function ChatHero({
  title,
  hint,
  loading,
  suggestions,
  onPickSuggestion,
}: {
  title: string;
  hint: string;
  loading?: boolean;
  suggestions?: HeroSuggestion[];
  onPickSuggestion?: (prompt: string) => void;
}) {
  return (
    <div className={`xy-chat-hero${loading ? ' is-loading' : ''}`}>
      <div className="xy-chat-hero-glow" aria-hidden />
      <div className="xy-chat-hero-inner">
        {loading ? (
          <div className="xy-chat-hero-mark is-loading">
            <LoadingOutlined spin />
          </div>
        ) : null}
        <header className="xy-chat-hero-header">
          <h2 className="xy-chat-hero-title">{title}</h2>
          <p className="xy-chat-hero-hint">{hint}</p>
        </header>
        {!loading && suggestions && suggestions.length > 0 ? (
          <div className="xy-chat-hero-suggestions" role="list">
            {suggestions.map((item) => (
              <button
                key={item.id}
                type="button"
                role="listitem"
                className="xy-chat-hero-suggestion"
                onClick={() => onPickSuggestion?.(item.prompt)}
              >
                <span className="xy-chat-hero-suggestion-icon" aria-hidden>
                  {item.icon}
                </span>
                <span className="xy-chat-hero-suggestion-title">{item.title}</span>
                <span className="xy-chat-hero-suggestion-prompt">{item.prompt}</span>
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

/** /goal 创建：乐观气泡只显示目标正文，避免 /goal 行与 continue 正文各一条。 */
function resolveOptimisticUserContent(text: string): string | null {
  if (!text.startsWith('/goal')) return text;
  const raw = text.slice('/goal'.length).trim();
  if (!raw) return null;
  const lower = raw.toLowerCase();
  if (lower === 'pause' || lower === 'resume' || lower === 'clear') return null;
  if (lower.startsWith('edit ') || lower.startsWith('budget ')) return null;
  let objective = raw;
  const budgetMatch = raw.match(/\s--budget\s+(\d+)\s*$/i);
  if (budgetMatch) objective = raw.slice(0, budgetMatch.index).trim();
  return objective || null;
}

/** 被谁调用：Layout 主聊天路由。做什么：单会话消息区 + composer + 权限/附件/项目选择；经 useWs 发 chat.send。 */
export default function ChatView() {
  const storeActiveSessionId = useChatStore((s) => s.activeSessionId);
  const composeDraft = useChatStore((s) => s.composeDraft);
  const { send, addHandler, mainSessionId } = useWs();
  const activeSessionId =
    storeActiveSessionId === COMPOSE_DRAFT_SESSION_ID
      ? COMPOSE_DRAFT_SESSION_ID
      : storeActiveSessionId || mainSessionId;
  const session = useChatStore((s) =>
    storeActiveSessionId ? s.sessions[storeActiveSessionId] : null,
  );
  const creatingDraftRef = useRef(false);
  const config = useConfigStore((s) => s.config);
  const capabilities = useConfigStore((s) => s.capabilities);
  const sessionList = useSessionStore((s) => s.sessions);
  const activeListSession = useMemo(
    () => sessionList.find((item) => item.id === storeActiveSessionId) || null,
    [sessionList, storeActiveSessionId],
  );
  const { t } = useI18n();
  const [input, setInput] = useState('');
  const [cmdHintIdx, setCmdHintIdx] = useState(-1);
  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort>('high');
  const [deepThinking, setDeepThinking] = useState(false);
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [rewindOpen, setRewindOpen] = useState(false);
  const [rewindTurnId, setRewindTurnId] = useState<string | undefined>();
  const [editingTurnId, setEditingTurnId] = useState<string | undefined>();
  const [editDraft, setEditDraft] = useState('');
  const [projectName, setProjectName] = useState('');
  const [projectDir, setProjectDir] = useState('');
  const [gitBranch, setGitBranch] = useState('');
  const [gitBranches, setGitBranches] = useState<GitBranchListItem[]>([]);
  /** false：尚未拿到当前目录的 git 探测结果（可用缓存先行展示） */
  const [gitStatusReady, setGitStatusReady] = useState(false);
  const [gitIsRepo, setGitIsRepo] = useState(false);
  const projectDirRef = useRef('');
  projectDirRef.current = projectDir;
  const [recentProjects, setRecentProjects] = useState<string[]>(() => loadRecentProjects());

  useEffect(() => {
    const serverRecent = config?.workspace?.recentDirs;
    const lastDir = config?.workspace?.lastDir;
    if (!serverRecent?.length && !lastDir) return;
    setRecentProjects(
      mergeRecentProjects([
        ...(lastDir ? [lastDir] : []),
        ...(serverRecent || []),
      ]),
    );
  }, [config?.workspace?.lastDir, config?.workspace?.recentDirs]);
  const [blankModalOpen, setBlankModalOpen] = useState(false);
  const [blankFolders, setBlankFolders] = useState<string[]>([]);
  const [blankPrimary, setBlankPrimary] = useState('');
  const [blankName, setBlankName] = useState('');
  const [composerHovered, setComposerHovered] = useState(false);
  const [composerFocused, setComposerFocused] = useState(false);
  const [composerMenuOpen, setComposerMenuOpen] = useState(false);
  const composerMenusRef = useRef(new Set<string>());
  const projectClearedRef = useRef(false);
  /** 用户手动改过 composer 项目时，不要被当前会话归属立刻写回。切会话后解除。 */
  const userOverrodeComposerRef = useRef(false);
  const lastComposerSessionRef = useRef<string | null>(null);
  const lastEscRef = useRef(0);
  const cmdListRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const {
    scrollerRef: messagesScrollRef,
    onScroll: onMessagesScroll,
    pinToBottom: pinMessagesToBottom,
  } = useStickToBottomScroll(
    [session?.messages, session?.streamingText, session?.reasoningText, session?.turnSegments, activeSessionId],
  );
  const mainCapability = capabilities.find((item) => item.task === 'main');
  const configModel = (config?.provider.model || '').trim();
  const capabilityModel = (mainCapability?.model || '').trim();
  const currentModel = configModel || capabilityModel;
  const lastModelRef = useRef(currentModel);
  if (currentModel) lastModelRef.current = currentModel;
  const displayModel = currentModel || lastModelRef.current;
  const {
    models: providerModels,
    loading: modelsLoading,
    refresh: refreshModels,
  } = useProviderModels({
    providerType: config?.provider.type || mainCapability?.provider,
    baseUrl: config?.provider.baseUrl,
    apiKey: config?.provider.apiKey,
    currentModel: displayModel,
    enabled: Boolean(config?.provider.type || mainCapability?.provider),
  });
  const projectLabel = basenamePath(projectDir) || projectName.replace(/_[0-9a-f]{8}$/i, '') || t('chat.projectUnknown');
  const allowClearProject = Boolean(
    composeDraft ||
      !activeListSession ||
      !sessionProjectDir(activeListSession),
  );
  const showGitBranch = Boolean(projectDir && gitIsRepo && (gitBranch || !gitStatusReady));
  const branchLabel = !gitStatusReady
    ? (gitBranch || t('chat.branchLoading'))
    : gitBranch || t('chat.branchUnknown');

  const modelOptions = providerModels;

  const branchMenuItems = useMemo(() => {
    if (gitBranches.length > 0) {
      return gitBranches.map((branch) => ({
        key: branch.name,
        label: branch.current ? `${branch.name} ✓` : branch.name,
      }));
    }
    // 列表未到但已知当前分支：不要闪「未检测到」
    if (gitBranch) {
      return [{ key: gitBranch, label: `${gitBranch} ✓` }];
    }
    if (!gitStatusReady) {
      return [{
        key: '__loading__',
        label: <span style={{ color: 'var(--xy-text-muted)' }}>{t('chat.branchLoading')}</span>,
        disabled: true,
      }];
    }
    return [{
      key: '__empty__',
      label: <span style={{ color: 'var(--xy-text-muted)' }}>{t('chat.branchEmpty')}</span>,
      disabled: true,
    }];
  }, [gitBranch, gitBranches, gitStatusReady, t]);

  // Filter commands based on input
  const cmdHints = useMemo(() => {
    if (!input.startsWith('/')) return [];
    const prefix = input.slice(1).toLowerCase();
    return COMMANDS.filter((c) => c.name.startsWith(prefix));
  }, [input]);

  // Reset hint index when hints change
  useEffect(() => { setCmdHintIdx(-1); }, [cmdHints.length]);

  // Auto-scroll selected command hint into view
  useEffect(() => {
    if (cmdHintIdx < 0 || !cmdListRef.current) return;
    const items = cmdListRef.current.querySelectorAll('.xy-cmd-item');
    items[cmdHintIdx]?.scrollIntoView({ block: 'nearest' });
  }, [cmdHintIdx]);

  const refreshGitStatus = useCallback(() => {
    if (!activeSessionId || activeSessionId === COMPOSE_DRAFT_SESSION_ID) return;
    send('git.status', { sessionId: activeSessionId });
    send('git.branches', { sessionId: activeSessionId });
  }, [activeSessionId, send]);

  useEffect(() => {
    if (!activeSessionId || activeSessionId === COMPOSE_DRAFT_SESSION_ID) return;
    send('system.status', {});
    refreshGitStatus();
  }, [activeSessionId, send, refreshGitStatus]);

  // 项目目录变更：先灌缓存避免闪空文案，再后台刷新
  useEffect(() => {
    if (!projectDir || projectClearedRef.current) {
      if (!projectDir) {
        setGitBranch('');
        setGitBranches([]);
        setGitStatusReady(false);
        setGitIsRepo(false);
      }
      return;
    }
    const cached = readGitBranchCache(projectDir);
    if (cached) {
      setGitBranch(cached.branch);
      setGitBranches(cached.branches);
      setGitIsRepo(cached.isRepo);
      setGitStatusReady(true);
    } else {
      setGitStatusReady(false);
      setGitBranch('');
      setGitBranches([]);
      setGitIsRepo(false);
    }
    if (activeSessionId) refreshGitStatus();
  }, [projectDir, activeSessionId, refreshGitStatus]);

  useEffect(() => {
    return addHandler((msg) => {
      if (msg.type === 'system.status' || msg.type === 'workspace.opened') {
        const payload = (msg.type === 'workspace.opened'
          ? (msg.payload as { status?: SystemStatusPayload; projectDir?: string; projectName?: string }).status
            || msg.payload
          : msg.payload) as SystemStatusPayload & { projectDir?: string; projectName?: string };
        const dir = normalizeRecentProjectPath(payload.projectDir || payload.runtime?.cwd || '');
        const name = basenamePath(dir) || payload.projectName || '';
        // composer 以「当前会话归属 / 用户手动选择」为准，禁止被 gateway cwd 脏写回。
        if (projectClearedRef.current || !dir) return;
        const draft = useChatStore.getState().composeDraft;
        const userPicked = userOverrodeComposerRef.current;
        if (!userPicked) {
          if (draft?.kind === 'recent') return;
          if (draft?.kind === 'project' && dir !== draft.projectDir) return;
          if (!draft) {
            const activeId = useChatStore.getState().activeSessionId;
            const listed = useSessionStore
              .getState()
              .sessions.find((item) => item.id === activeId);
            const sessionDir = listed ? sessionProjectDir(listed) : undefined;
            if (!sessionDir) return; // 无项目会话：保持 composer 为空
            if (dir !== sessionDir) return;
          }
        } else if (
          draft?.kind === 'recent' ||
          (!draft && useChatStore.getState().activeSessionId === COMPOSE_DRAFT_SESSION_ID)
        ) {
          // 用户在「新对话」准备态选了文件夹 → 同步为 project draft
          useChatStore.getState().beginComposeDraft({ kind: 'project', projectDir: dir });
        } else if (draft?.kind === 'project' && dir !== draft.projectDir) {
          useChatStore.getState().beginComposeDraft({ kind: 'project', projectDir: dir });
        }
        projectDirRef.current = dir;
        setProjectDir(dir);
        setProjectName(name);
        rememberProject(dir);
        setRecentProjects(loadRecentProjects());
        if (msg.type === 'workspace.opened') {
          refreshGitStatus();
        }
      }
      if (msg.type === 'git.status' || msg.type === 'git.checkout') {
        const payload = msg.payload as { status?: GitStatusSnapshot; branch?: string };
        const rawBranch = payload.status?.branch || payload.branch || '';
        const isRepo = !isGitRepoBranchSentinel(rawBranch.trim());
        const nextBranch = normalizeGitBranchName(rawBranch);
        setGitIsRepo(isRepo);
        setGitBranch(nextBranch);
        if (!isRepo) {
          setGitBranches([]);
        } else {
          setGitBranches((prev) => {
            if (!nextBranch) return prev;
            if (prev.some((item) => item.name === nextBranch)) {
              return prev.map((item) => ({
                ...item,
                current: item.name === nextBranch,
              }));
            }
            return prev.length === 0
              ? [{ name: nextBranch, current: true }]
              : prev;
          });
        }
        setGitStatusReady(true);
        const dir = projectDirRef.current;
        if (dir) {
          const prevCache = readGitBranchCache(dir);
          writeGitBranchCache(dir, {
            branch: nextBranch,
            branches: isRepo && nextBranch
              ? (prevCache?.branches?.length
                ? prevCache.branches.map((item) => ({
                    ...item,
                    current: item.name === nextBranch,
                  }))
                : [{ name: nextBranch, current: true }])
              : [],
            isRepo,
          });
        }
      }
      if (msg.type === 'git.branches') {
        const payload = msg.payload as {
          branches?: GitBranchListItem[];
          error?: string;
        };
        const branches = payload.branches || [];
        setGitBranches(branches);
        const current = branches.find((item) => item.current);
        if (current?.name) {
          setGitBranch(current.name);
          setGitIsRepo(true);
        } else if (payload.error) {
          setGitIsRepo(false);
          setGitBranch('');
        }
        setGitStatusReady(true);
        const dir = projectDirRef.current;
        if (dir) {
          const prevCache = readGitBranchCache(dir);
          writeGitBranchCache(dir, {
            branch: current?.name || prevCache?.branch || '',
            branches,
            isRepo: prevCache?.isRepo !== false,
          });
        }
      }
    });
  }, [addHandler, refreshGitStatus]);

  // 仅显示「思考中」且长时间无任何流式内容时，自动解除卡死状态（常见于事件订阅丢失）。
  useEffect(() => {
    if (!activeSessionId || !session?.isProcessing) return;
    const hasStreamActivity =
      !!session.streamingText.trim() ||
      !!session.reasoningText.trim() ||
      session.turnSegments.length > 0;
    if (hasStreamActivity) return;
    const timer = window.setTimeout(() => {
      const current = useChatStore.getState().getSession(activeSessionId);
      if (!current.isProcessing) return;
      const stillIdle =
        !current.streamingText.trim() &&
        !current.reasoningText.trim() &&
        current.turnSegments.length === 0;
      if (!stillIdle) return;
      useChatStore.getState().syncProcessingState(activeSessionId, false);
      // 引擎可能已写完回合但事件未送达：拉一次历史，避免界面空白却其实已有回复。
      send('session.restore', { sessionId: activeSessionId });
      useChatStore.getState().addErrorMessage(
        activeSessionId,
        t('chat.processingTimeout'),
        { title: t('chat.processingTimeoutTitle') },
      );
    }, 90_000);
    return () => window.clearTimeout(timer);
  }, [
    activeSessionId,
    session?.isProcessing,
    session?.streamingText,
    session?.reasoningText,
    session?.turnSegments.length,
    send,
    t,
  ]);

  const openRewind = useCallback((turnId?: string) => {
    if (!activeSessionId) return;
    setRewindTurnId(turnId);
    setRewindOpen(true);
  }, [activeSessionId]);

  const handleSelectProject = useCallback((dir: string) => {
    if (!dir || dir === '__empty__') return;
    const normalized = normalizeRecentProjectPath(dir);
    if (!normalized) return;
    userOverrodeComposerRef.current = true;
    projectClearedRef.current = false;
    projectDirRef.current = normalized;
    setProjectDir(normalized);
    setProjectName(basenamePath(normalized));
    rememberProject(normalized);
    setRecentProjects(loadRecentProjects());

    const chat = useChatStore.getState();
    const active = chat.activeSessionId;
    const listed = active
      ? useSessionStore.getState().sessions.find((item) => item.id === active)
      : undefined;
    const emptyTarget =
      !!chat.composeDraft ||
      !active ||
      active === COMPOSE_DRAFT_SESSION_ID ||
      !listed ||
      !hasListedConversation(listed);

    // 空白/准备态选项目：首条消息归入该项目；已有对话则绑定到当前会话。
    if (emptyTarget) {
      chat.beginComposeDraft({ kind: 'project', projectDir: normalized });
      send('workspace.open', { path: normalized });
      return;
    }

    if (normalized === projectDir) {
      refreshGitStatus();
      return;
    }
    send('workspace.open', { path: normalized, sessionId: active });
  }, [activeSessionId, projectDir, refreshGitStatus, send]);

  const handleClearProject = useCallback(() => {
    const sid = activeSessionId;
    if (sid && sid !== COMPOSE_DRAFT_SESSION_ID) {
      const listed = useSessionStore.getState().sessions.find((s) => s.id === sid);
      if (listed && sessionProjectDir(listed)) {
        // 已绑定项目的正式会话不允许半途解绑（会从「项目」掉到「最近」）。
        return;
      }
    }
    userOverrodeComposerRef.current = true;
    projectClearedRef.current = true;
    projectDirRef.current = '';
    setProjectDir('');
    setProjectName('');
    setGitBranch('');
    setGitBranches([]);
    setGitStatusReady(false);
    setGitIsRepo(false);
    const draft = useChatStore.getState().composeDraft;
    if (draft?.kind === 'project') {
      useChatStore.getState().setComposeDraft({ kind: 'recent' });
    }
    if (sid && sid !== COMPOSE_DRAFT_SESSION_ID) {
      send('workspace.clear', { sessionId: sid });
      useSessionStore.getState().updateSession(sid, {
        projectDir: undefined,
        metadata: (() => {
          const listed = useSessionStore.getState().sessions.find((s) => s.id === sid);
          const meta = { ...(listed?.metadata || {}) };
          delete meta.projectDir;
          delete meta.workspace;
          return meta;
        })(),
      });
    }
  }, [activeSessionId, send]);

  /** 会话 ↔ composer：切到哪条会话，项目栏跟哪条的 projectDir；无则清空。 */
  useEffect(() => {
    if (composeDraft) return;
    if (!storeActiveSessionId || storeActiveSessionId === COMPOSE_DRAFT_SESSION_ID) return;
    if (lastComposerSessionRef.current !== storeActiveSessionId) {
      lastComposerSessionRef.current = storeActiveSessionId;
      userOverrodeComposerRef.current = false;
    }
    if (userOverrodeComposerRef.current) return;
    if (!activeListSession) return;
    const dir = sessionProjectDir(activeListSession);
    if (dir) {
      if (projectDirRef.current === dir && !projectClearedRef.current) return;
      projectClearedRef.current = false;
      projectDirRef.current = dir;
      setProjectDir(dir);
      setProjectName(basenamePath(dir));
      return;
    }
    if (projectClearedRef.current && !projectDirRef.current) return;
    projectClearedRef.current = true;
    projectDirRef.current = '';
    setProjectDir('');
    setProjectName('');
    setGitBranch('');
    setGitBranches([]);
    setGitStatusReady(false);
    setGitIsRepo(true);
  }, [
    composeDraft,
    storeActiveSessionId,
    activeListSession,
    activeListSession?.id,
    activeListSession?.projectDir,
    activeListSession?.metadata?.projectDir,
  ]);

  useEffect(() => {
    if (composeDraft?.kind === 'project') {
      userOverrodeComposerRef.current = false;
      projectClearedRef.current = false;
      projectDirRef.current = composeDraft.projectDir;
      setProjectDir(composeDraft.projectDir);
      setProjectName(basenamePath(composeDraft.projectDir));
      return;
    }
    if (composeDraft?.kind === 'recent') {
      userOverrodeComposerRef.current = false;
      handleClearProject();
      userOverrodeComposerRef.current = false;
    }
  }, [composeDraft, handleClearProject]);

  const ensureSessionForSend = useCallback(async (): Promise<{
    sessionId: string;
    mode: 'branch' | 'recent';
    projectDir?: string;
  } | null> => {
    const draft = useChatStore.getState().composeDraft;
    const current = useChatStore.getState().activeSessionId;
    // 有 compose 草稿时一律新建；只有已落定的真实会话才复用。
    if (current && current !== COMPOSE_DRAFT_SESSION_ID && !draft) {
      const listed = useSessionStore.getState().sessions.find((item) => item.id === current);
      const sessionDir = listed ? sessionProjectDir(listed) : undefined;
      return {
        sessionId: current,
        mode: sessionDir ? 'branch' : 'recent',
        projectDir: sessionDir,
      };
    }

    // 唯一规则：composer 当前有项目路径 → 进该项目；否则进「最近」。
    const composerDir =
      !projectClearedRef.current && projectDirRef.current.trim()
        ? projectDirRef.current.trim()
        : '';
    const mode: 'branch' | 'recent' = composerDir ? 'branch' : 'recent';
    const projectDirForCreate = composerDir || undefined;

    return new Promise((resolve, reject) => {
      const timer = window.setTimeout(() => {
        unsub();
        reject(new Error('session.create timeout'));
      }, 20000);
      const unsub = addHandler((msg: { type: string; payload: unknown }) => {
        if (msg.type !== 'session.created') return;
        const payload = msg.payload as { sessionId?: string; mode?: string };
        if (!payload.sessionId || payload.mode !== mode) return;
        // branch/recent 必须落到新建线程；若仍是 main 空壳则忽略（等真正的 web_*）。
        if (payload.sessionId === 'main' || payload.sessionId.startsWith('main-')) return;
        window.clearTimeout(timer);
        unsub();
        useChatStore.getState().setComposeDraft(null);
        resolve({
          sessionId: payload.sessionId,
          mode,
          projectDir: mode === 'branch' ? projectDirForCreate : undefined,
        });
      });
      if (mode === 'branch' && projectDirForCreate) {
        send('session.create', { mode: 'branch', projectDir: projectDirForCreate });
      } else {
        send('session.create', { mode: 'recent' });
      }
    });
  }, [addHandler, send]);

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text && attachments.length === 0) return;
    if (creatingDraftRef.current) return;
    if (text === '/rewind' || text.startsWith('/rewind ')) {
      setInput('');
      openRewind();
      return;
    }

    const draftBefore = useChatStore.getState().composeDraft;
    // 准备态 / 草稿必须新建；不要把消息打进当前历史会话。
    const needsCreate =
      !!draftBefore ||
      useChatStore.getState().activeSessionId === COMPOSE_DRAFT_SESSION_ID;

    let sessionId = useChatStore.getState().activeSessionId || mainSessionId;
    let createdMode: 'branch' | 'recent' | null = null;
    let createdProjectDir: string | undefined;
    if (needsCreate) {
      creatingDraftRef.current = true;
      try {
        const created = await ensureSessionForSend();
        if (!created) {
          creatingDraftRef.current = false;
          return;
        }
        sessionId = created.sessionId;
        createdMode = created.mode;
        createdProjectDir = created.projectDir;
      } catch {
        message.error(t('sessionSidebar.createFailed', '创建会话失败，请重试'));
        creatingDraftRef.current = false;
        return;
      }
      creatingDraftRef.current = false;
    }

    if (!sessionId || sessionId === COMPOSE_DRAFT_SESSION_ID) return;

    const optimistic = resolveOptimisticUserContent(text);
    if (optimistic) {
      useChatStore.getState().addUserMessage(
        sessionId,
        optimistic,
        attachments.length > 0 ? attachments : undefined,
      );
    }
    // 首条消息即视为可列表会话；归属只跟创建模式走，不用 composer 残留路径。
    const listPatch: Partial<SessionInfo> = {
      messageCount: Math.max(
        1,
        useSessionStore.getState().sessions.find((s) => s.id === sessionId)?.messageCount || 0,
      ),
      lastMessageAt: Date.now(),
      updatedAt: Date.now(),
    };
    if (createdMode === 'branch' && createdProjectDir) {
      listPatch.projectDir = createdProjectDir;
      listPatch.metadata = { projectDir: createdProjectDir, workspace: true };
    } else if (createdMode === 'recent' || draftBefore?.kind === 'recent') {
      listPatch.projectDir = undefined;
      listPatch.metadata = {};
    }
    useSessionStore.getState().updateSession(sessionId, listPatch);
    send('chat.send', {
      sessionId,
      content: text,
      deepThinking,
      attachments: attachments.length > 0 ? attachments : undefined,
    });
    setInput('');
    setAttachments([]);
    pinMessagesToBottom();
  }, [
    input,
    attachments,
    mainSessionId,
    send,
    deepThinking,
    openRewind,
    pinMessagesToBottom,
    ensureSessionForSend,
    t,
  ]);

  const handleRemoveRecentProject = useCallback((dir: string) => {
    forgetProject(dir);
    if (fsPathKey(dir) === fsPathKey(projectDir)) {
      handleClearProject();
    }
    const nextRecent = loadRecentProjects();
    setRecentProjects(nextRecent);
    const prevLast = normalizeRecentProjectPath(config?.workspace?.lastDir);
    const clearLast = !!prevLast && fsPathKey(prevLast) === fsPathKey(dir);
    useConfigStore.getState().updateConfig({
      workspace: {
        lastDir: clearLast ? '' : prevLast || '',
        recentDirs: nextRecent,
      },
    });
  }, [config?.workspace?.lastDir, handleClearProject, projectDir]);

  const pickSourceFolderForBlank = useCallback(() => {
    const unsub = addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type !== 'workspace.folderPicked') return;
      unsub();
      const payload = msg.payload as {
        path?: string | null;
        cancelled?: boolean;
        error?: string;
      };
      if (payload.path) {
        const next = normalizeRecentProjectPath(payload.path);
        if (!next) return;
        setBlankFolders((prev) => {
          if (prev.some((dir) => fsPathKey(dir) === fsPathKey(next))) return prev;
          return [...prev, next];
        });
        setBlankPrimary((prev) => prev || next);
        return;
      }
      if (payload.error) {
        message.warning(t('chat.projectPicker.browseFailed', '无法打开系统文件夹对话框'));
      }
    });
    send('workspace.pickFolder', {
      title: t('chat.projectPicker.pickParent', '选择源文件夹'),
      initialDir: blankPrimary || projectDir || recentProjects[0] || undefined,
    });
  }, [addHandler, blankPrimary, projectDir, recentProjects, send, t]);

  const handleNewBlankProject = useCallback(() => {
    setBlankName('');
    setBlankFolders([]);
    setBlankPrimary('');
    setBlankModalOpen(true);
  }, []);

  const removeBlankFolder = useCallback((dir: string) => {
    const key = fsPathKey(dir);
    setBlankFolders((prev) => {
      const next = prev.filter((item) => fsPathKey(item) !== key);
      setBlankPrimary((primary) => {
        if (fsPathKey(primary) !== key) return primary;
        return next[0] || '';
      });
      return next;
    });
  }, []);

  const setBlankFolderPrimary = useCallback((dir: string) => {
    const key = fsPathKey(dir);
    setBlankPrimary(dir);
    setBlankFolders((prev) => {
      const rest = prev.filter((item) => fsPathKey(item) !== key);
      const hit = prev.find((item) => fsPathKey(item) === key);
      return hit ? [hit, ...rest] : prev;
    });
  }, []);

  const handleOpenExistingFolder = useCallback(() => {
    const unsub = addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type !== 'workspace.folderPicked') return;
      unsub();
      const payload = msg.payload as {
        path?: string | null;
        cancelled?: boolean;
        error?: string;
      };
      if (payload.path) {
        handleSelectProject(payload.path);
        return;
      }
      if (payload.error) {
        message.warning(t('chat.projectPicker.browseFailed', '无法打开系统文件夹对话框'));
      }
    });
    send('workspace.pickFolder', {
      title: t('chat.projectPicker.useFolder'),
      initialDir: projectDir || recentProjects[0] || undefined,
    });
  }, [addHandler, handleSelectProject, projectDir, recentProjects, send, t]);

  const submitBlankProject = useCallback(() => {
    const parentDir = blankPrimary.trim();
    const name = blankName.trim();
    if (!parentDir || !name) {
      message.warning(t('chat.projectPicker.blankRequired'));
      return;
    }
    userOverrodeComposerRef.current = true;
    projectClearedRef.current = false;
    setBlankModalOpen(false);
    const unsub = addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type !== 'workspace.opened') return;
      unsub();
      const payload = msg.payload as { projectDir?: string };
      const createdDir = payload.projectDir?.trim();
      if (createdDir) {
        handleSelectProject(createdDir);
      }
    });
    window.setTimeout(() => unsub(), 20000);
    const bindSessionId =
      activeSessionId && activeSessionId !== COMPOSE_DRAFT_SESSION_ID
        ? activeSessionId
        : undefined;
    send('workspace.create', { parentDir, name, sessionId: bindSessionId });
  }, [activeSessionId, addHandler, blankPrimary, blankName, handleSelectProject, send, t]);

  const handleSelectBranch = useCallback((branch: string) => {
    if (!branch || branch === '__empty__') return;
    if (branch === gitBranch) return;
    send('git.checkout', { sessionId: activeSessionId, branch });
  }, [activeSessionId, gitBranch, send]);

  const refreshBranches = useCallback(() => {
    if (!activeSessionId || !gitIsRepo) return;
    send('git.branches', { sessionId: activeSessionId });
  }, [activeSessionId, gitIsRepo, send]);

  const handleModelChange = useCallback((model: string) => {
    if (!config || !model || model === config.provider.model) return;
    const nextProvider = { ...config.provider, model };
    send('config.update', { config: { provider: nextProvider } });
    useConfigStore.getState().updateConfig({ provider: nextProvider });
    message.success(t('chat.modelUpdated', undefined, model));
  }, [config, send, t]);

  const timelineMessages = useMemo(
    () => (session ? coalesceAssistantTurns(session.messages) : []),
    [session],
  );

  const lastEditableUser = useMemo(() => {
    if (!session || session.isProcessing) return null;
    for (let i = timelineMessages.length - 1; i >= 0; i--) {
      const msg = timelineMessages[i];
      if (msg.role !== 'user') continue;
      if (msg.attachments && msg.attachments.length > 0) return null;
      if (!msg.turnId) return null;
      return { index: i, turnId: msg.turnId, content: msg.content || '' };
    }
    return null;
  }, [session, timelineMessages]);

  const handleStartInlineEdit = useCallback(() => {
    if (!lastEditableUser) return;
    setEditingTurnId(lastEditableUser.turnId);
    setEditDraft(lastEditableUser.content);
  }, [lastEditableUser]);

  const handleCancelInlineEdit = useCallback(() => {
    setEditingTurnId(undefined);
    setEditDraft('');
  }, []);

  const handleSubmitInlineEdit = useCallback(() => {
    const text = editDraft.trim();
    if (!activeSessionId || !editingTurnId || !text) return;
    useChatStore.getState().prepareInlineRewind(activeSessionId, editingTurnId);
    send('session.rewind', {
      sessionId: activeSessionId,
      turnId: editingTurnId,
      mode: 'both',
      newContent: text,
    });
    setEditingTurnId(undefined);
    setEditDraft('');
  }, [activeSessionId, editingTurnId, editDraft, send]);

  useEffect(() => {
    if (session?.isProcessing && editingTurnId) {
      setEditingTurnId(undefined);
      setEditDraft('');
    }
  }, [session?.isProcessing, editingTurnId]);

  useEffect(() => {
    if (!activeSessionId || rewindOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      const target = e.target as HTMLElement | null;
      if (target?.closest('input, textarea, [contenteditable="true"]')) return;
      const now = Date.now();
      if (now - lastEscRef.current < 450) {
        e.preventDefault();
        openRewind();
        lastEscRef.current = 0;
      } else {
        lastEscRef.current = now;
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [activeSessionId, rewindOpen, openRewind]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Command hint navigation
    if (cmdHints.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setCmdHintIdx((prev) => Math.min(prev + 1, cmdHints.length - 1));
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setCmdHintIdx((prev) => Math.max(prev - 1, 0));
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        const idx = cmdHintIdx >= 0 ? cmdHintIdx : 0;
        setInput(`/${cmdHints[idx].name} `);
        setCmdHintIdx(-1);
        return;
      }
      if (e.key === 'Escape') {
        setCmdHintIdx(-1);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handlePickSuggestion = useCallback((prompt: string) => {
    setInput(prompt);
    setCmdHintIdx(-1);
    window.requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      const len = prompt.length;
      el.setSelectionRange(len, len);
    });
  }, []);

  const heroSuggestions = useMemo<HeroSuggestion[]>(
    () => [
      {
        id: 'explore',
        icon: <FolderOpenOutlined />,
        title: t('chat.heroSuggestExploreTitle'),
        prompt: t('chat.heroSuggestExplorePrompt'),
      },
      {
        id: 'debug',
        icon: <BugOutlined />,
        title: t('chat.heroSuggestDebugTitle'),
        prompt: t('chat.heroSuggestDebugPrompt'),
      },
      {
        id: 'implement',
        icon: <CodeOutlined />,
        title: t('chat.heroSuggestImplementTitle'),
        prompt: t('chat.heroSuggestImplementPrompt'),
      },
      {
        id: 'explain',
        icon: <ReadOutlined />,
        title: t('chat.heroSuggestExplainTitle'),
        prompt: t('chat.heroSuggestExplainPrompt'),
      },
    ],
    [t],
  );

  const handleCompact = () => {
    if (!activeSessionId) return;
    send('context.compact', { sessionId: activeSessionId, strategy: 'autocompact', extractMemories: true });
  };

  const MAX_FILE_SIZE = 10 * 1024 * 1024; // 10MB

  const handleFileSelect = useCallback((kind: 'image' | 'file') => {
    const input = kind === 'image' ? imageInputRef.current : fileInputRef.current;
    if (!input) return;
    input.onchange = () => {
      const files = input.files;
      if (!files || files.length === 0) return;
      const file = files[0];
      if (file.size > MAX_FILE_SIZE) {
        message.warning(t('chat.fileTooLarge'));
        input.value = '';
        return;
      }
      if (kind === 'image' && !mainCapability?.capabilities.vision) {
        message.warning(t('chat.visionNotSupported'));
        input.value = '';
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        const dataUrl = reader.result as string;
        setAttachments((prev) => [...prev, {
          kind,
          name: file.name,
          mimeType: file.type || 'application/octet-stream',
          sizeBytes: file.size,
          dataUrl,
        }]);
        input.value = '';
      };
      reader.readAsDataURL(file);
    };
    input.click();
  }, [mainCapability, t]);

  const handleRemoveAttachment = useCallback((index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleAttachmentPreview = useCallback((src: string) => {
    window.dispatchEvent(new CustomEvent('xy-image-preview', { detail: { src } }));
  }, []);

  const setComposerMenu = useCallback((id: string, open: boolean) => {
    const next = composerMenusRef.current;
    if (open) next.add(id);
    else next.delete(id);
    setComposerMenuOpen(next.size > 0);
  }, []);

  const permissionMode = toAgentMode(config?.permissions.mode);
  /** 悬停 / 输入焦点 / 菜单打开时展开后方 chrome，避免输入时边框突然错位 */
  const chromeOpen = composerHovered || composerMenuOpen;

  const handleModeChange = (mode: AgentMode) => {
    if (!config) return;
    const permissions = { ...config.permissions, mode };
    send('config.update', { config: { permissions } });
    useConfigStore.getState().updateConfig({ permissions });
  };

  return (
    <div className="xy-chat-root">
      <div className="xy-chat-area" style={{ flex: 1 }}>
        {!activeSessionId || !session ? (
          <ChatHero
            loading
            title={t('chat.connecting')}
            hint={t('chat.establishing')}
          />
        ) : (
          <>
            <div className="xy-chat-main">
              <div
                ref={messagesScrollRef}
                onScroll={onMessagesScroll}
                className={`xy-messages${
                  session.messages.length === 0 && !session.isProcessing ? ' is-landing' : ''
                }`}
              >
                {session.messages.length === 0 && !session.isProcessing && (
                  <ChatHero
                    title={t('chat.empty')}
                    hint={t('chat.emptyHint')}
                    suggestions={heroSuggestions}
                    onPickSuggestion={handlePickSuggestion}
                  />
                )}

                {timelineMessages.map((msg, i) => {
                  const isLastEditable =
                    lastEditableUser?.index === i && Boolean(lastEditableUser.turnId);
                  const isEditing =
                    isLastEditable && editingTurnId === lastEditableUser?.turnId;
                  return (
                  <ErrorBoundary key={msg.turnId ? `${msg.role}-${msg.turnId}-${i}` : i}>
                    <MessageBubble
                      message={msg}
                      canInlineEdit={isLastEditable}
                      isEditing={isEditing}
                      editDraft={isEditing ? editDraft : undefined}
                      onStartEdit={isLastEditable ? handleStartInlineEdit : undefined}
                      onCancelEdit={isEditing ? handleCancelInlineEdit : undefined}
                      onEditDraftChange={isEditing ? setEditDraft : undefined}
                      onSubmitEdit={isEditing ? handleSubmitInlineEdit : undefined}
                      sessionId={activeSessionId}
                      turnUsage={resolveTurnFooterUsage(timelineMessages, i)}
                    />
                  </ErrorBoundary>
                  );
                })}

                {session.isProcessing && (
                  <StreamingTimeline session={session} sessionId={activeSessionId} />
                )}

                {session.pendingPermission && (
                  <PermissionDialog
                    pending={session.pendingPermission}
                    onRespond={(allowed, remember) => {
                      send('permission.respond', {
                        sessionId: activeSessionId,
                        requestId: session.pendingPermission!.requestId,
                        approvalDigest: session.pendingPermission!.approvalDigest,
                        allowed,
                        remember,
                      });
                      useChatStore.getState().setPendingPermission(activeSessionId, null);
                    }}
                  />
                )}
                {session.pendingQuestion && (
                  <QuestionDialog
                    pending={session.pendingQuestion}
                    onRespond={(answers) => {
                      send('question.respond', {
                        sessionId: activeSessionId,
                        requestId: session.pendingQuestion!.requestId,
                        answers,
                      });
                      useChatStore.getState().setPendingQuestion(activeSessionId, null);
                    }}
                  />
                )}
              </div>

              <div className="xy-input-wrap">
                {activeSessionId ? <GoalBar sessionId={activeSessionId} /> : null}
                <div
                  className={`xy-composer-stack${chromeOpen ? ' is-chrome-open' : ''}`}
                  onMouseEnter={() => setComposerHovered(true)}
                  onMouseLeave={() => setComposerHovered(false)}
                  onFocusCapture={() => setComposerFocused(true)}
                  onBlurCapture={(e) => {
                    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                      setComposerFocused(false);
                    }
                  }}
                >
                  <div
                    className="xy-composer-chrome"
                    aria-label={t('chat.workspaceBar')}
                  >
                    <ComposerProjectPicker
                      projectDir={projectDir}
                      projectLabel={projectLabel}
                      recentProjects={recentProjects}
                      onSelect={handleSelectProject}
                      onRemoveRecent={handleRemoveRecentProject}
                      onClearProject={handleClearProject}
                      allowClear={allowClearProject}
                      onNewBlank={handleNewBlankProject}
                      onOpenFolder={handleOpenExistingFolder}
                      onMenuOpenChange={(open) => setComposerMenu('project', open)}
                    />
                    {showGitBranch ? (
                      <>
                        <span className="xy-composer-chrome-sep" aria-hidden />
                        <Dropdown
                          menu={{
                            items: branchMenuItems,
                            selectedKeys: gitBranch ? [gitBranch] : [],
                            onClick: ({ key }) => handleSelectBranch(String(key)),
                          }}
                          trigger={['click']}
                          placement="topLeft"
                          onOpenChange={(open) => {
                            setComposerMenu('branch', open);
                            if (open) refreshBranches();
                          }}
                        >
                          <button type="button" className="xy-composer-chrome-item" title={branchLabel}>
                            <BranchesOutlined />
                            <span className="xy-composer-chrome-label">{branchLabel}</span>
                          </button>
                        </Dropdown>
                      </>
                    ) : null}
                  </div>

                  <Modal
                    className="xy-create-project-modal"
                    title={t('chat.projectPicker.createTitle', '创建项目')}
                    open={blankModalOpen}
                    onOk={submitBlankProject}
                    onCancel={() => setBlankModalOpen(false)}
                    okText={t('chat.projectPicker.createConfirm', '创建项目')}
                    cancelText={t('common.cancel')}
                    destroyOnHidden
                    centered
                    width={520}
                    okButtonProps={{
                      disabled: !blankPrimary.trim() || !blankName.trim(),
                      className: 'xy-create-project-ok',
                    }}
                  >
                    <div className="xy-create-project">
                      <Input
                        className="xy-create-project-name"
                        prefix={<FolderOutlined />}
                        value={blankName}
                        onChange={(e) => setBlankName(e.target.value)}
                        placeholder={t('chat.projectPicker.namePlaceholder')}
                        onPressEnter={submitBlankProject}
                        autoFocus
                      />
                      <div className="xy-create-project-source">
                        <div className="xy-create-project-source-head">
                          <span>{t('chat.projectPicker.sourceFolder', '源文件夹')}</span>
                        </div>
                        <div className="xy-create-project-source-box">
                          {blankFolders.length === 0 ? (
                            <div className="xy-create-project-empty">
                              <button
                                type="button"
                                className="xy-create-project-source-hint"
                                onClick={pickSourceFolderForBlank}
                              >
                                <span>{t('chat.projectPicker.addFolderOnPc', '在此电脑上添加文件夹')}</span>
                                <DownOutlined className="xy-create-project-source-hint-caret" />
                              </button>
                              <button
                                type="button"
                                className="xy-create-project-add"
                                onClick={pickSourceFolderForBlank}
                              >
                                <FolderAddOutlined />
                                <span>{t('chat.projectPicker.addFolder', '添加')}</span>
                              </button>
                            </div>
                          ) : (
                            <>
                              {blankFolders.map((dir) => {
                                const isPrimary = fsPathKey(dir) === fsPathKey(blankPrimary);
                                return (
                                  <div
                                    key={fsPathKey(dir) || dir}
                                    className={`xy-create-project-source-row${isPrimary ? ' is-primary' : ''}`}
                                  >
                                    <FolderOutlined className="xy-create-project-folder-icon" />
                                    <span className="xy-create-project-source-path" title={dir}>
                                      {basenamePath(dir) || dir}
                                    </span>
                                    {isPrimary ? (
                                      <span className="xy-create-project-primary-badge">
                                        {t('chat.projectPicker.primary', '主要')}
                                      </span>
                                    ) : (
                                      <button
                                        type="button"
                                        className="xy-create-project-set-primary"
                                        onClick={() => setBlankFolderPrimary(dir)}
                                      >
                                        {t('chat.projectPicker.setPrimary', '设为主要')}
                                      </button>
                                    )}
                                    <button
                                      type="button"
                                      className="xy-create-project-source-clear"
                                      aria-label={t('chat.projectPicker.remove')}
                                      onClick={() => removeBlankFolder(dir)}
                                    >
                                      <CloseOutlined />
                                    </button>
                                  </div>
                                );
                              })}
                              <button
                                type="button"
                                className="xy-create-project-add-row"
                                onClick={pickSourceFolderForBlank}
                              >
                                <FolderAddOutlined />
                                <span>{t('chat.projectPicker.addFolderRow', '添加文件夹')}</span>
                              </button>
                            </>
                          )}
                        </div>
                      </div>
                    </div>
                  </Modal>

                  <div className="xy-composer" style={{ position: 'relative' }}>
                  {cmdHints.length > 0 && (
                    <div className="xy-cmd-list" ref={cmdListRef}>
                      {cmdHints.map((cmd, i) => (
                        <div
                          key={cmd.name}
                          className={`xy-cmd-item ${i === cmdHintIdx ? 'is-active' : ''}`}
                          onClick={() => { setInput(`/${cmd.name} `); setCmdHintIdx(-1); }}
                          onMouseEnter={() => setCmdHintIdx(i)}
                        >
                          <span className="xy-cmd-name">/{cmd.name}</span>
                          <span className="xy-cmd-desc">{t(cmd.descKey)}</span>
                        </div>
                      ))}
                    </div>
                  )}
                  {/* Attachment previews */}
                  {attachments.length > 0 && (
                    <div className="xy-attachment-preview">
                      {attachments.map((att, i) => (
                        att.kind === 'image' ? (
                          <div key={i} className="xy-attachment-thumb-wrap">
                            <img
                              src={att.dataUrl}
                              alt=""
                              className="xy-attachment-thumb xy-attachment-thumb-is-image"
                              onClick={() => handleAttachmentPreview(att.dataUrl)}
                            />
                            <button
                              type="button"
                              className="xy-attachment-remove-btn"
                              aria-label={t('chat.removeAttachment')}
                              onClick={(e) => {
                                e.stopPropagation();
                                handleRemoveAttachment(i);
                              }}
                            >
                              <CloseOutlined />
                            </button>
                          </div>
                        ) : (
                          <FileAttachmentCard
                            key={i}
                            name={att.name}
                            mimeType={att.mimeType}
                            sizeBytes={att.sizeBytes}
                            className="xy-file-attachment-card-composer"
                            removeLabel={t('chat.removeAttachment')}
                            onRemove={() => handleRemoveAttachment(i)}
                          />
                        )
                      ))}
                      <div className="xy-attachment-add-btn" onClick={() => handleFileSelect('image')}>
                        <Plus size={16} strokeWidth={2} />
                      </div>
                    </div>
                  )}
                  <TextArea
                    ref={inputRef}
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder={t('chat.placeholder')}
                    autoSize={{ minRows: 1, maxRows: 5 }}
                    variant="borderless"
                    style={{ flex: 1, background: 'transparent', fontSize: 14 }}
                  />
                  <div className="xy-composer-actions">
                    <div className="xy-composer-tools">
                      <Dropdown
                        menu={{
                          items: VISIBLE_PERMISSION_MODES.map((mode) => ({
                            key: mode,
                            label: t(`permission.${mode}`),
                          })),
                          onClick: ({ key }) => handleModeChange(key as AgentMode),
                          style: { minWidth: 100 },
                        }}
                        trigger={['click']}
                      >
                        <Button
                          type="text"
                          className={`xy-tool-dropdown-btn${permissionMode === 'plan' ? ' is-plan' : ''}`}
                          title={modeTooltip(permissionMode, t)}
                        >
                          <Settings2 size={15} strokeWidth={1.75} />
                          <span>{t(`permission.${permissionMode}`)}</span>
                        </Button>
                      </Dropdown>
                    </div>
                    <div className="xy-composer-right">
                      <ComposerModelEffort
                        model={displayModel}
                        models={modelOptions}
                        modelsLoading={modelsLoading}
                        onRefreshModels={refreshModels}
                        effort={reasoningEffort}
                        deepThinking={deepThinking}
                        onModelChange={handleModelChange}
                        onEffortChange={setReasoningEffort}
                        onDeepThinkingChange={setDeepThinking}
                      />
                      <Dropdown
                        menu={{
                          items: [
                            { key: 'image', icon: <PictureOutlined />, label: t('chat.uploadImage') },
                            { key: 'file', icon: <FileOutlined />, label: t('chat.uploadFile') },
                          ],
                          onClick: ({ key }) => handleFileSelect(key as 'image' | 'file'),
                        }}
                        trigger={['click']}
                        placement="topRight"
                      >
                        <Tooltip title={t('chat.uploadTooltip')}>
                          <Button
                            className="xy-plus-btn"
                            type="text"
                            icon={<Plus size={18} strokeWidth={2} />}
                            aria-label={t('chat.uploadTooltip')}
                          />
                        </Tooltip>
                      </Dropdown>
                      {/* Hidden file inputs */}
                      <input ref={imageInputRef} type="file" accept="image/*" style={{ display: 'none' }} />
                      <input ref={fileInputRef} type="file" style={{ display: 'none' }} />
                      {session.isProcessing && !input.trim() && attachments.length === 0 ? (
                        <Button className="xy-stop-btn" type="primary" shape="circle" icon={<StopFilled style={{ fontSize: 16 }} />} onClick={() => send('chat.abort', { sessionId: activeSessionId })} style={{ width: 36, height: 36 }} aria-label={t('chat.abort')} />
                      ) : (
                        <Button
                          className={`xy-send-btn ${input.trim() || attachments.length > 0 ? 'is-active' : ''}`}
                          type="primary"
                          shape="circle"
                          icon={<ArrowUp size={18} strokeWidth={2.25} />}
                          onClick={handleSend}
                          disabled={!input.trim() && attachments.length === 0}
                          style={{ width: 36, height: 36 }}
                          aria-label={t('chat.send')}
                        />
                      )}
                    </div>
                  </div>
                </div>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
      {activeSessionId && session ? (
        <ChatInspector
          sessionId={activeSessionId}
          segments={session.messages.flatMap((m) => m.segments || []).concat(session.turnSegments)}
          messageCount={session.messages.length}
          processing={session.isProcessing}
        />
      ) : null}
      {activeSessionId && (
        <RewindOverlay
          open={rewindOpen}
          sessionId={activeSessionId}
          initialTurnId={rewindTurnId}
          onClose={() => {
            setRewindOpen(false);
            setRewindTurnId(undefined);
          }}
        />
      )}
    </div>
  );
}

type AgentMode = VisiblePermissionMode;

/** 被谁调用：ChatView。做什么：权限 mode → UI AgentMode。 */
function toAgentMode(mode: AppConfig['permissions']['mode']): AgentMode {
  return resolveVisiblePermissionMode(mode);
}

/** 权限模式悬停说明（i18n permission.{mode}Tooltip）。 */
function modeTooltip(mode: AgentMode, t: (key: string, fallback?: string) => string): string {
  return t(`permission.${mode}Tooltip`);
}

/** 右侧检查器：本会话消息数、处理中状态、持久化提示。 */
function ChatInspector({
  sessionId, segments, messageCount, processing,
}: {
  sessionId: string;
  segments: Segment[];
  messageCount: number;
  processing: boolean;
}) {
  const { t } = useI18n();
  const toolCalls = segments.filter((s) => s.type === 'tool_call').length;
  const agentRuns = segments.filter((s) => s.type === 'agent_run').length;
  const sandboxEvents = segments.filter((s) => s.type === 'sandbox_event').length;
  const permissions = segments.filter((s) => s.type === 'permission').length;

  return (
    <div className="xy-inspector-dock">
      <div className="xy-inspector-edge" aria-hidden />
      <aside className="xy-chat-inspector">
      <div className="xy-inspector-section">
        <div className="xy-inspector-title">{t('inspector.session')}</div>
        <div className="xy-inspector-row"><span>ID</span><code>{sessionId.slice(-10)}</code></div>
        <div className="xy-inspector-row"><span>{t('common.status')}</span><Tag color={processing ? 'processing' : 'success'}>{processing ? t('status.running') : t('status.idle')}</Tag></div>
        <div className="xy-inspector-row"><span>{t('inspector.messages')}</span><strong>{messageCount}</strong></div>
      </div>
      <div className="xy-inspector-section">
        <div className="xy-inspector-title">{t('inspector.observability')}</div>
        <Metric icon={<ToolOutlined />} label={t('inspector.toolCalls')} value={toolCalls} />
        <Metric icon={<RobotOutlined />} label={t('inspector.subAgents')} value={agentRuns} />
        <Metric icon={<HistoryOutlined />} label={t('inspector.sandbox')} value={sandboxEvents} />
        <Metric icon={<LockOutlined />} label={t('inspector.permissions')} value={permissions} />
      </div>
      <div className="xy-inspector-section">
        <div className="xy-inspector-title">{t('inspector.persistence')}</div>
        <p className="xy-inspector-note">{t('inspector.persistenceNote')}</p>
      </div>
      </aside>
    </div>
  );
}

/** 检查器一行指标；可点击高亮。 */
function Metric({
  icon,
  label,
  value,
  hint,
  onClick,
  active,
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
  hint?: string;
  onClick?: () => void;
  active?: boolean;
}) {
  if (onClick) {
    return (
      <button
        type="button"
        className={`xy-inspector-metric is-clickable${active ? ' is-active' : ''}`}
        onClick={onClick}
        title={hint}
      >
        <span>{icon}</span>
        <span>{label}</span>
        <strong>{value}</strong>
      </button>
    );
  }
  return <div className="xy-inspector-metric"><span>{icon}</span><span>{label}</span><strong>{value}</strong></div>;
}
