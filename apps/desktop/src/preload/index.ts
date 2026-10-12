import { contextBridge, ipcRenderer } from 'electron';
import {
  IPC_CHANNEL,
  type WindowCommand,
  type TerminalCreateInput,
  type TerminalCreateResult,
  type EngineStatus,
} from '../shared/protocol';

export type XuanyuDesktopApi = {
  platform: NodeJS.Platform;
  coreInvoke(command: string, args?: unknown): Promise<unknown>;
  pickFolder(input?: { title?: string; initialDir?: string }): Promise<{
    path: string | null;
    cancelled: boolean;
    error?: string;
  }>;
  revealPath(input: { path: string }): Promise<{ path: string }>;
  onEvent(eventName: string, listener: (payload: unknown) => void): () => void;
  getEngineStatus(): Promise<EngineStatus>;
  restartEngine(): Promise<EngineStatus>;
  setTitleBarChrome(colors: { backgroundColor: string; symbolColor: string }): void;
  popupMenu(menuId: 'file' | 'edit' | 'view' | 'help', x: number, y: number): Promise<{ ok: boolean }>;
  onMenuAction(listener: (payload: { type: string; page?: string }) => void): () => void;
  windowCommand(command: WindowCommand): Promise<{ ok: boolean; maximized?: boolean }>;
  terminalCreate(input?: TerminalCreateInput): Promise<TerminalCreateResult>;
  terminalAttach(input: { id: string }): Promise<{ ok: boolean; buffered?: string }>;
  terminalWrite(input: { id: string; data: string }): Promise<{ ok: boolean }>;
  terminalResize(input: { id: string; cols: number; rows: number }): Promise<{ ok: boolean }>;
  terminalDispose(input: { id: string }): Promise<{ ok: boolean }>;
};

const api: XuanyuDesktopApi = {
  platform: process.platform,
  coreInvoke(command, args) {
    return ipcRenderer.invoke(IPC_CHANNEL.coreInvoke, { command, args: args ?? {} });
  },
  pickFolder(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.pickFolder, input ?? {});
  },
  revealPath(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.revealPath, input);
  },
  onEvent(eventName, listener) {
    const handler = (
      _event: Electron.IpcRendererEvent,
      message: { type: string; payload: unknown },
    ) => {
      if (message?.type !== eventName) return;
      listener(message.payload);
    };
    ipcRenderer.on(IPC_CHANNEL.event, handler);
    return () => {
      ipcRenderer.removeListener(IPC_CHANNEL.event, handler);
    };
  },
  getEngineStatus() {
    return ipcRenderer.invoke(IPC_CHANNEL.engineStatus);
  },
  restartEngine() {
    return ipcRenderer.invoke(IPC_CHANNEL.engineRestart);
  },
  setTitleBarChrome(colors) {
    ipcRenderer.send(IPC_CHANNEL.titleBarTheme, colors);
  },
  popupMenu(menuId, x, y) {
    return ipcRenderer.invoke(IPC_CHANNEL.popupMenu, { menuId, x, y });
  },
  onMenuAction(listener) {
    const handler = (_event: Electron.IpcRendererEvent, payload: { type: string; page?: string }) => {
      listener(payload);
    };
    ipcRenderer.on(IPC_CHANNEL.menuAction, handler);
    return () => {
      ipcRenderer.removeListener(IPC_CHANNEL.menuAction, handler);
    };
  },
  windowCommand(command) {
    return ipcRenderer.invoke(IPC_CHANNEL.windowCommand, command);
  },
  terminalCreate(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.terminalCreate, input ?? {});
  },
  terminalAttach(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.terminalAttach, input);
  },
  terminalWrite(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.terminalWrite, input);
  },
  terminalResize(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.terminalResize, input);
  },
  terminalDispose(input) {
    return ipcRenderer.invoke(IPC_CHANNEL.terminalDispose, input);
  },
};

contextBridge.exposeInMainWorld('xuanyu', api);
