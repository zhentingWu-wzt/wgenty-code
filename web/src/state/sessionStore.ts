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
  /** Tool-role entries: the invoked tool + args while running. */
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
  /** Context-window occupancy at the last API call (newer daemons; the
   *  TUI context bar renders the same measure). */
  context_tokens?: number;
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

/** Fine-grained turn phase — the subset of the TUI's AgentPhase
 *  (src/state/agent_phase.rs) the status bar renders. `connecting` and
 *  `compacting` only ever arrive via daemon `phase_changed` events (older
 *  daemons never send them); the rest are derived from the event stream by
 *  sessionRunner. The StatusBar renders the same labels as
 *  src/tui/components/status.rs so both frontends read identically. */
export type AgentPhase =
  | "thinking"
  | "connecting"
  | "streaming"
  | "preparing_tools"
  | "executing"
  | "compacting"
  | "error";

export interface AgentPhaseInfo {
  phase: AgentPhase;
  /** Tool name when phase === "executing". */
  toolName?: string;
  /** Retry position when phase === "connecting" (daemon truth only). */
  attempt?: number;
  maxRetries?: number;
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
  /** Real-time context-window occupancy (prompt tokens of the last LLM call),
   * updated live by `usage_update` events mid-turn and by the turn-end
   * `turn_context` snapshot. Null until the first update arrives. */
  contextTokens: number | null;
  /** Fine-grained turn phase (TUI-aligned), derived from SessionEvents by
   *  sessionRunner. Null when no turn is active. */
  agentPhase: AgentPhaseInfo | null;
  /** Wall-clock ms when the current turn started (StatusBar elapsed timer). */
  turnStartedAt: number | null;

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
  /** Insert a running tool placeholder at its stream position. */
  pushToolStart: (name: string, args: Record<string, unknown>) => string;
  /** Fill a tool placeholder with its execution result. */
  completeTool: (id: string, exec: ToolExecution) => void;
  finalizeAssistant: (id: string) => void;
  setError: (err: TurnError | null) => void;
  setRunning: (b: boolean) => void;
  setTurnContext: (data: TurnContextData) => void;
  /** Live context-occupancy setter (usage_update events). */
  setContextTokens: (n: number) => void;
  /** Turn phase setter (sessionRunner derives from SessionEvents). */
  setAgentPhase: (p: AgentPhaseInfo | null) => void;
  /** Turn start timestamp setter (paired with agentPhase). */
  setTurnStartedAt: (t: number | null) => void;
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
  // ── Streamed-delta batching (design D3) ─────────────────────────────────
  // `appendAssistant` fires once per provider chunk (often dozens per second
  // during a reasoning-heavy turn). Committing each one immediately re-renders
  // the whole message list per token batch — with ReactMarkdown + shiki in the
  // path that is O(total_content) work per event and froze the UI on long
  // streams. Instead, deltas accumulate here and are committed in one `set`
  // per animation frame. Any other message mutation flushes synchronously
  // first so stream order is preserved; `clear` discards the buffer.
  let pendingDeltas = new Map<string, { content: string; reasoning: string }>();
  let flushHandle: number | null = null;

  return create<SessionState>((set, get) => {
    const flushDeltas = (): void => {
      if (pendingDeltas.size === 0) return;
      const deltas = pendingDeltas;
      pendingDeltas = new Map();
      set((s) => ({
        messages: s.messages.map((m) => {
          const d = deltas.get(m.id);
          // Unknown ids (message removed meanwhile) drop their deltas silently.
          if (!d) return m;
          return {
            ...m,
            content: m.content + d.content,
            reasoning: d.reasoning ? (m.reasoning ?? "") + d.reasoning : m.reasoning,
          };
        }),
      }));
    };

    const scheduleFlush = (): void => {
      if (flushHandle !== null) return;
      if (typeof requestAnimationFrame === "function") {
        flushHandle = requestAnimationFrame(() => {
          flushHandle = null;
          flushDeltas();
        });
      } else {
        // Environments without rAF (non-DOM tests) — a short timeout keeps the
        // same batching semantics deterministically.
        flushHandle = setTimeout(() => {
          flushHandle = null;
          flushDeltas();
        }, 32) as unknown as number;
      }
    };

    return {
      messages: [],
      isRunning: false,
      lastError: null,
      connection: "unknown",
      modelName: null,
      pendingPermission: null,
      pendingSubagent: null,
      pendingQuestion: null,
      turnContext: null,
      contextTokens: null,
      agentPhase: null,
      turnStartedAt: null,

      pendingInputs: [],
      setConnection: (s) => set({ connection: s }),
      setModelName: (n) => set({ modelName: n }),

      pushUserMessage: (text) => {
        flushDeltas();
        set((s) => ({ messages: [...s.messages, { id: genId(), role: "user", content: text }] }));
      },

      popUserMessage: () => {
        flushDeltas();
        set((s) => {
          const msgs = [...s.messages];
          for (let i = msgs.length - 1; i >= 0; i--) {
            if (msgs[i].role === "user") {
              msgs.splice(i, 1);
              break;
            }
          }
          return { messages: msgs };
        });
      },

      pushLoadedMessage: (m) => {
        flushDeltas();
        set((s) => ({ messages: [...s.messages, m] }));
      },

      beginAssistantRound: (round) => {
        flushDeltas();
        const id = genId();
        set((s) => ({
          messages: [...s.messages, { id, role: "assistant", content: "", round, streaming: true }],
        }));
        return id;
      },

      appendAssistant: (id, ev) => {
        if (ev.type !== "contentDelta" && ev.type !== "reasoningDelta") return;
        const cur = pendingDeltas.get(id) ?? { content: "", reasoning: "" };
        if (ev.type === "contentDelta") cur.content += ev.text;
        else cur.reasoning += ev.text;
        pendingDeltas.set(id, cur);
        scheduleFlush();
      },

      pushToolStart: (name, args) => {
        flushDeltas();
        const id = genId();
        set((s) => ({
          messages: [
            ...s.messages,
            { id, role: "tool", content: "", streaming: true, toolName: name, toolArgs: args },
          ],
        }));
        return id;
      },

      completeTool: (id, exec) => {
        flushDeltas();
        set((s) => ({
          messages: s.messages.map((m) =>
            m.id === id ? { ...m, streaming: false, toolExec: exec } : m,
          ),
        }));
      },

      finalizeAssistant: (id) => {
        // Synchronous flush so the finalized bubble (and turn-end assertions)
        // sees the complete text immediately — the scheduled rAF may still be
        // pending (its later firing no-ops on the empty buffer).
        flushDeltas();
        set((s) => ({
          messages: s.messages.map((m) => (m.id === id ? { ...m, streaming: false } : m)),
        }));
      },

      setError: (msg) => set({ lastError: msg }),
      setTurnContext: (data) =>
        set({
          turnContext: data,
          // The turn-end snapshot is authoritative for the same measure the
          // live usage_update events carry.
          contextTokens: data.usage.context_tokens ?? get().contextTokens,
        }),
      setContextTokens: (n) => set({ contextTokens: n }),
      setAgentPhase: (p) => {
        // Delta handlers re-set "streaming" on every chunk; skip identical
        // values so phase subscribers don't re-render per token batch.
        if (p) {
          const cur = get().agentPhase;
          if (
            cur &&
            cur.phase === p.phase &&
            cur.toolName === p.toolName &&
            cur.attempt === p.attempt &&
            cur.maxRetries === p.maxRetries
          ) {
            return;
          }
        }
        set({ agentPhase: p });
      },
      setTurnStartedAt: (t) => set({ turnStartedAt: t }),
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
        set({ isRunning: false, agentPhase: null, turnStartedAt: null });
      },

      clear: () => {
        // Drop uncommitted deltas entirely — a cleared session must not receive
        // text from the turn it just left behind.
        if (flushHandle !== null) {
          if (typeof requestAnimationFrame === "function") cancelAnimationFrame(flushHandle);
          else clearTimeout(flushHandle as unknown as ReturnType<typeof setTimeout>);
        }
        flushHandle = null;
        pendingDeltas = new Map();
        set({
          messages: [],
          lastError: null,
          pendingPermission: null,
          pendingSubagent: null,
          pendingQuestion: null,
          isRunning: false,
          pendingInputs: [],
          agentPhase: null,
          turnStartedAt: null,
        });
      },
    };
  });
}

export type SessionStore = ReturnType<typeof createSessionStore>;
