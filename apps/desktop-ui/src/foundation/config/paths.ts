/**
 * 应用数据路径解析（与参考项目 src/paths.ts 对齐）。
 */

/** 全局应用数据目录名（`.xuanyu`）。 */
export const APP_DIR_NAME = '.xuanyu';

/** 全局配置文件名。 */
export const PROJECT_CONFIG_FILE_NAME = 'xuanyu.json';

/**
 * 去掉 Windows canonicalize 的 `\\?\` / `\\?\UNC\` 前缀。
 * 展示、分组键、最近项目列表都应走这里，避免 UI 出现 `\\?\D:\...`。
 */
export function normalizeFsPath(value?: string | null): string {
  const raw = (value || '').trim();
  if (!raw) return '';
  if (raw.startsWith('\\\\?\\UNC\\')) return `\\\\${raw.slice('\\\\?\\UNC\\'.length)}`;
  if (raw.startsWith('\\\\?\\')) return raw.slice('\\\\?\\'.length);
  if (raw.startsWith('//?/UNC/')) return `//${raw.slice('//?/UNC/'.length)}`;
  if (raw.startsWith('//?/')) return raw.slice('//?/'.length);
  return raw;
}

/**
 * 最近项目 / workspace 偏好落盘用的统一形态：去扩展前缀、统一 `/`、去掉尾部斜杠。
 * 避免同一目录因 `D:\a` / `D:/a` / `\\?\D:\a` 被存成多条。
 */
export function normalizeRecentProjectPath(value?: string | null): string {
  let s = normalizeFsPath(value).replace(/\\/g, '/');
  if (!s) return '';
  while (s.length > 1 && s.endsWith('/')) {
    // 保留盘符根 `D:/`
    if (/^[A-Za-z]:\/$/.test(s)) break;
    s = s.slice(0, -1);
  }
  return s;
}

/** 路径相等比较键（大小写不敏感），用于最近项目去重。 */
export function fsPathKey(value?: string | null): string {
  return normalizeRecentProjectPath(value).toLowerCase();
}

/** UI 展示用路径（与 normalizeFsPath 相同，语义上强调「给人看」）。 */
export function displayFsPath(value?: string | null): string {
  return normalizeFsPath(value);
}

/** Init / 无 IPC 时的占位 home（真实路径由 Rust configInfo 返回）。 */
export function displayHomeDir(): string {
  return '~';
}

/** 未配置 appDataDir 时的默认应用数据目录（`{用户主目录}/.xuanyu`）。 */
export function globalAppDir(homeDir: string = displayHomeDir()): string {
  return `${homeDir}/${APP_DIR_NAME}`;
}

/**
 * 全局配置文件路径（固定，不可通过配置或环境变量改写）。
 * 始终为 `{用户主目录}/.xuanyu/xuanyu.json`。
 */
export function resolveGlobalConfigFile(homeDir: string = displayHomeDir()): string {
  return `${globalAppDir(homeDir)}/${PROJECT_CONFIG_FILE_NAME}`;
}
