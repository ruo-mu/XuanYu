/**
 * 主题 Context：模式、预设、自定义 palette，写入 localStorage 并同步 CSS 变量。
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
} from 'react';

import type { CustomThemeStore, ResolvedTheme, ThemeMode, ThemeState } from './types';
import { DEFAULT_PRESET_ID, getPresetById } from './presets';
import { buildThemeFromStore, paletteToCssVars } from './utils';
import { syncElectronTitleBarChrome } from '../utils/electronChrome';

const STORAGE_KEY = 'xy-theme-v2';

/** 被谁调用：ThemeProvider。做什么：解析 localStorage 自定义主题。 */
function parseCustomTheme(raw: unknown): CustomThemeStore | null {
  if (!raw || typeof raw !== 'object') return null;
  const obj = raw as Record<string, unknown>;
  if (obj.light && obj.dark && typeof obj.light === 'object' && typeof obj.dark === 'object') {
    return raw as CustomThemeStore;
  }
  return null;
}

interface ThemeContextValue {
  mode: ThemeMode;
  presetId: string;
  custom: CustomThemeStore | null;
  resolved: ResolvedTheme;
  setMode: (mode: ThemeMode) => void;
  toggleMode: () => void;
  setPreset: (presetId: string) => void;
  setCustomTheme: (store: CustomThemeStore) => void;
  resetCustom: () => void;
  antdTokens: {
    colorPrimary: string;
    colorLink: string;
    colorSuccess: string;
    colorWarning: string;
    colorError: string;
    colorBgBase: string;
    colorTextBase: string;
    colorBorder: string;
  };
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

/** 被谁调用：ThemeProvider。做什么：读主题状态。 */
function loadState(): ThemeState {
  if (typeof window === 'undefined') {
    return { mode: 'dark', presetId: DEFAULT_PRESET_ID, custom: null };
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as ThemeState;
      if (parsed.mode === 'light' || parsed.mode === 'dark') {
        return {
          mode: parsed.mode,
          presetId: parsed.presetId || DEFAULT_PRESET_ID,
          custom: parseCustomTheme(parsed.custom),
        };
      }
    }
  } catch {
    /* ignore */
  }
  const mode: ThemeMode = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  return { mode, presetId: DEFAULT_PRESET_ID, custom: null };
}

/** preset + 自定义色 → CSS 变量与 Ant Design token。 */
function resolveTheme(state: ThemeState): ResolvedTheme {
  const isCustom = state.presetId === 'custom' && !!state.custom;
  if (isCustom && state.custom) {
    const tokens = buildThemeFromStore(state.custom, state.mode);
    return {
      mode: state.mode,
      presetId: 'custom',
      presetName: 'Custom',
      tokens,
      isCustom: true,
    };
  }
  const preset = getPresetById(state.presetId) || getPresetById(DEFAULT_PRESET_ID)!;
  const tokens = state.mode === 'dark' ? preset.dark : preset.light;
  return {
    mode: state.mode,
    presetId: preset.id,
    presetName: preset.name,
    tokens,
    isCustom: false,
  };
}

/** 被谁调用：主题变更。做什么：写 CSS 变量到 document。 */
function applyTokensToDom(mode: ThemeMode, tokens: ReturnType<typeof paletteToCssVars>) {
  const root = document.documentElement;
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
  Object.entries(tokens).forEach(([key, value]) => {
    root.style.setProperty(key, value);
  });
}

/** 提供主题状态并在首屏把 CSS 变量写到 documentElement。 */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<ThemeState>(loadState);
  const resolved = useMemo(() => resolveTheme(state), [state]);

  useLayoutEffect(() => {
    applyTokensToDom(resolved.mode, paletteToCssVars(resolved.tokens, resolved.mode));
    syncElectronTitleBarChrome();
  }, [resolved]);

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  }, [state]);

  const setMode = useCallback((mode: ThemeMode) => {
    setState((s) => ({ ...s, mode }));
  }, []);

  const toggleMode = useCallback(() => {
    setState((s) => ({ ...s, mode: s.mode === 'light' ? 'dark' : 'light' }));
  }, []);

  const setPreset = useCallback((presetId: string) => {
    setState((s) => ({ ...s, presetId, custom: presetId === 'custom' ? s.custom : null }));
  }, []);

  const setCustomTheme = useCallback((store: CustomThemeStore) => {
    setState((s) => ({ ...s, presetId: 'custom', custom: store }));
  }, []);

  const resetCustom = useCallback(() => {
    setState((s) => ({ ...s, presetId: DEFAULT_PRESET_ID, custom: null }));
  }, []);

  const antdTokens = useMemo(() => ({
    colorPrimary: resolved.tokens.primary,
    colorLink: resolved.tokens.primary,
    colorSuccess: resolved.tokens.success,
    colorWarning: resolved.tokens.warning,
    colorError: resolved.tokens.danger,
    colorBgBase: resolved.tokens.bg,
    colorTextBase: resolved.tokens.text,
    colorBorder: resolved.tokens.border,
  }), [resolved.tokens]);

  const value = useMemo<ThemeContextValue>(() => ({
    mode: state.mode,
    presetId: state.presetId,
    custom: state.custom,
    resolved,
    setMode,
    toggleMode,
    setPreset,
    setCustomTheme,
    resetCustom,
    antdTokens,
  }), [state, resolved, setMode, toggleMode, setPreset, setCustomTheme, resetCustom, antdTokens]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

/** 读取当前主题；必须包在 ThemeProvider 内。 */
export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}

export { DEFAULT_PRESET_ID, THEME_PRESETS, getPresetById, getPresetLabel, getPresetDescription } from './presets';
export type {
  BrandColorInput,
  CustomThemeStore,
  PaletteTokens,
  ThemeCategoryInput,
  ThemeMode,
  ThemePreset,
} from './types';

if (typeof window !== 'undefined') {
  const boot = loadState();
  const bootResolved = resolveTheme(boot);
  applyTokensToDom(bootResolved.mode, paletteToCssVars(bootResolved.tokens, bootResolved.mode));
  syncElectronTitleBarChrome();
}
