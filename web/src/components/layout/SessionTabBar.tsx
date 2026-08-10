import { useRef } from "react";
import { X } from "lucide-react";
import { useSessionManager, type SessionStatus } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import { cn } from "../../lib/utils";
import { DisplayModeToggle } from "./DisplayModeToggle";

const STATUS_DOT: Record<SessionStatus, string> = {
  running: "bg-primary",
  awaiting_approval: "bg-warning",
  idle: "bg-success",
  error: "bg-danger",
};

/** 会话 tab 栏：每个打开的会话一个 tab。点击激活、中键/X 关闭、HTML5 拖拽排序。 */
export function SessionTabBar() {
  const openTabs = useUiStore((s) => s.openTabs);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const dragId = useRef<string | null>(null);

  const activate = (id: string) => useSessionManager.getState().setActive(id);

  const close = (id: string) => {
    const mgr = useSessionManager.getState();
    const next = useUiStore.getState().closeTab(id);
    if (mgr.activeId === id && next) mgr.setActive(next);
  };

  if (openTabs.length === 0) return null;

  return (
    <div className="flex h-9 shrink-0 items-stretch border-b border-border bg-sidebar">
      <div className="flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto px-1">
        {openTabs.map((id) => {
          const entry = entries[id];
          if (!entry) return null;
          const active = id === activeId;
          return (
            <div
              key={id}
              data-active={active}
              draggable
              onDragStart={(e) => {
                e.dataTransfer.setData("text/plain", id);
                dragId.current = id;
              }}
              onDragEnd={() => (dragId.current = null)}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => {
                if (dragId.current && dragId.current !== id) {
                  useUiStore.getState().moveTab(dragId.current, id);
                }
                dragId.current = null;
              }}
              className={cn(
                "group flex h-8 max-w-40 cursor-pointer items-center gap-1.5 rounded-t-md border border-b-0 border-transparent px-2.5 text-[12px]",
                active
                  ? "border-border bg-background text-foreground"
                  : "text-muted-foreground hover:bg-accent",
              )}
              onClick={() => activate(id)}
              onAuxClick={(e) => {
                if (e.button === 1) close(id);
              }}
            >
              <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", STATUS_DOT[entry.status])} />
              <span className="truncate">{entry.name}</span>
              <button
                type="button"
                data-close
                aria-label={`Close ${entry.name}`}
                className="ml-0.5 hidden rounded-sm p-0.5 hover:bg-accent group-hover:block"
                onClick={(e) => {
                  e.stopPropagation();
                  close(id);
                }}
              >
                <X size={11} />
              </button>
            </div>
          );
        })}
      </div>
      <DisplayModeToggle />
    </div>
  );
}
