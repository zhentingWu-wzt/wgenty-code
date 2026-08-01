/**
 * Session store — the bridge between the agent loop and the UI, one instance
 * per chat session.
 *
 * Holds display-oriented message state (streaming-aware), run status, the
 * connection probe result, and a pending-permission slot that
 * `PermissionModal` resolves.
 *
 * The agent loop (`agent/loop.ts`) never touches React; it talks to this store
 * purely through the callbacks we build in `App.tsx`. This keeps the loop
 * testable in isolation and the React layer free of control-flow logic.
 *
 * Created via `createSessionStore()` so each session gets fully isolated
 * state; components subscribe through `sessionContext.tsx`.
 */
import { create } from "zustand";
import type { PermissionDecision, PermissionRequiredInfo, StructuredApproval } from "../api/types";
import type { StreamEvent } from "../api/sseParser";
import type { ToolExecution } from "../agent/loop";

/** One displayable chat message (richer than the wire `ChatMessage`). */
export interface DisplayMessage {
  id: string;
  role: "user" | "assistant" | "tool";
  /** Rendered text content (streamed for assistant turns). */
  content: string;
  /** Optional reasoning/extraction trace shown above content. */
  reasoning?: string;
  /** Tool executions attached to this assistant message (rendered as cards). */
  toolExecs?: ToolExecution[];
  /** For tool messages: id of the tool call this is a result of. */
  toolCallId?: string;
  /** Whether this assistant message is still streaming. */
  streaming?: boolean;
  /** Round index within the current turn (assistant messages only). */
  round?: number;
}

export type ConnectionStatus = "unknown" | "connected" | "disconnected";

/**
 * Structured turn error (design D7.3). `kind` distinguishes transport failures
 * (daemon down / network) — which a retry can fix — from upstream LLM errors
 * (rejected prompt, rate limit) which it can't. Only transport errors carry a
 * `retry` callback.
 */
export interface TurnError {
  message: string;
  kind: "transport" | "upstream";
  retry?: () => void;
}

interface PendingPermission {
  info: PermissionRequiredInfo;
  resolve: (decision: PermissionDecision) => void;
}

let nextId = 1;
const genId = (): string => `m${nextId++}`;

export interface SessionState {
  messages: DisplayMessage[];
  isRunning: boolean;
  /** Error from the most recent turn (shown inline, cleared on next send). */
  lastError: TurnError | null;
  connection: ConnectionStatus;
  modelName: string | null;
  pendingPermission: PendingPermission | null;
  /** Subagent async permission (pushed via trace SSE). Null when none pending. */
  pendingSubagent: StructuredApproval | null;

  // ── Actions ──────────────────────────────────────────────────────────────
  setConnection: (s: ConnectionStatus) => void;
  setModelName: (n: string | null) => void;
  /** Push a pre-built display message (used when loading a session's history). */
  pushLoadedMessage: (m: DisplayMessage) => void;
  pushUserMessage: (text: string) => void;
  /** Start a new assistant message that will be streamed into. */
  beginAssistantRound: (round: number) => string;
  /** Append streamed content/reasoning to the assistant message with `id`. */
  appendAssistant: (id: string, ev: StreamEvent) => void;
  /** Mark the streaming assistant message done and attach tool executions. */
  attachToolExec: (assistantId: string, exec: ToolExecution) => void;
  finalizeAssistant: (id: string) => void;
  setError: (err: TurnError | null) => void;
  setRunning: (b: boolean) => void;
  /** Surface a permission prompt; returns a promise the modal resolves. */
  requestPermission: (info: PermissionRequiredInfo) => Promise<PermissionDecision>;
  resolvePermission: (decision: PermissionDecision) => void;
  /** Push a subagent permission prompt (from trace SSE). */
  pushSubagentPermission: (approval: StructuredApproval) => void;
  /** Dismiss the current subagent prompt (after the hook has resolved it). */
  clearSubagentPermission: () => void;
  /** App registers the current turn's AbortController so Stop can abort it. */
  registerAbort: (controller: AbortController | null) => void;
  /** Abort the running turn (no-op if nothing running). */
  stopRunning: () => void;
  clear: () => void;
}

export function createSessionStore() {
  // Per-instance holder for the running turn's AbortController (was module-
  // level in the singleton era — must be per-session now). Kept out of React
  // state on purpose: it's a mutable imperative handle, not render data, and
  // storing it in state would cause needless re-renders.
  let currentAbort: AbortController | null = null;

  return create<SessionState>((set, get) => ({
    messages: [],
    isRunning: false,
    lastError: null,
    connection: "unknown",
    modelName: null,
    pendingPermission: null,
    pendingSubagent: null,

    setConnection: (s) => set({ connection: s }),
    setModelName: (n) => set({ modelName: n }),

    pushUserMessage: (text) =>
      set((s) => ({ messages: [...s.messages, { id: genId(), role: "user", content: text }] })),

    pushLoadedMessage: (m) => set((s) => ({ messages: [...s.messages, m] })),

    beginAssistantRound: (round) => {
      const id = genId();
      set((s) => ({
        messages: [...s.messages, { id, role: "assistant", content: "", round, streaming: true }],
      }));
      return id;
    },

    appendAssistant: (id, ev) =>
      set((s) => ({
        messages: s.messages.map((m) => {
          if (m.id !== id) return m;
          if (ev.type === "contentDelta") return { ...m, content: m.content + ev.text };
          if (ev.type === "reasoningDelta")
            return { ...m, reasoning: (m.reasoning ?? "") + ev.text };
          return m;
        }),
      })),

    attachToolExec: (assistantId, exec) =>
      set((s) => ({
        messages: s.messages.map((m) =>
          m.id === assistantId ? { ...m, toolExecs: [...(m.toolExecs ?? []), exec] } : m,
        ),
      })),

    finalizeAssistant: (id) =>
      set((s) => ({
        messages: s.messages.map((m) => (m.id === id ? { ...m, streaming: false } : m)),
      })),

    setError: (msg) => set({ lastError: msg }),
    setRunning: (b) => set({ isRunning: b }),

    requestPermission: (info) =>
      new Promise<PermissionDecision>((resolve) => {
        set({ pendingPermission: { info, resolve } });
      }),

    resolvePermission: (decision) => {
      const pending = get().pendingPermission;
      if (pending) {
        pending.resolve(decision);
        set({ pendingPermission: null });
      }
    },

    pushSubagentPermission: (approval) => {
      // Don't overwrite a prompt the user is actively looking at; the trace hook
      // resolves the current one before the next pending event arrives in
      // practice (the bridge blocks the subagent until resolved).
      if (!get().pendingSubagent) set({ pendingSubagent: approval });
    },

    clearSubagentPermission: () => set({ pendingSubagent: null }),

    registerAbort: (controller) => {
      currentAbort = controller;
    },

    stopRunning: () => {
      if (currentAbort) {
        currentAbort.abort();
        currentAbort = null;
      }
      // isRunning is cleared by the loop's finally block once it unwinds; we
      // don't set it here to avoid a double-state-write race with that finally.
    },

    clear: () =>
      set({
        messages: [],
        lastError: null,
        pendingPermission: null,
        pendingSubagent: null,
        isRunning: false,
      }),
  }));
}

export type SessionStore = ReturnType<typeof createSessionStore>;
