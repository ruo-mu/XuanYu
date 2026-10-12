/**
 * 工作台页面注册表：hash 路由、Dock 标题与是否出现在导航。
 */

/** 扩展页内 Tab（插件 / MCP / 技能）。 */
export type ExtensionsTab = 'plugins' | 'mcp' | 'skills';

/** 主应用各页的 path、i18n key 与导航可见性。 */
export const PAGE_REGISTRY = [
  { key: 'chat', path: '/chat', titleKey: 'page.chat', fallback: '对话', navigation: false },
  { key: 'extensions', path: '/capabilities', titleKey: 'page.extensions', fallback: '扩展', navigation: true },
  { key: 'tasks', path: '/tasks', titleKey: 'page.tasks', fallback: '子 Agent', navigation: false },
  { key: 'memory', path: '/memory', titleKey: 'page.memory', fallback: '记忆', navigation: false },
  { key: 'resources', path: '/resources', titleKey: 'page.resources', fallback: '资源', navigation: false },
  { key: 'channels', path: '/channels', titleKey: 'page.channels', fallback: '频道', navigation: true },
  { key: 'system', path: '/system', titleKey: 'page.system', fallback: '系统', navigation: false },
  { key: 'setup', path: '/setup', titleKey: 'page.setup', fallback: '配置向导', navigation: false },
] as const;

export type PageKey = (typeof PAGE_REGISTRY)[number]['key'];

const LEGACY_CAPABILITY_PATHS: Record<string, ExtensionsTab> = {
  '/capabilities/plugins': 'plugins',
  '/capabilities/mcp': 'mcp',
  '/capabilities/skills': 'skills',
};

const LEGACY_WORKSPACE_PATHS: Record<string, string> = {
  '/tasks': 'tasks',
  '/memory': 'memory',
  '/resources': 'resources',
};

/** 从 hash 解析扩展页当前 Tab。 */
export function extensionsTabFromLocation(): ExtensionsTab {
  const hash = window.location.hash.replace(/^#/, '') || '/chat';
  const path = hash.split('?')[0] || '/chat';
  const legacy = LEGACY_CAPABILITY_PATHS[path];
  if (legacy) return legacy;
  if (path !== '/capabilities') return 'plugins';
  const query = hash.includes('?') ? hash.slice(hash.indexOf('?') + 1) : '';
  const tab = new URLSearchParams(query).get('tab');
  if (tab === 'mcp' || tab === 'skills') return tab;
  return 'plugins';
}

/** 旧 hash 路径归一化到新路由（已在目标路由则不动）。 */
export function normalizeLegacyRoute(): void {
  const hash = window.location.hash.replace(/^#/, '') || '/chat';
  const path = hash.split('?')[0] || '/chat';

  const capabilityTab = LEGACY_CAPABILITY_PATHS[path];
  if (capabilityTab) {
    navigateToExtensions(capabilityTab);
    return;
  }

  const section = LEGACY_WORKSPACE_PATHS[path];
  if (section) {
    const next = `/system?section=${section}`;
    if (window.location.hash !== `#${next}`) {
      window.location.hash = next;
    }
  }
}

/** 从 location.hash 解析当前页，未知路径回落到对话。 */
export function pageFromLocation(): PageKey {
  const hash = window.location.hash.replace(/^#/, '') || '/chat';
  const path = hash.split('?')[0] || '/chat';

  if (path === '/capabilities' || LEGACY_CAPABILITY_PATHS[path]) {
    return 'extensions';
  }

  if (LEGACY_WORKSPACE_PATHS[path]) {
    return 'system';
  }

  return PAGE_REGISTRY.find((page) => page.path === path)?.key || 'chat';
}

/** 把 hash 改到指定页（已在该页则不动）。 */
export function navigateToPage(page: PageKey): void {
  const path = PAGE_REGISTRY.find((item) => item.key === page)?.path || '/chat';
  if (window.location.hash !== `#${path}`) window.location.hash = path;
}

/** 打开扩展页并选中 Tab。 */
export function navigateToExtensions(tab: ExtensionsTab = 'plugins'): void {
  const next = tab === 'plugins' ? '/capabilities' : `/capabilities?tab=${tab}`;
  if (window.location.hash !== `#${next}`) {
    window.location.hash = next;
  }
}

/** 从 hash 解析频道页当前渠道 type。 */
export function channelTypeFromLocation(fallback = 'wechat'): string {
  const hash = window.location.hash.replace(/^#/, '') || '/channels';
  const path = hash.split('?')[0] || '/channels';
  if (path !== '/channels') return fallback;
  const query = hash.includes('?') ? hash.slice(hash.indexOf('?') + 1) : '';
  const type = new URLSearchParams(query).get('type')?.trim();
  return type || fallback;
}

/** 打开频道页并选中渠道 type。 */
export function navigateToChannelType(type: string): void {
  const next = type === 'wechat' ? '/channels' : `/channels?type=${encodeURIComponent(type)}`;
  if (window.location.hash !== `#${next}`) {
    window.location.hash = next;
  }
}
