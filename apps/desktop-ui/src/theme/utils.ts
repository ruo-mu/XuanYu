/**
 * 主题工具：hex/rgba 换算、从品牌色生成 palette、写出 CSS 变量。
 */

import type {
  BrandColorInput,
  CustomThemeStore,
  PaletteTokens,
  ThemeCategoryInput,
  ThemeMode,
} from './types';
import { categoriesFromBrandColors, getDefaultCategories } from './themeDefaults';

/** #rgb / #rrggbb → [r,g,b]；非法输入当黑。 */
function hexToRgb(hex: string): [number, number, number] {
  const raw = hex.replace('#', '');
  const full = raw.length === 3 ? raw.split('').map((c) => c + c).join('') : raw;
  const num = Number.parseInt(full, 16);
  return [(num >> 16) & 255, (num >> 8) & 255, num & 255];
}

/** 被谁调用：主题色工具。做什么：颜色加 alpha。 */
function rgbaFromColor(color: string, alpha: number): string {
  if (color.startsWith('rgba')) return color;
  if (color.startsWith('rgb(')) {
    const inner = color.slice(4, -1);
    return `rgba(${inner}, ${alpha})`;
  }
  const [r, g, b] = hexToRgb(color);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** 从 rgba(...)/rgba(...) 抽 alpha；纯色当 1。 */
function colorAlpha(color: string): number {
  const m = color.match(/rgba?\([^)]*,\s*([\d.]+)\s*\)/);
  return m ? Number.parseFloat(m[1]) : 1;
}

/** hex 向白色混合 amount（0–1）变亮。 */
function lighten(hex: string, amount: number): string {
  const [r, g, b] = hexToRgb(hex);
  const mix = (c: number) => Math.round(c + (255 - c) * amount);
  return `#${[mix(r), mix(g), mix(b)].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** hex 乘 (1-amount) 变暗。 */
function darken(hex: string, amount: number): string {
  const [r, g, b] = hexToRgb(hex);
  const mix = (c: number) => Math.round(c * (1 - amount));
  return `#${[mix(r), mix(g), mix(b)].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** 从 7 大类设计 token 生成完整运行时 palette */
export function buildPaletteFromCategories(
  input: ThemeCategoryInput,
  mode: ThemeMode,
): PaletteTokens {
  const isDark = mode === 'dark';
  const { brand, accent, semantic, text, background, surface, border, ambient } = input;

  const glassHighlight = isDark
    ? 'rgba(255, 255, 255, 0.1)'
    : rgbaFromColor('#ffffff', Math.min(colorAlpha(surface.glass) + 0.25, 0.85));

  const glassShadow = isDark
    ? `0 30px 80px rgba(0, 0, 0, 0.45), inset 0 1px 0 ${glassHighlight}, 0 0 0 1px rgba(255, 255, 255, 0.04)`
    : `0 30px 80px rgba(0, 0, 0, 0.12), inset 0 1px 0 ${glassHighlight}, 0 0 0 1px rgba(0, 0, 0, 0.04)`;

  const sidebarHover = isDark
    ? 'rgba(255, 255, 255, 0.06)'
    : rgbaFromColor('#ffffff', 0.35);

  return {
    bg: background.base,
    bgElevated: background.elevated,
    bgSoft: background.muted,
    bgSubtle: isDark ? background.muted : background.base,
    sidebarBg: 'transparent',
    sidebarHover,
    sidebarActive: rgbaFromColor(brand.primary, isDark ? 0.14 : 0.1),
    headerBg: surface.glass,
    surfaceGlass: surface.glass,
    surfaceGlassStrong: surface.glassStrong,
    gridDot: 'transparent',
    text: text.primary,
    textSoft: text.secondary,
    textMuted: text.secondary,
    textFaint: text.disabled,
    border: border.default,
    borderStrong: border.strong,
    borderAccent: rgbaFromColor(brand.primary, isDark ? 0.35 : 0.28),
    primary: brand.primary,
    primaryHover: isDark ? lighten(brand.primary, 0.1) : darken(brand.primary, 0.08),
    primarySoft: rgbaFromColor(brand.primary, isDark ? 0.14 : 0.1),
    accent: accent.secondary,
    accentSoft: rgbaFromColor(accent.secondary, isDark ? 0.12 : 0.08),
    accent2: accent.tertiary,
    accentGradient: `linear-gradient(135deg, ${brand.primary}, ${accent.secondary}, ${accent.tertiary})`,
    danger: semantic.danger,
    warning: semantic.warning,
    success: semantic.success,
    shadowSm: isDark ? '0 1px 2px rgba(0,0,0,0.4)' : '0 4px 12px rgba(0, 0, 0, 0.06)',
    shadowMd: isDark ? '0 16px 48px rgba(0,0,0,0.5)' : '0 16px 48px rgba(0, 0, 0, 0.1)',
    shadowCard: glassShadow,
    shadowGlow: isDark
      ? `0 20px 60px rgba(0,0,0,0.4), 0 0 40px ${rgbaFromColor(brand.primary, 0.12)}`
      : `0 20px 60px rgba(0, 0, 0, 0.1), 0 0 32px ${rgbaFromColor(brand.primary, 0.08)}`,
    shadowFloat: isDark
      ? `0 40px 100px rgba(0, 0, 0, 0.55), inset 0 1px 0 ${glassHighlight}`
      : `0 40px 100px rgba(0, 0, 0, 0.14), inset 0 1px 0 ${glassHighlight}`,
    blurGlass: isDark ? '48px' : '40px',
    glassHighlight,
    orb1: ambient.orb1,
    orb2: ambient.orb2,
    orb3: ambient.orb3,
    meshGradient: isDark
      ? `linear-gradient(145deg, ${background.base} 0%, ${background.elevated} 48%, ${background.muted} 100%)`
      : `linear-gradient(145deg, ${background.muted} 0%, ${background.elevated} 48%, ${background.muted} 100%)`,
  };
}

/** 从品牌三色生成 palette（预设用） */
export function buildCustomPalette(input: BrandColorInput, mode: ThemeMode): PaletteTokens {
  return buildPaletteFromCategories(
    categoriesFromBrandColors(input.primary, input.accent, input.accent2, mode),
    mode,
  );
}

/** 从自定义主题 store 取当前 mode 的分类，生成运行时 PaletteTokens。 */
export function buildThemeFromStore(store: CustomThemeStore, mode: ThemeMode): PaletteTokens {
  const categories = store[mode] ?? getDefaultCategories(mode);
  return buildPaletteFromCategories(categories, mode);
}

/**
 * Dock：中性哑光台面 + 无色图标（不吃品牌紫/青）。
 * 台面刻意抬离页面底色，保证轮廓可读；选中态只靠左侧圆点用 primary。
 */
function buildDockCssVars(tokens: PaletteTokens, mode: ThemeMode): Record<string, string> {
  const isDark = mode === 'dark';
  const ink = (alpha: number) => rgbaFromColor(tokens.text, alpha);
  const white = (alpha: number) => rgbaFromColor('#ffffff', alpha);
  const black = (alpha: number) => `rgba(0, 0, 0, ${alpha})`;

  if (isDark) {
    // 抬升石墨台：比页面底更亮一档，边线 + 投影勾出体积
    const shelfDeep = lighten(tokens.bgSubtle.startsWith('#') ? tokens.bgSubtle : '#18181e', 0.08);
    const shelfMid = lighten(tokens.bgElevated.startsWith('#') ? tokens.bgElevated : '#101014', 0.14);
    const shelfLip = lighten(shelfMid, 0.12);
    return {
      '--xy-dock-platform-bg': `linear-gradient(105deg, ${rgbaFromColor(shelfDeep, 0.97)} 0%, ${rgbaFromColor(shelfMid, 0.96)} 48%, ${rgbaFromColor(shelfLip, 0.94)} 100%)`,
      '--xy-dock-platform-border': white(0.2),
      '--xy-dock-platform-shadow': `36px 0 48px ${black(0.55)}, 12px 0 20px ${black(0.35)}, inset 1px 0 0 ${white(0.22)}, inset -14px 0 28px ${black(0.28)}, inset 0 0 0 1px ${black(0.25)}`,
      '--xy-dock-platform-sheen': `linear-gradient(90deg, transparent 8%, ${white(0.03)} 40%, ${white(0.14)} 58%, ${white(0.05)} 78%, transparent 100%), linear-gradient(180deg, ${white(0.08)} 0%, transparent 22%, transparent 78%, ${black(0.18)} 100%)`,
      '--xy-dock-platform-edge': `linear-gradient(to left, ${white(0.28)}, transparent)`,
      // 中性深灰图标，无品牌染色
      '--xy-dock-icon-body': `linear-gradient(160deg, ${lighten('#3a3a42', 0.12)} 0%, #2e2e34 46%, #242428 100%)`,
      '--xy-dock-icon-border': white(0.16),
      '--xy-dock-icon-inset': `inset 0 1px 0 ${white(0.22)}, inset 0 -8px 14px ${black(0.28)}`,
      '--xy-dock-icon-facet': `linear-gradient(125deg, ${white(0.28)} 0%, ${white(0.08)} 22%, transparent 48%), linear-gradient(180deg, ${white(0.1)} 0%, transparent 36%)`,
      '--xy-dock-icon-rim': `inset 0 0 0 1px ${white(0.08)}`,
      '--xy-dock-icon-shadow': `0 8px 18px -8px ${black(0.65)}, 0 2px 4px ${black(0.4)}`,
      '--xy-dock-icon-shadow-hot': `0 10px 22px -8px ${black(0.7)}, 0 2px 6px ${black(0.45)}`,
      '--xy-dock-icon-glyph': white(0.92),
      '--xy-dock-icon-glyph-filter': `drop-shadow(0 1px 1px ${black(0.45)})`,
      '--xy-dock-face-drop': `drop-shadow(-3px 2px 5px ${black(0.4)})`,
      '--xy-dock-reflect-opacity': '0.28',
      '--xy-dock-reflect-opacity-hot': '0.36',
      '--xy-dock-reflect-filter': 'blur(1.6px) brightness(0.85) saturate(0.55)',
      '--xy-dock-reflect-filter-hot': 'blur(1.4px) brightness(0.9) saturate(0.6)',
      '--xy-dock-grain-opacity': '0.12',
    };
  }

  // 浅色：偏暖灰石台，比页面底略深，避免白糊成一团
  const stoneDeep = darken(tokens.bgSubtle.startsWith('#') ? tokens.bgSubtle : '#ececee', 0.1);
  const stoneMid = darken(tokens.bgSoft.startsWith('#') ? tokens.bgSoft : '#e8e8ec', 0.04);
  return {
    '--xy-dock-platform-bg': `linear-gradient(105deg, ${rgbaFromColor(stoneDeep, 0.98)} 0%, ${rgbaFromColor(stoneMid, 0.96)} 42%, ${white(0.88)} 78%, ${white(0.94)} 100%)`,
    '--xy-dock-platform-border': ink(0.14),
    '--xy-dock-platform-shadow': `32px 0 44px ${ink(0.14)}, 10px 0 18px ${ink(0.08)}, inset 1px 0 0 ${white(0.9)}, inset -12px 0 22px ${ink(0.05)}, inset 0 0 0 1px ${ink(0.04)}`,
    '--xy-dock-platform-sheen': `linear-gradient(90deg, transparent 10%, ${white(0.2)} 42%, ${white(0.55)} 60%, ${white(0.25)} 80%, transparent 100%), linear-gradient(180deg, ${white(0.5)} 0%, transparent 26%, transparent 74%, ${ink(0.05)} 100%)`,
    '--xy-dock-platform-edge': `linear-gradient(to left, ${white(0.75)}, transparent)`,
    // 中性浅瓷图标，无紫晕
    '--xy-dock-icon-body': `linear-gradient(160deg, ${white(0.98)} 0%, #f0f0f3 48%, #e4e4ea 100%)`,
    '--xy-dock-icon-border': ink(0.12),
    '--xy-dock-icon-inset': `inset 0 1px 0 ${white(1)}, inset 0 -8px 14px ${ink(0.04)}`,
    '--xy-dock-icon-facet': `linear-gradient(125deg, ${white(0.95)} 0%, ${white(0.35)} 20%, transparent 46%), linear-gradient(180deg, ${white(0.4)} 0%, transparent 34%)`,
    '--xy-dock-icon-rim': `inset 0 0 0 1px ${white(0.55)}`,
    '--xy-dock-icon-shadow': `0 8px 18px -10px ${ink(0.22)}, 0 2px 5px ${ink(0.1)}`,
    '--xy-dock-icon-shadow-hot': `0 10px 22px -10px ${ink(0.26)}, 0 2px 6px ${ink(0.12)}`,
    '--xy-dock-icon-glyph': ink(0.82),
    '--xy-dock-icon-glyph-filter': `drop-shadow(0 1px 0 ${white(0.85)})`,
    '--xy-dock-face-drop': `drop-shadow(-3px 2px 5px ${ink(0.14)})`,
    '--xy-dock-reflect-opacity': '0.22',
    '--xy-dock-reflect-opacity-hot': '0.3',
    '--xy-dock-reflect-filter': 'blur(1.6px) brightness(0.96) saturate(0.5)',
    '--xy-dock-reflect-filter-hot': 'blur(1.4px) brightness(0.98) saturate(0.55)',
    '--xy-dock-grain-opacity': '0.08',
  };
}

/**
 * 被谁调用：ThemeProvider / 注入 :root。
 * 做什么：PaletteTokens → --xy-* CSS 变量（含 Dock 相关变量）。
 */
function resolveChromeBg(tokens: PaletteTokens, mode: ThemeMode): string {
  return mode === 'dark' ? tokens.bg : tokens.bgSoft;
}

function resolveWorkspaceSurfaceBg(tokens: PaletteTokens, mode: ThemeMode): string {
  return mode === 'dark' ? tokens.bgSoft : tokens.bgElevated;
}

export function paletteToCssVars(tokens: PaletteTokens, mode: ThemeMode = 'dark'): Record<string, string> {
  return {
    '--xy-bg': resolveWorkspaceSurfaceBg(tokens, mode),
    '--xy-bg-elevated': tokens.bgElevated,
    '--xy-chrome-bg': resolveChromeBg(tokens, mode),
    '--xy-bg-soft': tokens.bgSoft,
    '--xy-bg-subtle': tokens.bgSubtle,
    '--xy-sidebar-bg': tokens.sidebarBg,
    '--xy-sidebar-hover': tokens.sidebarHover,
    '--xy-sidebar-active': tokens.sidebarActive,
    '--xy-header-bg': tokens.headerBg,
    '--xy-surface-glass': tokens.surfaceGlass,
    '--xy-surface-glass-strong': tokens.surfaceGlassStrong,
    '--xy-grid-dot': tokens.gridDot,
    '--xy-workspace-bg': tokens.meshGradient,
    '--xy-text': tokens.text,
    '--xy-text-soft': tokens.textSoft,
    '--xy-text-muted': tokens.textMuted,
    '--xy-text-faint': tokens.textFaint,
    '--xy-border': tokens.border,
    '--xy-border-strong': tokens.borderStrong,
    '--xy-border-accent': tokens.borderAccent,
    '--xy-primary': tokens.primary,
    '--xy-primary-hover': tokens.primaryHover,
    '--xy-primary-soft': tokens.primarySoft,
    '--xy-accent': tokens.accent,
    '--xy-accent-soft': tokens.accentSoft,
    '--xy-accent-2': tokens.accent2,
    '--xy-accent-gradient': tokens.accentGradient,
    '--xy-danger': tokens.danger,
    '--xy-warning': tokens.warning,
    '--xy-success': tokens.success,
    '--xy-shadow-sm': tokens.shadowSm,
    '--xy-shadow-md': tokens.shadowMd,
    '--xy-shadow-card': tokens.shadowCard,
    '--xy-shadow-glow': tokens.shadowGlow,
    '--xy-shadow-float': tokens.shadowFloat,
    '--xy-blur-glass': tokens.blurGlass,
    '--xy-glass-highlight': tokens.glassHighlight,
    '--xy-orb-1': tokens.orb1,
    '--xy-orb-2': tokens.orb2,
    '--xy-orb-3': tokens.orb3,
    ...buildDockCssVars(tokens, mode),
  };
}

/** 从当前 preset 载入到自定义草稿 */
export function categoriesFromPresetTokens(
  lightTokens: PaletteTokens,
  darkTokens: PaletteTokens,
): CustomThemeStore {
  return {
    light: tokensToCategories(lightTokens),
    dark: tokensToCategories(darkTokens),
  };
}

/** categoriesFromPresetTokens：把扁平 palette 拆回主题编辑用的分类结构。 */
function tokensToCategories(tokens: PaletteTokens): ThemeCategoryInput {
  return {
    brand: { primary: tokens.primary },
    accent: { secondary: tokens.accent, tertiary: tokens.accent2 },
    semantic: { success: tokens.success, warning: tokens.warning, danger: tokens.danger },
    text: { primary: tokens.text, secondary: tokens.textMuted, disabled: tokens.textFaint },
    background: { base: tokens.bg, elevated: tokens.bgElevated, muted: tokens.bgSoft },
    surface: { glass: tokens.surfaceGlass, glassStrong: tokens.surfaceGlassStrong },
    border: { default: tokens.border, strong: tokens.borderStrong },
    ambient: { orb1: tokens.orb1, orb2: tokens.orb2, orb3: tokens.orb3 },
  };
}
