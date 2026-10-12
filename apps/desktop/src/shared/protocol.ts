/**
 * 桌面端唯一通信契约：
 * Renderer → Preload → Main(IPC) → Engine(JSON-RPC)
 * 禁止假 WebSocket / 浏览器 fallback / 双路径适配。
 */

export type PermissionMode = 'default' | 'plan' | 'auto';

export type SessionSummary = {
  id: string;
  title: string;
  updatedAt: number;
  projectDir?: string;
  pinned?: boolean;
};

export type ChatMessage = {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  createdAt: number;
};

export type EngineStatus = {
  state: 'stopped' | 'starting' | 'ready' | 'error';
  version?: string;
  detail?: string;
};

export const IPC_CHANNEL = {
  coreInvoke: 'xuanyu:core-invoke',
  event: 'xuanyu:event',
  engineStatus: 'xuanyu:engine-status',
  engineRestart: 'xuanyu:engine-restart',
  titleBarTheme: 'xuanyu:titlebar-theme',
  pickFolder: 'xuanyu:pick-folder',
  revealPath: 'xuanyu:reveal-path',
  popupMenu: 'xuanyu:popup-menu',
  menuAction: 'xuanyu:menu-action',
  windowCommand: 'xuanyu:window-command',
  terminalCreate: 'xuanyu:terminal-create',
  terminalAttach: 'xuanyu:terminal-attach',
  terminalWrite: 'xuanyu:terminal-write',
  terminalResize: 'xuanyu:terminal-resize',
  terminalDispose: 'xuanyu:terminal-dispose',
} as const;

export type WindowCommand =
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
  | 'reload';

export type TerminalShellKind = 'powershell' | 'cmd' | 'bash';

export type TerminalCreateInput = {
  cwd?: string;
  cols?: number;
  rows?: number;
  shell?: TerminalShellKind;
};

export type TerminalCreateResult = {
  id: string;
  title: string;
  cwd: string;
  shell: TerminalShellKind;
};
