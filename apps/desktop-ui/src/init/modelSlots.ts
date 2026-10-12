/**
 * 初始化向导的模型槽位：主模型 + 标题/摘要/记忆/意图/关键事实辅助槽。
 */

/** 辅助任务槽位 id。 */
export type AuxModelTask = 'title' | 'summary' | 'memory' | 'intent' | 'keyfacts';

/** 槽位：主模型或某一辅助任务。 */
export type ModelSlotId = 'main' | AuxModelTask;

/** 向导里一行槽位的 i18n 名称与说明。 */
export interface ModelSlotDef {
  id: ModelSlotId;
  kind: 'main' | 'auxiliary';
  nameKey: string;
  hintKey: string;
}

/** 向导展示的全部模型槽位。 */
export const MODEL_SLOTS: ModelSlotDef[] = [
  {
    id: 'main',
    kind: 'main',
    nameKey: 'init.models.slot.main.name',
    hintKey: 'init.models.slot.main.hint',
  },
  {
    id: 'title',
    kind: 'auxiliary',
    nameKey: 'init.models.slot.title.name',
    hintKey: 'init.models.slot.title.hint',
  },
  {
    id: 'summary',
    kind: 'auxiliary',
    nameKey: 'init.models.slot.summary.name',
    hintKey: 'init.models.slot.summary.hint',
  },
  {
    id: 'memory',
    kind: 'auxiliary',
    nameKey: 'init.models.slot.memory.name',
    hintKey: 'init.models.slot.memory.hint',
  },
  {
    id: 'intent',
    kind: 'auxiliary',
    nameKey: 'init.models.slot.intent.name',
    hintKey: 'init.models.slot.intent.hint',
  },
  {
    id: 'keyfacts',
    kind: 'auxiliary',
    nameKey: 'init.models.slot.keyfacts.name',
    hintKey: 'init.models.slot.keyfacts.hint',
  },
];

/** 单个槽位可覆盖的供应商、密钥、模型与采样参数。 */
export interface AgentModelPatch {
  provider?: string;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  maxTokens?: number;
  temperature?: number;
}

/** 读取辅助槽配置；未填则空对象（继承主模型）。 */
export function readAuxModel(
  auxiliaryModels: Record<string, AgentModelPatch> | undefined,
  task: AuxModelTask,
): AgentModelPatch {
  return (auxiliaryModels?.[task] as AgentModelPatch | undefined) || {};
}

/** 未填供应商/模型/密钥时视为继承主模型。 */
export function isAuxInherited(
  auxiliaryModels: Record<string, AgentModelPatch> | undefined,
  task: AuxModelTask,
): boolean {
  const value = readAuxModel(auxiliaryModels, task);
  return !value.provider && !value.model && !value.apiKey && !value.baseUrl;
}

/** 审阅页展示「供应商 / 模型」一行。 */
export function formatModelLine(provider: string | undefined, model: string | undefined): string {
  const p = provider?.trim();
  const m = model?.trim();
  if (p && m) return `${p} / ${m}`;
  if (m) return m;
  if (p) return p;
  return '';
}
