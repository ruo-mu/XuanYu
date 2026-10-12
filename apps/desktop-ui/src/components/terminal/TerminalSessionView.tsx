/**
 * 单个终端会话：挂载 xterm，经 preload 读写本机 PTY。
 */

import React, { useEffect, useRef } from 'react';
import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useTheme } from '../../theme/ThemeContext';
import type { ThemeMode } from '../../theme/types';

export type TerminalSessionViewProps = {
  id: string;
  active: boolean;
};

function readCssVar(name: string, fallback: string): string {
  if (typeof window === 'undefined') return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function buildTerminalTheme(mode: ThemeMode): ITheme {
  const text = readCssVar('--xy-text', mode === 'light' ? '#1c1c22' : '#e8e8ec');
  const brand = readCssVar('--xy-brand', mode === 'light' ? '#3b6fd9' : '#7aa2ff');

  // 背景透明，透出工作区氛围底，避免独立色块
  if (mode === 'light') {
    return {
      background: '#00000000',
      foreground: text,
      cursor: brand,
      cursorAccent: '#ffffff',
      selectionBackground: 'rgba(59, 111, 217, 0.22)',
      selectionForeground: text,
      black: '#2a2a32',
      red: '#c23b3b',
      green: '#2f8f5b',
      yellow: '#b0861f',
      blue: '#3b6fd9',
      magenta: '#8a4fbf',
      cyan: '#2a8a9e',
      white: '#5c5c68',
      brightBlack: '#7a7a86',
      brightRed: '#e05555',
      brightGreen: '#3cb879',
      brightYellow: '#d4a017',
      brightBlue: '#5b8def',
      brightMagenta: '#a66fd4',
      brightCyan: '#3aa9c0',
      brightWhite: '#1c1c22',
    };
  }

  return {
    background: '#00000000',
    foreground: text,
    cursor: brand,
    cursorAccent: '#121218',
    selectionBackground: 'rgba(122, 162, 255, 0.28)',
    selectionForeground: '#f2f2f6',
    black: '#0e0e12',
    red: '#ff6b6b',
    green: '#5dcea0',
    yellow: '#e6c07b',
    blue: '#7aa2ff',
    magenta: '#c792ea',
    cyan: '#56d4dd',
    white: '#c8c8d0',
    brightBlack: '#6e6e7a',
    brightRed: '#ff8a8a',
    brightGreen: '#7ee0b5',
    brightYellow: '#f0d28a',
    brightBlue: '#9bb8ff',
    brightMagenta: '#d7a8f5',
    brightCyan: '#7ae4ec',
    brightWhite: '#f4f4f8',
  };
}

export default function TerminalSessionView({ id, active }: TerminalSessionViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const activeRef = useRef(active);
  const ptySizeRef = useRef({ cols: 0, rows: 0 });
  const resizeTimerRef = useRef<number | null>(null);
  const { mode } = useTheme();
  activeRef.current = active;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const syncPtySize = (cols: number, rows: number) => {
      if (cols < 2 || rows < 1) return;
      if (ptySizeRef.current.cols === cols && ptySizeRef.current.rows === rows) return;
      ptySizeRef.current = { cols, rows };
      void window.xuanyu?.terminalResize?.({ id, cols, rows });
    };

    const fitLocal = (opts?: { syncPty?: boolean }) => {
      const term = termRef.current;
      const fit = fitRef.current;
      if (!term || !fit) return;
      try {
        fit.fit();
      } catch {
        return;
      }
      if (opts?.syncPty !== false) {
        syncPtySize(term.cols, term.rows);
      }
    };

    const scheduleFit = () => {
      if (resizeTimerRef.current != null) {
        window.clearTimeout(resizeTimerRef.current);
      }
      // 合并连续布局抖动，避免 ConPTY/cmd 因频繁 resize 反复打印横幅
      resizeTimerRef.current = window.setTimeout(() => {
        resizeTimerRef.current = null;
        if (!activeRef.current) return;
        fitLocal({ syncPty: true });
      }, 120);
    };

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: 'bar',
      cursorWidth: 1.5,
      fontSize: 12.5,
      lineHeight: 1.3,
      letterSpacing: 0.2,
      fontFamily:
        '"Cascadia Code", "JetBrains Mono", "Sarasa Mono SC", "Microsoft YaHei Mono", Consolas, Menlo, monospace',
      fontWeight: 400,
      fontWeightBold: 600,
      theme: buildTerminalTheme(mode),
      allowTransparency: true,
      scrollback: 5000,
      smoothScrollDuration: 60,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;

    const unsubData = window.xuanyu?.onEvent?.('terminal.data', (payload) => {
      const message = payload as { id?: string; data?: string };
      if (message?.id !== id || typeof message.data !== 'string') return;
      term.write(message.data);
    });
    const unsubExit = window.xuanyu?.onEvent?.('terminal.exit', (payload) => {
      const message = payload as { id?: string };
      if (message?.id !== id) return;
      term.writeln('\r\n[process exited]');
    });

    let attached = false;
    void (async () => {
      try {
        // 必须先 attach 再 resize：否则未附着期间每次 resize 都会把 cmd 横幅堆进 buffer
        const result = await window.xuanyu?.terminalAttach?.({ id });
        if (attached) return;
        attached = true;
        if (result?.buffered) {
          term.write(result.buffered);
        }
        requestAnimationFrame(() => {
          fitLocal({ syncPty: true });
        });
      } catch {
        /* ignore */
      }
    })();

    const onData = term.onData((data) => {
      void window.xuanyu?.terminalWrite?.({ id, data });
    });

    // 尺寸同步只走 scheduleFit / syncPtySize，避免 onResize 再打一遍
    const observer = new ResizeObserver(() => {
      if (!activeRef.current) return;
      scheduleFit();
    });
    observer.observe(host);

    return () => {
      if (resizeTimerRef.current != null) {
        window.clearTimeout(resizeTimerRef.current);
        resizeTimerRef.current = null;
      }
      unsubData?.();
      unsubExit?.();
      onData.dispose();
      observer.disconnect();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
    // Keep PTY binding stable for the tab lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  useEffect(() => {
    if (!active) return;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    requestAnimationFrame(() => {
      try {
        const prev = { cols: term.cols, rows: term.rows };
        fit.fit();
        if (term.cols !== prev.cols || term.rows !== prev.rows) {
          if (
            ptySizeRef.current.cols !== term.cols ||
            ptySizeRef.current.rows !== term.rows
          ) {
            ptySizeRef.current = { cols: term.cols, rows: term.rows };
            void window.xuanyu?.terminalResize?.({
              id,
              cols: term.cols,
              rows: term.rows,
            });
          }
        }
        term.focus();
      } catch {
        /* ignore */
      }
    });
  }, [active, id]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = buildTerminalTheme(mode);
  }, [mode]);

  return (
    <div
      className={`xy-terminal-session${active ? ' is-active' : ''}`}
      aria-hidden={!active}
      ref={hostRef}
    />
  );
}
