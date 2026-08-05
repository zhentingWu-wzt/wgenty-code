/**
 * 单向同步：sessionManager → uiStore.openTabs。
 * - 会话被激活（新建/点选）时自动补开 tab
 * - 会话被删除/归档移除时剪掉对应 tab
 * 返回取消订阅函数。App 启动时调用一次。
 */
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";

export function startUiSync(): () => void {
  // 初始同步：订阅前的既有状态（冷启动 bootstrap 已建会话）也要补齐。
  const s = useSessionManager.getState();
  const ui = useUiStore.getState();
  if (s.activeId && !ui.openTabs.includes(s.activeId)) ui.openTab(s.activeId);
  const stale = ui.openTabs.filter((id) => !s.entries[id]);
  if (stale.length > 0) ui.pruneTabs(stale);

  return useSessionManager.subscribe((s, prev) => {
    const ui = useUiStore.getState();
    if (s.activeId && s.activeId !== prev.activeId && !ui.openTabs.includes(s.activeId)) {
      ui.openTab(s.activeId);
    }
    const stale = ui.openTabs.filter((id) => !s.entries[id]);
    if (stale.length > 0) ui.pruneTabs(stale);
  });
}
