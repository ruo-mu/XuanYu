/** 系统设置侧栏 section id（与 hash ?section= 对齐）。 */

export const SETTINGS_SECTIONS = [
  'models',
  'team',
  'tasks',
  'permission',
  'sandbox',
  'context',
  'storage',
  'status',
] as const;

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export type SettingsNavGroup = {
  id: string;
  labelKey: string;
  fallback: string;
  items: Array<{
    id: SettingsSection;
    labelKey: string;
    fallback: string;
    descKey: string;
    descFallback: string;
  }>;
};

/** Shared by AppSidebar settings panel and SystemSettingsLayout headers. */
export const SETTINGS_NAV_GROUPS: SettingsNavGroup[] = [
  {
    id: 'models',
    labelKey: 'settings.group.models',
    fallback: '模型',
    items: [
      {
        id: 'models',
        labelKey: 'settings.nav.models',
        fallback: '主模型',
        descKey: 'settings.desc.models',
        descFallback: '供应商、密钥与主模型生成参数',
      },
      {
        id: 'team',
        labelKey: 'settings.nav.team',
        fallback: '团队与辅助',
        descKey: 'settings.desc.team',
        descFallback: '子 Agent 与辅助任务用哪套模型',
      },
      {
        id: 'tasks',
        labelKey: 'settings.nav.tasks',
        fallback: '子 Agent',
        descKey: 'settings.desc.tasks',
        descFallback: '子 Agent 团队与任务编排',
      },
    ],
  },
  {
    id: 'runtime',
    labelKey: 'settings.group.runtime',
    fallback: '运行',
    items: [
      {
        id: 'permission',
        labelKey: 'settings.nav.permission',
        fallback: '权限',
        descKey: 'settings.desc.permission',
        descFallback: '工具确认策略与禁用列表',
      },
      {
        id: 'sandbox',
        labelKey: 'settings.nav.sandbox',
        fallback: '沙箱',
        descKey: 'settings.desc.sandbox',
        descFallback: '命令隔离与断网策略',
      },
      {
        id: 'context',
        labelKey: 'settings.nav.context',
        fallback: '上下文与记忆',
        descKey: 'settings.desc.context',
        descFallback: '压缩与记忆注入频率',
      },
    ],
  },
  {
    id: 'data',
    labelKey: 'settings.group.data',
    fallback: '数据',
    items: [
      {
        id: 'storage',
        labelKey: 'settings.nav.storage',
        fallback: '存储',
        descKey: 'settings.desc.storage',
        descFallback: 'PostgreSQL 连接（会话与长期记忆）',
      },
    ],
  },
  {
    id: 'system',
    labelKey: 'settings.group.system',
    fallback: '系统',
    items: [
      {
        id: 'status',
        labelKey: 'settings.nav.status',
        fallback: '运行状态',
        descKey: 'settings.desc.status',
        descFallback: '健康检查与实时日志',
      },
    ],
  },
];

/** 被谁调用：系统路由。做什么：校验 section id。 */
export function isSettingsSection(value: string): value is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(value);
}

/** 需要底栏保存的配置类 section。 */
export function isEditableSettingsSection(section: SettingsSection): boolean {
  return section !== 'status' && section !== 'tasks';
}

/** Look up nav meta for the active settings section. */
export function settingsNavMeta(section: SettingsSection) {
  for (const group of SETTINGS_NAV_GROUPS) {
    const item = group.items.find((entry) => entry.id === section);
    if (item) return item;
  }
  return SETTINGS_NAV_GROUPS[0].items[0];
}

/** 被谁调用：系统页挂载。做什么：从 hash/path 解析当前 section。 */
export function settingsSectionFromLocation(): SettingsSection {
  const hash = window.location.hash.replace(/^#/, '') || '/system';
  const query = hash.includes('?') ? hash.slice(hash.indexOf('?') + 1) : '';
  const section = new URLSearchParams(query).get('section') || '';
  return isSettingsSection(section) ? section : 'models';
}

/** 被谁调用：设置侧栏。做什么：改 location 切 section。 */
export function navigateToSettingsSection(section: SettingsSection): void {
  const next = `/system?section=${section}`;
  if (window.location.hash !== `#${next}`) {
    window.location.hash = next;
  }
}
