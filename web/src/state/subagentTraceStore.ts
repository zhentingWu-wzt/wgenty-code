/**
 * Subagent trace tree store — maintains a live view of the subagent execution
 * tree, fed by `progress` events from the daemon's trace SSE stream.
 *
 * Each TraceEvent carries `node_id` + `parent_id`, forming a tree. We store a
 * flat map keyed by `node_id` and reconstruct the tree in the UI via
 * `parent_id` lookups. Nodes are updated in-place on each progress event;
 * stale nodes (from a previous turn) are cleared when a new root node arrives.
 */
import { create } from "zustand";
import type { TraceEvent } from "../api/types";

export interface SubagentNode {
  nodeId: string;
  parentId: string | null;
  label: string;
  status: string;
  round: number | null;
  currentTool: string | null;
  elapsedMs: number;
  cumulativeTokens: number;
  lastUpdated: number;
}

export interface SubagentTraceState {
  nodes: Map<string, SubagentNode>;
  rootSessionId: string | null;

  /** Upsert a node from a trace event. Clears the tree when a new root arrives. */
  upsertFromEvent: (ev: TraceEvent) => void;
  /** Clear all nodes (e.g. on session switch). */
  clear: () => void;
}

export const useSubagentTraceStore = create<SubagentTraceState>((set, get) => ({
  nodes: new Map(),
  rootSessionId: null,

  upsertFromEvent: (ev) => {
    const node: SubagentNode = {
      nodeId: ev.node_id,
      parentId: ev.parent_id ?? null,
      label: ev.label,
      status: ev.status,
      round: ev.round ?? null,
      currentTool: ev.current_tool ?? null,
      elapsedMs: ev.elapsed_ms,
      cumulativeTokens: ev.cumulative_tokens,
      lastUpdated: Date.now(),
    };

    set((state) => {
      const nodes = new Map(state.nodes);
      const isRoot = !ev.parent_id;

      // When a new root arrives, clear previous turn's tree (the old root
      // has completed). This keeps the view focused on the current activity.
      if (isRoot && state.rootSessionId && state.rootSessionId !== ev.session_id) {
        nodes.clear();
      }

      nodes.set(node.nodeId, node);
      return {
        nodes,
        rootSessionId: isRoot ? ev.session_id : state.rootSessionId,
      };
    });
  },

  clear: () => {
    get().nodes.clear();
    set({ nodes: new Map(), rootSessionId: null });
  },
}));

/**
 * Build a children map from the flat node map, keyed by parent_id.
 * Root nodes (parent_id null) are collected separately.
 */
export function buildChildrenMap(
  nodes: Map<string, SubagentNode>,
): { roots: SubagentNode[]; children: Map<string, SubagentNode[]> } {
  const roots: SubagentNode[] = [];
  const children = new Map<string, SubagentNode[]>();

  for (const node of nodes.values()) {
    if (!node.parentId) {
      roots.push(node);
    } else {
      const siblings = children.get(node.parentId);
      if (siblings) {
        siblings.push(node);
      } else {
        children.set(node.parentId, [node]);
      }
    }
  }

  // Sort by lastUpdated for stable display.
  roots.sort((a, b) => a.lastUpdated - b.lastUpdated);
  for (const siblings of children.values()) {
    siblings.sort((a, b) => a.lastUpdated - b.lastUpdated);
  }

  return { roots, children };
}
