import { PanelLeftOpen } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { ProjectTree } from "../../features/sessions/ProjectTree";
import { useUiStore } from "../../state/uiStore";
import { Button } from "../ui/button";

/** 左侧栏：会话树（project → worktree → session）。折叠状态在 uiStore；
 *  移动端抽屉行为在 Task 7 统一处理。 */
export function LeftSidebar({ client }: { client: DaemonClient }) {
  const collapsed = useUiStore((s) => s.leftCollapsed);
  const toggleLeft = useUiStore((s) => s.toggleLeft);

  if (collapsed) {
    return (
      <aside className="flex w-8 shrink-0 flex-col items-center border-r border-border bg-sidebar py-1">
        <Button variant="ghost" size="icon" onClick={toggleLeft} title="Show sidebar">
          <PanelLeftOpen size={15} />
        </Button>
      </aside>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground">
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        <ProjectTree client={client} />
      </div>
    </aside>
  );
}
