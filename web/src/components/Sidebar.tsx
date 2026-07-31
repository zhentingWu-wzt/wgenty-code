import { DaemonClient } from "../api/client";
import { useSidebarStore, type SidebarTab } from "../state/sidebarStore";
import { SessionsPanel } from "./SessionsPanel";
import { TodosPanel } from "./TodosPanel";
import { TasksPanel } from "./TasksPanel";
import { ModelPanel } from "./ModelPanel";

const TABS: Array<{ id: SidebarTab; label: string }> = [
  { id: "sessions", label: "Sessions" },
  { id: "todos", label: "Todos" },
  { id: "tasks", label: "Tasks" },
  { id: "model", label: "Model" },
];

/**
 * Collapsible left sidebar with tab-switched panels. Memory/Config tabs are
 * added by Tier 5/6 later (registered here once those panels exist).
 */
export function Sidebar({ client }: { client: DaemonClient }) {
  const collapsed = useSidebarStore((s) => s.collapsed);
  const activeTab = useSidebarStore((s) => s.activeTab);
  const setActiveTab = useSidebarStore((s) => s.setActiveTab);
  const toggle = useSidebarStore((s) => s.toggleCollapsed);

  if (collapsed) {
    return (
      <aside className="sidebar sidebar-collapsed">
        <button type="button" className="sidebar-expand-btn" onClick={toggle} title="Show sidebar">
          ▸
        </button>
      </aside>
    );
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-title">Panels</span>
        <button type="button" className="sidebar-collapse-btn" onClick={toggle} title="Hide sidebar">
          ◂
        </button>
      </div>
      <nav className="sidebar-tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            className={`sidebar-tab ${activeTab === t.id ? "active" : ""}`}
            onClick={() => setActiveTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>
      <div className="sidebar-body">
        {activeTab === "sessions" && <SessionsPanel client={client} />}
        {activeTab === "todos" && <TodosPanel client={client} />}
        {activeTab === "tasks" && <TasksPanel client={client} />}
        {activeTab === "model" && <ModelPanel client={client} />}
      </div>
    </aside>
  );
}
