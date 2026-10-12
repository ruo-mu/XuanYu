import { app, BrowserWindow, nativeTheme, shell } from 'electron';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { readFileSync } from 'node:fs';
import { RustEngineClient } from './engine/EngineClient';
import { AppServer } from './app/AppServer';
import { registerIpc } from './ipc/registerIpc';
import { buildAppMenus } from './appMenu';
import { TerminalManager } from './terminal/TerminalManager';

let mainWindow: BrowserWindow | null = null;
let terminalManager: TerminalManager | null = null;
let engine: RustEngineClient | null = null;

process.on('unhandledRejection', (reason) => {
  console.error('[app] unhandled rejection', reason);
});
process.on('uncaughtException', (err) => {
  console.error('[app] uncaught exception', err);
});

/**
 * 读取安装器写入的启动标记（~\.xuanyu\launch.conf）。
 *
 * 便携式安装时安装器会写入 "dataDir=<安装目录>\data"，此时引擎的所有数据
 * （内置 PostgreSQL、插件、skill、mcp、沙箱等）都位于安装目录下；未安装或
 * 使用现有数据库时返回 undefined，引擎回落到默认的 ~\.xuanyu。
 *
 * 注意：Unicode 版 NSIS 写出的文件是 UTF-16LE（可能带 BOM），这里同时容忍
 * UTF-8，避免其他工具改写后无法解析。
 */
function resolveLaunchDataDir(): string | undefined {
  const marker = join(homedir(), '.xuanyu', 'launch.conf');
  let buffer: Buffer;
  try {
    buffer = readFileSync(marker);
  } catch {
    return undefined;
  }
  const isUtf16Le = buffer.includes(0x00);
  let text = isUtf16Le ? buffer.toString('utf16le') : buffer.toString('utf8');
  if (isUtf16Le && text.charCodeAt(0) === 0xfeff) text = text.slice(1); // strip BOM
  for (const line of text.split(/\r?\n/)) {
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    if (line.slice(0, eq).trim() === 'dataDir') return line.slice(eq + 1).trim();
  }
  return undefined;
}

/** Must stay in sync with --xy-chrome-bg / --xy-text in desktop-ui styles. */
const DEFAULT_CHROME = {
  light: { backgroundColor: '#e8e8ec', symbolColor: '#1a1a1e' },
  dark: { backgroundColor: '#0c0c10', symbolColor: '#ececef' },
} as const;

type TitleBarChromeColors = {
  backgroundColor: string;
  symbolColor: string;
};

function cssColorToHex(color: string): string {
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

function normalizeChromeColors(colors: TitleBarChromeColors): TitleBarChromeColors | null {
  const backgroundColor = cssColorToHex(colors.backgroundColor);
  const symbolColor = cssColorToHex(colors.symbolColor);
  if (!backgroundColor || !symbolColor) return null;
  return { backgroundColor, symbolColor };
}

function defaultChromeColors(dark = nativeTheme.shouldUseDarkColors): TitleBarChromeColors {
  return dark ? DEFAULT_CHROME.dark : DEFAULT_CHROME.light;
}

function applyWindowChrome(win: BrowserWindow, colors: TitleBarChromeColors): void {
  if (win.isDestroyed()) return;
  const normalized = normalizeChromeColors(colors);
  if (!normalized) return;
  win.setBackgroundColor(normalized.backgroundColor);
}

function readRendererChromeColors(win: BrowserWindow): Promise<TitleBarChromeColors | null> {
  return win.webContents
    .executeJavaScript(`(() => {
      const styles = getComputedStyle(document.documentElement);
      return {
        backgroundColor: styles.getPropertyValue('--xy-chrome-bg').trim(),
        symbolColor: styles.getPropertyValue('--xy-text').trim(),
      };
    })()`)
    .then((colors: TitleBarChromeColors) => normalizeChromeColors(colors))
    .catch(() => null);
}

function createWindow(): void {
  const isMac = process.platform === 'darwin';
  const isWin = process.platform === 'win32';
  const chrome = defaultChromeColors();

  mainWindow = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 960,
    minHeight: 640,
    show: false,
    title: '玄语 Agent',
    backgroundColor: chrome.backgroundColor,
    autoHideMenuBar: isWin,
    ...(isMac
      ? {
          titleBarStyle: 'hiddenInset',
          trafficLightPosition: { x: 14, y: 12 },
        }
      : {}),
    // Win10 上 titleBarOverlay（WCO）经常不画原生 caption，无边框 + 渲染进程自绘窗口按钮。
    ...(isWin ? { frame: false } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.mjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      spellcheck: false,
      devTools: !app.isPackaged,
    },
  });

  mainWindow.webContents.on('did-finish-load', () => {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    void readRendererChromeColors(mainWindow).then((colors) => {
      if (colors) applyWindowChrome(mainWindow!, colors);
    });
  });

  mainWindow.on('ready-to-show', () => {
    mainWindow?.show();
  });

  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: 'deny' };
  });

  if (process.env.ELECTRON_RENDERER_URL) {
    void mainWindow.loadURL(process.env.ELECTRON_RENDERER_URL);
  } else {
    void mainWindow.loadFile(join(__dirname, '../renderer/index.html'));
  }
}

function buildMenu(getWindow: () => BrowserWindow | null): void {
  buildAppMenus(getWindow);
}

app.commandLine.appendSwitch('enable-features', 'CanvasOopRasterization');

// 单实例锁：第二次启动时聚焦已有窗口。
const gotSingleInstanceLock = app.requestSingleInstanceLock();
if (!gotSingleInstanceLock) {
  app.quit();
} else {
  app.whenReady().then(async () => {
    engine = new RustEngineClient({ dataDir: resolveLaunchDataDir() });
    const server = new AppServer(engine);
    terminalManager = new TerminalManager(() => mainWindow);

    registerIpc(server, () => mainWindow, {
      onTitleBarChrome: (colors) => {
        if (!mainWindow || mainWindow.isDestroyed()) return;
        applyWindowChrome(mainWindow, colors);
      },
      terminals: terminalManager,
    });

    // 先创建并显示主窗口，渲染端立刻播放开屏动画；引擎在后台启动，
    // 不再阻塞画面（随后由渲染端在开屏结束且引擎就绪后再进入主界面）。
    createWindow();
    buildMenu(() => mainWindow);
    if (mainWindow && process.platform !== 'darwin') {
      mainWindow.setMenuBarVisibility(false);
    }

    try {
      await engine.start();
    } catch (err) {
      console.error('[engine] failed to start', err);
    }

    nativeTheme.on('updated', () => {
      if (!mainWindow || mainWindow.isDestroyed()) return;
      void readRendererChromeColors(mainWindow).then((colors) => {
        applyWindowChrome(mainWindow!, colors ?? defaultChromeColors());
      });
    });

    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
  });

  app.on('window-all-closed', () => {
    terminalManager?.disposeAll();
    if (process.platform !== 'darwin') app.quit();
  });

  app.on('before-quit', () => {
    if (engine) {
      engine.stop().catch((err) => {
        console.error('[engine] failed to stop on quit', err);
      });
    }
  });
}
