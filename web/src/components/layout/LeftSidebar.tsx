import { PanelLeftOpen } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { ProjectTree } from "../../features/sessions/ProjectTree";
import { useUiStore } from "../../state/uiStore";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";

/** 左侧栏：会话树（project → worktree → session）。折叠状态在 uiStore。
 *  移动端（<md）为滑出抽屉：展开态 fixed 覆盖 + 点击遮罩收起；折叠态整个
 *  隐藏，由顶栏 PanelLeft 按钮唤起。 */
export function LeftSidebar({ client }: { client: DaemonClient }) {
  const collapsed = useUiStore((s) => s.leftCollapsed);
  const toggleLeft = useUiStore((s) => s.toggleLeft);

  if (collapsed) {
    return (
      <aside className="flex w-8 shrink-0 flex-col items-center border-r border-border bg-sidebar py-1 max-md:hidden">
        <Button variant="ghost" size="icon" onClick={toggleLeft} title="Show sidebar">
          <PanelLeftOpen size={15} />
        </Button>
      </aside>
    );
  }

  return (
    <>
      {/* 移动端抽屉遮罩：桌面端不渲染（hidden），<md 时 fixed 铺满，点击收起。 */}
      <div
        aria-hidden
        className="hidden max-md:fixed max-md:inset-0 max-md:z-30 max-md:block max-md:bg-black/40"
        onClick={toggleLeft}
      />
      <aside
        className={cn(
          "flex w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground",
          "max-md:fixed max-md:inset-y-10 max-md:left-0 max-md:z-40 max-md:shadow-xl",
        )}
      >
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          <ProjectTree client={client} />
        </div>
      </aside>
    </>
  );
}
