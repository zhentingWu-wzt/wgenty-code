import type { DaemonClient } from "../api/client";
import { TodosPanel } from "./TodosPanel";
import { TasksPanel } from "./TasksPanel";
import { MemoryPanel } from "./MemoryPanel";
import { SubagentPanel } from "./SubagentPanel";
import { CheckpointsPanel } from "./CheckpointsPanel";

/** Right column: active-session context + global panels (marked as such).
 *
 *  The "(global)" markers live here, not inside the reused panels: those
 *  panels render no title of their own (titles used to come from the old
 *  tabbed Sidebar), so each is wrapped in a ctx-section with a rail title. */
export function ContextPanel({ client }: { client: DaemonClient }) {
  return (
    <aside className="contextpanel">
      <SubagentPanel client={client} />
      <section className="ctx-section">
        <span className="rail-section-title">Todos (global)</span>
        <TodosPanel client={client} />
      </section>
      <section className="ctx-section">
        <span className="rail-section-title">Tasks (global)</span>
        <TasksPanel client={client} />
      </section>
      <CheckpointsPanel client={client} />
      <section className="ctx-section">
        <span className="rail-section-title">Memory</span>
        <MemoryPanel client={client} />
      </section>
    </aside>
  );
}
