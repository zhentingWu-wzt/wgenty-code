import type { ReactNode } from "react";
import { useUiStore } from "../../state/uiStore";
import { cn } from "../../lib/utils";

/**
 * 聊天内容避让层：胶囊展开且有计划内容时（uiStore.todoPillOverlay），
 * 给聊天 <main> 加右边距让其左移，避免与右上角悬浮的计划清单重叠。
 * 桌面端完全让出 w-72 + 右边距（≈300px）；窄屏（<md）最多让 40%，
 * 防止聊天列被压得过窄。折叠/切标签页时 TodoPill 卸载并把 overlay
 * 复位，边距平滑过渡回 0。
 */
export function TodoPillShift({ children }: { children: ReactNode }) {
  const overlay = useUiStore((s) => s.todoPillOverlay);
  return (
    <div
      data-testid="todo-pill-shift"
      className={cn(
        "h-full min-w-0 transition-[margin-right] duration-200 ease-out",
        overlay ? "md:mr-[300px] max-md:mr-[40%]" : "mr-0",
      )}
    >
      {children}
    </div>
  );
}
