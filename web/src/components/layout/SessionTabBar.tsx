import { useRef } from "react";
import { FileMinus, FileText, Network, X } from "lucide-react";
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

/** subagent 详情 tab id 前缀。 */
const SUBAGENT_PREFIX = "subagent:";

/** 文件预览 tab id 前缀。 */
const PREVIEW_PREFIX = "preview:";

/** 文件 diff tab id 前缀。 */
const DIFF_PREFIX = "diff:";

/** 会话 tab 栏：每个打开的会话、subagent 详情或文件预览一个 tab。点击激活、
 *  中键/X 关闭、HTML5 拖拽排序。subagent tab 用 Network 图标，预览 tab 用
 *  FileText 图标（标签取文件名，title 悬浮完整 relPath），会话 tab 用状态点。 */
export function SessionTabBar() {
  const openTabs = useUiStore((s) => s.openTabs);
  const entries = useSessionManager((s) => s.entries);
  const subagentTabs = useUiStore((s) => s.subagentTabs);
  const previewTabs = useUiStore((s) => s.previewTabs);
  const diffTabs = useUiStore((s) => s.diffTabs);
  const activeTabId = useUiStore((s) => s.activeTabId);
  const dragId = useRef<string | null>(null);

  const activate = (id: string) => {
    // subagent/preview tab 只切活动 tab，不动活跃会话（Composer 仍指当前会话）。
    if (
      id.startsWith(SUBAGENT_PREFIX) ||
      id.startsWith(PREVIEW_PREFIX) ||
      id.startsWith(DIFF_PREFIX)
    ) {
      useUiStore.getState().setActiveTab(id);
    } else {
      useSessionManager.getState().setActive(id);
      // uiSync 只在 activeId 变化时同步 activeTabId；从 subagent/preview tab
      // 点回当前活跃会话时 activeId 前后相同，必须显式切回，否则界面停在原 tab。
      useUiStore.getState().setActiveTab(id);
    }
  };

  const close = (id: string) => {
    const ui = useUiStore.getState();
    const next = ui.closeTab(id);
    if (ui.activeTabId !== id || !next) return;
    if (
      next.startsWith(SUBAGENT_PREFIX) ||
      next.startsWith(PREVIEW_PREFIX) ||
      next.startsWith(DIFF_PREFIX)
    ) {
      ui.setActiveTab(next);
    } else {
      useSessionManager.getState().setActive(next);
    }
  };

  if (openTabs.length === 0) return null;

  return (
    <div className="flex h-9 shrink-0 items-stretch border-b border-border bg-sidebar">
      <div className="flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto px-1">
        {openTabs.map((id) => {
          const isSubagent = id.startsWith(SUBAGENT_PREFIX);
          const isPreview = id.startsWith(PREVIEW_PREFIX);
          const isDiff = id.startsWith(DIFF_PREFIX);
          const meta = isSubagent ? subagentTabs[id] : undefined;
          const previewMeta = isPreview ? previewTabs[id] : undefined;
          const diffMeta = isDiff ? diffTabs[id] : undefined;
          const entry = !isSubagent && !isPreview && !isDiff ? entries[id] : undefined;
          if (!meta && !previewMeta && !diffMeta && !entry) return null;
          const active = id === activeTabId;
          const previewLabel = previewMeta
            ? previewMeta.relPath.split("/").pop() || previewMeta.relPath
            : null;
          const diffLabel = diffMeta ? diffMeta.relPath.split("/").pop() || diffMeta.relPath : null;
          const title = meta?.label ?? entry?.name ?? id;
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
              {isSubagent ? (
                <Network size={12} className="shrink-0 text-primary" />
              ) : isDiff ? (
                <FileMinus size={12} className="shrink-0 text-primary" />
              ) : isPreview ? (
                <FileText size={12} className="shrink-0 text-primary" />
              ) : (
                <span
                  className={cn(
                    "h-1.5 w-1.5 shrink-0 rounded-full",
                    entry ? STATUS_DOT[entry.status] : "",
                  )}
                />
              )}
              <span
                className="truncate"
                title={previewMeta ? previewMeta.relPath : diffMeta ? diffMeta.relPath : undefined}
              >
                {previewLabel ?? diffLabel ?? title}
              </span>
              <button
                type="button"
                data-close
                aria-label={`Close ${previewLabel ?? title}`}
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
