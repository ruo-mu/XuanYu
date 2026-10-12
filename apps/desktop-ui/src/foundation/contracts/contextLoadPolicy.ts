/**
 * 上下文段与加载策略契约（类型在 foundation；解析函数在 execution）。
 */

/** 可按需注入的上下文段名。 */
export type ContextSectionName = 'memory' | 'memory_profile' | 'project_instructions';

/** 各上下文段的加载策略。 */
export interface ContextLoadPolicy {
  /** 把显式记忆画像块注入模型请求。 */
  injectMemoryProfile: boolean;
  /** 把检索到的长期记忆片段注入模型请求。 */
  injectRetrievedMemory: boolean;
  /** 注入完整项目指令文件（AGENTS.md / XUANYU.md）。 */
  injectProjectInstructions: boolean;
  /** 注入子 Agent 委派路由表。 */
  injectDelegation: boolean;
  /** 注入 cwd/平台等项目元数据（不是完整指令文件）。 */
  injectProjectContext: boolean;
  /** 注入用户配置的自定义指令（config.systemPrompt）。 */
  injectCustomInstructions: boolean;
  /** 自动注入检索记忆时的最低相关分。 */
  memoryAutoInjectMinScore: number;
  /** 设时从项目指令/画像播种关键事实。 */
  seedKeyFactsFromStaticContext: boolean;
  /** 把运行时关键事实块注入模型请求。 */
  injectKeyFacts: boolean;
  /** 开轮自动激活的段（在 load_memories 之前）。 */
  autoActivateSections: ContextSectionName[];
}
