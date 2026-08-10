import { create } from "zustand";

/**
 * Chat 渲染模式：turn 内文字与工具调用的排版方式。
 *
 * - "single"：整个 turn 一个气泡——所有文字在上、所有工具卡片在最下（历史行为）。
 * - "rounds"：每个 LLM 轮次一个气泡——该轮文字 + 该轮工具卡片（贴近真实时序）。
 * - "timeline"：工具作为独立条目按到达顺序穿插在轮次文字之间（与 TUI 一致），
 *   执行期间显示 running 占位卡。
 */
export type DisplayMode = "single" | "rounds" | "timeline";

const STORAGE_KEY = "wgenty.displayMode";

function load(): DisplayMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "single" || v === "rounds" || v === "timeline") return v;
  } catch {
    // storage 不可用（隐私模式等）——回退默认
  }
  // Default to "timeline": tool calls interleave with text in arrival order
  // (matching the TUI). Users who prefer the compact single-bubble layout can
  // switch via the DisplayModeToggle in the session tab bar.
  return "timeline";
}

interface DisplayPrefsState {
  mode: DisplayMode;
  setMode: (mode: DisplayMode) => void;
}

/** 全局展示偏好：跨会话生效，切换后历史重新加载也按新模式渲染。 */
export const useDisplayPrefs = create<DisplayPrefsState>((set) => ({
  mode: load(),
  setMode: (mode) => {
    try {
      localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      // 非致命：不持久化也照常切换
    }
    set({ mode });
  },
}));
