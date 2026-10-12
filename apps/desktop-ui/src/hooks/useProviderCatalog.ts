/**
 * 供应商目录：桌面端直接读本地目录，不依赖异步 WS 回包。
 */

import { useMemo } from 'react';

import { useI18n } from '../i18n';
import { getProviderCatalog } from '../init/initApi';
import type { ProviderCatalogEntry } from '../protocol';

/** 返回当前语言下的供应商目录。 */
export function useProviderCatalog(): ProviderCatalogEntry[] {
  const { locale } = useI18n();
  return useMemo(
    () => getProviderCatalog(locale === 'en' ? 'en' : 'zh-CN'),
    [locale],
  );
}
