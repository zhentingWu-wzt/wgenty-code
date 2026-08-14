/**
 * Subagent directory store — per-session cached snapshots of the daemon's
 * agent directory tree (`GET /agents/directory`), refreshed by the
 * `useSubagentDirectory` polling hook.
 *
 * Unlike the SSE-fed trace store (push, current turn only), this is a polled
 * per-session cache: the whole-session hierarchy (including completed agents)
 * survives across turns, and switching sessions shows the cached tree
 * immediately instead of an empty panel while the next poll lands.
 */
import { create } from "zustand";
import type { AgentDirectoryEntry, AgentDirectoryResponse } from "../api/types";

/** Cached directory snapshot for one session. */
interface SessionDirectory {
  tree: AgentDirectoryEntry | null;
  /** Wall-clock ms of the last successful `apply`. */
  fetchedAt: number;
  /** True when polling failed repeatedly after the last successful fetch —
   * the cached tree is kept for display but flagged as outdated. */
  stale: boolean;
}

interface SubagentDirectoryState {
  bySession: Record<string, SessionDirectory>;
  /** Apply a successful fetch for a session (clears stale). */
  apply: (sessionId: string, res: AgentDirectoryResponse) => void;
  /** Mark a session stale after consecutive failures (keep cached tree). */
  markStale: (sessionId: string) => void;
  /** Drop a session's cached directory (e.g. session removed). */
  forget: (sessionId: string) => void;
}

export const useSubagentDirectoryStore = create<SubagentDirectoryState>((set) => ({
  bySession: {},
  apply: (sessionId, res) =>
    set((s) => ({
      bySession: {
        ...s.bySession,
        [sessionId]: { tree: res.root, fetchedAt: Date.now(), stale: false },
      },
    })),
  markStale: (sessionId) =>
    set((s) => {
      const cur = s.bySession[sessionId];
      if (!cur) return s;
      return { bySession: { ...s.bySession, [sessionId]: { ...cur, stale: true } } };
    }),
  forget: (sessionId) =>
    set((s) => {
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      return { bySession };
    }),
}));

/** Lifecycle statuses that count as "actively running" for badge counts. */
const ACTIVE_STATUSES = new Set(["running", "thinking", "pending"]);

/**
 * Flatten a directory tree into badge counts for panel headers: `running`
 * counts nodes whose lowercased status is running/thinking/pending; `total`
 * counts every node including the root. Null-safe for sessions that have
 * never been polled.
 */
export function flattenCount(tree: AgentDirectoryEntry | null): {
  running: number;
  total: number;
} {
  let running = 0;
  let total = 0;
  const visit = (node: AgentDirectoryEntry): void => {
    total += 1;
    if (ACTIVE_STATUSES.has(node.status.toLowerCase())) running += 1;
    for (const child of node.children) visit(child);
  };
  if (tree) visit(tree);
  return { running, total };
}
