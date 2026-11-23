import type { TodoItem } from "../../api/types";

/**
 * Todo 计划列表渲染（TasksPanel 与聊天区悬浮 TodoPill 共用）。
 * 只负责按状态着色与展示 activeForm（进行中的动态措辞），
 * 不包含数据获取逻辑。
 */
export function TodoListItems({
  items,
  emptyLabel = "No plan",
}: {
  items: TodoItem[];
  emptyLabel?: string;
}) {
  return (
    <>
      {items.map((t, i) => (
        <li
          key={i}
          className="flex items-start gap-2 rounded-sm px-2 py-1 text-[13px] hover:bg-accent"
        >
          <span
            className={
              t.status === "completed"
                ? "mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-success"
                : t.status === "in_progress"
                  ? "mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-primary"
                  : "mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-muted-foreground"
            }
          />
          <span className="min-w-0 break-words">
            {t.status === "in_progress" && t.active_form ? t.active_form : t.content}
          </span>
        </li>
      ))}
      {items.length === 0 && (
        <li className="px-2 py-1 text-[12px] text-muted-foreground">{emptyLabel}</li>
      )}
    </>
  );
}
