export type TitleBarChromeColors = {
  backgroundColor: string;
  symbolColor: string;
};

/** Normalize CSS colors to #RRGGBB for Electron titleBarOverlay. */
export function cssColorToHex(color: string): string {
  const value = color.trim();
  if (!value) return value;
  if (value.startsWith('#')) {
    if (value.length === 4) {
      const [, r, g, b] = value;
      return `#${r}${r}${g}${g}${b}${b}`.toLowerCase();
    }
    return value.slice(0, 7).toLowerCase();
  }

  const rgbMatch = value.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (rgbMatch) {
    const hex = [rgbMatch[1], rgbMatch[2], rgbMatch[3]]
      .map((part) => Number(part).toString(16).padStart(2, '0'))
      .join('');
    return `#${hex}`;
  }

  return value;
}

/** Tag the document root when running inside the Electron shell. */
export function bootstrapElectronChrome(): void {
  if (typeof window === 'undefined' || !window.xuanyu?.coreInvoke) return;

  const root = document.documentElement;
  root.classList.add('xy-electron');
  const platform = window.xuanyu.platform;
  if (platform) {
    root.classList.add(`xy-platform-${platform}`);
  }

  scheduleTitleBarChromeSync();
}

function scheduleTitleBarChromeSync(): void {
  syncElectronTitleBarChrome();
  for (const delay of [0, 80, 300, 1000]) {
    window.setTimeout(() => syncElectronTitleBarChrome(), delay);
  }
}

/** Read shell chrome colors from CSS variables (title bar + sidebar + Win caption). */
export function readElectronChromeColors(): TitleBarChromeColors | null {
  if (typeof window === 'undefined') return null;
  const styles = getComputedStyle(document.documentElement);
  const backgroundColor = cssColorToHex(styles.getPropertyValue('--xy-chrome-bg'));
  const symbolColor = cssColorToHex(styles.getPropertyValue('--xy-text'));
  if (!backgroundColor || !symbolColor) return null;
  return { backgroundColor, symbolColor };
}

/** Keep Windows titleBarOverlay aligned with in-app chrome (sidebar + title bar). */
export function syncElectronTitleBarChrome(): void {
  if (typeof window === 'undefined') return;
  if (window.xuanyu?.platform !== 'win32') return;
  const colors = readElectronChromeColors();
  if (!colors) return;
  window.xuanyu.setTitleBarChrome?.(colors);
}
