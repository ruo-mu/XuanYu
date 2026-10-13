import { app, BrowserWindow, nativeTheme, shell } from 'electron';
import { join } from 'node:path';
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
    engine = new RustEngineClient();
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
