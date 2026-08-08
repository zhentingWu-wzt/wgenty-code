import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { GetTodosResponse, TaskInfo } from "../../api/types";

/** 右栏 Tasks 面板：当前会话 todos（GET /todos）+ 后台任务列表（GET /tasks）。只读。 */
export function TasksPanel({ client }: { client: DaemonClient }) {
  const [todos, setTodos] = useState<GetTodosResponse | null>(null);
  const [tasks, setTasks] = useState<TaskInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([client.getTodos(), client.listTasks()])
      .then(([t, k]) => {
        setTodos(t);
        setTasks(k.tasks);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  if (error) return <div className="p-3 text-danger">{error}</div>;

  return (
    <div className="flex flex-col gap-3 p-2">
      <section>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">Todos</h3>
        <ul className="flex flex-col gap-0.5">
          {(todos?.items ?? []).map((t, i) => (
            <li key={i} className="flex items-center gap-2 rounded-sm px-2 py-1 text-[13px] hover:bg-accent">
              <span
                className={
                  t.status === "completed"
                    ? "h-1.5 w-1.5 rounded-full bg-success"
                    : t.status === "in_progress"
                      ? "h-1.5 w-1.5 rounded-full bg-primary"
                      : "h-1.5 w-1.5 rounded-full bg-muted-foreground"
                }
              />
              {t.content}
            </li>
          ))}
          {todos?.items.length === 0 && (
            <li className="px-2 py-1 text-[12px] text-muted-foreground">No todos</li>
          )}
        </ul>
      </section>
      <section>
        <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase text-muted-foreground">Tasks</h3>
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
    </div>
  );
}
