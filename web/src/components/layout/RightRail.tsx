import {
  Brain,
  Eye,
  FolderTree,
  GitBranch,
  History,
  ListTodo,
  Network,
  Plug,
  Settings,
  Sparkles,
  Undo2,
  type LucideIcon,
} from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { useUiStore, type RightPanelId } from "../../state/uiStore";
import { cn } from "../../lib/utils";
import { SkillsPanel } from "../../features/panels/SkillsPanel";
import { MemoryPanel } from "../../features/panels/MemoryPanel";
import { CheckpointsPanel } from "../../features/panels/CheckpointsPanel";
import { SessionsPanel } from "../../features/panels/SessionsPanel";
import { TasksPanel } from "../../features/panels/TasksPanel";
import { ConfigPanel } from "../../features/panels/ConfigPanel";
import { McpPanel } from "../../features/panels/McpPanel";
import { SubagentTreePanel } from "../../features/panels/SubagentTreePanel";
import { InspectorPanel } from "../../features/panels/InspectorPanel";
import { FilesPanel } from "../../features/files/FilesPanel";
import { SourceControlPanel } from "../../features/scm/SourceControlPanel";

const ITEMS: { id: RightPanelId; icon: LucideIcon; label: string }[] = [
  { id: "sessions", icon: History, label: "Sessions" },
  { id: "files", icon: FolderTree, label: "Files" },
  { id: "scm", icon: GitBranch, label: "Source Control" },
  { id: "skills", icon: Sparkles, label: "Skills" },
  { id: "memory", icon: Brain, label: "Memory" },
  { id: "checkpoints", icon: Undo2, label: "Checkpoints" },
  { id: "tasks", icon: ListTodo, label: "Tasks" },
  { id: "subagents", icon: Network, label: "Subagents" },
  { id: "inspector", icon: Eye, label: "Inspector" },
  { id: "mcp", icon: Plug, label: "MCP Servers" },
  { id: "config", icon: Settings, label: "Config" },
];

const PANEL_TITLE: Record<RightPanelId, string> = {
  sessions: "Sessions",
  files: "Files",
  scm: "Source Control",
  skills: "Skills",
  memory: "Memory",
  checkpoints: "Checkpoints",
  tasks: "Tasks",
  subagents: "Subagents",
  inspector: "Inspector",
  mcp: "MCP Servers",
  config: "Config",
};

/** 右栏：36px activity bar + 可切换面板。点已激活图标收起（uiStore.toggleRightPanel）。
 * 移动端（<md）面板为 fixed 覆盖抽屉（对齐 LeftSidebar 模式），不再内联
 * 挤压聊天列 —— 375px 屏上 w-72 内联会把聊天区压到 ~40px。 */
export function RightRail({ client }: { client: DaemonClient }) {
  const rightPanel = useUiStore((s) => s.rightPanel);
  const toggleRightPanel = useUiStore((s) => s.toggleRightPanel);

  return (
    <div className="flex shrink-0 border-l border-border">
      {rightPanel && (
        <div
          data-testid="right-panel-host"
          className={cn(
            "flex w-72 flex-col bg-sidebar",
            "max-md:fixed max-md:bottom-6 max-md:right-9 max-md:top-10 max-md:z-40 max-md:border-l max-md:border-border max-md:shadow-xl",
          )}
        >
          <div className="flex h-9 shrink-0 items-center border-b border-border px-3 text-[12px] font-semibold">
            {PANEL_TITLE[rightPanel]}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {rightPanel === "sessions" && <SessionsPanel client={client} />}
            {rightPanel === "files" && <FilesPanel client={client} />}
            {rightPanel === "scm" && <SourceControlPanel client={client} />}
            {rightPanel === "skills" && <SkillsPanel client={client} />}
            {rightPanel === "memory" && <MemoryPanel client={client} />}
            {rightPanel === "checkpoints" && <CheckpointsPanel client={client} />}
            {rightPanel === "tasks" && <TasksPanel client={client} />}
            {rightPanel === "mcp" && <McpPanel client={client} />}
            {rightPanel === "config" && <ConfigPanel client={client} />}
            {rightPanel === "subagents" && <SubagentTreePanel />}
            {rightPanel === "inspector" && <InspectorPanel />}
          </div>
        </div>
      )}
      <div className="flex w-9 flex-col items-center gap-0.5 bg-sidebar py-1">
        {ITEMS.map(({ id, icon: Icon, label }) => (
          <button
            key={id}
            type="button"
            title={label}
            onClick={() => toggleRightPanel(id)}
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md",
              rightPanel === id
                ? "bg-sidebar-accent text-foreground"
                : "text-muted-foreground hover:bg-sidebar-accent hover:text-foreground",
            )}
          >
            <Icon size={15} />
          </button>
        ))}
      </div>
    </div>
  );
}
