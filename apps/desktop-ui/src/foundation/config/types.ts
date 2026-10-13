/**
 * 应用配置的 TypeScript 类型定义。
 *
 * 描述配置文件被加载后在运行时使用的结构。
 * Desktop 校验与默认值由 Rust core / init wizard 负责；Node 参考仓库另有 Zod schema。
 */

/** 支持的模型供应商标识；新增供应商时需要同步 schema、provider 实现和模型清单。 */
export type ProviderType =
  | 'openai'
  | 'anthropic'
  | 'mistral'
  | 'amazon'
  | 'azure'
  | 'google'
  | 'deepseek'
  | 'tencent'
  | 'mimo'
  | 'longcat'
  | 'huawei'
  | 'qianfan'
  | 'volcano'
  | 'zhipu'
  | 'moonshot'
  | 'minimax'
  | 'modelscope'
  | 'openrouter'
  | 'siliconflow'
  | 'byteplus'
  | 'github'
  | 'ollama'
  | 'opencode'
  | 'xai'
  | 'kimi-code'
  | 'custom';

/** 单条权限规则的结果：允许、询问用户或拒绝。 */
export type ConfigPermissionLevel = 'allow' | 'confirm' | 'deny';
/** 默认权限级别：allow / confirm / block。 */
export type ConfigDefaultPermissionLevel = 'confirm' | 'deny';

/** 权限模式，用于把多条工具规则组合成不同的交互策略。 */
export type ConfigPermissionMode = 'default' | 'plan' | 'auto';

/** 主模型供应商配置，至少需要 provider 类型和模型名。 */
export interface ProviderSettings {
  /** 模型供应商类型，例如 `openai`、`anthropic` 或 `custom`。 */
  type: ProviderType;
  /** 供应商 API Key。 */
  apiKey?: string;
  /** 自定义或兼容 OpenAI 协议的服务地址。 */
  baseUrl?: string;
  /** 供应商侧的模型 ID。 */
  model: string;
  /** 单次请求最大输出 token 数。 */
  maxTokens?: number;
  /** 采样温度，数值越高输出越发散。 */
  temperature?: number;
  retry?: ModelRetrySettings;
  fallback?: AgentModelSettings;
}

/** 深度思考强度：low / medium / high / max / ultracode。 */
export type ReasoningEffort = 'low' | 'medium' | 'high' | 'max' | 'ultracode';

/** 子任务或辅助模型配置，所有字段可选以便继承主模型配置。 */
export interface AgentModelSettings {
  provider?: ProviderType;
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  maxTokens?: number;
  temperature?: number;
  /** 是否开启深度思考（子 Agent 等）。默认 false。 */
  deepThinking?: boolean;
  /** 深度思考强度；仅 deepThinking=true 时生效。 */
  reasoningEffort?: ReasoningEffort;
  retry?: ModelRetrySettings;
  fallback?: AgentModelSettings;
}

/** 可被路由到辅助模型的具体任务类型。 */
export type AuxiliaryModelTask =
  | 'title'
  | 'summary'
  | 'memory'
  | 'intent'
  | 'keyfacts';

/** 所有模型任务种类，包含主模型和辅助模型任务。 */
export type ModelTaskKind = AuxiliaryModelTask | 'main';

/** 模型调用限流配置，分别控制每分钟请求数和并发请求数。 */
export interface ModelRateLimitSettings {
  rpm?: number;
  concurrency?: number;
}

/** 模型 HTTP 请求重试与超时配置。 */
export interface ModelRetrySettings {
  maxAttempts?: number;
  baseDelayMs?: number;
  timeoutMs?: number;
  jitter?: boolean;
}

/** 对单个模型能力和成本的手工修正，用于覆盖模型清单中的默认能力。 */
export interface ModelCapabilityOverride {
  stream?: boolean;
  toolCall?: boolean;
  reasoning?: boolean;
  thinkingDisable?: boolean;
  jsonMode?: boolean;
  usage?: boolean;
  vision?: boolean;
  maxContext?: number;
  maxOutput?: number;
  rateLimit?: ModelRateLimitSettings;
  cost?: {
    /** 输入 token 单价。 */
    input?: number;
    /** 输出 token 单价。 */
    output?: number;
    /** 缓存命中输入 token 单价。 */
    cachedInput?: number;
  };
}

/** 模型路由配置：决定哪些任务可以使用辅助模型、限流和能力覆盖。 */
export interface ModelRoutingSettings {
  enabled: boolean;
  auxiliaryModels?: Partial<Record<AuxiliaryModelTask, AgentModelSettings>>;
  rateLimits?: Partial<Record<ModelTaskKind | 'auxiliary', ModelRateLimitSettings>>;
  retry?: ModelRetrySettings;
  fallback?: AgentModelSettings;
  capabilities?: Record<string, ModelCapabilityOverride>;
}

/** 权限配置，控制工具调用是否直接允许、弹窗询问或拒绝。 */
export interface PermissionSettings {
  /** 未匹配任何规则时采用的默认权限级别。 */
  default: ConfigDefaultPermissionLevel;
  /** 全局权限模式，影响是否自动执行或强制询问。 */
  mode?: ConfigPermissionMode;
  /** 按工具名或通配模式匹配的权限规则列表。 */
  rules: Array<{
    pattern: string;
    level: ConfigPermissionLevel;
  }>;
  /** 永久允许的工具或模式列表（跳过确认，不是启用开关）。 */
  whitelist: string[];
}

/** Web 的基础主题配置。 */
export interface ThemeSettings {
  primaryColor: string;
  secondaryColor: string;
  accentColor: string;
  codeTheme: 'dark' | 'light';
}

/** 外部通道配置；type 由已安装的频道适配器插件声明。 */
export interface ChannelConfig {
  type: string;
  name: string;
  enabled: boolean;
  token?: string;
  mentionOnly?: boolean;
  allowlist?: string[];
  extra?: Record<string, string>;
  tenantId?: string;
  connectionId?: string;
}

/**
 * 本地通信配置。
 *
 * 桌面端固定走 Electron IPC，不存在 TCP 网关：`port` 恒为 0、`bind` 恒为 `desktop-ipc`。
 * 该结构只为兼容引擎侧配置字段保留，用户不可配置。
 */
export interface GatewayConfig {
  enabled: boolean;
  port: number;
  bind: 'desktop-ipc' | 'loopback' | 'lan' | 'all';
  token?: string;
}

/** 桌面端写死的通信配置：不开 TCP 监听，全部流量走 Electron IPC。 */
export const DESKTOP_GATEWAY_DEFAULT: GatewayConfig = {
  enabled: false,
  port: 0,
  bind: 'desktop-ipc',
};

/**
 * 工作区偏好（写入全局配置文件 `xuanyu.json`）。
 * 网关启动时恢复 lastDir 为 process.cwd()，避免每次重启丢回安装目录。
 */
export interface WorkspaceSettings {
  /** 上次打开的项目绝对路径。 */
  lastDir?: string;
  /** 最近打开的项目路径（新在前）。 */
  recentDirs?: string[];
}

/** Web 搜索集成配置，包含开关、供应商和可选端点。 */
export interface WebSearchConfig {
  enabled: boolean;
  provider?: string;
  apiKey?: string;
  endpoint?: string;
  maxResults?: number;
}

/** 工具沙箱模式，决定命令和文件写入可以触达的范围。 */
export type SandboxMode =
  | 'readonly'
  | 'workspace-write'
  | 'worktree-write'
  | 'container';

/** 沙箱内网络访问策略。 */
export type SandboxNetworkPolicy = 'off' | 'package-registry-only' | 'allowlist' | 'full';

/** 沙箱与文件系统隔离配置。 */
export interface SandboxConfig {
  /** 是否启用沙箱保护。 */
  enabled: boolean;
  /** 未显式指定时使用的默认沙箱模式。 */
  defaultMode?: SandboxMode;
  /**
   * OS 原生沙箱（macOS Seatbelt / Linux bwrap；Windows 当前无后端）。
   * 默认关闭；开启后若无法真正包装进程则拒绝执行（fail-closed），不依赖 Docker。
   */
  osSandbox?: {
    enabled?: boolean;
    /** workspace-write/readonly 是否断网，默认 true。 */
    networkOff?: boolean;
  };
  /** 允许作为工具 cwd 的目录列表。 */
  cwdAllowlist: string[];
  /** 对判定为 risky 的 exec 是否自动创建 git worktree（需仓库）。 */
  autoForRiskyCommands: boolean;
  /**
   * 可选容器 runner（高级）。默认关闭；日常隔离走 osSandbox，不依赖 Docker。
   */
  container?: {
    enabled: boolean;
    runner: 'docker' | 'wsl';
    image?: string;
    cpu?: string;
    memory?: string;
    timeoutMs?: number;
    disk?: string;
    network?: SandboxNetworkPolicy;
    networkAllowlist?: string[];
  };
}

/** 子 Agent 编排配置，控制角色模型和 watchdog。并发与嵌套深度由运行时写死。 */
export interface SubAgentSettings {
  configDir?: string;
  /** 长时间无输出或无进展时的自动恢复策略。 */
  watchdog?: {
    silentMs: number;
    noProgressIterations: number;
    maxRetries: number;
  };
  models?: Record<string, AgentModelSettings>;
}

/** Agent loop 编排保护栏。 */
export interface AgentLoopSettings {
  /** 自动验证失败后的最大自动修复轮次（默认 5）。 */
  maxAutoFixPasses?: number;
}

/** 上下文管理配置，控制自动压缩触发水位与记忆注入阈值。 */
export interface ContextManagementSettings {
  /** 超过该输入上下文占比时触发自动压缩（默认 0.85）。 */
  autoCompactThreshold?: number;
  /** 自动注入检索记忆时的最低相关度分数。 */
  memoryAutoInjectMinScore?: number;
  /**
   * Desktop/Rust load mode overlay (`lazy` | `balanced` | `eager`).
   * Node gateway uses intent-driven injects only; keep as optional desktop enhancer.
   */
  loadMode?: 'lazy' | 'balanced' | 'eager';
  /** Max retrieved memory snippets to auto-inject. */
  memoryAutoInjectLimit?: number;
  /** Include cross-session memories when auto-injecting. */
  memoryIncludeCrossSession?: boolean;
  /** Explicit off-switch for retrieved memory inject. */
  injectRetrievedMemory?: boolean;
}

/** Skill 加载配置，控制项目级和用户级 skill 搜索路径。 */
export interface SkillSettings {
  enabled: boolean;
  configDir?: string;
  userDir?: string;
}

/** 本地插件和 marketplace 配置。 */
export interface PluginSettings {
  enabled: boolean;
  directories: string[];
  marketplacePath?: string;
}

/** 长期记忆持久化配置，目前后端固定为 PostgreSQL。 */
export interface MemorySettings {
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
}

/** 完整应用配置对象，加载后会在运行时模块之间传递。 */
export interface AppConfig {
  /** Web 界面语言。 */
  locale?: 'zh-CN' | 'en';
  /** 主 LLM 供应商配置。 */
  provider: ProviderSettings;
  /** 工具权限配置。 */
  permissions: PermissionSettings;
  /** 主题配色配置。 */
  theme: ThemeSettings;
  /** 系统提示词追加内容。 */
  systemPrompt: string;
  /** 最大对话历史 token 数。 */
  maxContextTokens: number;
  /** 显式禁用的工具列表（唯一启用/禁用面）。 */
  disabledTools: string[];
  /** 外部通道集成列表。 */
  channels?: ChannelConfig[];
  /** Gateway 服务配置。 */
  gateway?: GatewayConfig;
  /** 工作区偏好：上次/最近打开的项目目录。 */
  workspace?: WorkspaceSettings;
  /** Web 搜索集成配置。 */
  webSearch?: WebSearchConfig;
  /** 沙箱与文件系统隔离配置。 */
  sandbox?: SandboxConfig;
  /** 多 Agent 编排配置。 */
  subAgents?: SubAgentSettings;
  /** Agent loop 编排策略。 */
  agentLoop?: AgentLoopSettings;
  /** 模型能力矩阵与按任务路由配置。 */
  modelRouting?: ModelRoutingSettings;
  /** 项目/领域 Skill 加载配置。 */
  skills?: SkillSettings;
  /** 插件发现配置。 */
  plugins?: PluginSettings;
  /** 长期记忆持久化配置。 */
  memory?: MemorySettings;
  /** 上下文压缩阈值配置。 */
  contextManagement?: ContextManagementSettings;
  /**
   * 治理预算与配额。未配置则不限制。
   * - quotas.toolCalls：本会话 Runtime 内工具治理放行次数上限
   * - budget：evaluate 带 budgetUsage 时生效；工具路径在配置了 budget 时每次计 1 个 token 单位
   */
  governance?: GovernanceSettings;
}

/** 可选治理限额（默认不限制）。 */
export interface GovernanceSettings {
  budget?: {
    maxTokens?: number;
    maxCostUsd?: number;
  };
  quotas?: {
    /** 工具调用（经 governance.evaluate 放行）次数上限。 */
    toolCalls?: number;
  };
}
