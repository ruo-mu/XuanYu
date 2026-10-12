/**
 * 内置主题预设：每套含 id、中英名称与亮/暗 palette。
 */

import type { ThemePreset } from './types';
import { buildCustomPalette } from './utils';

/** 被谁调用：presets 表。做什么：构造 ThemePreset 条目。 */
function preset(
  id: string,
  name: string,
  nameEn: string,
  description: string,
  descriptionEn: string,
  primary: string,
  accent: string,
  accent2: string,
): ThemePreset {
  const light = buildCustomPalette({ primary, accent, accent2 }, 'light');
  const dark = buildCustomPalette({ primary, accent, accent2 }, 'dark');
  return {
    id,
    name,
    nameEn,
    description,
    descriptionEn,
    preview: [primary, accent, accent2],
    light,
    dark,
  };
}

/** 8 套完整现代化配色 */
export const THEME_PRESETS: ThemePreset[] = [
  preset(
    'cyber-cyan',
    '赛博青',
    'Cyber Cyan',
    '青蓝主调 + 靛紫辅色，经典控制台科技感',
    'Cyan-blue primary with indigo accent — classic console tech feel',
    '#06b6d4',
    '#6366f1',
    '#10b981',
  ),
  preset(
    'aurora-violet',
    '极光紫',
    'Aurora Violet',
    '深紫渐变 + 粉蓝点缀，梦幻极光风格',
    'Deep violet gradient with pink-blue highlights — aurora dreamscape',
    '#8b5cf6',
    '#ec4899',
    '#38bdf8',
  ),
  preset(
    'emerald-matrix',
    '矩阵绿',
    'Emerald Matrix',
    '荧光绿 + 青蓝，黑客终端矩阵感',
    'Neon green with teal — hacker terminal matrix vibe',
    '#10b981',
    '#14b8a6',
    '#22d3ee',
  ),
  preset(
    'solar-amber',
    '日冕金',
    'Solar Amber',
    '琥珀金 + 橙红，温暖能量仪表盘',
    'Amber gold with orange-red — warm energy dashboard',
    '#f59e0b',
    '#f97316',
    '#ef4444',
  ),
  preset(
    'rose-neon',
    '霓虹玫',
    'Rose Neon',
    '玫红 + 紫罗兰，赛博朋克霓虹街景',
    'Rose pink with violet — cyberpunk neon streetscape',
    '#f43f5e',
    '#d946ef',
    '#818cf8',
  ),
  preset(
    'ocean-deep',
    '深海蓝',
    'Ocean Deep',
    '深海蓝 + 天青，沉稳企业级 SaaS',
    'Deep ocean blue with sky cyan — enterprise SaaS calm',
    '#3b82f6',
    '#0ea5e9',
    '#06b6d4',
  ),
  preset(
    'monochrome-pro',
    '极简灰',
    'Monochrome Pro',
    '锌灰 + 冰蓝点缀，极简专业风',
    'Zinc gray with ice-blue accent — minimal professional',
    '#71717a',
    '#a1a1aa',
    '#38bdf8',
  ),
  preset(
    'lava-pulse',
    '熔岩红',
    'Lava Pulse',
    '熔岩红 + 橙黄，高能量告警面板',
    'Lava red with orange-yellow — high-energy alert panel',
    '#ef4444',
    '#f97316',
    '#eab308',
  ),
];

export const DEFAULT_PRESET_ID = 'cyber-cyan';

/** 被谁调用：主题 UI。做什么：按 id 取预设。 */
export function getPresetById(id: string): ThemePreset | undefined {
  return THEME_PRESETS.find((p) => p.id === id);
}

/** 被谁调用：主题 UI。做什么：预设显示名。 */
export function getPresetLabel(preset: ThemePreset, locale: 'zh-CN' | 'en'): string {
  return locale === 'en' ? preset.nameEn : preset.name;
}

/** 被谁调用：主题 UI。做什么：预设说明。 */
export function getPresetDescription(preset: ThemePreset, locale: 'zh-CN' | 'en'): string {
  return locale === 'en' ? preset.descriptionEn : preset.description;
}
