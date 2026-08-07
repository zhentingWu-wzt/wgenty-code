import { useEffect, useRef, useState } from "react";
import { FolderPlus, PanelLeftOpen } from "lucide-react";
import { DirectoryPickerModal } from "../ui/DirectoryPickerModal";
import { toast } from "sonner";
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
  const leftWidth = useUiStore((s) => s.leftWidth);
  const toggleLeft = useUiStore((s) => s.toggleLeft);

  // Bumped after a successful "Add project" so the tree refetches its list
  // (projects live inside ProjectTree; the key is the simplest cross-component
  // invalidation).
  const [treeRefreshKey, setTreeRefreshKey] = useState(0);

  // Directory picker modal: replaces the old window.prompt — browsers cannot
  // expose true local paths, so the daemon lists sub-directories instead.
  const [pickerOpen, setPickerOpen] = useState(false);

  const addProject = async (path: string) => {
    if (!path.trim()) return;
    try {
      const info = await client.addProject(path.trim());
      toast.success(`Project ${info.name} added`);
      setTreeRefreshKey((k) => k + 1);
    } catch (e) {
      // The daemon returns plain-text 400s (missing dir / duplicate) — show as-is.
      toast.error(`Add project failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  // 拖拽调宽：pointermove/up 挂在 window 上，结束时移除；ref 记录清理函数，
  // unmount 时也兜底清理，避免监听泄漏。
  const dragCleanupRef = useRef<(() => void) | null>(null);
  useEffect(() => () => dragCleanupRef.current?.(), []);

  const startDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = useUiStore.getState().leftWidth;
    const onMove = (ev: PointerEvent) => {
      useUiStore.getState().setLeftWidth(startWidth + ev.clientX - startX);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      dragCleanupRef.current = null;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    dragCleanupRef.current = onUp;
  };

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
        style={{ width: leftWidth }}
        className={cn(
          "relative flex shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground",
          "max-md:fixed max-md:inset-y-10 max-md:left-0 max-md:z-40 max-md:shadow-xl",
        )}
      >
        <div className="flex h-8 shrink-0 items-center justify-between border-b border-sidebar-border px-2">
          <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            Projects
          </span>
          <button
            type="button"
            title="Add project"
            className="inline-flex items-center rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
            onClick={() => setPickerOpen(true)}
          >
            <FolderPlus size={13} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          <ProjectTree client={client} refreshKey={treeRefreshKey} />
        </div>
        {/* 右边缘拖拽手柄：仅桌面端（移动端抽屉宽度固定）。 */}
        <div
          aria-hidden
          onPointerDown={startDrag}
          className="absolute inset-y-0 right-0 w-1 cursor-col-resize hover:bg-accent max-md:hidden"
        />
      </aside>

      {/* 目录选择器：打开时从 home（或上次位置）浏览，确认后注册项目。 */}
      <DirectoryPickerModal
        open={pickerOpen}
        client={client}
        onOpenChange={setPickerOpen}
        onConfirm={(path) => void addProject(path)}
      />
    </>
  );
}
