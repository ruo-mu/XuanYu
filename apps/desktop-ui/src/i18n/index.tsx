/**
 * i18n 翻译系统 — 数据驱动，零 if/else 语言切换。
 *
 * 语言包静态打包进主 chunk，首屏同步可读，避免异步 import 导致显示 key。
 */

import React, { createContext, useContext, useState, useCallback } from 'react';

import enMessages from './en.json';
import zhCNMessages from './zhCN.json';

export type Locale = 'zh-CN' | 'en';

type Messages = Record<string, string>;

const CATALOG: Record<Locale, Messages> = {
  'zh-CN': zhCNMessages as Messages,
  en: enMessages as Messages,
};

/** `{0}` `{1}` 占位替换；缺参数保留原文。 */
function formatTemplate(template: string, args: Array<string | number>): string {
  if (args.length === 0) return template;
  return template.replace(/\{(\d+)\}/g, (match, index) => {
    const value = args[Number(index)];
    return value === undefined ? match : String(value);
  });
}

/**
 * 首屏 locale：SSR 默认 zh-CN；否则 localStorage `xy-locale`，再退 navigator.language。
 */
function resolveInitialLocale(): Locale {
  if (typeof window === 'undefined') return 'zh-CN';
  const saved = localStorage.getItem('xy-locale');
  if (saved === 'zh-CN' || saved === 'en') return saved;
  const browserLang = navigator.language || '';
  return browserLang.startsWith('zh') ? 'zh-CN' : 'en';
}

interface I18nContextType {
  locale: Locale;
  setLocale: (l: Locale) => void;
  t: (key: string, fallback?: string, ...args: Array<string | number>) => string;
  /** 语言包已静态打包，恒为 true */
  loaded: boolean;
}

const I18nContext = createContext<I18nContextType>({
  locale: 'zh-CN',
  setLocale: () => {},
  t: (k: string, f?: string, ...args: Array<string | number>) => formatTemplate(f || k, args),
  loaded: true,
});

/** 读 I18nContext：locale / setLocale / t。 */
export function useI18n(): I18nContextType {
  return useContext(I18nContext);
}

/** Shorthand hook: returns t function only */
export function useT(): (key: string, fallback?: string, ...args: Array<string | number>) => string {
  return useContext(I18nContext).t;
}

/** 中英对照文案：zh-CN 用第一参，否则第二参，再走 formatTemplate。 */
export function useLocaleText(): (zh: string, en: string, ...args: Array<string | number>) => string {
  const { locale } = useI18n();
  return useCallback(
    (zh: string, en: string, ...args: Array<string | number>) =>
      formatTemplate(locale === 'zh-CN' ? zh : en, args),
    [locale],
  );
}

/**
 * 被谁调用：App 根。
 * 做什么：提供 locale / setLocale / t；切语言写 localStorage。
 */
export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(resolveInitialLocale);

  const setLocale = useCallback((l: Locale) => {
    if (l !== 'zh-CN' && l !== 'en') return;
    localStorage.setItem('xy-locale', l);
    setLocaleState((prev) => (prev === l ? prev : l));
  }, []);

  const t = useCallback(
    (key: string, fallback?: string, ...args: Array<string | number>) => {
      const template = CATALOG[locale][key] || fallback || key;
      return formatTemplate(template, args);
    },
    [locale],
  );

  return React.createElement(
    I18nContext.Provider,
    { value: { locale, setLocale, t, loaded: true } },
    children,
  );
}
