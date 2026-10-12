/**
 * session-store.ts - 会话列表状态仓库
 *
 * 用途：管理会话列表、按会话聚合的资源统计和全局分析快照。
 * 自动去重并按最近活跃时间排序，保证列表一致性。
 */

import { create } from 'zustand';
import type { AnalyticsSnapshot, SessionInfo, SessionStats } from '../protocol';

interface SessionStore {
  /** 当前拿到的会话列表。 */
  sessions: SessionInfo[];
  /** 按会话聚合的资源统计。 */
  stats: SessionStats[];
  /** 全局分析快照。 */
  analytics: AnalyticsSnapshot | null;
  setSessions: (sessions: SessionInfo[]) => void;
  addSession: (session: SessionInfo) => void;
  removeSession: (sessionId: string) => void;
  updateSession: (sessionId: string, update: Partial<SessionInfo>) => void;
  setStats: (stats: SessionStats[]) => void;
  setAnalytics: (analytics: AnalyticsSnapshot | null) => void;
}

/** Zustand 会话列表 store；侧栏与统计页订阅。 */
export const useSessionStore = create<SessionStore>((set) => ({
  sessions: [],
  stats: [],
  analytics: null,

  setSessions: (sessions) => set({ sessions: sortSessions(dedupeSessions(sessions)) }),

  addSession: (session) =>
    set((s) => ({ sessions: sortSessions(dedupeSessions([session, ...s.sessions])) })),

  removeSession: (sessionId) =>
    set((s) => ({ sessions: s.sessions.filter((sess) => sess.id !== sessionId) })),

  updateSession: (sessionId, update) =>
    set((s) => ({
      sessions: sortSessions(
        s.sessions.map((sess) => {
          if (sess.id !== sessionId) return sess;
          const next = { ...sess, ...update };
          // 显式清空 projectDir（「最近」会话）时去掉工作区归属
          if ('projectDir' in update && update.projectDir == null) {
            next.projectDir = undefined;
            if (next.metadata) {
              const meta = { ...next.metadata };
              delete meta.projectDir;
              delete meta.workspace;
              next.metadata = meta;
            }
          }
          return next;
        }),
      ),
    })),

  setStats: (stats) => set({ stats }),
  setAnalytics: (analytics) => set({ analytics }),
}));

/** 同 id 后写覆盖前写，只保留一份。 */
function dedupeSessions(sessions: SessionInfo[]): SessionInfo[] {
  const map = new Map<string, SessionInfo>();
  for (const session of sessions) map.set(session.id, { ...map.get(session.id), ...session });
  return Array.from(map.values());
}

/** 按 lastMessageAt / updatedAt / createdAt 降序。 */
function sortSessions(sessions: SessionInfo[]): SessionInfo[] {
  return [...sessions].sort(
    (a, b) =>
      (b.lastMessageAt || b.updatedAt || b.createdAt) -
      (a.lastMessageAt || a.updatedAt || a.createdAt),
  );
}
