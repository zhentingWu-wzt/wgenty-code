/**
 * UI 布局状态：主题、左右栏显隐、右栏当前面板。
 * 只持有 UI 事实，不碰会话数据（会话在 sessionManager）。
 */
import { create } from "zustand";
import { applyTheme, readStoredTheme, type ThemeMode } from "../lib/theme";

export type RightPanelId =
  | "sessions"
  | "skills"
  | "memory"
  | "checkpoints"
  | "tasks"
  | "config"
  | "mcp"
  | "subagents"
  | "inspector";

/** Metadata for a subagent detail tab (`subagent:<nodeId>`). */
export interface SubagentTabMeta {
  nodeId: string;
  label: string;
  /** Root daemon session id the subagent belongs to (capability API param). */
  rootSessionId: string;
}

/** Metadata for a workspace file preview tab (`preview:<absPath>`).
 *  Mirrors the subagent tab pattern (design D5) — no separate store. */
export interface PreviewTabMeta {
  /** Registered workspace root (canonicalized) the file belongs to. */
  workspaceRoot: string;
  absPath: string;
  relPath: string;
  /** Extension-based first guess by the caller; the panel re-checks against
   *  the actual fetchFile response kind. */
  kind: "text" | "binary";
}

interface UiState {
  theme: ThemeMode;
  leftCollapsed: boolean;
  /** 左栏展开态宽度（px），可拖拽调整，clamp 到 [180, 400]。 */
  leftWidth: number;
  rightPanel: RightPanelId | null;

  setTheme: (t: ThemeMode) => void;
  toggleLeft: () => void;
  setLeftWidth: (w: number) => void;
  setRightPanel: (p: RightPanelId | null) => void;
  /** 点已激活的图标 = 收起右栏；点其他图标 = 切换面板。 */
  toggleRightPanel: (p: RightPanelId) => void;

  openTabs: string[];
  openTab: (id: string) => void;
  /** 移除 tab；返回应激活的邻居 id（无剩余 tab 时为 null）。 */
  closeTab: (id: string) => string | null;
  /** 把 id 移动到 targetId 的位置（向下拖时落在其后）。 */
  moveTab: (id: string, targetId: string) => void;
  pruneTabs: (ids: string[]) => void;

  /** 统一活动 tab：session id 或 `subagent:<nodeId>`。null = 无活动 tab。 */
  activeTabId: string | null;
  setActiveTab: (id: string | null) => void;
  /** subagent 详情 tab 元数据，key = tab id（`subagent:<nodeId>`）。 */
  subagentTabs: Record<string, SubagentTabMeta>;
  /** 打开（或聚焦）一个 subagent 详情 tab 并激活。 */
  openSubagentTab: (meta: SubagentTabMeta) => void;

  /** 文件预览 tab 元数据，key = tab id（`preview:<absPath>`）。 */
  previewTabs: Record<string, PreviewTabMeta>;
  /** 打开（或聚焦）一个文件预览 tab 并激活；同 path 幂等（不加新 tab）。 */
  openPreviewTab: (meta: PreviewTabMeta) => void;
}

export const useUiStore = create<UiState>((set) => ({
  theme: readStoredTheme(),
  leftCollapsed: false,
  leftWidth: 256,
  rightPanel: null,

  setTheme: (theme) => {
    applyTheme(theme);
    set({ theme });
  },
  toggleLeft: () => set((s) => ({ leftCollapsed: !s.leftCollapsed })),
  setLeftWidth: (w) => set({ leftWidth: Math.min(400, Math.max(180, Math.round(w))) }),
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
      // subagent / preview tab 关闭时顺带清理各自的元数据。
      const subagentTabs =
        id.startsWith("subagent:") && s.subagentTabs[id]
          ? (() => {
              const c = { ...s.subagentTabs };
              delete c[id];
              return c;
            })()
          : s.subagentTabs;
      const previewTabs =
        id.startsWith("preview:") && s.previewTabs[id]
          ? (() => {
              const c = { ...s.previewTabs };
              delete c[id];
              return c;
            })()
          : s.previewTabs;
      return { openTabs, subagentTabs, previewTabs };
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

  activeTabId: null,
  setActiveTab: (activeTabId) => set({ activeTabId }),
  subagentTabs: {},
  openSubagentTab: (meta) => {
    const tabId = `subagent:${meta.nodeId}`;
    set((s) => {
      const openTabs = s.openTabs.includes(tabId) ? s.openTabs : [...s.openTabs, tabId];
      return {
        openTabs,
        subagentTabs: { ...s.subagentTabs, [tabId]: meta },
        activeTabId: tabId,
      };
    });
  },

  previewTabs: {},
  openPreviewTab: (meta) => {
    const tabId = `preview:${meta.absPath}`;
    set((s) => {
      const openTabs = s.openTabs.includes(tabId) ? s.openTabs : [...s.openTabs, tabId];
      return {
        openTabs,
        previewTabs: { ...s.previewTabs, [tabId]: meta },
        activeTabId: tabId,
      };
    });
  },
}));
