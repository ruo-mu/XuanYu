/**
 * 底部终端面板的打开状态与标签页。
 */

import { create } from 'zustand';

export type TerminalShellKind = 'powershell' | 'cmd' | 'shell';

export type TerminalTab = {
  id: string;
  title: string;
  cwd: string;
  shell: TerminalShellKind;
};

type TerminalStore = {
  open: boolean;
  height: number;
  tabs: TerminalTab[];
  activeId: string | null;
  setOpen: (open: boolean) => void;
  toggleOpen: () => void;
  setHeight: (height: number) => void;
  setActiveId: (id: string | null) => void;
  upsertTab: (tab: TerminalTab) => void;
  removeTab: (id: string) => void;
  clearTabs: () => void;
};

const MIN_HEIGHT = 140;
const MAX_HEIGHT = 420;
const DEFAULT_HEIGHT = 180;

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  open: false,
  height: DEFAULT_HEIGHT,
  tabs: [],
  activeId: null,

  setOpen: (open) => set({ open }),
  toggleOpen: () => set({ open: !get().open }),
  setHeight: (height) =>
    set({
      height: Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, Math.round(height))),
    }),
  setActiveId: (id) => set({ activeId: id }),
  upsertTab: (tab) =>
    set((state) => {
      const exists = state.tabs.some((item) => item.id === tab.id);
      const tabs = exists
        ? state.tabs.map((item) => (item.id === tab.id ? tab : item))
        : [...state.tabs, tab];
      return { tabs, activeId: tab.id, open: true };
    }),
  removeTab: (id) =>
    set((state) => {
      const tabs = state.tabs.filter((item) => item.id !== id);
      const activeId =
        state.activeId === id ? tabs[tabs.length - 1]?.id ?? null : state.activeId;
      return {
        tabs,
        activeId,
        open: tabs.length > 0 ? state.open : false,
      };
    }),
  clearTabs: () => set({ tabs: [], activeId: null, open: false }),
}));

export const TERMINAL_PANEL_MIN = MIN_HEIGHT;
export const TERMINAL_PANEL_MAX = MAX_HEIGHT;
