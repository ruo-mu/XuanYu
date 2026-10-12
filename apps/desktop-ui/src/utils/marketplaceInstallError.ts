/**
 * 市场安装失败：后端错误串 → 用户可见 i18n key / 中英文文案。
 * MarketplaceHub / PluginManager 等安装回调调用。
 */

export type MarketplaceInstallErrorKey =
  | 'plugins.installFailedNetwork'
  | 'plugins.installFailedClone'
  | 'plugins.installFailedExpired'
  | 'plugins.installFailedUnavailable'
  | 'plugins.installFailedPermission'
  | 'plugins.installFailed';

const FALLBACKS: Record<MarketplaceInstallErrorKey, [string, string]> = {
  'plugins.installFailedNetwork': [
    '无法连接下载源（如 GitHub），请检查网络或代理后重试',
    'Could not reach the download source (e.g. GitHub). Check your network or proxy and try again.',
  ],
  'plugins.installFailedClone': [
    '拉取插件仓库失败，请稍后重试',
    'Failed to clone the plugin repository. Please try again later.',
  ],
  'plugins.installFailedExpired': [
    '安装确认已过期，请重新点击安装',
    'The install confirmation expired. Please click install again.',
  ],
  'plugins.installFailedUnavailable': [
    '该插件暂不可安装',
    'This plugin is not available to install.',
  ],
  'plugins.installFailedPermission': [
    '安装目录无写入权限，请检查本地目录权限',
    'No write permission for the install directory. Check local folder permissions.',
  ],
  'plugins.installFailed': [
    '安装失败，请稍后重试',
    'Install failed. Please try again later.',
  ],
};

/**
 * 按关键词分支归类：网络 / clone / 审批过期 / 不可装 / 权限，否则通用失败。
 */
export function classifyMarketplaceInstallError(raw?: string): MarketplaceInstallErrorKey {
  const text = (raw || '').toLowerCase();
  if (!text.trim()) return 'plugins.installFailed';

  if (
    text.includes("couldn't connect")
    || text.includes('failed to connect')
    || text.includes('enotfound')
    || text.includes('etimedout')
    || text.includes('econnrefused')
    || text.includes('econnreset')
    || text.includes('eai_again')
    || text.includes('getaddrinfo')
    || text.includes('network is unreachable')
    || text.includes('tunnel connection failed')
  ) {
    return 'plugins.installFailedNetwork';
  }

  if (
    text.includes('git clone')
    || text.includes('cloning into')
    || text.includes('repository not found')
    || text.includes('remote: not found')
  ) {
    return 'plugins.installFailedClone';
  }

  if (
    text.includes('expired')
    || text.includes('approval does not match')
    || text.includes('digest changed')
  ) {
    return 'plugins.installFailedExpired';
  }

  if (
    text.includes('not installable')
    || text.includes('entry not found')
    || text.includes('marketplace entry not found')
  ) {
    return 'plugins.installFailedUnavailable';
  }

  if (
    text.includes('eacces')
    || text.includes('eperm')
    || text.includes('permission denied')
    || text.includes('access is denied')
  ) {
    return 'plugins.installFailedPermission';
  }

  return 'plugins.installFailed';
}

/** 走 i18n `t`；原始错误仅 console.warn，UI 用友好文案。 */
export function formatMarketplaceInstallError(
  raw: string | undefined,
  t: (key: string, fallback?: string) => string,
): string {
  const key = classifyMarketplaceInstallError(raw);
  if (raw?.trim()) {
    console.warn('[marketplace.install]', raw);
  }
  return t(key, FALLBACKS[key][0]);
}

/** 无 i18n 时用 localeText(zh,en) 直接出文案。 */
export function formatMarketplaceInstallErrorLocale(
  raw: string | undefined,
  lt: (zh: string, en: string) => string,
): string {
  const key = classifyMarketplaceInstallError(raw);
  if (raw?.trim()) {
    console.warn('[marketplace.install]', raw);
  }
  const [zh, en] = FALLBACKS[key];
  return lt(zh, en);
}
