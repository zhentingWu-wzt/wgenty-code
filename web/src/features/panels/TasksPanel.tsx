import { useCallback, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { GetTodosResponse, TaskInfo } from "../../api/types";
import { usePolling } from "../../hooks/usePolling";
import { TodoListItems } from "./TodoListItems";

/**
 * 计划面板：当前活跃会话 todos（GET /todos?session_id=...，agent 的
 * update_plan 计划清单）。历史遗留的后台任务看板（GET /tasks）默认折叠
 * —— 它是持久化任务板而非当轮计划，绝大多数场景用户只想看计划进度；
 * 展开后只读展示。3s 轮询刷新。
 */
export function TasksPanel({ client, sessionId }: { client: DaemonClient; sessionId?: string }) {
  const [todos, setTodos] = useState<GetTodosResponse | null>(null);
  const [tasks, setTasks] = useState<TaskInfo[] | null>(null);
  const [showTasks, setShowTasks] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [t, k] = await Promise.all([client.getTodos(sessionId), client.listTasks(sessionId)]);
      setTodos(t);
      setTasks(k.tasks);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client, sessionId]);

  // 3s poll — frequent enough to catch todo status changes mid-turn without
  // hammering the daemon.
  usePolling(refresh, true, 3000);

  if (error) return <div className="p-3 text-danger">{error}</div>;

  const done = todos?.items.filter((t) => t.status === "completed").length ?? 0;
  const total = todos?.items.length ?? 0;

  return (
    <div className="flex flex-col gap-3 p-2">
      <section>
        <h3 className="flex items-center justify-between px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">
          Plan
          {total > 0 && (
            <span className="font-normal normal-case">
              {done}/{total}
            </span>
          )}
        </h3>
        <ul className="flex flex-col gap-0.5">
          <TodoListItems items={todos?.items ?? []} emptyLabel="No plan" />
        </ul>
      </section>
      {showTasks && (
        <section>
          <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">
            Task board
          </h3>
          <ul className="flex flex-col gap-0.5">
            {(tasks ?? []).filter((t) => t.status !== "deleted").map((t) => (
              <li key={t.id} className="rounded-sm px-2 py-1 hover:bg-accent">
                <div className="text-[13px]">{t.subject}</div>
                <div className="text-[11px] text-muted-foreground">
                  {t.status} · {t.priority}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      <button
        type="button"
        onClick={() => setShowTasks((v) => !v)}
        className="flex items-center gap-1 self-start px-1 text-[11px] text-muted-foreground hover:text-foreground"
      >
        {showTasks ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        Task board
      </button>
    </div>
  );
}
