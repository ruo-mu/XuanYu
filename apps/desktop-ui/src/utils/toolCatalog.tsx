/**
 * 工具目录展示：内建工具中英标签、描述，以及执行类工具标记。
 */

import React from 'react';
import { Space, Tag, Typography } from 'antd';
import type { ToolCatalogEntry } from '../protocol';

const { Text } = Typography;

export type Locale = 'zh-CN' | 'en';

const EXECUTE_TOOL_NAMES = new Set([
  'exec',
  'write_process_stdin',
  'stop_process',
]);

const TOOL_LABELS: Record<string, { zh: string; en: string; desc: string }> = {
  load_tools: { zh: '加载工具', en: 'Load Tools', desc: '按名激活 deferred 工具的完整 schema' },
  activate_skill: { zh: '激活技能', en: 'Activate Skill', desc: '加载 Skill 全文并按其声明挂工具' },
  ask_user: { zh: '询问用户', en: 'Ask User', desc: '结构化提问并等待用户回答' },
  search_memory: { zh: '搜索记忆', en: 'Search Memory', desc: '按关键词检索长期记忆（不注入）' },
  load_memories: { zh: '加载记忆', en: 'Load Memories', desc: '激活记忆/画像/项目规则分区' },
  write_memory: { zh: '写入记忆', en: 'Write Memory', desc: '更新长期画像（identity/user/project/…）' },
  file_read: { zh: '读取文件', en: 'Read File', desc: '读文本/图片/PDF；图片走多模态' },
  file_write: { zh: '写入文件', en: 'Write File', desc: '新建文件，或仅在需要时整文件重写' },
  file_edit: { zh: '编辑文件', en: 'Edit File', desc: '首选：精确替换一处文本' },
  todo_write: { zh: '任务清单', en: 'Todo Write', desc: '本任务步骤板（唯一清单）' },
  update_goal: { zh: '完成主目标', en: 'Complete Objective', desc: '验证后标记用户 /goal 主目标完成' },
  get_goal: { zh: '查询目标', en: 'Get Goal', desc: '读取当前会话 Goal 状态' },
  enter_plan_mode: { zh: '进入计划模式', en: 'Enter Plan Mode', desc: '切换到只读规划权限模式' },
  exit_plan_mode: { zh: '退出计划模式', en: 'Exit Plan Mode', desc: '计划批准后打开写/执行权限' },
  search: { zh: '文本搜索', en: 'Text Search', desc: 'ripgrep 搜文件内容' },
  glob: { zh: '文件匹配', en: 'Glob Files', desc: '按 glob 找文件路径' },
  list_files: { zh: '列出文件', en: 'List Files', desc: '树形列出目录' },
  exec: { zh: '执行命令', en: 'Run Command', desc: '运行终端命令；超时自动后台' },
  list_processes: { zh: '列出进程', en: 'List Processes', desc: '查看后台进程' },
  read_process_output: { zh: '读取进程输出', en: 'Read Process Output', desc: '查看后台进程日志' },
  write_process_stdin: { zh: '写入进程输入', en: 'Write Process Stdin', desc: '向后台进程写入 stdin' },
  stop_process: { zh: '停止进程', en: 'Stop Process', desc: '停止后台进程' },
  agent: { zh: '嵌套子 Agent', en: 'Nested Agent', desc: '同步等待一个或多个专家子 Agent 完成（submit_task 别名）' },
  submit_task: { zh: '提交子任务', en: 'Submit Task', desc: '向控制面提交一个或多个子 Agent 任务' },
  get_task_status: { zh: '任务状态', en: 'Task Status', desc: '查询子任务状态' },
  list_tasks: { zh: '任务列表', en: 'List Tasks', desc: '列出会话子任务' },
  cancel_task: { zh: '取消任务', en: 'Cancel Task', desc: '取消进行中的子任务' },
  search_tools: { zh: '搜索工具', en: 'Search Tools', desc: '按关键词发现可加载工具' },
  web_search: { zh: '网页搜索', en: 'Web Search', desc: '搜索互联网资料' },
  web_fetch: { zh: '读取网页', en: 'Fetch Web Page', desc: '读取指定网页内容' },
  git_status: { zh: 'Git 状态', en: 'Git Status', desc: '查看工作区变更状态' },
  git_diff: { zh: 'Git 差异', en: 'Git Diff', desc: '查看文件差异' },
  git_log: { zh: 'Git 日志', en: 'Git Log', desc: '查看提交历史' },
  git_blame: { zh: 'Git 追溯', en: 'Git Blame', desc: '查看行级提交来源' },
  git_branch: { zh: 'Git 分支', en: 'Git Branch', desc: '管理或查看分支' },
  git_commit: { zh: 'Git 提交', en: 'Git Commit', desc: '创建代码提交' },
  git_patch_create: { zh: '创建补丁', en: 'Create Patch', desc: '导出变更补丁' },
  git_patch_apply: { zh: '套用补丁', en: 'Apply Git Patch', desc: '应用补丁文件' },
  sandbox_modes: { zh: '沙箱模式', en: 'Sandbox Modes', desc: '查看可用隔离执行模式' },
  sandbox_create: { zh: '创建沙箱', en: 'Create Sandbox', desc: '创建独立工作区用于试验' },
  sandbox_list: { zh: '沙箱列表', en: 'List Sandboxes', desc: '查看已有沙箱和状态' },
  sandbox_diff: { zh: '沙箱差异', en: 'Sandbox Diff', desc: '对比沙箱和主工作区改动' },
  sandbox_apply: { zh: '应用沙箱改动', en: 'Apply Sandbox', desc: '把沙箱变更合并到工作区' },
  sandbox_rollback: { zh: '回滚沙箱', en: 'Rollback Sandbox', desc: '撤销或清理沙箱变更' },
};

/** 初始化向导等静态场景可用的内置工具列表（按类别分组）。 */
export const POLICY_TOOL_NAMES = [
  'load_tools',
  'activate_skill',
  'ask_user',
  'search_memory',
  'load_memories',
  'file_read',
  'file_write',
  'file_edit',
  'todo_write',
  'enter_plan_mode',
  'exit_plan_mode',
  'update_goal',
  'list_files',
  'glob',
  'search',
  'exec',
  'list_processes',
  'read_process_output',
  'write_process_stdin',
  'stop_process',
  'submit_task',
  'agent',
  'get_task_status',
  'list_tasks',
  'cancel_task',
  'search_tools',
  'web_search',
  'web_fetch',
  'git_status',
  'git_diff',
  'git_log',
  'git_blame',
  'git_branch',
  'git_commit',
  'git_patch_create',
  'git_patch_apply',
  'write_memory',
  'sandbox_modes',
  'sandbox_create',
  'sandbox_list',
  'sandbox_diff',
  'sandbox_apply',
  'sandbox_rollback',
] as const;

const CATEGORY_LABELS: Record<string, { zh: string; en: string }> = {
  file: { zh: '文件', en: 'Files' },
  filesystem: { zh: '文件系统', en: 'Filesystem' },
  search: { zh: '搜索', en: 'Search' },
  process: { zh: '进程与命令', en: 'Process' },
  web: { zh: '网页', en: 'Web' },
  git: { zh: 'Git', en: 'Git' },
  memory: { zh: '记忆', en: 'Memory' },
  sandbox: { zh: '沙箱', en: 'Sandbox' },
  control: { zh: '任务调度', en: 'Task Control' },
  system: { zh: '系统与任务', en: 'System & Tasks' },
  mcp: { zh: 'MCP 扩展', en: 'MCP' },
  plugin: { zh: '插件', en: 'Plugin' },
  other: { zh: '其他', en: 'Other' },
};

/** 工具名 → 中英文展示名；mcp__* 剥前缀；未知原样。 */
export function toolDisplayName(name: string, locale: Locale): string {
  const item = TOOL_LABELS[name];
  if (!item) return name.startsWith('mcp__') ? `MCP：${name.replace(/^mcp__/, '')}` : name;
  return locale === 'zh-CN' ? item.zh : item.en;
}

/** 策略配置用：中文名 (tool_name)，未知工具名原样返回。 */
export function toolPolicyLabel(name: string, locale: Locale): string {
  const display = toolDisplayName(name, locale);
  return display === name ? name : `${display} (${name})`;
}

/** Select options：POLICY_TOOL_NAMES → value/label。 */
export function toolPolicySelectOptions(locale: Locale) {
  return POLICY_TOOL_NAMES.map((name) => ({
    value: name,
    label: toolPolicyLabel(name, locale),
  }));
}

/** 权限策略多选 Tag 渲染；label 走 toolPolicyLabel。 */
export function renderToolPolicyTag(
  props: { label: React.ReactNode; value: string | number; closable: boolean; onClose: (e: React.MouseEvent<HTMLElement>) => void },
  locale: Locale,
) {
  const name = String(props.value);
  return (
    <Tag closable={props.closable} onClose={props.onClose} style={{ marginInlineEnd: 4 }}>
      {toolPolicyLabel(name, locale)}
    </Tag>
  );
}

/** 工具描述：内建用 TOOL_LABELS.desc，否则后端 description。 */
export function toolDescription(tool: ToolCatalogEntry, locale: Locale): string {
  const item = TOOL_LABELS[tool.name];
  if (item) return locale === 'zh-CN' ? item.desc : tool.description || item.en;
  return tool.description || (locale === 'zh-CN' ? '扩展工具' : 'Extension tool');
}

/** category → 中英文分类标签；未知归「其他」。 */
export function toolCategoryLabel(category: string | undefined, locale: Locale): string {
  const key = category || 'other';
  const item = CATEGORY_LABELS[key];
  if (!item) return locale === 'zh-CN' ? '其他' : 'Other';
  return locale === 'zh-CN' ? item.zh : item.en;
}

/** 风险文案：执行 / 写入 / 只读。 */
export function toolRiskLabel(tool: ToolCatalogEntry, locale: Locale): string {
  if (isExecuteTool(tool)) return locale === 'zh-CN' ? '执行' : 'Execute';
  if (tool.isWrite) return locale === 'zh-CN' ? '写入' : 'Write';
  return locale === 'zh-CN' ? '只读' : 'Read';
}

/** 风险 Tag 色：执行 red、写入 orange、只读 blue。 */
export function toolRiskColor(tool: ToolCatalogEntry): string {
  if (isExecuteTool(tool)) return 'red';
  if (tool.isWrite) return 'orange';
  return 'blue';
}

/** isExecute 或内建执行类工具名。 */
export function isExecuteTool(tool: ToolCatalogEntry): boolean {
  return tool.isExecute || EXECUTE_TOOL_NAMES.has(tool.name);
}

/** 工具来源标签：builtin / mcp / plugin / internal / runtime。 */
export function toolSourceLabel(source: ToolCatalogEntry['source'], locale: Locale): string {
  const labels: Record<ToolCatalogEntry['source'], { zh: string; en: string }> = {
    builtin: { zh: '内置', en: 'Builtin' },
    mcp: { zh: 'MCP', en: 'MCP' },
    plugin: { zh: '插件', en: 'Plugin' },
    internal: { zh: '内部', en: 'Internal' },
    runtime: { zh: '运行时', en: 'Runtime' },
  };
  const item = labels[source];
  return locale === 'zh-CN' ? item.zh : item.en;
}

/** 配置页工具 Select 的 options（含 title 搜索串）。 */
export function toolSelectOptions(tools: ToolCatalogEntry[], locale: Locale) {
  return tools.map((tool) => ({
    value: tool.name,
    label: `${toolDisplayName(tool.name, locale)} (${tool.name})`,
    title: `${toolDisplayName(tool.name, locale)} ${tool.name} ${toolDescription(tool, locale)}`,
    tool,
  }));
}

/** Select optionRender：有 tool 数据则 ToolOptionLabel，否则原 label。 */
export function renderToolSelectOption(option: { data?: { tool?: ToolCatalogEntry }; label?: React.ReactNode }, locale: Locale) {
  const tool = option.data?.tool;
  return tool ? <ToolOptionLabel tool={tool} locale={locale} /> : option.label;
}

/** 下拉项：展示名 + code + 描述 + 风险 Tag。 */
export function ToolOptionLabel({ tool, locale }: { tool: ToolCatalogEntry; locale: Locale }) {
  return (
    <div className="xy-tool-option">
      <div className="xy-tool-option-main">
        <span className="xy-tool-option-name">{toolDisplayName(tool.name, locale)}</span>
        <Text code className="xy-tool-option-code">{tool.name}</Text>
      </div>
      <div className="xy-tool-option-meta">
        <span>{toolDescription(tool, locale)}</span>
        <Tag color={toolRiskColor(tool)}>{toolRiskLabel(tool, locale)}</Tag>
      </div>
    </div>
  );
}

/** 已选摘要：展示名 + code + 风险 Tag。 */
export function ToolSummaryLabel({ tool, locale }: { tool: ToolCatalogEntry; locale: Locale }) {
  return (
    <Space size={6} wrap>
      <Text strong>{toolDisplayName(tool.name, locale)}</Text>
      <Text code>{tool.name}</Text>
      <Tag color={toolRiskColor(tool)}>{toolRiskLabel(tool, locale)}</Tag>
    </Space>
  );
}
