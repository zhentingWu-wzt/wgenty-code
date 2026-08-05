/**
 * 单向同步：sessionManager → uiStore.openTabs。
 * - 会话被激活（新建/点选）时自动补开 tab
 * - 会话被删除/归档移除时剪掉对应 tab
 * 返回取消订阅函数。App 启动时调用一次。
 */
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";

export function startUiSync(): () => void {
  return useSessionManager.subscribe((s, prev) => {
    const ui = useUiStore.getState();
    if (s.activeId && s.activeId !== prev.activeId && !ui.openTabs.includes(s.activeId)) {
      ui.openTab(s.activeId);
    }
    const stale = ui.openTabs.filter((id) => !s.entries[id]);
    if (stale.length > 0) ui.pruneTabs(stale);
  });
}
