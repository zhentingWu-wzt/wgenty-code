import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { GetTodosResponse } from "../../api/types";
import { usePolling } from "../../hooks/usePolling";
import { useUiStore } from "../../state/uiStore";
import { TodoListItems } from "../panels/TodoListItems";

/**
 * 聊天区右上角常驻计划胶囊：跟随当前活跃会话（session_id 查询参数路由到
 * 该会话所属项目的 todo 状态）。默认展开显示完整计划清单，点击标题行折叠
 * 为进度条 + N/M 计数。展开/折叠状态提升到 uiStore（todoPillExpanded），
 * App 据此让聊天内容左移避让。无计划时整个组件不渲染，不占空间。
 * 3s 轮询 /todos（与 TasksPanel 一致）。
 */
export function TodoPill({ client, sessionId }: { client: DaemonClient; sessionId?: string }) {
  const [todos, setTodos] = useState<GetTodosResponse | null>(null);
  const expanded = useUiStore((s) => s.todoPillExpanded);
  const setTodoPillExpanded = useUiStore((s) => s.setTodoPillExpanded);

  const refresh = useCallback(async () => {
    const t = await client.getTodos(sessionId);
    setTodos(t);
  }, [client, sessionId]);

  // Polling errors are swallowed inside usePolling — the pill just keeps the
  // last known state until the next tick succeeds.
  usePolling(refresh, true, 3000);

  // 会话切换时立即清空旧数据并马上拉取新会话的计划：usePolling 不感知
  // 参数变化（fnRef 机制），下一 tick 最长 3s 后才到，期间不能显示上一个
  // 会话的计划。首帧跳过（usePolling 的首个 tick 已覆盖）。
  const mountedRef = useRef(false);
  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true;
      return;
    }
    setTodos(null);
    void refresh();
  }, [refresh]);

  const setTodoPillOverlay = useUiStore((s) => s.setTodoPillOverlay);
  const items = todos?.items ?? [];
  // 布局派生：展开且真的有内容才需要聊天区让位（写入 uiStore，App 消费）。
  // 无计划时 overlay 回落 false，<main> 恢复全宽。
  const overlay = expanded && items.length > 0;
  const prevOverlayRef = useRef<boolean | null>(null);
  useEffect(() => {
    if (prevOverlayRef.current !== overlay) {
      prevOverlayRef.current = overlay;
      setTodoPillOverlay(overlay);
    }
  }, [overlay, setTodoPillOverlay]);
  // 卸载（切到 subagent/preview/diff 标签页或欢迎页）时复位 overlay，
  // 让位边距不能泄漏到不显示胶囊的视图。
  useEffect(() => {
    return () => {
      if (prevOverlayRef.current) {
        setTodoPillOverlay(false);
      }
    };
  }, [setTodoPillOverlay]);

  if (items.length === 0) return null;

  const done = items.filter((t) => t.status === "completed").length;
  const total = items.length;
  const allDone = done === total;

  return (
    <div className="pointer-events-none absolute top-2 right-3 z-30">
      <div className="pointer-events-auto flex flex-col items-end">
        <button
          type="button"
          data-testid="todo-pill"
          onClick={() => setTodoPillExpanded(!expanded)}
          aria-expanded={expanded}
          className="flex h-7 items-center gap-2 rounded-full border border-border bg-popover pl-2 pr-2.5 shadow-sm transition-colors hover:bg-accent"
        >
          {/* 进度条：两段式底色，完成比例用 primary 高亮 */}
          <span className="relative h-1.5 w-16 overflow-hidden rounded-full bg-muted">
            <span
              className={`absolute inset-y-0 left-0 rounded-full ${allDone ? "bg-success" : "bg-primary"}`}
              style={{ width: `${Math.round((done / total) * 100)}%` }}
            />
          </span>
          <span className={`text-[11px] font-medium tabular-nums ${allDone ? "text-success" : "text-foreground"}`}>
            {done}/{total}
          </span>
          {expanded ? (
            <ChevronUp size={12} className="text-muted-foreground" />
          ) : (
            <ChevronDown size={12} className="text-muted-foreground" />
          )}
        </button>
        {expanded && (
          <ul
            data-testid="todo-pill-popover"
            className="mt-1.5 max-h-72 w-72 overflow-y-auto overscroll-contain rounded-md border border-border bg-popover py-1 shadow-lg"
          >
            <TodoListItems items={items} emptyLabel="No plan" />
          </ul>
        )}
      </div>
    </div>
  );
}
