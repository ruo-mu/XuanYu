/**
 * 工作区偏好：从 xuanyu.json 恢复上次打开的目录。
 */

import * as fs from 'fs';
import * as path from 'path';

import { fsPathKey, normalizeRecentProjectPath } from './paths';
import type { AppConfig } from './types';

/** 若配置了 lastDir 且目录仍存在，则 chdir 到该路径。返回最终 cwd。 */
export function restoreWorkspaceFromConfig(config: AppConfig): {
  restored: boolean;
  cwd: string;
  skippedReason?: string;
} {
  const raw = config.workspace?.lastDir?.trim();
  if (!raw) {
    return { restored: false, cwd: process.cwd(), skippedReason: 'no lastDir' };
  }
  const resolved = path.resolve(raw);
  try {
    if (!fs.existsSync(resolved) || !fs.statSync(resolved).isDirectory()) {
      return {
        restored: false,
        cwd: process.cwd(),
        skippedReason: `missing directory: ${resolved}`,
      };
    }
    if (path.resolve(process.cwd()) === resolved) {
      return { restored: true, cwd: resolved };
    }
    process.chdir(resolved);
    return { restored: true, cwd: resolved };
  } catch (err) {
    return {
      restored: false,
      cwd: process.cwd(),
      skippedReason: err instanceof Error ? err.message : String(err),
    };
  }
}

/** 把新打开的目录合并进 workspace 偏好（lastDir + recentDirs）。 */
export function buildWorkspacePreferenceUpdate(
  config: AppConfig,
  dir: string,
): { workspace: NonNullable<AppConfig['workspace']> } {
  const resolved = normalizeRecentProjectPath(path.resolve(dir));
  const key = fsPathKey(resolved);
  const seen = new Set<string>(key ? [key] : []);
  const prev = (config.workspace?.recentDirs || [])
    .map((item) => normalizeRecentProjectPath(path.resolve(item.trim())))
    .filter(Boolean);
  const recentDirs = [
    resolved,
    ...prev.filter((item) => {
      const itemKey = fsPathKey(item);
      if (!itemKey || seen.has(itemKey)) return false;
      seen.add(itemKey);
      return true;
    }),
  ].slice(0, 12);
  return {
    workspace: {
      lastDir: resolved,
      recentDirs,
    },
  };
}

/** 从最近列表移除一项；若移除的是 lastDir 则清空 lastDir。 */
export function buildWorkspaceForgetUpdate(
  config: AppConfig,
  dir: string,
): { workspace: NonNullable<AppConfig['workspace']> } {
  const resolved = normalizeRecentProjectPath(path.resolve(dir));
  const key = fsPathKey(resolved);
  const seen = new Set<string>();
  const recentDirs = (config.workspace?.recentDirs || [])
    .map((item) => normalizeRecentProjectPath(path.resolve(item.trim())))
    .filter(Boolean)
    .filter((item) => {
      const itemKey = fsPathKey(item);
      if (!itemKey || itemKey === key || seen.has(itemKey)) return false;
      seen.add(itemKey);
      return true;
    })
    .slice(0, 12);
  const prevLast = normalizeRecentProjectPath(config.workspace?.lastDir);
  const clearLast = !!prevLast && fsPathKey(prevLast) === key;
  return {
    workspace: {
      lastDir: clearLast ? '' : prevLast || '',
      recentDirs,
    },
  };
}
