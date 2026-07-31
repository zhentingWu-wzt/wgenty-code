/**
 * Chat store — the bridge between the agent loop and the UI.
 *
 * Holds display-oriented message state (streaming-aware), run status, the
 * connection probe result, and a pending-permission slot that
 * `PermissionModal` resolves.
 *
 * The agent loop (`agent/loop.ts`) never touches React; it talks to this store
 * purely through the callbacks we build in `App.tsx`. This keeps the loop
 * testable in isolation and the React layer free of control-flow logic.
 */
import { create } from "zustand";
import type { PermissionDecision, PermissionRequiredInfo } from "../api/types";
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

interface PendingPermission {
  info: PermissionRequiredInfo;
  resolve: (decision: PermissionDecision) => void;
}

let nextId = 1;
const genId = (): string => `m${nextId++}`;

interface ChatState {
  messages: DisplayMessage[];
  isRunning: boolean;
  /** Error from the most recent turn (shown inline, cleared on next send). */
  lastError: string | null;
  connection: ConnectionStatus;
  modelName: string | null;
  pendingPermission: PendingPermission | null;

  // ── Actions ──────────────────────────────────────────────────────────────
  setConnection: (s: ConnectionStatus) => void;
  setModelName: (n: string | null) => void;
  pushUserMessage: (text: string) => void;
  /** Start a new assistant message that will be streamed into. */
  beginAssistantRound: (round: number) => string;
  /** Append streamed content/reasoning to the assistant message with `id`. */
  appendAssistant: (id: string, ev: StreamEvent) => void;
  /** Mark the streaming assistant message done and attach tool executions. */
  attachToolExec: (assistantId: string, exec: ToolExecution) => void;
  finalizeAssistant: (id: string) => void;
  setError: (msg: string | null) => void;
  setRunning: (b: boolean) => void;
  /** Surface a permission prompt; returns a promise the modal resolves. */
  requestPermission: (info: PermissionRequiredInfo) => Promise<PermissionDecision>;
  resolvePermission: (decision: PermissionDecision) => void;
  clear: () => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: [],
  isRunning: false,
  lastError: null,
  connection: "unknown",
  modelName: null,
  pendingPermission: null,

  setConnection: (s) => set({ connection: s }),
  setModelName: (n) => set({ modelName: n }),

  pushUserMessage: (text) =>
    set((s) => ({ messages: [...s.messages, { id: genId(), role: "user", content: text }] })),

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
        if (ev.type === "reasoningDelta") return { ...m, reasoning: (m.reasoning ?? "") + ev.text };
        return m;
      }),
    })),

  attachToolExec: (assistantId, exec) =>
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === assistantId
          ? { ...m, toolExecs: [...(m.toolExecs ?? []), exec] }
          : m,
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

  clear: () =>
    set({ messages: [], lastError: null, pendingPermission: null, isRunning: false }),
}));
