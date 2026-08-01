/**
 * Session registry — the multi-session backbone of the command center.
 *
 * Owns one SessionStore per session (created via `createSessionStore`), the
 * active-session pointer, and per-session display metadata (status, preview)
 * that the LeftRail session list renders. Status is derived from loop events
 * by `agent/sessionRunner.ts` — this module only stores it.
 *
 * Also holds app-level connection/modelName (moved off the old singleton
 * chatStore: they're global facts, not per-session).
 */
import { create } from "zustand";
import { createSessionStore, type SessionStore } from "./sessionStore";
import type { ConnectionStatus } from "./sessionStore";

export type SessionStatus = "running" | "awaiting_approval" | "idle" | "error";

export interface SessionEntry {
  id: string;
  daemonId: string | null;
  name: string;
  store: SessionStore;
  status: SessionStatus;
  lastPreview: string;
  updatedAt: number;
}

interface SessionManagerState {
  entries: Record<string, SessionEntry>;
  /** Display order (creation order; LeftRail renders in this order). */
  order: string[];
  activeId: string | null;
  connection: ConnectionStatus;
  modelName: string | null;

  createLocalSession: (name?: string) => string;
  removeSession: (id: string) => void;
  setActive: (id: string) => void;
  setStatus: (id: string, status: SessionStatus) => void;
  setPreview: (id: string, text: string) => void;
  setDaemonId: (id: string, daemonId: string) => void;
  setConnection: (s: ConnectionStatus) => void;
  setModelName: (n: string | null) => void;
}

let counter = 1;

function patchEntry(
  entries: Record<string, SessionEntry>,
  id: string,
  patch: Partial<SessionEntry>,
): Record<string, SessionEntry> {
  const e = entries[id];
  if (!e) return entries;
  return { ...entries, [id]: { ...e, ...patch, updatedAt: Date.now() } };
}

export const useSessionManager = create<SessionManagerState>((set, get) => ({
  entries: {},
  order: [],
  activeId: null,
  connection: "unknown",
  modelName: null,

  createLocalSession: (name) => {
    const id = `web-${Date.now()}-${counter++}`;
    const entry: SessionEntry = {
      id,
      daemonId: null,
      name: name ?? `Session ${counter - 1}`,
      store: createSessionStore(),
      status: "idle",
      lastPreview: "",
      updatedAt: Date.now(),
    };
    set((s) => ({
      entries: { ...s.entries, [id]: entry },
      order: [...s.order, id],
      activeId: id,
    }));
    return id;
  },

  removeSession: (id) =>
    set((s) => {
      const entries = { ...s.entries };
      delete entries[id];
      const order = s.order.filter((x) => x !== id);
      const activeId = s.activeId === id ? (order[order.length - 1] ?? null) : s.activeId;
      return { entries, order, activeId };
    }),

  setActive: (id) => {
    if (get().entries[id]) set({ activeId: id });
  },

  setStatus: (id, status) => set((s) => ({ entries: patchEntry(s.entries, id, { status }) })),

  setPreview: (id, text) =>
    set((s) => ({ entries: patchEntry(s.entries, id, { lastPreview: text.slice(0, 120) }) })),

  setDaemonId: (id, daemonId) => set((s) => ({ entries: patchEntry(s.entries, id, { daemonId }) })),

  setConnection: (connection) => set({ connection }),
  setModelName: (modelName) => set({ modelName }),
}));

/**
 * Imperative accessor for the active session's store, for code that runs
 * outside React render (event handlers, module-level send loop, SSE hooks).
 * Returns null when no session exists yet (pre-bootstrap).
 */
export function getActiveSessionStore(): SessionStore | null {
  const m = useSessionManager.getState();
  return m.activeId ? m.entries[m.activeId].store : null;
}

/** Selector helper: count of sessions waiting on a permission decision. */
export const selectPendingApprovalCount = (s: SessionManagerState): number =>
  Object.values(s.entries).filter((e) => e.status === "awaiting_approval").length;
