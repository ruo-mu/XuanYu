/**
 * 初始化向导时区选项：运行时 ICU 的 IANA 列表，失败时回退到常用区。
 */

export type InitTimezoneLocale = 'zh-CN' | 'en';

/** 向导默认时区。 */
export const DEFAULT_TIMEZONE = 'Asia/Shanghai';

export interface InitTimezoneOption {
  value: string;
  label: string;
}

const FALLBACK_TIMEZONES = [
  'UTC',
  'Asia/Shanghai',
  'Asia/Hong_Kong',
  'Asia/Taipei',
  'Asia/Tokyo',
  'Asia/Seoul',
  'Asia/Singapore',
  'Asia/Kolkata',
  'Asia/Dubai',
  'Europe/London',
  'Europe/Paris',
  'Europe/Berlin',
  'Europe/Moscow',
  'America/New_York',
  'America/Chicago',
  'America/Denver',
  'America/Los_Angeles',
  'America/Sao_Paulo',
  'Australia/Sydney',
  'Pacific/Auckland',
] as const;

const optionsCache = new Map<string, InitTimezoneOption[]>();

/** zh-CN 用 zh-CN，其余用 en-US，给 Intl.DateTimeFormat。 */
function intlLocale(locale: InitTimezoneLocale): string {
  return locale === 'zh-CN' ? 'zh-CN' : 'en-US';
}

/** Full IANA list from runtime ICU (same source as Intl / OS timezone database). */
export function getAllTimezones(): string[] {
  if (typeof Intl !== 'undefined' && 'supportedValuesOf' in (Intl as typeof Intl & { supportedValuesOf?: (key: string) => string[] })) {
    try {
      return (Intl as typeof Intl & { supportedValuesOf: (key: string) => string[] }).supportedValuesOf('timeZone');
    } catch {
      // fall through
    }
  }
  return [...FALLBACK_TIMEZONES];
}

/** Intl 时区显示名；不支持该 locale/style 时返回 undefined，调用方回退 IANA id。 */
function readTimeZoneName(
  timezone: string,
  locale: InitTimezoneLocale,
  style: 'long' | 'longGeneric' | 'shortOffset',
): string | undefined {
  try {
    return new Intl.DateTimeFormat(intlLocale(locale), {
      timeZone: timezone,
      timeZoneName: style,
    })
      .formatToParts(new Date())
      .find((part) => part.type === 'timeZoneName')?.value;
  } catch {
    return undefined;
  }
}

/** 「时区名（偏移）· IANA」；无本地化名则只显示 id 与偏移。 */
export function formatTimezoneLabel(timezone: string, locale: InitTimezoneLocale): string {
  try {
    const offset = readTimeZoneName(timezone, locale, 'shortOffset') ?? '';
    const localizedName =
      readTimeZoneName(timezone, locale, 'long')
      ?? readTimeZoneName(timezone, locale, 'longGeneric');

    const hasLocalizedName = localizedName && localizedName !== timezone && localizedName !== offset;

    if (locale === 'zh-CN') {
      if (hasLocalizedName) return `${localizedName}（${offset}）· ${timezone}`;
      return offset ? `${timezone}（${offset}）` : timezone;
    }

    if (hasLocalizedName) return `${localizedName} (${offset}) · ${timezone}`;
    return offset ? `${timezone} (${offset})` : timezone;
  } catch {
    return timezone;
  }
}

/** 浏览器 resolvedOptions().timeZone；失败回 UTC。 */
export function detectBrowserTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || DEFAULT_TIMEZONE;
  } catch {
    return DEFAULT_TIMEZONE;
  }
}

/** 时区下拉按展示名排序（中文 zh-CN，否则 en）。 */
function sortLabel(a: InitTimezoneOption, b: InitTimezoneOption, locale: InitTimezoneLocale): number {
  const collator = locale === 'zh-CN' ? 'zh-CN' : 'en';
  return a.label.localeCompare(b.label, collator, { sensitivity: 'base' });
}

/** 初始化向导时区下拉：全量 IANA 选项；当前值不在列表时插到前面。 */
export function buildInitTimezoneOptions(
  locale: InitTimezoneLocale,
  current?: string,
): InitTimezoneOption[] {
  const cacheKey = locale;
  let options = optionsCache.get(cacheKey);
  if (!options) {
    options = getAllTimezones()
      .map((value) => ({ value, label: formatTimezoneLabel(value, locale) }))
      .sort((a, b) => sortLabel(a, b, locale));
    optionsCache.set(cacheKey, options);
  }

  if (!current || options.some((item) => item.value === current)) {
    return options;
  }

  return [
    { value: current, label: formatTimezoneLabel(current, locale) },
    ...options,
  ].sort((a, b) => sortLabel(a, b, locale));
}

/** 向导初值：已保存用已保存，否则浏览器 Intl 时区。 */
export function resolveInitialTimezone(saved?: string): string {
  if (saved) return saved;
  return detectBrowserTimezone();
}
