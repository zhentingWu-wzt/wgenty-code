/**
 * Session store — the bridge between the agent loop and the UI, one instance
 * per chat session.
 *
 * Holds display-oriented message state (streaming-aware), run status, the
 * connection probe result, and a pending-permission slot that
 * `PermissionModal` resolves.
 *
 * The server-side observer (`agent/sessionRunner.ts`) never touches React; it
 * drives this store directly (no App.tsx callback wiring). This keeps the
 * runner testable in isolation and the React layer free of control-flow logic.
 *
 * Created via `createSessionStore()` so each session gets fully isolated
 * state; components subscribe through `sessionContext.tsx`.
 */
import { create } from "zustand";
import type {
  PermissionDecision,
  PermissionRequiredInfo,
  QuestionPayload,
  StructuredApproval,
} from "../api/types";
import type { StreamEvent } from "../api/sseParser";
import type { ToolExecution } from "../agent/types";

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
  /** Tool-role entries (timeline mode): the invoked tool + args while running. */
  toolName?: string;
  toolArgs?: Record<string, unknown>;
  /** Tool-role entries: final execution result once tool_result arrives. */
  toolExec?: ToolExecution;
  /** For tool messages: id of the tool call this is a result of. */
  toolCallId?: string;
  /** Whether this assistant message is still streaming. */
  streaming?: boolean;
  /** Round index within the current turn (assistant messages only). */
  round?: number;
}

export type ConnectionStatus = "unknown" | "connected" | "disconnected";

/** Inspector turn-context data — broadcast by daemon after each turn. */
export interface TurnContextLayer {
  label: string;
  source: string;
  char_count: number;
}
export interface TurnContextMemory {
  importance: number;
  memory_type: string;
  content_preview: string;
}
export interface TurnContextMessage {
  role: string;
  content: string;
}
export interface TurnContextUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  /** Most recent prompt size — the current context occupancy numerator. */
  last_prompt_tokens?: number;
  /** Model context window — the occupancy denominator. */
  context_window?: number;
}
export interface TurnContextData {
  layers: TurnContextLayer[];
  recalled_memories: TurnContextMemory[];
  new_messages: TurnContextMessage[];
  reminder: { to_model: string; to_transcript: string | null } | null;
  usage: TurnContextUsage;
}

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
  /** ask_user_question prompt (pushed via trace SSE). Null when none pending. */
  pendingQuestion: QuestionPayload | null;
  /** Turn context from the most recent turn (inspector data: layers, memories,
   * messages, reminder, token usage). Null before the first turn completes. */
  turnContext: TurnContextData | null;

  /** FIFO queue of messages waiting to run after the current turn completes.
   *  Mirrors the TUI's `pending_inputs`: while a turn runs, new sends are
   *  queued here and drained by `runSessionTurn` on clean completion. */
  pendingInputs: string[];

  // ── Actions ──────────────────────────────────────────────────────────────
  setConnection: (s: ConnectionStatus) => void;
  setModelName: (n: string | null) => void;
  /** Push a pre-built display message (used when loading a session's history). */
  pushLoadedMessage: (m: DisplayMessage) => void;
  pushUserMessage: (text: string) => void;
  /** Remove the most recent user message (roll back an optimistic send). */
  popUserMessage: () => void;
  /** Start a new assistant message that will be streamed into. */
  beginAssistantRound: (round: number) => string;
  /** Append streamed content/reasoning to the assistant message with `id`. */
  appendAssistant: (id: string, ev: StreamEvent) => void;
  /** Mark the streaming assistant message done and attach tool executions. */
  attachToolExec: (assistantId: string, exec: ToolExecution) => void;
  /** Timeline mode: insert a running tool placeholder at its stream position. */
  pushToolStart: (name: string, args: Record<string, unknown>) => string;
  /** Timeline mode: fill a tool placeholder with its execution result. */
  completeTool: (id: string, exec: ToolExecution) => void;
  finalizeAssistant: (id: string) => void;
  setError: (err: TurnError | null) => void;
  setRunning: (b: boolean) => void;
  setTurnContext: (data: TurnContextData) => void;
  /** Append a message to the per-session queue (sent while a turn runs). */
  enqueueInput: (text: string) => void;
  /** Pop the next queued message (FIFO). Returns undefined when empty. */
  shiftPendingInput: () => string | undefined;
  /** Discard all queued messages. */
  clearPendingInputs: () => void;
  /** Replace the queued message at `index` with `text`. */
  editPendingInput: (index: number, text: string) => void;
  /** Remove the queued message at `index`. */
  removePendingInput: (index: number) => void;
  /** Surface a permission prompt; returns a promise the modal resolves. */
  requestPermission: (info: PermissionRequiredInfo) => Promise<PermissionDecision>;
  resolvePermission: (decision: PermissionDecision) => void;
  /** Push a subagent permission prompt (from trace SSE). */
  pushSubagentPermission: (approval: StructuredApproval) => void;
  /** Dismiss the current subagent prompt (after the hook has resolved it). */
  clearSubagentPermission: () => void;
  /** Push an ask_user_question prompt (from trace SSE). */
  pushQuestion: (q: QuestionPayload) => void;
  /** Dismiss the current question prompt. */
  clearQuestion: () => void;
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
    pendingQuestion: null,
    turnContext: null,

    pendingInputs: [],
    setConnection: (s) => set({ connection: s }),
    setModelName: (n) => set({ modelName: n }),

    pushUserMessage: (text) =>
      set((s) => ({ messages: [...s.messages, { id: genId(), role: "user", content: text }] })),

    popUserMessage: () =>
      set((s) => {
        const msgs = [...s.messages];
        for (let i = msgs.length - 1; i >= 0; i--) {
          if (msgs[i].role === "user") {
            msgs.splice(i, 1);
            break;
          }
        }
        return { messages: msgs };
      }),

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

    pushToolStart: (name, args) => {
      const id = genId();
      set((s) => ({
        messages: [
          ...s.messages,
          { id, role: "tool", content: "", streaming: true, toolName: name, toolArgs: args },
        ],
      }));
      return id;
    },

    completeTool: (id, exec) =>
      set((s) => ({
        messages: s.messages.map((m) =>
          m.id === id ? { ...m, streaming: false, toolExec: exec } : m,
        ),
      })),

    finalizeAssistant: (id) =>
      set((s) => ({
        messages: s.messages.map((m) => (m.id === id ? { ...m, streaming: false } : m)),
      })),

    setError: (msg) => set({ lastError: msg }),
    setTurnContext: (data) => set({ turnContext: data }),
    setRunning: (b) => set({ isRunning: b }),

    enqueueInput: (text) => set((s) => ({ pendingInputs: [...s.pendingInputs, text] })),
    shiftPendingInput: () => {
      const list = get().pendingInputs;
      let i = 0;
      while (i < list.length && list[i].trim() === "") i += 1;
      if (i >= list.length) {
        if (list.length > 0) set({ pendingInputs: [] });
        return undefined;
      }
      set({ pendingInputs: list.slice(i + 1) });
      return list[i];
    },
    clearPendingInputs: () => set({ pendingInputs: [] }),
    editPendingInput: (index, text) =>
      set((s) => {
        if (index < 0 || index >= s.pendingInputs.length) return {};
        const next = [...s.pendingInputs];
        next[index] = text;
        return { pendingInputs: next };
      }),
    removePendingInput: (index) =>
      set((s) => ({ pendingInputs: s.pendingInputs.filter((_, i) => i !== index) })),

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

    pushQuestion: (q) => {
      if (!get().pendingQuestion) set({ pendingQuestion: q });
    },
    clearQuestion: () => set({ pendingQuestion: null }),

    registerAbort: (controller) => {
      currentAbort = controller;
    },

    stopRunning: () => {
      if (currentAbort) {
        currentAbort.abort();
        currentAbort = null;
      }
      // Clear running immediately: the abort unwinds the loop's fetches
      // asynchronously (or may not reach a wedged fetch at all), and the
      // composer gates sends on isRunning — leaving it set makes the Stop
      // button look dead and queues every later message forever. The loop's
      // finally writes the same value again; the double write is idempotent.
      set({ isRunning: false });
    },

    clear: () =>
      set({
        messages: [],
        lastError: null,
        pendingPermission: null,
        pendingSubagent: null,
        pendingQuestion: null,
        isRunning: false,
        pendingInputs: [],
      }),
  }));
}

export type SessionStore = ReturnType<typeof createSessionStore>;
