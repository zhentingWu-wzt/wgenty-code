import { DaemonClient } from "../api/client";
import { usePolling } from "../hooks/usePolling";
import { useSidebarStore } from "../state/sidebarStore";

const POLL_MS = 5000;

/** Task graph snapshot: progress counts + task list. */
export function TasksPanel({ client }: { client: DaemonClient }) {
  const tasks = useSidebarStore((s) => s.tasks);
  const taskProgress = useSidebarStore((s) => s.taskProgress);
  const setTasks = useSidebarStore((s) => s.setTasks);
  const setTaskProgress = useSidebarStore((s) => s.setTaskProgress);

  usePolling(
    async () => {
      const [t, p] = await Promise.all([client.listTasks(), client.taskProgress()]);
      setTasks(t);
      setTaskProgress(p);
    },
    true,
    POLL_MS,
  );

  const ready = taskProgress?.ready ?? 0;
  const blocked = taskProgress?.blocked ?? 0;

  return (
    <div className="tasks-panel">
      <div className="task-progress">
        <span className="task-prog-chip ready">{ready} ready</span>
        <span className="task-prog-chip blocked">{blocked} blocked</span>
      </div>
      {!tasks || tasks.tasks.length === 0 ? (
        <div className="panel-empty">No tasks.</div>
      ) : (
        <ul className="task-list">
          {tasks.tasks.map((t) => (
            <li key={t.id} className={`task-item task-${t.status}`}>
              <div className="task-head">
                <span className="task-priority" data-priority={t.priority}>
                  {t.priority}
                </span>
                <span className="task-subject">{t.subject}</span>
              </div>
              {t.description && <div className="task-desc">{t.description}</div>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
