/** 归属规则：
 * - projectDir：侧栏「项目」分组键（工作区绝对路径，经 fsPathKey 归一）
 * - projectName：仅 DB/隔离用存储键，不参与 UI 分类
 * - 无 projectDir 的 Web 会话 →「最近」
 * - pinned：单独进「置顶」区；取消置顶后回到项目或最近
 */

import type { SessionInfo } from '../../protocol';
import { fsPathKey, normalizeFsPath, normalizeRecentProjectPath } from '../../foundation/config/paths';

const PROJECT_VISIBLE_LIMIT = 6;

export type SidebarOrganizeMode = 'byProject' | 'flat';
export type SidebarSortMode = 'priority' | 'recent' | 'manual';

/** 会话绑定的工作区路径（线缆或 metadata）；统一斜杠与尾部斜杠后再参与分组。 */
export function sessionProjectDir(session: SessionInfo): string | undefined {
  const fromWire = session.projectDir?.trim();
  if (fromWire) return normalizeRecentProjectPath(fromWire) || undefined;
  const fromMeta = session.metadata?.projectDir;
  if (typeof fromMeta === 'string' && fromMeta.trim()) {
    return normalizeRecentProjectPath(fromMeta) || undefined;
  }
  return undefined;
}

/** 频道来源会话，不进项目树。 */
export function isChannelSession(session: SessionInfo): boolean {
  return session.metadata?.source === 'channel';
}

/** 归入「最近」：未绑定工作区，或频道等非项目会话。 */
export function isRecentScopedSession(session: SessionInfo): boolean {
  if (isChannelSession(session)) return true;
  return !sessionProjectDir(session);
}

/** 至少有过一轮对话才进侧栏；空白新建不占列表。 */
export function hasListedConversation(session: SessionInfo): boolean {
  if ((session.messageCount ?? 0) > 0) return true;
  if (session.lastMessageAt) return true;
  return false;
}

/** Web 工作区会话（主会话或项目下分支）。 */
export function isWorkspaceSession(session: SessionInfo): boolean {
  if (isRecentScopedSession(session)) return false;
  if (session.id.startsWith('main_')) return true;
  if (session.metadata?.persistent === true) return true;
  if (session.metadata?.workspace === true) return true;
  return !!sessionProjectDir(session);
}

/**
 * 项目分组键：同一目录只占一个文件夹。
 * Windows 上 `D:/a` 与 `D:\a` 必须合并；比较用小写键，展示仍用规范化路径。
 */
export function projectGroupKey(session: SessionInfo): string {
  const dir = sessionProjectDir(session);
  if (!dir) return '';
  return fsPathKey(dir);
}

/** 文件夹展示名（路径末段）。 */
export function projectGroupLabel(key: string): string {
  // key 可能是 fsPathKey（小写）；优先还原展示形态
  const trimmed = normalizeRecentProjectPath(key) || normalizeFsPath(key);
  if (!trimmed) return '';
  const normalized = trimmed.replace(/\\/g, '/');
  const base = normalized.split('/').filter(Boolean).pop();
  if (base) return base;
  return trimmed.replace(/_[0-9a-f]{8}$/i, '');
}

/** 相对时间：侧栏悬浮卡用。 */
export function formatSessionAge(ts: number | undefined, locale: 'zh-CN' | 'en'): string {
  if (!ts || !Number.isFinite(ts)) return '';
  const sec = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (sec < 60) return locale === 'zh-CN' ? '刚刚' : 'just now';
  const min = Math.floor(sec / 60);
  if (min < 60) return locale === 'zh-CN' ? `${min} 分钟` : `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return locale === 'zh-CN' ? `${hr} 小时` : `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return locale === 'zh-CN' ? `${day} 天` : `${day}d`;
  const week = Math.floor(day / 7);
  if (week < 5) return locale === 'zh-CN' ? `${week} 周` : `${week}w`;
  const month = Math.floor(day / 30);
  if (month < 12) return locale === 'zh-CN' ? `${month} 个月` : `${month}mo`;
  const year = Math.floor(day / 365);
  return locale === 'zh-CN' ? `${year} 年` : `${year}y`;
}

export interface ProjectSessionGroup {
  key: string;
  label: string;
  sessions: SessionInfo[];
}

export interface SessionSidebarGroups {
  /** 已置顶会话；有内容时侧栏才显示「置顶」区。 */
  pinned: SessionInfo[];
  projects: ProjectSessionGroup[];
  recent: SessionInfo[];
  /** 扁平列表（不含已置顶，避免与置顶区重复）。 */
  flat: SessionInfo[];
}

function sessionSortTime(session: SessionInfo): number {
  return session.lastMessageAt || session.updatedAt || session.createdAt;
}

function sortSessions(
  sessions: SessionInfo[],
  sortMode: SidebarSortMode,
  manualOrder: string[],
): SessionInfo[] {
  const list = [...sessions];
  if (sortMode === 'manual' && manualOrder.length > 0) {
    const rank = new Map(manualOrder.map((id, i) => [id, i]));
    return list.sort((a, b) => {
      const ra = rank.has(a.id) ? rank.get(a.id)! : Number.MAX_SAFE_INTEGER;
      const rb = rank.has(b.id) ? rank.get(b.id)! : Number.MAX_SAFE_INTEGER;
      if (ra !== rb) return ra - rb;
      return sessionSortTime(b) - sessionSortTime(a);
    });
  }
  if (sortMode === 'priority') {
    return list.sort((a, b) => {
      const pin = Number(!!b.pinned) - Number(!!a.pinned);
      if (pin !== 0) return pin;
      return sessionSortTime(b) - sessionSortTime(a);
    });
  }
  return list.sort((a, b) => sessionSortTime(b) - sessionSortTime(a));
}

export interface GroupSessionsOptions {
  organize: SidebarOrganizeMode;
  sort: SidebarSortMode;
  hiddenProjectKeys?: string[];
  pinnedProjectKeys?: string[];
  projectAliases?: Record<string, string>;
  manualOrder?: string[];
}

/** 将会话列表拆成置顶 / 项目分组 / 最近 / 扁平列表。 */
export function groupSessionsForSidebar(
  sessions: SessionInfo[],
  options: GroupSessionsOptions,
): SessionSidebarGroups {
  const hidden = new Set((options.hiddenProjectKeys || []).map((key) => fsPathKey(key)).filter(Boolean));
  const pinnedProjects = (options.pinnedProjectKeys || [])
    .map((key) => fsPathKey(key))
    .filter(Boolean);
  const aliases = options.projectAliases || {};
  const aliasByKey = new Map<string, string>();
  for (const [raw, label] of Object.entries(aliases)) {
    const key = fsPathKey(raw);
    if (key && label.trim()) aliasByKey.set(key, label.trim());
  }
  const workspaceByKey = new Map<string, { path: string; sessions: SessionInfo[] }>();
  const recent: SessionInfo[] = [];
  const pinned: SessionInfo[] = [];
  const active = sessions.filter(
    (session) => !session.archivedAt && hasListedConversation(session),
  );

  for (const session of active) {
    // 置顶单独成区；取消置顶后仍按 projectDir 回到项目或最近。
    if (session.pinned) {
      pinned.push(session);
      continue;
    }
    if (isRecentScopedSession(session)) {
      recent.push(session);
      continue;
    }
    if (isWorkspaceSession(session)) {
      const path = sessionProjectDir(session);
      const key = path ? fsPathKey(path) : '';
      if (!path || !key || hidden.has(key)) {
        recent.push(session);
        continue;
      }
      const bucket = workspaceByKey.get(key) || { path, sessions: [] };
      bucket.sessions.push(session);
      workspaceByKey.set(key, bucket);
    } else {
      recent.push(session);
    }
  }

  const projects: ProjectSessionGroup[] = Array.from(workspaceByKey.entries())
    .map(([key, bucket]) => ({
      key: bucket.path,
      label: aliasByKey.get(key) || projectGroupLabel(bucket.path) || bucket.path,
      sessions: sortSessions(bucket.sessions, options.sort, options.manualOrder || []),
    }))
    .filter((group) => group.sessions.length > 0)
    .sort((a, b) => {
      const ap = pinnedProjects.indexOf(fsPathKey(a.key));
      const bp = pinnedProjects.indexOf(fsPathKey(b.key));
      const aPinned = ap >= 0;
      const bPinned = bp >= 0;
      if (aPinned !== bPinned) return aPinned ? -1 : 1;
      if (aPinned && bPinned) return ap - bp;
      const aTime = a.sessions[0] ? sessionSortTime(a.sessions[0]) : 0;
      const bTime = b.sessions[0] ? sessionSortTime(b.sessions[0]) : 0;
      return bTime - aTime;
    });

  const sortedPinned = sortSessions(pinned, 'recent', options.manualOrder || []);
  const sortedRecent = sortSessions(recent, options.sort, options.manualOrder || []);
  const unpinned = active.filter((session) => !session.pinned);
  const flat = sortSessions(unpinned, options.sort, options.manualOrder || []);

  return { pinned: sortedPinned, projects, recent: sortedRecent, flat };
}

export { PROJECT_VISIBLE_LIMIT };
