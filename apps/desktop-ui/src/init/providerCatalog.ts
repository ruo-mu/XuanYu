/**
 * 供应商目录：接入点元数据（type / plans / protocol）。
 * 模型列表一律走实时探索 + 缓存，这里不放静态模型占位。
 */

export type InitLocale = 'zh-CN' | 'en';

export interface InitProviderPlan {
  id: string;
  label: string;
  description: string;
  baseUrl: string;
  protocol: 'openai' | 'anthropic';
}

export interface InitProviderDef {
  type: string;
  label: string;
  description: string;
  needsApiKey: boolean;
  plans: InitProviderPlan[];
}

function providerPlan(
  id: string,
  label: string,
  description: string,
  baseUrl: string,
  protocol: InitProviderPlan['protocol'],
): InitProviderPlan {
  return { id, label, description, baseUrl, protocol };
}

/** 被谁调用：init / modelCatalog。做什么：返回供应商接入点定义。 */
export function providerDefs(locale: InitLocale): InitProviderDef[] {
  const zh = locale === 'zh-CN';
  return [
    {
      type: 'mimo',
      label: zh ? '小米 MiMo' : 'Xiaomi MiMo',
      description: zh ? '小米多模态模型官方接入点' : 'Official Xiaomi MiMo endpoints',
      needsApiKey: true,
      plans: [
        providerPlan('mimo-payg-openai', zh ? '按量付费(OpenAI兼容)' : 'Pay-as-you-go (OpenAI compatible)', 'api.xiaomimimo.com', 'https://api.xiaomimimo.com/v1', 'openai'),
        providerPlan('mimo-payg-anthropic', zh ? '按量付费(Anthropic兼容)' : 'Pay-as-you-go (Anthropic compatible)', 'api.xiaomimimo.com/anthropic', 'https://api.xiaomimimo.com/anthropic', 'anthropic'),
        providerPlan('mimo-token-cn-openai', zh ? 'TokenPlan中国(OpenAI兼容)' : 'TokenPlan CN (OpenAI compatible)', 'token-plan-cn.xiaomimimo.com', 'https://token-plan-cn.xiaomimimo.com/v1', 'openai'),
        providerPlan('mimo-token-sgp-openai', zh ? 'TokenPlan新加坡(OpenAI兼容)' : 'TokenPlan Singapore (OpenAI compatible)', 'token-plan-sgp.xiaomimimo.com', 'https://token-plan-sgp.xiaomimimo.com/v1', 'openai'),
        providerPlan('mimo-token-ams-openai', zh ? 'TokenPlan阿姆斯特丹(OpenAI兼容)' : 'TokenPlan Amsterdam (OpenAI compatible)', 'token-plan-ams.xiaomimimo.com', 'https://token-plan-ams.xiaomimimo.com/v1', 'openai'),
      ],
    },
    {
      type: 'deepseek',
      label: 'DeepSeek',
      description: zh ? 'DeepSeek 官方接入点' : 'Official DeepSeek endpoints',
      needsApiKey: true,
      plans: [
        providerPlan('deepseek-openai', zh ? 'OpenAI兼容' : 'OpenAI compatible', 'api.deepseek.com/v1', 'https://api.deepseek.com/v1', 'openai'),
        providerPlan('deepseek-anthropic', zh ? 'Anthropic兼容' : 'Anthropic compatible', 'api.deepseek.com/anthropic', 'https://api.deepseek.com/anthropic', 'anthropic'),
      ],
    },
    {
      type: 'openai',
      label: 'OpenAI',
      description: zh ? 'OpenAI 官方 API' : 'OpenAI official API',
      needsApiKey: true,
      plans: [
        providerPlan('openai-official', zh ? '官方API' : 'Official API', 'api.openai.com', 'https://api.openai.com/v1', 'openai'),
      ],
    },
    {
      type: 'anthropic',
      label: 'Anthropic Claude',
      description: zh ? 'Anthropic 官方 API' : 'Anthropic official API',
      needsApiKey: true,
      plans: [
        providerPlan('anthropic-official', zh ? '官方API' : 'Official API', 'api.anthropic.com', 'https://api.anthropic.com', 'anthropic'),
      ],
    },
    {
      type: 'google',
      label: zh ? 'Google Gemini' : 'Google Gemini',
      description: zh ? 'Google Gemini 官方 API' : 'Google Gemini official API',
      needsApiKey: true,
      plans: [
        providerPlan('google-official', zh ? '官方API' : 'Official API', 'generativelanguage.googleapis.com', 'https://generativelanguage.googleapis.com/v1beta/openai', 'openai'),
      ],
    },
    {
      type: 'zhipu',
      label: zh ? '智谱 GLM' : 'Zhipu GLM',
      description: zh ? '智谱官方接入点' : 'Official Zhipu endpoints',
      needsApiKey: true,
      plans: [
        providerPlan('zhipu-general', zh ? '通用API' : 'General API', 'open.bigmodel.cn/api/paas/v4', 'https://open.bigmodel.cn/api/paas/v4', 'openai'),
        providerPlan('zhipu-coding', zh ? 'Coding编程套餐专属API' : 'Coding package API', 'open.bigmodel.cn/api/coding/paas/v4', 'https://open.bigmodel.cn/api/coding/paas/v4', 'openai'),
      ],
    },
    {
      type: 'siliconflow',
      label: zh ? '硅基流动 SiliconFlow' : 'SiliconFlow',
      description: zh ? 'SiliconFlow OpenAI 兼容 API' : 'SiliconFlow OpenAI-compatible API',
      needsApiKey: true,
      plans: [
        providerPlan('siliconflow-openai', zh ? 'OpenAI兼容' : 'OpenAI compatible', 'api.siliconflow.cn/v1', 'https://api.siliconflow.cn/v1', 'openai'),
      ],
    },
    {
      type: 'longcat',
      label: 'LongCat',
      description: zh ? 'LongCat 官方接入点' : 'Official LongCat endpoints',
      needsApiKey: true,
      plans: [
        providerPlan('longcat-openai', zh ? 'OpenAI兼容' : 'OpenAI compatible', 'api.longcat.chat/openai/v1', 'https://api.longcat.chat/openai/v1', 'openai'),
        providerPlan('longcat-anthropic', zh ? 'Anthropic兼容' : 'Anthropic compatible', 'api.longcat.chat/anthropic', 'https://api.longcat.chat/anthropic', 'anthropic'),
      ],
    },
    {
      type: 'moonshot',
      label: zh ? 'Kimi / Moonshot' : 'Kimi / Moonshot',
      description: zh ? 'Moonshot 官方接入点' : 'Official Moonshot endpoints',
      needsApiKey: true,
      plans: [
        providerPlan('moonshot-cn', zh ? '国内开放平台(OpenAI兼容)' : 'China platform (OpenAI compatible)', 'api.moonshot.cn/v1', 'https://api.moonshot.cn/v1', 'openai'),
        providerPlan('moonshot-global', zh ? '海外开放平台(OpenAI兼容)' : 'Global platform (OpenAI compatible)', 'api.moonshot.ai/v1', 'https://api.moonshot.ai/v1', 'openai'),
      ],
    },
    {
      type: 'mistral',
      label: 'Mistral AI',
      description: zh ? 'Mistral OpenAI 兼容 API' : 'Mistral OpenAI-compatible API',
      needsApiKey: true,
      plans: [
        providerPlan('mistral-openai', zh ? 'OpenAI兼容' : 'OpenAI compatible', 'api.mistral.ai/v1', 'https://api.mistral.ai/v1', 'openai'),
      ],
    },
    {
      type: 'openrouter',
      label: 'OpenRouter',
      description: zh ? '统一 API 网关' : 'Unified API gateway',
      needsApiKey: true,
      plans: [
        providerPlan('openrouter-default', zh ? '统一API网关' : 'Unified API gateway', 'openrouter.ai/api/v1', 'https://openrouter.ai/api/v1', 'openai'),
      ],
    },
    {
      type: 'ollama',
      label: zh ? 'Ollama（本地）' : 'Ollama (Local)',
      description: zh ? '本地模型服务' : 'Local model service',
      needsApiKey: false,
      plans: [
        providerPlan('ollama-local', zh ? '本地服务' : 'Local service', 'localhost:11434/v1', 'http://localhost:11434/v1', 'openai'),
      ],
    },
  ];
}
