/**
 * 主题缺省 token：亮/暗各一套品牌、语义、表面与环境光颜色。
 */

import type { CustomThemeStore, ThemeCategoryInput, ThemeMode } from './types';

/** 亮色 — 企业级设计 token 默认值（tech-ui 灰白毛玻璃基线） */
export const DEFAULT_THEME_CATEGORIES_LIGHT: ThemeCategoryInput = {
  brand: { primary: '#0891b2' },
  accent: { secondary: '#6366f1', tertiary: '#10b981' },
  semantic: { success: '#10b981', warning: '#f59e0b', danger: '#ef4444' },
  text: { primary: '#1a1a1e', secondary: '#4a4a50', disabled: '#6a6a72' },
  background: { base: '#ececee', elevated: '#f4f4f6', muted: '#e8e8ec' },
  surface: {
    glass: 'rgba(255, 255, 255, 0.72)',
    glassStrong: 'rgba(255, 255, 255, 0.88)',
  },
  border: { default: 'rgba(0, 0, 0, 0.08)', strong: 'rgba(0, 0, 0, 0.12)' },
  ambient: {
    orb1: 'rgba(186, 190, 210, 0.34)',
    orb2: 'rgba(168, 176, 200, 0.26)',
    orb3: 'rgba(178, 186, 212, 0.3)',
  },
};

/** 暗色 — 企业级设计 token 默认值 */
export const DEFAULT_THEME_CATEGORIES_DARK: ThemeCategoryInput = {
  brand: { primary: '#22d3ee' },
  accent: { secondary: '#818cf8', tertiary: '#34d399' },
  semantic: { success: '#34d399', warning: '#fbbf24', danger: '#f87171' },
  text: { primary: '#ececef', secondary: '#9494a0', disabled: '#686874' },
  background: { base: '#0c0c10', elevated: '#101014', muted: '#18181e' },
  surface: {
    glass: 'rgba(255, 255, 255, 0.06)',
    glassStrong: 'rgba(255, 255, 255, 0.09)',
  },
  border: { default: 'rgba(255, 255, 255, 0.12)', strong: 'rgba(255, 255, 255, 0.18)' },
  ambient: {
    orb1: 'rgba(78, 84, 108, 0.42)',
    orb2: 'rgba(60, 68, 88, 0.34)',
    orb3: 'rgba(70, 78, 100, 0.36)',
  },
};

/** 深拷贝默认分类 token，避免调用方改到常量。 */
export function getDefaultCategories(mode: ThemeMode): ThemeCategoryInput {
  const base = mode === 'dark' ? DEFAULT_THEME_CATEGORIES_DARK : DEFAULT_THEME_CATEGORIES_LIGHT;
  return JSON.parse(JSON.stringify(base)) as ThemeCategoryInput;
}

/** 浅色+深色各一份默认分类。 */
export function getDefaultCustomTheme(): CustomThemeStore {
  return {
    light: getDefaultCategories('light'),
    dark: getDefaultCategories('dark'),
  };
}

/** 从品牌三色生成完整类目（预设配色用） */
export function categoriesFromBrandColors(
  primary: string,
  accent: string,
  accent2: string,
  mode: ThemeMode,
): ThemeCategoryInput {
  const base = getDefaultCategories(mode);
  return {
    ...base,
    brand: { primary },
    accent: { secondary: accent, tertiary: accent2 },
  };
}
