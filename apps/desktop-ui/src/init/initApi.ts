/** Desktop init wizard draft. */
import { configGet, configInfo, configUpdate, initChannelPluginsList, initChannelPluginInstall } from '../ipc/desktopIpc';
import { DEFAULT_PERMISSION_RULES, DEFAULT_PERMISSIONS } from '../foundation/config/toolEnablement';
import { DESKTOP_GATEWAY_DEFAULT } from '../foundation/config/types';
import { globalAppDir, resolveGlobalConfigFile } from '../foundation/config/paths';
import type { AppConfig } from '../stores/configStore';
import type { ProviderCatalogEntry } from '../protocol';
import {
  providerDefs,
  type InitLocale,
  type InitProviderDef,
  type InitProviderPlan,
} from './providerCatalog';

export type { InitLocale, InitProviderDef, InitProviderPlan };
export { providerDefs };

/** 与全局 AppConfig 同形的向导配置草稿。 */
export type InitConfigDraft = Record<string, unknown> & {
  provider?: {
    type?: string;
    apiKey?: string;
    baseUrl?: string;
    model?: string;
    maxTokens?: number;
    temperature?: number;
    retry?: { maxAttempts?: number; baseDelayMs?: number; timeoutMs?: number; jitter?: boolean };
    fallback?: {
      provider?: string;
      apiKey?: string;
      baseUrl?: string;
      model?: string;
      maxTokens?: number;
      temperature?: number;
    };
  };
  gateway?: { enabled?: boolean; port?: number; bind?: string; token?: string };
  memory?: Record<string, unknown>;
  permissions?: Record<string, unknown>;
  sandbox?: Record<string, unknown>;
  contextManagement?: Record<string, unknown>;
  subAgents?: Record<string, unknown>;
  agentLoop?: Record<string, unknown>;
  modelRouting?: Record<string, unknown>;
  skills?: Record<string, unknown>;
  plugins?: Record<string, unknown>;
  webSearch?: Record<string, unknown>;
  channels?: Array<{
    type: string;
    name: string;
    enabled: boolean;
    token?: string;
    mentionOnly?: boolean;
    allowlist?: string[];
    extra?: Record<string, string>;
  }>;
  disabledTools?: string[];
  theme?: Record<string, unknown>;
  systemPrompt?: string;
  maxContextTokens?: number;
};

export interface InitDraft {
  locale: InitLocale;
  config: InitConfigDraft;
  providerPlanId?: string;
  restartGateway?: boolean;
}

export interface InitMeta {
  appDataDir: string;
  configPath: string;
  configExists: boolean;
  locale: InitLocale;
  platform: string;
  node: string;
  defaults: InitConfigDraft;
  providers: InitProviderDef[];
  searchProviders: Array<{ key: string; label: string; description: string; setupUrl: string }>;
  savedConfig?: InitConfigDraft;
}

export interface InitSaveResult {
  ok: boolean;
  configPath: string;
  errors?: string[];
  gatewayStarted?: boolean;
  gatewayUrl?: string;
  gatewayToken?: string;
  logs?: string[];
}

export interface InitPersistResult {
  ok: boolean;
  configPath: string;
  error?: string;
}

export interface InitChannelPlugin {
  id: string;
  name: string;
  version: string;
  description?: string;
  kind: string;
  channelType?: string;
  summary?: string;
  installed: boolean;
  installedPath?: string;
}

const INIT_CONFIG_KEY = 'xy-desktop-init-config';

export async function fetchInitMeta(locale: InitLocale): Promise<InitMeta> {
  const info = await loadConfigInfo();
  const configPath = info?.path || resolveGlobalConfigFile();
  const saved = await loadSavedConfig();
  const appDataDir = info?.dataDir || globalAppDir();
  return {
    appDataDir,
    configPath,
    configExists: info?.exists ?? !!saved,
    locale,
    platform: navigator.platform || 'desktop',
    node: 'electron-renderer',
    defaults: createDefaultConfig(locale),
    savedConfig: saved || undefined,
    providers: providerDefs(locale),
    searchProviders: [
      {
        key: 'brave',
        label: 'Brave',
        description: 'Brave Search API',
        setupUrl: 'https://api.search.brave.com/',
      },
      {
        key: 'bing',
        label: 'Bing',
        description: 'Bing Web Search',
        setupUrl: 'https://www.microsoft.com/bing/apis/bing-web-search-api',
      },
      {
        key: 'serpapi',
        label: 'SerpAPI',
        description: 'Google-compatible search API',
        setupUrl: 'https://serpapi.com/',
      },
    ],
  };
}

export async function validateInitDraft(config: InitConfigDraft): Promise<{ ok: boolean; errors: string[] }> {
  const errors: string[] = [];
  if (!config.provider?.type) errors.push('Provider is required.');
  if (!config.provider?.model) errors.push('Model is required.');
  return { ok: errors.length === 0, errors };
}

export async function persistInitDraft(draft: InitDraft): Promise<InitPersistResult> {
  const config = { ...draft.config, locale: draft.locale };
  try {
    await configUpdate(config as Partial<AppConfig>);
    window.localStorage.setItem(INIT_CONFIG_KEY, JSON.stringify(config));
    window.localStorage.setItem('xy-desktop-config', JSON.stringify(config));
    return { ok: true, configPath: resolveGlobalConfigFile() };
  } catch (err) {
    return {
      ok: false,
      configPath: resolveGlobalConfigFile(),
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

export async function fetchInitChannelPlugins(_appDir?: string): Promise<InitChannelPlugin[]> {
  const data = await initChannelPluginsList();
  return data.plugins || [];
}

export async function installInitChannelPlugin(
  pluginId: string,
  _appDir?: string,
): Promise<{ ok: boolean; plugin?: InitChannelPlugin; error?: string }> {
  return initChannelPluginInstall(pluginId);
}

export async function saveInitDraft(draft: InitDraft): Promise<InitSaveResult> {
  const config = { ...draft.config, locale: draft.locale };
  await configUpdate(config as Partial<AppConfig>);
  window.localStorage.setItem(INIT_CONFIG_KEY, JSON.stringify(config));
  window.localStorage.setItem('xy-desktop-config', JSON.stringify(config));
  return {
    ok: true,
    configPath: resolveGlobalConfigFile(),
    gatewayStarted: true,
    logs: ['Configuration saved to Rust desktop storage.'],
  };
}

export function hasUsableInitConfig(config: InitConfigDraft | null | undefined): boolean {
  const provider = config?.provider;
  if (!provider?.type || !provider.model) return false;
  return true;
}

async function loadConfigInfo(): Promise<{ path: string; exists: boolean; dataDir: string; projectName: string } | null> {
  try {
    return await configInfo();
  } catch {
    return null;
  }
}

async function loadSavedConfig(): Promise<InitConfigDraft | null> {
  const config = await configGet();
  if (config && Object.keys(config).length > 0) {
    return config as InitConfigDraft;
  }
  return null;
}

function createDefaultConfig(locale: InitLocale): InitConfigDraft {
  return {
    locale,
    provider: {
      type: 'openai',
      model: '',
      maxTokens: 4096,
      temperature: 0.7,
      retry: {
        maxAttempts: 5,
        baseDelayMs: 300,
        timeoutMs: 120_000,
        jitter: true,
      },
    },
    permissions: {
      default: DEFAULT_PERMISSIONS.default,
      mode: DEFAULT_PERMISSIONS.mode,
      rules: [...DEFAULT_PERMISSION_RULES],
      whitelist: [],
    },
    theme: {
      primaryColor: '#00D4AA',
      secondaryColor: '#7C3AED',
      accentColor: '#F59E0B',
      codeTheme: 'dark',
    },
    systemPrompt: '',
    maxContextTokens: 128000,
    disabledTools: [],
    // 通信方式写死：桌面端走 Electron IPC，不暴露网关配置。
    gateway: { ...DESKTOP_GATEWAY_DEFAULT },
    workspace: {
      recentDirs: [],
    },
    sandbox: {
      enabled: true,
      defaultMode: 'workspace-write',
      osSandbox: {
        enabled: false,
        networkOff: true,
      },
      cwdAllowlist: [],
      autoForRiskyCommands: true,
      container: {
        enabled: false,
        runner: 'docker',
        image: 'node:20-bookworm',
        timeoutMs: 30000,
        network: 'off',
        networkAllowlist: [],
      },
    },
    subAgents: {
      configDir: 'capabilities/agents',
      watchdog: {
        silentMs: 120_000,
        noProgressIterations: 8,
        maxRetries: 3,
      },
      models: {},
    },
    agentLoop: {
      maxAutoFixPasses: 5,
    },
    modelRouting: {
      enabled: true,
      auxiliaryModels: {},
      retry: {
        maxAttempts: 5,
        baseDelayMs: 300,
        timeoutMs: 120_000,
        jitter: true,
      },
      rateLimits: {
        title: { rpm: 6, concurrency: 1 },
        summary: { rpm: 4, concurrency: 1 },
        memory: { rpm: 4, concurrency: 1 },
        intent: { rpm: 12, concurrency: 2 },
        keyfacts: { rpm: 12, concurrency: 2 },
        auxiliary: { rpm: 12, concurrency: 2 },
        main: { concurrency: 1 },
      },
      capabilities: {},
    },
    skills: {
      enabled: true,
      configDir: 'capabilities/skills',
    },
    plugins: {
      enabled: true,
      directories: ['plugins'],
      marketplacePath: undefined,
    },
    // 数据库连接串由安装器写进全局配置（memory.backend / memory.postgres.connectionString）。
    // 向导只补充阈值类字段，不写 backend，避免覆盖安装器写入的连接串（configUpdate 是深合并）。
    memory: {
      maxEntries: 1000,
      cleanupImportanceThreshold: 3,
      postgres: {
        schema: 'public',
        tablePrefix: 'xy',
        vectorDimensions: 1536,
      },
    },
    contextManagement: {
      autoCompactThreshold: 0.85,
    },
    webSearch: {
      enabled: false,
      provider: 'brave',
    },
  };
}

/** 供应商元数据目录（不含静态模型列表）。 */
export function getProviderCatalog(locale: InitLocale = 'zh-CN'): ProviderCatalogEntry[] {
  return providerDefs(locale).map((provider) => ({
    type: provider.type,
    name: provider.label,
    defaultBaseUrl: provider.plans[0]?.baseUrl,
  }));
}
