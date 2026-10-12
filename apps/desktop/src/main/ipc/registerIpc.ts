import { ipcMain, dialog, shell, type BrowserWindow } from 'electron';
import { IPC_CHANNEL, type WindowCommand, type TerminalCreateInput } from '@shared/protocol';
import type { AppServer } from '../app/AppServer';
import { popupAppMenu, runWindowCommand, type AppMenuId } from '../appMenu';
import type { TerminalManager } from '../terminal/TerminalManager';

type CoreInvokePayload = {
  command: string;
  args?: unknown;
};

type RegisterIpcOptions = {
  onTitleBarChrome?: (colors: { backgroundColor: string; symbolColor: string }) => void;
  terminals?: TerminalManager;
};

export function registerIpc(
  server: AppServer,
  getWindow: () => BrowserWindow | null,
  options: RegisterIpcOptions = {},
): void {
  ipcMain.removeHandler(IPC_CHANNEL.coreInvoke);
  ipcMain.handle(IPC_CHANNEL.coreInvoke, async (_event, payload: CoreInvokePayload) => {
    if (!payload?.command) throw new Error('Missing command');
    return server.coreInvoke(payload.command, payload.args);
  });

  ipcMain.removeHandler(IPC_CHANNEL.pickFolder);
  ipcMain.handle(
    IPC_CHANNEL.pickFolder,
    async (_event, payload: { title?: string; initialDir?: string } = {}) => {
      const win = getWindow();
      const dialogOpts = {
        title: payload.title?.trim() || '选择项目文件夹',
        defaultPath: payload.initialDir?.trim() || undefined,
        properties: ['openDirectory', 'createDirectory'] as Array<
          'openDirectory' | 'createDirectory'
        >,
      };
      const result = win
        ? await dialog.showOpenDialog(win, dialogOpts)
        : await dialog.showOpenDialog(dialogOpts);
      if (result.canceled || !result.filePaths.length) {
        return { path: null, cancelled: true };
      }
      return { path: result.filePaths[0], cancelled: false };
    },
  );

  ipcMain.removeHandler(IPC_CHANNEL.revealPath);
  ipcMain.handle(IPC_CHANNEL.revealPath, async (_event, payload: { path?: string } = {}) => {
    const target = payload.path?.trim();
    if (!target) throw new Error('workspace.reveal 需要 path');
    shell.showItemInFolder(target);
    return { path: target };
  });

  ipcMain.removeAllListeners(IPC_CHANNEL.titleBarTheme);
  ipcMain.on(
    IPC_CHANNEL.titleBarTheme,
    (_event, payload: { backgroundColor?: string; symbolColor?: string }) => {
      const backgroundColor = payload?.backgroundColor?.trim();
      const symbolColor = payload?.symbolColor?.trim();
      if (!backgroundColor || !symbolColor) return;
      options.onTitleBarChrome?.({ backgroundColor, symbolColor });
    },
  );

  ipcMain.removeHandler(IPC_CHANNEL.popupMenu);
  ipcMain.handle(
    IPC_CHANNEL.popupMenu,
    (_event, payload: { menuId?: AppMenuId; x?: number; y?: number } = {}) => {
      const menuId = payload.menuId;
      if (menuId !== 'file' && menuId !== 'edit' && menuId !== 'view' && menuId !== 'help') {
        return { ok: false };
      }
      popupAppMenu(menuId, {
        x: Number(payload.x) || 0,
        y: Number(payload.y) || 0,
      });
      return { ok: true };
    },
  );

  ipcMain.removeHandler(IPC_CHANNEL.windowCommand);
  ipcMain.handle(IPC_CHANNEL.windowCommand, (_event, command: WindowCommand) => {
    return runWindowCommand(getWindow, command);
  });

  const terminals = options.terminals;
  ipcMain.removeHandler(IPC_CHANNEL.terminalCreate);
  ipcMain.handle(IPC_CHANNEL.terminalCreate, (_event, payload: TerminalCreateInput = {}) => {
    if (!terminals) throw new Error('Terminal host unavailable');
    return terminals.create(payload);
  });

  ipcMain.removeHandler(IPC_CHANNEL.terminalAttach);
  ipcMain.handle(IPC_CHANNEL.terminalAttach, (_event, payload: { id?: string } = {}) => {
    if (!terminals) throw new Error('Terminal host unavailable');
    const id = payload.id?.trim();
    if (!id) return { ok: false };
    return terminals.attach(id);
  });

  ipcMain.removeHandler(IPC_CHANNEL.terminalWrite);
  ipcMain.handle(
    IPC_CHANNEL.terminalWrite,
    (_event, payload: { id?: string; data?: string } = {}) => {
      if (!terminals) throw new Error('Terminal host unavailable');
      const id = payload.id?.trim();
      if (!id) return { ok: false };
      return terminals.write(id, payload.data ?? '');
    },
  );

  ipcMain.removeHandler(IPC_CHANNEL.terminalResize);
  ipcMain.handle(
    IPC_CHANNEL.terminalResize,
    (_event, payload: { id?: string; cols?: number; rows?: number } = {}) => {
      if (!terminals) throw new Error('Terminal host unavailable');
      const id = payload.id?.trim();
      if (!id) return { ok: false };
      return terminals.resize(id, Number(payload.cols) || 80, Number(payload.rows) || 24);
    },
  );

  ipcMain.removeHandler(IPC_CHANNEL.terminalDispose);
  ipcMain.handle(IPC_CHANNEL.terminalDispose, (_event, payload: { id?: string } = {}) => {
    if (!terminals) throw new Error('Terminal host unavailable');
    const id = payload.id?.trim();
    if (!id) return { ok: false };
    return terminals.dispose(id);
  });

  ipcMain.removeHandler(IPC_CHANNEL.engineStatus);
  ipcMain.handle(IPC_CHANNEL.engineStatus, () => server.engine.getStatus());

  ipcMain.removeHandler(IPC_CHANNEL.engineRestart);
  ipcMain.handle(IPC_CHANNEL.engineRestart, async () => {
    return server.engine.restart();
  });

  server.engine.onStatusChange((status) => {
    const win = getWindow();
    if (!win || win.isDestroyed()) return;
    win.webContents.send(IPC_CHANNEL.event, {
      type: 'engine.status',
      payload: status,
    });
  });

  server.engine.getRpc().onEvent((event) => {
    const win = getWindow();
    if (!win || win.isDestroyed()) return;
    win.webContents.send(IPC_CHANNEL.event, {
      type: event.event,
      payload: event.payload,
    });
  });
}
