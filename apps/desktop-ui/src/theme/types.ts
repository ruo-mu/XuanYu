/** 单套配色在某一明暗模式下的完整 token */
export interface PaletteTokens {
  bg: string;
  bgElevated: string;
  bgSoft: string;
  bgSubtle: string;
  sidebarBg: string;
  sidebarHover: string;
  sidebarActive: string;
  headerBg: string;
  surfaceGlass: string;
  surfaceGlassStrong: string;
  gridDot: string;
  text: string;
  textSoft: string;
  textMuted: string;
  textFaint: string;
  border: string;
  borderStrong: string;
  borderAccent: string;
  primary: string;
  primaryHover: string;
  primarySoft: string;
  accent: string;
  accentSoft: string;
  accent2: string;
  accentGradient: string;
  danger: string;
  warning: string;
  success: string;
  shadowSm: string;
  shadowMd: string;
  shadowCard: string;
  shadowGlow: string;
  shadowFloat: string;
  blurGlass: string;
  glassHighlight: string;
  orb1: string;
  orb2: string;
  orb3: string;
  meshGradient: string;
}

export interface ThemePreset {
  id: string;
  name: string;
  nameEn: string;
  description: string;
  descriptionEn: string;
  preview: [string, string, string];
  light: PaletteTokens;
  dark: PaletteTokens;
}

/**
 * 主题 token 输入 — 品牌 / 强调 / 语义 / 中性 / 表面 / 文字 / 描边。
 */
export interface ThemeCategoryInput {
  /** 品牌 — 主按钮、链接、选中态 */
  brand: { primary: string };
  /** 强调 — 渐变、次要高亮 */
  accent: { secondary: string; tertiary: string };
  /** 语义 — 成功 / 警告 / 错误 */
  semantic: { success: string; warning: string; danger: string };
  /** 文字 — 标题正文 / 次要 / 禁用 */
  text: { primary: string; secondary: string; disabled: string };
  /** 背景 — 页面底 / 抬升 / 弱化 */
  background: { base: string; elevated: string; muted: string };
  /** 表面 — 卡片、毛玻璃、浮层 */
  surface: { glass: string; glassStrong: string };
  /** 边框 — 默认 / 强调 */
  border: { default: string; strong: string };
  /** 氛围 — 背景光晕 blob */
  ambient: { orb1: string; orb2: string; orb3: string };
}

/** 明暗模式各自独立配置 */
export interface CustomThemeStore {
  light: ThemeCategoryInput;
  dark: ThemeCategoryInput;
}

/** 品牌三色输入 — 用于从预设 preview 色生成 palette */
export interface BrandColorInput {
  primary: string;
  accent: string;
  accent2: string;
}

export type ThemeMode = 'light' | 'dark';

export interface ThemeState {
  mode: ThemeMode;
  presetId: string;
  custom: CustomThemeStore | null;
}

export interface ResolvedTheme {
  mode: ThemeMode;
  presetId: string;
  presetName: string;
  tokens: PaletteTokens;
  isCustom: boolean;
}

export type ThemeCategoryKey = keyof ThemeCategoryInput;
