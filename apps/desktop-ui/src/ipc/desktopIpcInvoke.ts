import type { UnlistenFn } from './ipcTypes';

declare global {
  interface Window {
    xuanyu?: {
      platform?: NodeJS.Platform;
      coreInvoke(command: string, args?: unknown): Promise<unknown>;
      pickFolder?(input?: {
        title?: string;
        initialDir?: string;
      }): Promise<{ path: string | null; cancelled: boolean; error?: string }>;
      revealPath?(input: { path: string }): Promise<{ path: string }>;
      onEvent(eventName: string, listener: (payload: unknown) => void): () => void;
      getEngineStatus?(): Promise<{
        state: 'stopped' | 'starting' | 'ready' | 'error';
        version?: string;
        detail?: string;
      }>;
      restartEngine?(): Promise<{
        state: 'stopped' | 'starting' | 'ready' | 'error';
        version?: string;
        detail?: string;
      }>;
      setTitleBarChrome?(colors: { backgroundColor: string; symbolColor: string }): void;
      popupMenu?(
        menuId: 'file' | 'edit' | 'view' | 'help',
        x: number,
        y: number,
      ): Promise<{ ok: boolean }>;
      onMenuAction?(listener: (payload: { type: string; page?: string }) => void): () => void;
      windowCommand?(
        command:
          | 'quit'
          | 'close'
          | 'minimize'
          | 'maximize'
          | 'unmaximize'
          | 'toggleMaximize'
          | 'zoomIn'
          | 'zoomOut'
          | 'resetZoom'
          | 'toggleFullscreen'
          | 'reload',
      ): Promise<{ ok: boolean; maximized?: boolean }>;
      terminalCreate?(input?: {
        cwd?: string;
        cols?: number;
        rows?: number;
        shell?: 'powershell' | 'cmd' | 'shell';
      }): Promise<{
        id: string;
        title: string;
        cwd: string;
        shell: 'powershell' | 'cmd' | 'shell';
      }>;
      terminalAttach?(input: { id: string }): Promise<{ ok: boolean; buffered?: string }>;
      terminalWrite?(input: { id: string; data: string }): Promise<{ ok: boolean }>;
      terminalResize?(input: {
        id: string;
        cols: number;
        rows: number;
      }): Promise<{ ok: boolean }>;
      terminalDispose?(input: { id: string }): Promise<{ ok: boolean }>;
    };
  }
}

export function unwrapRustResult<T>(value: unknown): T {
  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    if ('Ok' in record && !('Err' in record)) {
      return record.Ok as T;
    }
    if ('Err' in record && typeof record.Err === 'string') {
      throw new Error(record.Err);
    }
  }
  return value as T;
}

export async function invoke<T = any>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (typeof window === 'undefined' || !window.xuanyu?.coreInvoke) {
    throw new Error('Desktop IPC unavailable: Electron preload not loaded');
  }
  const raw = await window.xuanyu.coreInvoke(command, args ?? {});
  return unwrapRustResult<T>(raw);
}

export async function listen<T>(
  eventName: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (typeof window === 'undefined' || !window.xuanyu?.onEvent) {
    throw new Error('Desktop IPC unavailable: Electron preload not loaded');
  }
  return window.xuanyu.onEvent(eventName, (payload) => {
    handler({ payload: payload as T });
  });
}

export function isDesktopIpcAvailable(): boolean {
  return typeof window !== 'undefined' && Boolean(window.xuanyu?.coreInvoke);
}

export async function workspacePickFolder(input?: {
  title?: string;
  initialDir?: string;
}): Promise<{ path: string | null; cancelled: boolean; error?: string }> {
  if (typeof window === 'undefined' || !window.xuanyu?.pickFolder) {
    throw new Error('Desktop IPC unavailable: folder picker not loaded');
  }
  return window.xuanyu.pickFolder(input);
}

export async function workspaceReveal(input: {
  path: string;
}): Promise<{ path: string }> {
  if (typeof window === 'undefined' || !window.xuanyu?.revealPath) {
    throw new Error('Desktop IPC unavailable: reveal handler not loaded');
  }
  return window.xuanyu.revealPath(input);
}
