/**
 * UI 布局状态：主题、左右栏显隐、右栏当前面板。
 * 只持有 UI 事实，不碰会话数据（会话在 sessionManager）。
 */
import { create } from "zustand";
import { applyTheme, readStoredTheme, type ThemeMode } from "../lib/theme";

export type RightPanelId = "sessions" | "skills" | "memory" | "checkpoints" | "tasks";

interface UiState {
  theme: ThemeMode;
  leftCollapsed: boolean;
  rightPanel: RightPanelId | null;

  setTheme: (t: ThemeMode) => void;
  toggleLeft: () => void;
  setRightPanel: (p: RightPanelId | null) => void;
  /** 点已激活的图标 = 收起右栏；点其他图标 = 切换面板。 */
  toggleRightPanel: (p: RightPanelId) => void;

  openTabs: string[];
  openTab: (id: string) => void;
  /** 移除 tab；返回应激活的邻居 id（无剩余 tab 时为 null）。 */
  closeTab: (id: string) => string | null;
  /** 把 id 拖到 targetId 的位置。 */
  moveTab: (id: string, targetId: string) => void;
  pruneTabs: (ids: string[]) => void;
}

export const useUiStore = create<UiState>((set) => ({
  theme: readStoredTheme(),
  leftCollapsed: false,
  rightPanel: null,

  setTheme: (theme) => {
    applyTheme(theme);
    set({ theme });
  },
  toggleLeft: () => set((s) => ({ leftCollapsed: !s.leftCollapsed })),
  setRightPanel: (rightPanel) => set({ rightPanel }),
  toggleRightPanel: (p) => set((s) => ({ rightPanel: s.rightPanel === p ? null : p })),

  openTabs: [],
  openTab: (id) =>
    set((s) => (s.openTabs.includes(id) ? s : { openTabs: [...s.openTabs, id] })),
  closeTab: (id) => {
    let next: string | null = null;
    set((s) => {
      const idx = s.openTabs.indexOf(id);
      const openTabs = s.openTabs.filter((t) => t !== id);
      // 优先右侧邻居，删尾 tab 时取新的末尾。idx < 0 时 id 本就不在，无需激活切换。
      next = idx < 0 ? null : (openTabs[Math.min(idx, openTabs.length - 1)] ?? null);
      return { openTabs };
    });
    return next;
  },
  moveTab: (id, targetId) =>
    set((s) => {
      const from = s.openTabs.indexOf(id);
      const to = s.openTabs.indexOf(targetId);
      if (from < 0 || to < 0 || from === to) return s;
      const openTabs = s.openTabs.filter((t) => t !== id);
      openTabs.splice(to, 0, id);
      return { openTabs };
    }),
  pruneTabs: (ids) =>
    set((s) => ({ openTabs: s.openTabs.filter((t) => !ids.includes(t)) })),
}));
