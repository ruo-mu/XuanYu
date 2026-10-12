/**
 * 子 Agent 角色 id → 中英文展示名。
 * 未知角色保留原值，便于排查新角色接入。
 */

const ROLE_LABELS: Record<string, [string, string]> = {
  architect: ['架构师', 'Architect'],
  developer: ['开发工程师', 'Developer'],
  explorer: ['探索者', 'Explorer'],
  reviewer: ['审查员', 'Reviewer'],
  writer: ['文档工程师', 'Docs Engineer'],
  'doc-writer': ['文档工程师', 'Docs Engineer'],
};

/** 去掉首尾空白；缺省空串。 */
export function normalizeSubAgentRole(role?: string): string {
  return role?.trim() || '';
}

/** 已知角色 → 中英文标签；未知保留原值，空则 `-`。 */
export function subAgentRoleLabel(role?: string, locale: 'zh-CN' | 'en' = 'zh-CN'): string {
  const value = normalizeSubAgentRole(role);
  const label = ROLE_LABELS[value];
  return label ? label[locale === 'zh-CN' ? 0 : 1] : value || '-';
}
