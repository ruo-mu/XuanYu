/**
 * config-store.ts - 配置状态仓库
 *
 * 用途：管理应用配置、扩展列表和模型能力探测结果。
 * 所有配置字段通过浅合并更新，保持与后端配置结构一致。
 */

import { create } from 'zustand';
import type { ExtensionsListPayload, ModelCapabilityRoute, ToolCatalogEntry } from '../protocol';

/** 辅助模型任务键：标题 / 摘要 / 记忆 / 意图 / 关键事实。 */
export type AuxiliaryModelTask =
  | 'title'
  | 'summary'
  | 'memory'
  | 'intent'
  | 'keyfacts';

/** 子 Agent 或辅助路由可覆盖的模型字段。 */
export interface AgentModelSettings {
  /** 模型提供方。 */
  provider?: string;
  /** API key。 */
  apiKey?: string;
  /** 自定义 base URL。 */
  baseUrl?: string;
  /** 模型名。 */
  model?: string;
  /** 最大输出 token。 */
  maxTokens?: number;
  /** 采样温度。 */
  temperature?: number;
  /** 是否开启深度思考。 */
  deepThinking?: boolean;
  /** 深度思考强度。 */
  reasoningEffort?: 'low' | 'medium' | 'high' | 'max' | 'ultracode';
}

/** 与后端 AppConfig 对齐的前端配置快照。 */
export interface AppConfig {
  /** 界面语言。 */
  locale?: 'zh-CN' | 'en';
  /** 主模型配置。 */
  provider: {
    type: string;
    apiKey?: string;
    baseUrl?: string;
    model: string;
    maxTokens?: number;
    temperature?: number;
  };
  permissions: {
    default: 'confirm' | 'deny';
    mode?: 'default' | 'plan' | 'auto';
    rules: Array<{ pattern: string; level: string }>;
    whitelist: string[];
  };
  theme: {
    primaryColor: string;
    secondaryColor: string;
    accentColor: string;
    codeTheme: 'dark' | 'light';
  };
  systemPrompt: string;
  maxContextTokens: number;
  disabledTools: string[];
  channels?: Array<{
    type: string;
    name: string;
    enabled: boolean;
    token?: string;
    mentionOnly?: boolean;
    allowlist?: string[];
    extra?: Record<string, string>;
  }>;
  memory?: {
    backend: 'postgres';
    maxEntries: number;
    cleanupImportanceThreshold: number;
    embedding?: {
      provider?: 'openai' | 'local' | 'hash';
      model?: string;
      dimensions?: number;
      apiKey?: string;
      baseUrl?: string;
      timeoutMs?: number;
      batchSize?: number;
    };
    postgres?: {
      schema?: string;
      tablePrefix?: string;
      vectorDimensions?: number;
    };
  };
  sandbox?: {
    enabled: boolean;
    cwdAllowlist: string[];
    autoForRiskyCommands: boolean;
    defaultMode?: 'readonly' | 'workspace-write' | 'worktree-write' | 'container';
    osSandbox?: {
      enabled?: boolean;
      networkOff?: boolean;
    };
    container?: {
      enabled: boolean;
      runner: 'docker' | 'wsl';
      image?: string;
      network?: 'off' | 'package-registry-only' | 'allowlist' | 'full';
    };
  };
  subAgents?: {
    configDir?: string;
    watchdog?: {
      silentMs: number;
      noProgressIterations: number;
      maxRetries: number;
    };
    models?: Record<string, AgentModelSettings>;
  };
  agentLoop?: {
    maxAutoFixPasses?: number;
  };
  modelRouting?: {
    enabled: boolean;
    auxiliaryModels?: Partial<Record<AuxiliaryModelTask, AgentModelSettings>>;
    rateLimits?: Record<string, { rpm?: number; concurrency?: number }>;
    capabilities?: Record<
      string,
      {
        stream?: boolean;
        toolCall?: boolean;
        reasoning?: boolean;
        thinkingDisable?: boolean;
        jsonMode?: boolean;
        usage?: boolean;
        maxContext?: number;
        maxOutput?: number;
        rateLimit?: { rpm?: number; concurrency?: number };
        cost?: { input?: number; output?: number; cachedInput?: number };
      }
    >;
  };
  webSearch?: {
    enabled: boolean;
    provider?: string;
    apiKey?: string;
    endpoint?: string;
  };
  skills?: {
    enabled: boolean;
    configDir?: string;
    userDir?: string;
  };
  plugins?: {
    enabled: boolean;
    directories: string[];
    marketplacePath?: string;
  };
  gateway?: {
    enabled: boolean;
    port: number;
    bind: string;
  };
  workspace?: {
    lastDir?: string;
    recentDirs?: string[];
  };
  contextManagement?: {
    autoCompactThreshold?: number;
    memoryAutoInjectMinScore?: number;
    loadMode?: 'lazy' | 'balanced' | 'eager';
    memoryAutoInjectLimit?: number;
    memoryIncludeCrossSession?: boolean;
    injectRetrievedMemory?: boolean;
  };
  governance?: {
    budget?: { maxTokens?: number; maxCostUsd?: number };
    quotas?: { toolCalls?: number };
  };
}

interface ConfigStore {
  /** 当前配置快照。 */
  config: AppConfig | null;
  /** 已安装扩展。 */
  extensions: ExtensionsListPayload | null;
  /** 模型能力列表。 */
  capabilities: ModelCapabilityRoute[];
  /** 后端真实工具目录。 */
  toolCatalog: ToolCatalogEntry[];
  setConfig: (config: AppConfig) => void;
  setExtensions: (extensions: ExtensionsListPayload | null) => void;
  setCapabilities: (capabilities: ModelCapabilityRoute[]) => void;
  setToolCatalog: (tools: ToolCatalogEntry[]) => void;
  updateConfig: (partial: Partial<AppConfig>) => void;
}

/** Zustand 配置仓库；useWebSocket config.* / tools.catalog 写入。 */
export const useConfigStore = create<ConfigStore>((set) => ({
  config: null,
  extensions: null,
  capabilities: [],
  toolCatalog: [],

  setConfig: (config) => set({ config }),
  setExtensions: (extensions) => set({ extensions }),
  setCapabilities: (capabilities) => set({ capabilities }),
  setToolCatalog: (toolCatalog) => set({ toolCatalog }),

  updateConfig: (partial) =>
    set((s) => {
      if (!s.config) return { config: null };
      return {
        config: {
          ...s.config,
          ...partial,
          // provider / permissions 等嵌套对象必须深合并，避免只改 model 时把 type/apiKey 冲掉。
          provider: partial.provider
            ? { ...s.config.provider, ...partial.provider }
            : s.config.provider,
          permissions: partial.permissions
            ? { ...s.config.permissions, ...partial.permissions }
            : s.config.permissions,
          theme: partial.theme ? { ...s.config.theme, ...partial.theme } : s.config.theme,
          workspace: partial.workspace
            ? { ...s.config.workspace, ...partial.workspace }
            : s.config.workspace,
          memory: partial.memory
            ? ({ ...s.config.memory, ...partial.memory } as AppConfig['memory'])
            : s.config.memory,
          sandbox: partial.sandbox
            ? ({ ...s.config.sandbox, ...partial.sandbox } as AppConfig['sandbox'])
            : s.config.sandbox,
          subAgents: partial.subAgents
            ? ({ ...s.config.subAgents, ...partial.subAgents } as AppConfig['subAgents'])
            : s.config.subAgents,
          modelRouting: partial.modelRouting
            ? ({ ...s.config.modelRouting, ...partial.modelRouting } as AppConfig['modelRouting'])
            : s.config.modelRouting,
        } as AppConfig,
      };
    }),
}));
