/** ThemeMode 定义在此处（而非 uiStore），避免 theme.ts ↔ uiStore.ts 循环依赖。 */
export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "wgenty-theme";

export function readStoredTheme(): ThemeMode {
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

export function resolveDark(mode: ThemeMode): boolean {
  return (
    mode === "dark" ||
    (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

/** 切换 documentElement 的 .dark class 并持久化选择。 */
export function applyTheme(mode: ThemeMode): void {
  document.documentElement.classList.toggle("dark", resolveDark(mode));
  localStorage.setItem(STORAGE_KEY, mode);
}
