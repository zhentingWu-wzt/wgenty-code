import { useState } from "react";
import { ChevronDown, ChevronRight, Network } from "lucide-react";
import { useSubagentTraceStore, type SubagentNode } from "../../state/subagentTraceStore";
import { useSubagentDirectoryStore, flattenCount } from "../../state/subagentDirectoryStore";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import type { AgentDirectoryEntry } from "../../api/types";

/**
 * Subagent execution tree panel — renders the whole-session agent hierarchy
 * from the per-session directory cache (polled `GET /agents/directory`), which
 * survives across turns and session switches.
 *
 * The directory is the structural source of truth (labels, statuses, tokens,
 * parent/child links). The SSE-fed trace store is a read-only supplement: a
 * live trace node with the same agent id contributes its fresher `currentTool`
 * only — the panel no longer depends on the trace store's clear-on-new-root
 * semantics.
 *
 * Children are sorted running-first (running/thinking/pending, directory
 * order kept among them), finished siblings follow newest-started-first.
 * The header shows live badge counts plus a muted `离线` badge when polling
 * has failed repeatedly (cached tree kept). Clicking any node — root included
 * — opens its detail tab (`subagent:<nodeId>`).
 */
export function SubagentTreePanel() {
  const openSubagentTab = useUiStore((s) => s.openSubagentTab);
  // Resolve the directory cache key the same way as useSubagentDirectory /
  // App.tsx: daemon id when bound, local id otherwise.
  const activeEntry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : undefined));
  const sid = activeEntry ? (activeEntry.daemonId ?? activeEntry.id) : null;
  const bucket = useSubagentDirectoryStore((s) => (sid ? s.bySession[sid] : undefined));
  const traceNodes = useSubagentTraceStore((s) => s.nodes);

  // Never polled (or session just created): the first fetch has not landed.
  if (!bucket || !bucket.tree) {
    return (
      <div className="p-2">
        <div className="p-2 text-[12px] text-muted-foreground">加载中…</div>
      </div>
    );
  }

  const counts = flattenCount(bucket.tree);
  const childrenMap = new Map<string, SubagentNode[]>();
  const rootNode = buildDirectoryTree(bucket.tree, sid ?? "", null, traceNodes, childrenMap);

  const onOpen = (node: SubagentNode) =>
    openSubagentTab({
      nodeId: node.nodeId,
      label: node.label,
      rootSessionId: node.sessionId,
    });

  return (
    <div className="p-2">
      <div className="mb-1 flex items-center gap-2 px-1 text-[10px]">
        <span className="text-muted-foreground">
          ● {counts.running} running / {counts.total} total
        </span>
        {bucket.stale && (
          <span className="rounded-sm bg-accent px-1 text-muted-foreground">离线</span>
        )}
      </div>
      {bucket.tree.children.length === 0 ? (
        <div className="p-2 text-[12px] text-muted-foreground">
          本会话暂无子代理,发起任务后此处显示
        </div>
      ) : (
        <>
          <div className="mb-1 px-1 text-[10px] text-muted-foreground">
            点击节点在新标签页查看详情
          </div>
          <ul className="flex flex-col gap-0.5">
            <TreeNode
              key={rootNode.nodeId}
              node={rootNode}
              childrenMap={childrenMap}
              depth={0}
              onOpen={onOpen}
            />
          </ul>
        </>
      )}
    </div>
  );
}

/** Lifecycle statuses that sort a child ahead of finished siblings. */
const ACTIVE_CHILD_STATUSES = new Set(["running", "thinking", "pending"]);

/**
 * Convert one directory entry into a renderable SubagentNode. `nodeId` is the
 * directory's `agent_id`; a live trace node with the same agent id (if any)
 * contributes its `currentTool`, which the polled snapshot may lag behind.
 */
function toSubagentNode(
  entry: AgentDirectoryEntry,
  sessionId: string,
  parentId: string | null,
  traceNodes: Map<string, SubagentNode>,
): SubagentNode {
  const trace = traceNodes.get(entry.agent_id);
  return {
    nodeId: entry.agent_id,
    parentId,
    sessionId,
    label: entry.label,
    status: entry.status,
    round: entry.round ?? null,
    currentTool: trace ? trace.currentTool : null,
    resultText: trace ? trace.resultText : null,
    elapsedMs: entry.elapsed_ms,
    cumulativeTokens: entry.cumulative_tokens,
    lastUpdated: Date.now(),
  };
}

/**
 * Sort sibling directory entries: running-ish first (stable — directory order
 * preserved among them), then finished agents by `started_at` descending.
 */
function sortSiblings(entries: AgentDirectoryEntry[]): AgentDirectoryEntry[] {
  return [...entries].sort((a, b) => {
    const aActive = ACTIVE_CHILD_STATUSES.has(a.status.toLowerCase()) ? 0 : 1;
    const bActive = ACTIVE_CHILD_STATUSES.has(b.status.toLowerCase()) ? 0 : 1;
    if (aActive !== bActive) return aActive - bActive;
    if (aActive === 0) return 0;
    return b.started_at - a.started_at;
  });
}

/**
 * Recursively convert a directory tree into the flat `SubagentNode` shape the
 * `TreeNode` renderer consumes, filling `childrenMap` (keyed by node id) with
 * each node's sorted children along the way.
 */
function buildDirectoryTree(
  entry: AgentDirectoryEntry,
  sessionId: string,
  parentId: string | null,
  traceNodes: Map<string, SubagentNode>,
  childrenMap: Map<string, SubagentNode[]>,
): SubagentNode {
  const node = toSubagentNode(entry, sessionId, parentId, traceNodes);
  const kids = sortSiblings(entry.children).map((child) =>
    buildDirectoryTree(child, sessionId, entry.agent_id, traceNodes, childrenMap),
  );
  childrenMap.set(node.nodeId, kids);
  return node;
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

      {/* Terminal result preview (full text in the detail tab) */}
      {node.resultText && (
        <div
          className="whitespace-pre-wrap break-words px-1 pb-0.5 text-[10px] text-muted-foreground/80"
          style={{ paddingLeft: `${depth * 12 + 20}px` }}
          title={node.resultText}
        >
          {node.resultText.length > 200
            ? `${node.resultText.slice(0, 200)}…`
            : node.resultText}
        </div>
      )}

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
