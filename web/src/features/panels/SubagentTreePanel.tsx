import { useState } from "react";
import { ChevronDown, ChevronRight, Network } from "lucide-react";
import { useSubagentTraceStore, buildChildrenMap, type SubagentNode } from "../../state/subagentTraceStore";
import { useUiStore } from "../../state/uiStore";

/**
 * Subagent execution tree panel - shows a live view of the subagent hierarchy,
 * fed by `progress` events from the daemon's trace SSE stream.
 *
 * Each node displays label, status, current tool (if executing), elapsed time,
 * and cumulative tokens. Nodes are expandable to reveal children. Clicking a
 * node opens a dedicated detail tab (`subagent:<nodeId>`) backed by the
 * capability-scoped agent API. The tree auto-clears when a new root node
 * arrives (previous turn completed).
 */
export function SubagentTreePanel() {
  const nodes = useSubagentTraceStore((s) => s.nodes);
  const openSubagentTab = useUiStore((s) => s.openSubagentTab);
  const { roots, children } = buildChildrenMap(nodes);

  if (roots.length === 0) {
    return (
      <div className="p-2">
        <div className="p-2 text-[12px] text-muted-foreground">
          No active subagents. The tree populates when a turn spawns subagents.
        </div>
      </div>
    );
  }

  const onOpen = (node: SubagentNode) =>
    openSubagentTab({
      nodeId: node.nodeId,
      label: node.label,
      rootSessionId: node.sessionId,
    });

  return (
    <div className="p-2">
      <div className="mb-1 px-1 text-[10px] text-muted-foreground">
        点击节点在新标签页查看详情
      </div>
      <ul className="flex flex-col gap-0.5">
        {roots.map((node) => (
          <TreeNode key={node.nodeId} node={node} childrenMap={children} depth={0} onOpen={onOpen} />
        ))}
      </ul>
    </div>
  );
}

function TreeNode({
  node,
  childrenMap,
  depth,
  onOpen,
}: {
  node: SubagentNode;
  childrenMap: Map<string, SubagentNode[]>;
  depth: number;
  onOpen: (node: SubagentNode) => void;
}) {
  const [expanded, setExpanded] = useState(true);
  const kids = childrenMap.get(node.nodeId) ?? [];
  const hasChildren = kids.length > 0;

  const statusColor = (status: string) => {
    const s = status.toLowerCase();
    if (s === "running" || s === "thinking") return "text-green-500";
    if (s === "done" || s === "completed") return "text-muted-foreground";
    if (s === "error" || s === "failed") return "text-danger";
    return "text-muted-foreground";
  };

  return (
    <li>
      <div
        className="flex cursor-pointer items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent"
        style={{ paddingLeft: `${depth * 12 + 4}px` }}
        title="点击打开详情"
        onClick={() => onOpen(node)}
      >
        {/* Expand/collapse toggle */}
        {hasChildren ? (
          <button
            type="button"
            className="shrink-0 text-muted-foreground"
            onClick={(e) => {
              e.stopPropagation();
              setExpanded(!expanded);
            }}
          >
            {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          </button>
        ) : (
          <span className="w-[11px] shrink-0" />
        )}

        <Network size={11} className="shrink-0 text-primary" />
        {/* Label + status */}
        <span className="min-w-0 flex-1 truncate text-[12px]">{node.label}</span>
        <span className={`shrink-0 text-[10px] ${statusColor(node.status)}`}>{node.status}</span>

        {/* Current tool (if executing) */}
        {node.currentTool && (
          <span className="shrink-0 text-[10px] text-primary" title="current tool">
            🔧 {node.currentTool}
          </span>
        )}
      </div>

      {/* Detail line */}
      <div
        className="flex items-center gap-3 px-1 pb-0.5 text-[10px] text-muted-foreground"
        style={{ paddingLeft: `${depth * 12 + 20}px` }}
      >
        {node.round !== null && <span>round {node.round}</span>}
        <span>{(node.elapsedMs / 1000).toFixed(1)}s</span>
        {node.cumulativeTokens > 0 && <span>{(node.cumulativeTokens / 1000).toFixed(1)}k tok</span>}
      </div>

      {/* Children */}
      {expanded && hasChildren && (
        <ul className="flex flex-col gap-0.5">
          {kids.map((kid) => (
            <TreeNode key={kid.nodeId} node={kid} childrenMap={childrenMap} depth={depth + 1} onOpen={onOpen} />
          ))}
        </ul>
      )}
    </li>
  );
}
