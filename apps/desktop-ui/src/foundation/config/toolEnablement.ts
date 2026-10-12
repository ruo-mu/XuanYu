/**
 * 工具启用：disabledTools 通配、子代理 tools 交集，以及默认权限规则。
 */

import type { AppConfig } from './types';
import type { PermissionConfig, PermissionRule } from '../contracts';

/** 子代理主体：可选 id、角色与工具名通配列表。 */
export interface ToolSubject {
  id?: string;
  role?: string;
  tools?: string[];
}

/** 带 name 的工具描述，供过滤启用集。 */
export interface ToolNameLike {
  name: string;
}

/**
 * 工具名 / 通配 pattern 匹配（disabledTools、permissions.rules、whitelist 共用）。
 * 支持 `*`（段内）、`**`（跨段）、`?`（单字符）。
 */
export function matchesToolPattern(toolName: string, pattern: string): boolean {
  if (!pattern) return false;
  if (pattern === toolName || pattern === '*') return true;
  if (!pattern.includes('*') && !pattern.includes('?')) return false;
  const escaped = pattern
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*\*/g, '##GS##')
    .replace(/\*/g, '[^/]*')
    .replace(/##GS##/g, '.*')
    .replace(/\?/g, '.');
  return new RegExp(`^${escaped}$`).test(toolName);
}

/** 任一 pattern 命中即为真。 */
export function matchesAnyToolPattern(toolName: string, patterns: readonly string[]): boolean {
  return patterns.some((pattern) => matchesToolPattern(toolName, pattern));
}

/** 未出现在 disabledTools 中则为启用。 */
export function isToolEnabled(config: AppConfig, toolName: string): boolean {
  return !matchesAnyToolPattern(toolName, config.disabledTools || []);
}

/** 按配置过滤仍启用的工具。 */
export function filterEnabledTools<T extends ToolNameLike>(config: AppConfig, tools: T[]): T[] {
  return tools.filter((tool) => isToolEnabled(config, tool.name));
}

/** 子代理：角色定义 tools ∩ 全局启用。声明为空则仅受 disabledTools 约束。 */
export function resolveSubAgentTools<T extends ToolNameLike>(
  config: AppConfig,
  definition: ToolSubject,
  tools: T[],
): T[] {
  const enabled = filterEnabledTools(config, tools);
  const patterns = definition.tools || [];
  if (patterns.length === 0) return enabled;
  return enabled.filter((tool) => matchesAnyToolPattern(tool.name, patterns));
}

/** 与 schema defaultConfig.permissions.rules 同源，避免 PermissionManager 兜底漂移。 */
export const DEFAULT_PERMISSION_RULES: PermissionRule[] = [
  { pattern: 'file_read', level: 'allow' },
  { pattern: 'search', level: 'allow' },
  { pattern: 'glob', level: 'allow' },
  { pattern: 'list_files', level: 'allow' },
  { pattern: 'web_search', level: 'allow' },
  { pattern: 'web_fetch', level: 'allow' },
  { pattern: 'ask_user', level: 'allow' },
  { pattern: 'enter_plan_mode', level: 'allow' },
  { pattern: 'exit_plan_mode', level: 'allow' },
  { pattern: 'web_*', level: 'confirm' },
  { pattern: 'mcp__*', level: 'confirm' },
  { pattern: 'file_write', level: 'confirm' },
  { pattern: 'file_edit', level: 'confirm' },
  { pattern: 'exec', level: 'confirm' },
  { pattern: 'list_processes', level: 'allow' },
  { pattern: 'read_process_output', level: 'allow' },
  { pattern: 'write_process_stdin', level: 'confirm' },
  { pattern: 'stop_process', level: 'confirm' },
  { pattern: 'sandbox_*', level: 'confirm' },
  { pattern: 'git_status', level: 'allow' },
  { pattern: 'git_diff', level: 'allow' },
  { pattern: 'git_log', level: 'allow' },
  { pattern: 'git_blame', level: 'allow' },
  { pattern: 'git_branch', level: 'confirm' },
  { pattern: 'git_commit', level: 'confirm' },
  { pattern: 'git_patch_create', level: 'allow' },
  { pattern: 'git_patch_apply', level: 'confirm' },
  { pattern: 'write_memory', level: 'allow' },
  { pattern: 'agent', level: 'allow' },
  { pattern: 'submit_task', level: 'allow' },
  { pattern: 'get_task_status', level: 'allow' },
  { pattern: 'list_tasks', level: 'allow' },
  { pattern: 'cancel_task', level: 'allow' },
  { pattern: 'load_tools', level: 'allow' },
  { pattern: 'search_tools', level: 'allow' },
  { pattern: 'search_memory', level: 'allow' },
  { pattern: 'load_memories', level: 'allow' },
  { pattern: 'activate_skill', level: 'allow' },
  { pattern: 'todo_write', level: 'allow' },
  { pattern: 'update_goal', level: 'allow' },
];

/** 默认权限：未匹配规则时 confirm，含内置 allow/confirm 规则表。 */
export const DEFAULT_PERMISSIONS: PermissionConfig = {
  default: 'confirm',
  mode: 'default',
  rules: DEFAULT_PERMISSION_RULES,
  whitelist: [],
};
