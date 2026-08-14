/**
 * 单向同步：sessionManager -> uiStore.openTabs / activeTabId。
 * - 会话被激活（新建/点选）时自动补开 tab，并把活动 tab 切回该会话
 *   （从 subagent 详情 tab 切回会话）
 * - 会话被删除/归档移除时剪掉对应 tab（subagent:<…> tab 无 session entry，
 *   永不在此剪除）
 * 返回取消订阅函数。App 启动时调用一次。
 */
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";

/** subagent detail tab id 前缀；这类 tab 没有 session entry，不能被 prune。 */
const SUBAGENT_TAB_PREFIX = "subagent:";

export function startUiSync(): () => void {
  // 初始同步：订阅前的既有状态（冷启动 bootstrap 已建会话）也要补齐。
  const s = useSessionManager.getState();
  const ui = useUiStore.getState();
  if (s.activeId && !ui.openTabs.includes(s.activeId)) ui.openTab(s.activeId);
  const stale = ui.openTabs.filter(
    (id) => !id.startsWith(SUBAGENT_TAB_PREFIX) && !s.entries[id],
  );
  if (stale.length > 0) ui.pruneTabs(stale);
  if (s.activeId && !ui.activeTabId) ui.setActiveTab(s.activeId);

  return useSessionManager.subscribe((s, prev) => {
    const ui = useUiStore.getState();
    if (s.activeId && s.activeId !== prev.activeId) {
      if (!ui.openTabs.includes(s.activeId)) ui.openTab(s.activeId);
      // 会话激活（新建/点选）会从 subagent 详情 tab 抢回焦点到会话本身。
      ui.setActiveTab(s.activeId);
    }
    const stale = ui.openTabs.filter(
      (id) => !id.startsWith(SUBAGENT_TAB_PREFIX) && !s.entries[id],
    );
    if (stale.length > 0) ui.pruneTabs(stale);
  });
}
