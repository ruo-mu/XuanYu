/**
 * 工作台可见权限模式：default / plan / auto（与参考项目对齐）。
 */

/** 配置面板与聊天栏展示的三种权限模式。 */
export type VisiblePermissionMode = 'default' | 'plan' | 'auto';

/** 工作台可见权限模式列表（顺序即 UI 展示序）。 */
export const VISIBLE_PERMISSION_MODES: VisiblePermissionMode[] = ['default', 'plan', 'auto'];

/** 解析配置里的权限模式；仅接受三模式，遗留值映射后返回。 */
export function resolveVisiblePermissionMode(value: unknown): VisiblePermissionMode {
  if (value === undefined || value === null || value === '') return 'default';
  const normalized = String(value).trim().toLowerCase();
  if (normalized === 'default' || normalized === 'ask' || normalized === 'strict') return 'default';
  if (normalized === 'plan') return 'plan';
  if (normalized === 'auto' || normalized === 'all' || normalized === 'yolo') return 'auto';
  // 未知值兜底为 default，避免菜单切换时整页渲染崩掉
  return 'default';
}
