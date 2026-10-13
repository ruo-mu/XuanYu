import { randomUUID } from 'node:crypto';
import { homedir } from 'node:os';
import { existsSync } from 'node:fs';
import type { BrowserWindow } from 'electron';
import pty from 'node-pty';
import { IPC_CHANNEL, type TerminalShellKind } from '../../shared/protocol';

type PtyProcess = ReturnType<typeof pty.spawn>;

type TerminalSession = {
  id: string;
  title: string;
  cwd: string;
  shell: TerminalShellKind;
  process: PtyProcess;
  attached: boolean;
  buffer: string[];
  cols: number;
  rows: number;
};

function defaultShellKind(): TerminalShellKind {
  if (process.platform === 'win32') return 'powershell';
  return 'shell';
}

function resolveShell(kind?: TerminalShellKind): { file: string; args: string[]; shell: TerminalShellKind } {
  const requested = kind || defaultShellKind();

  if (process.platform === 'win32') {
    if (requested === 'cmd') {
      const comspec = process.env.ComSpec?.trim();
      if (comspec && existsSync(comspec)) {
        return { file: comspec, args: [], shell: 'cmd' };
      }
      return { file: 'cmd.exe', args: [], shell: 'cmd' };
    }
    return { file: 'powershell.exe', args: [], shell: 'powershell' };
  }

  const shell = process.env.SHELL?.trim() || '/bin/sh';
  return { file: shell, args: [], shell: 'shell' };
}

function sanitizeCwd(cwd?: string | null): string {
  const trimmed = cwd?.trim();
  if (trimmed && existsSync(trimmed)) return trimmed;
  return homedir();
}

function tabTitle(cwd: string, shell: TerminalShellKind): string {
  if (shell === 'cmd') return `cmd · ${shortPath(cwd)}`;
  if (shell === 'powershell') return shortPath(cwd);
  return shortPath(cwd);
}

function shortPath(cwd: string): string {
  const normalized = cwd.replace(/[\\/]+$/, '');
  if (normalized.length <= 42) return normalized;
  return `…${normalized.slice(-40)}`;
}

/** 管理本机伪终端会话，经 IPC 与渲染进程同步。 */
export class TerminalManager {
  private sessions = new Map<string, TerminalSession>();
  private getWindow: () => BrowserWindow | null;

  constructor(getWindow: () => BrowserWindow | null) {
    this.getWindow = getWindow;
  }

  create(input: { cwd?: string; cols?: number; rows?: number; shell?: TerminalShellKind } = {}): {
    id: string;
    title: string;
    cwd: string;
    shell: TerminalShellKind;
  } {
    const cwd = sanitizeCwd(input.cwd);
    const cols = Math.max(20, Math.floor(input.cols ?? 100));
    const rows = Math.max(5, Math.floor(input.rows ?? 24));
    const { file, args, shell } = resolveShell(input.shell);
    const id = randomUUID();

    const env = { ...process.env } as Record<string, string>;
    delete env.ELECTRON_RUN_AS_NODE;

    const processPty = pty.spawn(file, args, {
      name: 'xterm-256color',
      cols,
      rows,
      cwd,
      env,
      useConpty: process.platform === 'win32',
    });

    const session: TerminalSession = {
      id,
      title: tabTitle(cwd, shell),
      cwd,
      shell,
      process: processPty,
      attached: false,
      buffer: [],
      cols,
      rows,
    };
    this.sessions.set(id, session);

    processPty.onData((data) => {
      const current = this.sessions.get(id);
      if (!current) return;
      if (!current.attached) {
        current.buffer.push(data);
        return;
      }
      this.emit('terminal.data', { id, data });
    });
    processPty.onExit(({ exitCode }) => {
      this.sessions.delete(id);
      this.emit('terminal.exit', { id, exitCode: exitCode ?? 0 });
    });

    return { id, title: session.title, cwd, shell };
  }

  attach(id: string): { ok: boolean; buffered?: string } {
    const session = this.sessions.get(id);
    if (!session) return { ok: false };
    session.attached = true;
    const buffered = session.buffer.join('');
    session.buffer = [];
    return { ok: true, buffered };
  }

  write(id: string, data: string): { ok: boolean } {
    const session = this.sessions.get(id);
    if (!session) return { ok: false };
    session.process.write(data);
    return { ok: true };
  }

  resize(id: string, cols: number, rows: number): { ok: boolean } {
    const session = this.sessions.get(id);
    if (!session) return { ok: false };
    const nextCols = Math.max(20, Math.floor(cols));
    const nextRows = Math.max(5, Math.floor(rows));
    if (session.cols === nextCols && session.rows === nextRows) {
      return { ok: true };
    }
    try {
      session.process.resize(nextCols, nextRows);
      session.cols = nextCols;
      session.rows = nextRows;
      // 未附着时丢弃因 resize 产生的重复横幅，只保留后续实时输出
      if (!session.attached) {
        session.buffer = [];
      }
    } catch {
      return { ok: false };
    }
    return { ok: true };
  }

  dispose(id: string): { ok: boolean } {
    const session = this.sessions.get(id);
    if (!session) return { ok: false };
    this.sessions.delete(id);
    try {
      session.process.kill();
    } catch {
      /* ignore */
    }
    return { ok: true };
  }

  disposeAll(): void {
    for (const id of [...this.sessions.keys()]) {
      this.dispose(id);
    }
  }

  private emit(type: string, payload: unknown): void {
    const win = this.getWindow();
    if (!win || win.isDestroyed()) return;
    win.webContents.send(IPC_CHANNEL.event, { type, payload });
  }
}
