import { DaemonClient } from "../api/client";
import { usePolling } from "../hooks/usePolling";
import { useChatStore } from "../state/chatStore";
import { useSidebarStore } from "../state/sidebarStore";

const POLL_MS = 5000;

/** Live todo list from `GET /api/v1/todos`. Polls while the agent is running. */
export function TodosPanel({ client }: { client: DaemonClient }) {
  const isRunning = useChatStore((s) => s.isRunning);
  const todos = useSidebarStore((s) => s.todos);
  const setTodos = useSidebarStore((s) => s.setTodos);

  // Poll while running; also do one initial fetch on mount.
  usePolling(
    async () => {
      const res = await client.getTodos();
      setTodos(res);
    },
    true,
    POLL_MS,
  );
  // Silence the unused-var lint for isRunning-driven polling cadence: we keep
  // polling regardless of run state so the panel stays current, but the poller
  // could be gated on isRunning if the constant fetch becomes a concern.
  void isRunning;

  if (!todos || todos.items.length === 0) {
    return <div className="panel-empty">No todos.</div>;
  }

  return (
    <ul className="todo-list">
      {todos.items.map((t, i) => (
        <li key={i} className={`todo-item todo-${t.status}`}>
          <span className="todo-status">{statusGlyph(t.status)}</span>
          <span className="todo-content">{t.active_form || t.content}</span>
        </li>
      ))}
    </ul>
  );
}

function statusGlyph(status: string): string {
  switch (status) {
    case "completed":
      return "✓";
    case "in_progress":
      return "▸";
    default:
      return "○";
  }
}
