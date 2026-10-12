/**
 * 工作台与 init 向导共用的 Ant Design ThemeConfig。
 */

import { theme as antdTheme, type ThemeConfig } from 'antd';

import type { PaletteTokens } from './types';

const APP_FONT =
  '"Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Microsoft YaHei", sans-serif';

export interface AntdThemeInput {
  antdTokens: Record<string, string>;
  tokens: PaletteTokens;
  isDark: boolean;
}

/** 主体工作台与 init 向导共用的 Ant Design 主题配置 */
export function buildAntdAppTheme(input: AntdThemeInput, scope: 'main' | 'init' = 'main'): ThemeConfig {
  const { antdTokens, tokens: t, isDark } = input;

  const components: ThemeConfig['components'] = {
    Button: {
      primaryShadow: 'none',
      borderRadius: 12,
      colorPrimary: t.primary,
      colorPrimaryHover: t.primaryHover,
    },
    Card: {
      borderRadiusLG: 16,
      colorBgContainer: t.surfaceGlassStrong,
      colorBorderSecondary: t.border,
    },
    Input: {
      colorBgContainer: scope === 'init' ? t.bgSoft : t.surfaceGlass,
      colorBorder: t.borderStrong,
      borderRadius: 12,
      colorText: t.text,
      activeBorderColor: t.primary,
      hoverBorderColor: t.primaryHover,
    },
    Select: {
      colorBgContainer: scope === 'init' ? t.bgSoft : t.surfaceGlass,
      colorBorder: t.borderStrong,
      colorText: t.text,
    },
    InputNumber: {
      colorBgContainer: scope === 'init' ? t.bgSoft : t.surfaceGlass,
      colorBorder: t.borderStrong,
      colorText: t.text,
    },
    Layout: {
      bodyBg: t.bg,
      headerBg: t.headerBg,
      siderBg: t.sidebarBg,
    },
    Drawer: {
      colorBgElevated: t.bgSoft,
    },
    Modal:
      scope === 'init'
        ? {
            contentBg: t.surfaceGlassStrong,
            headerBg: 'transparent',
            titleColor: t.text,
            titleFontSize: 17,
            colorIcon: t.textMuted,
            colorIconHover: t.text,
            footerBg: 'transparent',
          }
        : {
            contentBg: t.bgElevated,
            headerBg: t.bgElevated,
          },
    Popover: {
      colorBgElevated: t.bgElevated,
    },
    Dropdown: {
      colorBgElevated: t.bgElevated,
      controlItemBgHover: t.sidebarHover,
      controlItemBgActive: t.primarySoft,
    },
    Menu: {
      itemBg: 'transparent',
      itemColor: t.text,
      itemHoverBg: t.sidebarHover,
      itemSelectedBg: t.primarySoft,
      itemSelectedColor: t.text,
      activeBarBorderWidth: 0,
    },
    Table: {
      headerBg: t.bgSubtle,
      borderColor: t.border,
    },
    Segmented: {
      itemSelectedBg: t.primarySoft,
      itemSelectedColor: t.primary,
      trackBg: t.sidebarHover,
    },
    Slider: {
      trackBg: t.primary,
      trackHoverBg: t.primaryHover,
      handleColor: t.primary,
      handleActiveColor: t.primaryHover,
      railBg: t.border,
      dotBorderColor: t.border,
    },
    Progress: {
      defaultColor: t.primary,
    },
  };

  if (scope === 'init') {
    components.Form = { labelColor: t.textSoft };
    components.Steps = {
      colorText: t.textSoft,
      colorTextDescription: t.textMuted,
      colorPrimary: t.primary,
    };
  }

  return {
    algorithm: isDark ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
    token: {
      ...antdTokens,
      borderRadius: 12,
      fontFamily: APP_FONT,
      colorText: t.text,
      colorTextSecondary: t.textSoft,
      colorTextTertiary: t.textMuted,
      colorBgContainer: t.surfaceGlassStrong,
      colorBorder: t.border,
    },
    components,
  };
}
