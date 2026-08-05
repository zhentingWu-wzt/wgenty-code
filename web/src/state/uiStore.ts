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
}));
