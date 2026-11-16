/**
 * Session registry — the multi-session backbone of the command center.
 *
 * Owns one SessionStore per session (created via `createSessionStore`), the
 * active-session pointer, and per-session display metadata (status, preview,
 * effective model/mode) that the session list and active-session UI render.
 * Connection and the global context window remain app-level facts.
 */
import { create } from "zustand";
import { createSessionStore, type SessionStore } from "./sessionStore";
import type { ConnectionStatus } from "./sessionStore";
import type { GlobalEvent, PermissionMode, WorktreeBinding } from "../api/types";

export type SessionStatus = "running" | "awaiting_approval" | "idle" | "error";

export interface SessionModelState {
  provider: string | null;
  model: string;
  isOverride: boolean;
}

export interface SessionModeState {
  mode: PermissionMode;
  isOverride: boolean;
}

export interface SessionEntry {
  id: string;
  daemonId: string | null;
  name: string;
  /** Whether `name` was explicitly set by the user (vs the "Session N"
   *  placeholder). Unnamed sessions are auto-titled daemon-side from the
   *  first user message; the local tab mirrors that title after turn 1. */
  named: boolean;
  store: SessionStore;
  status: SessionStatus;
  lastPreview: string;
  updatedAt: number;
  model: SessionModelState | null;
  mode: SessionModeState | null;
  modelRevision: number;
  modeRevision: number;
  modelSeq: number;
  modeSeq: number;
  /** Bound worktree (N:1); undefined = main checkout. */
  worktree?: WorktreeBinding;
  /** Owning project's canonical path; null = main project (local sessions
   *  default to null and therefore group under the main project). */
  projectPath: string | null;
}

/** Options for sessions created with a daemon-side identity (bound sessions
 * use the daemon session id as their single identity). */
export interface CreateSessionOptions {
  id?: string;
  daemonId?: string;
  worktree?: WorktreeBinding;
  projectPath?: string | null;
  /** Set false to create without activating (e.g. bootstrap history restore,
   * where every restored session would otherwise become the active one — and
   * thus open a tab via uiSync). Default true. */
  activate?: boolean;
}

interface SessionManagerState {
  entries: Record<string, SessionEntry>;
  /** Display order (creation order; LeftRail renders in this order). */
  order: string[];
  activeId: string | null;
  connection: ConnectionStatus;
  /** Global default model's context window (tokens), used by the StatusBar. */
  contextWindow: number | null;

  createLocalSession: (name?: string, opts?: CreateSessionOptions) => string;
  removeSession: (id: string) => void;
  setActive: (id: string) => void;
  /** Deactivate (no active session) — the center pane shows the welcome
   *  screen. Used when the last tab closes. */
  clearActive: () => void;
  setStatus: (id: string, status: SessionStatus) => void;
  setPreview: (id: string, text: string) => void;
  setDaemonId: (id: string, daemonId: string) => void;
  /** Set (`WorktreeBinding`) or clear (`null`) a session's worktree binding. */
  setWorktree: (id: string, wt: WorktreeBinding | null) => void;
  /** Reassign a session to a project (canonical path; null = main project). */
  setProjectPath: (id: string, projectPath: string | null) => void;
  setConnection: (s: ConnectionStatus) => void;
  setContextWindow: (n: number | null) => void;
  setSessionModel: (id: string, model: SessionModelState | null) => void;
  setSessionMode: (id: string, mode: SessionModeState | null) => void;
  getSessionModelRevision: (id: string) => number;
  getSessionModeRevision: (id: string) => number;
  setSessionModelIfRevision: (
    id: string,
    revision: number,
    model: SessionModelState | null,
  ) => void;
  setSessionModeIfRevision: (id: string, revision: number, mode: SessionModeState | null) => void;
  resetModelModeEventSeq: () => void;
  applyModelChanged: (event: GlobalEvent) => void;
  applyLocalModelChanged: (data: Record<string, unknown>) => void;
  applyModeChanged: (event: GlobalEvent) => void;
  /** Rename a session locally (marks it named — placeholders can be replaced
   *  by the mirrored daemon auto-title). */
  renameSession: (id: string, name: string) => void;
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

function entryIdForRoutedId(entries: Record<string, SessionEntry>, routedId: string) {
  return Object.values(entries).find(
    (entry) => entry.daemonId === routedId || entry.id === routedId,
  )?.id;
}

function isPermissionMode(value: unknown): value is PermissionMode {
  return value === "normal" || value === "accept_edits" || value === "yolo";
}

export const useSessionManager = create<SessionManagerState>((set, get) => ({
  entries: {},
  order: [],
  activeId: null,
  connection: "unknown",
  contextWindow: null,

  createLocalSession: (name, opts) => {
    // opts.id (bound sessions) bypasses the web-* generator entirely.
    const id = opts?.id ?? `web-${Date.now()}-${counter++}`;
    const entry: SessionEntry = {
      id,
      daemonId: opts?.daemonId ?? null,
      name: name ?? `Session ${counter - 1}`,
      named: name !== undefined,
      store: createSessionStore(),
      status: "idle",
      lastPreview: "",
      updatedAt: Date.now(),
      model: null,
      mode: null,
      modelRevision: 0,
      modeRevision: 0,
      modelSeq: 0,
      modeSeq: 0,
      projectPath: opts?.projectPath ?? null,
      ...(opts?.worktree ? { worktree: opts.worktree } : {}),
    };
    set((s) => ({
      entries: { ...s.entries, [id]: entry },
      order: [...s.order, id],
      ...(opts?.activate === false ? {} : { activeId: id }),
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

  clearActive: () => set({ activeId: null }),

  setStatus: (id, status) => set((s) => ({ entries: patchEntry(s.entries, id, { status }) })),

  setPreview: (id, text) =>
    set((s) => ({ entries: patchEntry(s.entries, id, { lastPreview: text.slice(0, 120) }) })),

  setDaemonId: (id, daemonId) => set((s) => ({ entries: patchEntry(s.entries, id, { daemonId }) })),

  setWorktree: (id, wt) =>
    set((s) => ({
      entries: patchEntry(s.entries, id, { worktree: wt ?? undefined }),
    })),

  setProjectPath: (id, projectPath) =>
    set((s) => ({ entries: patchEntry(s.entries, id, { projectPath }) })),

  setConnection: (connection) => set({ connection }),
  setContextWindow: (contextWindow) => set({ contextWindow }),
  setSessionModel: (id, model) =>
    set((s) => {
      const entry = s.entries[id];
      if (!entry) return s;
      return {
        entries: patchEntry(s.entries, id, {
          model,
          modelRevision: entry.modelRevision + 1,
        }),
      };
    }),
  setSessionMode: (id, mode) =>
    set((s) => {
      const entry = s.entries[id];
      if (!entry) return s;
      return {
        entries: patchEntry(s.entries, id, {
          mode,
          modeRevision: entry.modeRevision + 1,
        }),
      };
    }),
  getSessionModelRevision: (id) => get().entries[id]?.modelRevision ?? 0,
  getSessionModeRevision: (id) => get().entries[id]?.modeRevision ?? 0,
  setSessionModelIfRevision: (id, revision, model) =>
    set((s) => {
      const entry = s.entries[id];
      if (!entry || entry.modelRevision !== revision) return s;
      return {
        entries: patchEntry(s.entries, id, { model, modelRevision: revision + 1 }),
      };
    }),
  setSessionModeIfRevision: (id, revision, mode) =>
    set((s) => {
      const entry = s.entries[id];
      if (!entry || entry.modeRevision !== revision) return s;
      return {
        entries: patchEntry(s.entries, id, { mode, modeRevision: revision + 1 }),
      };
    }),
  resetModelModeEventSeq: () =>
    set((s) => ({
      entries: Object.fromEntries(
        Object.entries(s.entries).map(([id, entry]) => [id, { ...entry, modelSeq: 0, modeSeq: 0 }]),
      ),
    })),

  applyModelChanged: ({ seq, data }) => {
    const provider = data["provider"];
    const model = data["model_name"];
    const reset = data["reset"] === true;
    if (
      (typeof provider !== "string" && !(reset && provider === null)) ||
      typeof model !== "string"
    ) {
      return;
    }
    const sessionId = data["session_id"];
    set((s) => {
      if (typeof sessionId === "string") {
        const id = entryIdForRoutedId(s.entries, sessionId);
        if (!id) return s;
        const entry = s.entries[id];
        if (seq <= entry.modelSeq) return s;
        return {
          entries: patchEntry(s.entries, id, {
            model: { provider, model, isOverride: !reset },
            modelRevision: entry.modelRevision + 1,
            modelSeq: seq,
          }),
        };
      }
      const entries = Object.fromEntries(
        Object.entries(s.entries).map(([id, entry]) => {
          if (seq <= entry.modelSeq) return [id, entry];
          if (entry.model?.isOverride !== false) {
            return [id, { ...entry, modelSeq: seq }];
          }
          return [
            id,
            {
              ...entry,
              model: { provider, model, isOverride: false },
              modelRevision: entry.modelRevision + 1,
              modelSeq: seq,
              updatedAt: Date.now(),
            },
          ];
        }),
      );
      return { entries };
    });
  },

  applyLocalModelChanged: (data) => {
    const provider = data["provider"];
    const model = data["model_name"];
    if (typeof provider !== "string" || typeof model !== "string") return;
    set((s) => ({
      entries: Object.fromEntries(
        Object.entries(s.entries).map(([id, entry]) =>
          entry.model?.isOverride !== false
            ? [id, entry]
            : [
                id,
                {
                  ...entry,
                  model: { provider, model, isOverride: false },
                  modelRevision: entry.modelRevision + 1,
                  updatedAt: Date.now(),
                },
              ],
        ),
      ),
    }));
  },

  applyModeChanged: ({ seq, data }) => {
    const sessionId = data["session_id"];
    const mode = data["mode"];
    if (typeof sessionId !== "string" || !isPermissionMode(mode)) return;
    set((s) => {
      const id = entryIdForRoutedId(s.entries, sessionId);
      if (!id) return s;
      const entry = s.entries[id];
      if (seq <= entry.modeSeq) return s;
      return {
        entries: patchEntry(s.entries, id, {
          mode: {
            mode,
            isOverride: data["reset"] === true ? false : data["is_override"] !== false,
          },
          modeRevision: entry.modeRevision + 1,
          modeSeq: seq,
        }),
      };
    });
  },

  renameSession: (id, name) =>
    set((s) => ({ entries: patchEntry(s.entries, id, { name, named: true }) })),
}));

/** Mirror of the daemon's `title_from_first_user_message`: collapse
 *  whitespace, cap at 48 chars + ellipsis. Keeps the local tab label in
 *  lockstep with the daemon-side auto-title without a refetch. */
export function sessionTitleFromMessage(text: string): string {
  const collapsed = text.trim().split(/\s+/).join(" ");
  const chars = Array.from(collapsed);
  return chars.length > 48 ? `${chars.slice(0, 48).join("")}…` : collapsed;
}

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
