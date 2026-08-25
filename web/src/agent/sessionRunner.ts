/**
 * Runs one agent turn for a session as a SERVER-SIDE observer (Change 2 of the
 * server-side agent-loop design). The daemon owns the loop (LLM calls + tool
 * execution + persistence); we POST /run, then subscribe to the session's
 * event stream on the shared WebSocket push channel and mirror SessionEvents
 * into the session store for rendering.
 *
 * Replaces the old client-side runAgentLoop driver. Closing the browser no
 * longer kills the turn — the daemon keeps running; reconnect on return.
 *
 * This is THE send entry point — App and any future session UI call
 * `runSessionTurn` and nothing else. Module-level (not a component closure):
 * it only touches the session's store and the passed-in client.
 */
import type { DaemonClient } from "../api/client";
import { wsChannel } from "../api/wsChannel";
import { toast } from "sonner";
import { sessionTitleFromMessage, useSessionManager } from "../state/sessionManager";
import type { SessionStore } from "../state/sessionStore";
import type { SessionEvent, SessionEventKind, SessionRunStatus } from "../api/types";
import type { ToolExecution } from "./types";

/** Pending tool invocation (started but not yet resulted). */
interface PendingTool {
  name: string;
  args: Record<string, unknown>;
  /** Store message id of the running placeholder (pushed at tool_start). */
  msgId?: string;
}

/** Mutable per-turn render state shared with handleEvent. */
interface RenderCtx {
  /** id of the current assistant bubble (null until the first text arrives). */
  assistantId: string | null;
  /** Current LLM round number (1-based). */
  round: number;
  /** Set when a turn_done(finish_reason=tool_calls) ends an LLM round; the
   *  next text then opens a new bubble with the incremented round. */
  boundary: boolean;
  pendingTools: PendingTool[];
}

/** How a ws-backed turn observation ended. */
type TurnOutcome = "finished" | "aborted" | "idle" | "stalled";

/** turn_done/turn_error end the turn — except a tool_calls round boundary. */
function isTerminalEvent(ev: SessionEvent): boolean {
  const roundBoundary = ev.kind === "turn_done" && ev.data.finish_reason === "tool_calls";
  return (ev.kind === "turn_done" || ev.kind === "turn_error") && !roundBoundary;
}

/** After the terminal event, keep the subscription open this long waiting for
 * the daemon's turn_context (published AFTER turn_done, once the final save
 * completes — typically single-digit ms on local disk). Unsubscribing on
 * turn_done races the unsubscribe ahead of the turn_context and permanently
 * loses the turn-end snapshot; if the save still exceeds this window the live
 * usage_update values (identical measure) simply stay on screen. */
const TURN_CONTEXT_GRACE_MS = 500;

/** sync_lost reconciliation probe cadence. The first tick lands here after a
 * sync_lost (post-realign events would disarm the probe before it fires). */
const SYNC_LOST_RECONCILE_MS = 5_000;

/**
 * Observe one session's turn over the shared ws push channel and await its
 * end. The channel owns connection/reconnect and replays from its cursor on
 * reattach (sync_lost realign included), so this is a thin shell: filter to
 * the run, mirror events, resolve on the terminal event.
 *
 * Exported for unit tests (the subscribe/status/fetchRunState seams let the
 * state machine run without the singleton channel or a live daemon).
 *
 * - `acquireRunId` (runSessionTurn): events arriving before the POST /run
 *   response carries the run id are buffered and replayed once it is known —
 *   a fast turn must not finish invisibly in that gap. Resolving `null`
 *   (legacy daemon queued the message with an empty run_id) switches to
 *   adoption mode: no run filter, settle on the first terminal.
 * - Without it (observeDaemonRun): the run id is learned from the first event.
 * - `idleTimeoutMs`: resolve "idle" if not a single event arrived in time
 *   (observer attach gap — the run already finished; its history loads the
 *   normal way).
 * - `stallTimeoutMs`: surface "stalled" when the channel cannot hold ANY
 *   connection for this long mid-turn (daemon down) instead of hanging the
 *   UI in "running" forever — the ws successor of the SSE eventless-drop
 *   guard. A connected-but-quiet turn is NOT a stall (slow LLM is normal).
 * - `fetchRunState`: after a sync_lost (terminal may have fired inside the
 *   lost window) poll the run-state endpoint; settle "finished" once the
 *   awaited run is no longer active while the stream stays quiet.
 */
export function awaitTurnOverWs(opts: {
  daemonSessionId: string;
  abort: AbortSignal;
  onEvent: (ev: SessionEvent) => void;
  isTerminal: (ev: SessionEvent) => boolean;
  /** POST /run. Resolves `null` when a legacy daemon queued the message
   *  (empty `run_id`) — the run id is then adopted from the first event
   *  instead of filtering everything out. */
  acquireRunId?: () => Promise<string | null>;
  idleTimeoutMs?: number;
  stallTimeoutMs?: number;
  /** GET /sessions/:id/run — reconciliation probe armed after a sync_lost
   *  (the terminal event may have fired inside the lost window). */
  fetchRunState?: () => Promise<SessionRunStatus | null>;
  /** Test seams; default to the singleton ws channel. */
  subscribe?: (sessionId: string, handler: (ev: SessionEvent) => void) => { unsubscribe(): void };
  channelStatus?: () => string;
}): Promise<TurnOutcome> {
  const { daemonSessionId, abort, onEvent, isTerminal } = opts;
  return new Promise<TurnOutcome>((resolve, reject) => {
    let settled = false;
    let runId: string | null = null;
    let runIdKnown = false;
    let received = 0;
    const buffer: SessionEvent[] = [];

    const settle = (outcome: TurnOutcome) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(outcome);
    };
    const fail = (err: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(err);
    };

    let terminalSeen = false;
    let graceTimer: ReturnType<typeof setTimeout> | null = null;
    const finishGrace = () => {
      if (graceTimer !== null) clearTimeout(graceTimer);
      graceTimer = null;
      settle("finished");
    };

    // ── sync_lost reconciliation ──────────────────────────────────────────
    // sync_lost means the daemon's replay window no longer covers our cursor
    // (buffer eviction / restart): the awaited terminal event may already
    // have fired inside the lost window and will never arrive. While the
    // stream stays quiet, poll the run-state endpoint; once the awaited run
    // is no longer active (idle, or replaced by another run) settle
    // "finished" instead of hanging on "running" forever. Any live event
    // disarms the probe — normal terminal handling resumes.
    let reconcileTimer: ReturnType<typeof setInterval> | null = null;
    const armReconcile = () => {
      if (reconcileTimer !== null || !opts.fetchRunState) return;
      reconcileTimer = setInterval(() => {
        void (async () => {
          if (settled) return;
          let st: SessionRunStatus | null = null;
          try {
            st = await opts.fetchRunState!();
          } catch {
            return; // probe failed: keep waiting (stall watchdog still guards)
          }
          if (st === null) return;
          // Adoption mode (runId null): anything still active or queued
          // keeps the wait alive. Run-scoped mode: only OUR run does.
          const keepWaiting =
            runId !== null ? st.run_id === runId : st.run_id !== null || st.queued > 0;
          if (!keepWaiting) settle("finished");
        })();
      }, SYNC_LOST_RECONCILE_MS);
    };
    const disarmReconcile = () => {
      if (reconcileTimer !== null) {
        clearInterval(reconcileTimer);
        reconcileTimer = null;
      }
    };

    const process = (ev: SessionEvent): void => {
      if (settled) return; // post-terminal buffered events are not consumed
      if (ev.kind === "sync_lost") {
        armReconcile(); // the channel realigns its cursor on its own
        return;
      }
      disarmReconcile();
      if (!runIdKnown) {
        if (opts.acquireRunId) {
          buffer.push(ev); // pre-runId events: replayed once the id is known
          return;
        }
        runId = ev.run_id; // observer / queued-legacy mode: adopt the first run
        runIdKnown = true;
      }
      if (runId !== null && ev.run_id !== runId) return; // stale: an earlier run's events
      // The daemon publishes turn_context after turn_done/turn_error (final
      // save first), so a terminal event opens a short grace window: consume
      // ONLY that snapshot, then settle early. Any other trailing event is
      // dropped exactly like the pre-grace behavior.
      if (terminalSeen) {
        if (ev.kind === "turn_context") {
          received += 1;
          onEvent(ev);
          finishGrace();
        }
        return;
      }
      received += 1;
      onEvent(ev);
      if (isTerminal(ev)) {
        terminalSeen = true;
        graceTimer = setTimeout(finishGrace, TURN_CONTEXT_GRACE_MS);
      }
    };

    const subscribe =
      opts.subscribe ??
      ((sid: string, handler: (ev: SessionEvent) => void) =>
        wsChannel.subscribeSession(sid, handler));
    const sub = subscribe(daemonSessionId, process);

    const onAbort = () => settle(received === 0 && opts.idleTimeoutMs ? "idle" : "aborted");
    abort.addEventListener("abort", onAbort, { once: true });

    const idleTimer =
      opts.idleTimeoutMs !== undefined
        ? setTimeout(() => {
            if (received === 0) settle("idle");
          }, opts.idleTimeoutMs)
        : null;

    // Stall watchdog: accumulate time with the channel NOT open. A turn can
    // legitimately run quiet for minutes while connected; only a sustained
    // inability to hold any connection is a transport failure.
    const channelStatus = opts.channelStatus ?? (() => wsChannel.status());
    let closedMs = 0;
    const stallProbe =
      opts.stallTimeoutMs !== undefined
        ? setInterval(() => {
            closedMs = channelStatus() === "open" ? 0 : closedMs + 5_000;
            if (closedMs >= opts.stallTimeoutMs!) settle("stalled");
          }, 5_000)
        : null;

    function cleanup(): void {
      sub.unsubscribe();
      abort.removeEventListener("abort", onAbort);
      if (idleTimer !== null) clearTimeout(idleTimer);
      if (graceTimer !== null) clearTimeout(graceTimer);
      if (stallProbe !== null) clearInterval(stallProbe);
      disarmReconcile();
    }

    if (opts.acquireRunId) {
      const acquire = opts.acquireRunId;
      void (async () => {
        try {
          // `null` = legacy queued response (empty run_id): adoption mode.
          runId = await acquire();
          runIdKnown = true;
          for (const ev of buffer.splice(0)) process(ev);
        } catch (err) {
          fail(err);
        }
      })();
    }
  });
}

export async function runSessionTurn(
  client: DaemonClient,
  sessionId: string,
  text: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  const store = entry.store;

  // 1. Ensure we have a daemon-side session id (POST /run needs one).
  let daemonId = entry.daemonId;
  if (!daemonId) {
    try {
      // Placeholder names ("Session N") are not sent: an unnamed daemon
      // session gets auto-titled from its first user message instead of
      // keeping a placeholder (or UUID) forever.
      const created = await client.createSession({
        name: entry.named ? entry.name : undefined,
      });
      daemonId = created.id;
      m.setDaemonId(sessionId, daemonId);
    } catch (e) {
      store.getState().setError({
        message: e instanceof Error ? e.message : String(e),
        kind: "transport",
      });
      m.setStatus(sessionId, "error");
      return;
    }
  }

  // 2. Optimistic local render of the user message + running state.
  store.getState().pushUserMessage(text);
  // First user turn of this session? The daemon auto-titles unnamed sessions
  // from this message; the local tab mirrors it after a clean finish.
  const isFirstUserTurn = store.getState().messages.length === 1;
  store.getState().setError(null);
  store.getState().setRunning(true);
  // TUI-aligned phase: a turn starts in "thinking" (the daemon is assembling
  // the prompt / awaiting the first model chunk) — see AgentPhase::Thinking.
  store.getState().setAgentPhase({ phase: "thinking" });
  store.getState().setTurnStartedAt(Date.now());
  m.setStatus(sessionId, "running");
  m.setPreview(sessionId, "");

  // AbortController lets the Stop button cancel the SSE reader; the actual
  // turn cancellation is POST /cancel (see stopSessionTurn below).
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  // Render state for this turn.
  const ctx: RenderCtx = {
    assistantId: null,
    round: 1,
    boundary: false,
    pendingTools: [],
  };

  // Track how this turn ended so the finally block knows whether to drain the
  // queued-input FIFO. Only a clean finish auto-sends the next message — an
  // error or explicit Stop leaves the queue intact for the user to retry.
  let outcome: "ok" | "stopped" | "error" = "ok";

  try {
    // 3. Subscribe BEFORE starting the run. The ws session subscription is
    //    live-only until events flow, so subscribing after POST /run can miss
    //    the whole turn — a fast turn finishes in the gap, and the awaiter
    //    would then wait forever for a turn_done that already fired (the
    //    "sent a message, nothing happens" race). Events predating the POST
    //    response are buffered until the run id is known.
    // 4. Await the turn's end over the shared channel: the channel replays
    //    from its cursor when the connection drops mid-turn (the daemon's
    //    per-session buffer covers the gap), and the stall watchdog turns a
    //    sustained daemon outage into a transport error instead of an
    //    eternal "running" spinner.
    const outcomeWs = await awaitTurnOverWs({
      daemonSessionId: daemonId,
      abort: abort.signal,
      onEvent: (ev) => handleEvent(ev, store, sessionId, ctx),
      isTerminal: isTerminalEvent,
      acquireRunId: async () => {
        const resp = await client.runSession(daemonId, text, abort.signal);
        // Legacy daemon queued the message (empty run_id): adopt the id from
        // the first event rather than filtering every event out (the
        // pre-fix "sent a message, nothing happens" freeze). Current daemons
        // always return the pre-minted id, queued or not.
        return resp.run_id || null;
      },
      fetchRunState: () => client.getRunStatus(daemonId),
      stallTimeoutMs: 60_000,
    });
    if (outcomeWs === "stalled") {
      throw new Error("session event channel disconnected (daemon unreachable)");
    }
  } catch (err) {
    const isAbort =
      abort.signal.aborted ||
      (err instanceof DOMException && err.name === "AbortError") ||
      (err instanceof Error && err.message === "aborted");
    if (isAbort) {
      outcome = "stopped";
      // User hit stop — the reader/fetch was cancelled; the daemon turn may
      // still be running server-side. Status set by stopSessionTurn.
    } else {
      const msg = err instanceof Error ? err.message : String(err);
      outcome = "error";
      store.getState().setError({
        message: msg,
        kind: "transport",
        retry: () => runSessionTurn(client, sessionId, text),
      });
      m.setStatus(sessionId, "error");
      toast.error(`${entry.name}: connection lost`);
    }
  } finally {
    store.getState().registerAbort(null);
    if (ctx.assistantId) store.getState().finalizeAssistant(ctx.assistantId);
    store.getState().setRunning(false);
    store.getState().setAgentPhase(null);
    store.getState().setTurnStartedAt(null);
    if (m.entries[sessionId]?.store.getState().lastError === null) {
      m.setStatus(sessionId, "idle");
    }
    // Drain the next queued message only on a clean finish — not on error or
    // explicit stop. Mirrors the TUI's pending_inputs / start_next_turn.
    if (outcome === "ok") {
      const cur = m.entries[sessionId];
      if (isFirstUserTurn && cur && !cur.named) {
        m.renameSession(sessionId, sessionTitleFromMessage(text));
      }
      const next = store.getState().shiftPendingInput();
      if (next) void runSessionTurn(client, sessionId, next);
    }
  }
}

/** Cancel an active server-side turn (Stop button). */
export async function stopSessionTurn(client: DaemonClient, sessionId: string): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry?.daemonId) return;

  // Abort the SSE reader locally (so the fetch loop unwinds).
  entry.store.getState().stopRunning();

  // Tell the daemon to cancel the run.
  try {
    await client.cancelRun(entry.daemonId);
  } catch {
    // Best-effort; the daemon may have already finished.
  }
  m.setStatus(sessionId, "idle");
}

/** Local session ids with an observer already attached (dedup). */
const observingRuns = new Set<string>();

/**
 * Attach to a DAEMON-INITIATED run (e.g. the task-group synthesis continuation
 * the daemon's scheduler spawns when subagents finish) and mirror its events
 * into the store — web otherwise only renders runs it started itself, so
 * server-side continuations were invisible until a manual refresh.
 *
 * `daemonSessionId` is the daemon-side session id carried by the global
 * `task_group_result` event. No-op when the session is unknown here, already
 * has a locally-driven turn (that path renders its own events), or already
 * has an observer attached.
 */
export async function observeDaemonRun(
  client: DaemonClient,
  daemonSessionId: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = Object.values(m.entries).find((e) => e.daemonId === daemonSessionId);
  if (!entry) return;
  const sessionId = entry.id;
  const store = entry.store;
  if (store.getState().isRunning || observingRuns.has(sessionId)) return;
  observingRuns.add(sessionId);

  store.getState().setError(null);
  store.getState().setRunning(true);
  store.getState().setAgentPhase({ phase: "thinking" });
  store.getState().setTurnStartedAt(Date.now());
  m.setStatus(sessionId, "running");
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  const ctx: RenderCtx = {
    assistantId: null,
    round: 1,
    boundary: false,
    pendingTools: [],
  };

  try {
    // Idle guard: the daemon broadcasts task_group_result right before it
    // spawns the continuation, so events should arrive almost immediately.
    // If none do (the run finished in the attach gap, or the broadcast was
    // for another client's claim), resolve "idle" — don't hold "running"
    // forever; the turn is persisted and shows up via the normal history
    // load. The run id is adopted from the first event seen.
    await awaitTurnOverWs({
      daemonSessionId,
      abort: abort.signal,
      onEvent: (ev) => handleEvent(ev, store, sessionId, ctx),
      isTerminal: isTerminalEvent,
      idleTimeoutMs: 20_000,
      stallTimeoutMs: 60_000,
      // Reconcile a sync_lost that swallows the continuation's terminal
      // event: adoption mode keeps waiting while any run is active or any
      // message is queued, settles once the session goes idle.
      fetchRunState: () => client.getRunStatus(daemonSessionId),
    });
  } catch {
    // Transport failure while acquiring the run: exit quietly. An observed
    // run is daemon-owned — its lifecycle doesn't depend on us.
  } finally {
    observingRuns.delete(sessionId);
    store.getState().registerAbort(null);
    if (ctx.assistantId) store.getState().finalizeAssistant(ctx.assistantId);
    store.getState().setRunning(false);
    store.getState().setAgentPhase(null);
    store.getState().setTurnStartedAt(null);
    const mgr = useSessionManager.getState();
    if (mgr.entries[sessionId]?.store.getState().lastError === null) {
      mgr.setStatus(sessionId, "idle");
    }
    // A message queued while the observed run held "running" drains now.
    const next = store.getState().shiftPendingInput();
    if (next) void runSessionTurn(client, sessionId, next);
  }
}

/**
 * Ensure a text target bubble exists, splitting LLM rounds: after a
 * turn_done(tool_calls) boundary, the first text of the next round closes the
 * previous bubble and opens a new one with round+1.
 */
function openBubbleForText(ctx: RenderCtx, store: SessionStore): void {
  if (ctx.boundary) {
    ctx.boundary = false;
    ctx.round += 1;
    if (ctx.assistantId) store.getState().finalizeAssistant(ctx.assistantId);
    ctx.assistantId = store.getState().beginAssistantRound(ctx.round);
  } else if (!ctx.assistantId) {
    ctx.assistantId = store.getState().beginAssistantRound(ctx.round);
  }
}

/** Map a SessionEvent to store mutations (the rendering contract).
 *
 * Besides per-event rendering, each event advances the TUI-aligned
 * `agentPhase` (mirrors src/tui/components/status.rs transitions):
 * delta → streaming; turn_done(tool_calls) → preparing_tools;
 * tool_start → executing{name}; tool_result → thinking (next round);
 * turn_error → error. */
function handleEvent(
  ev: SessionEvent,
  store: SessionStore,
  sessionId: string,
  ctx: RenderCtx,
): void {
  const s = store.getState();
  switch (ev.kind as SessionEventKind) {
    case "content_delta": {
      const text = String(ev.data.text ?? "");
      s.setAgentPhase({ phase: "streaming" });
      openBubbleForText(ctx, store);
      s.appendAssistant(ctx.assistantId!, { type: "contentDelta", text });
      useSessionManager.getState().setPreview(sessionId, text);
      break;
    }
    case "reasoning_delta": {
      const text = String(ev.data.text ?? "");
      s.setAgentPhase({ phase: "streaming" });
      openBubbleForText(ctx, store);
      s.appendAssistant(ctx.assistantId!, { type: "reasoningDelta", text });
      break;
    }
    case "tool_start": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      s.setAgentPhase({ phase: "executing", toolName: name });
      // The placeholder appears at its stream position so the user sees the
      // call start (running card) before the result arrives.
      const msgId = store.getState().pushToolStart(name, args);
      ctx.pendingTools.push({ name, args, msgId });
      break;
    }
    case "tool_result": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      const content = String(ev.data.content ?? "");
      // Tools finished: the next LLM round starts (Thinking, per TUI).
      s.setAgentPhase({ phase: "thinking" });
      const pending = ctx.pendingTools.shift();
      const exec: ToolExecution = {
        call: {
          id: `server-${ev.seq}`,
          type: "function",
          function: {
            name: pending?.name ?? name,
            arguments: JSON.stringify(pending?.args ?? args),
          },
        },
        response: { success: !content.toLowerCase().startsWith("error"), content },
      };
      if (pending?.msgId) store.getState().completeTool(pending.msgId, exec);
      break;
    }
    case "turn_done": {
      const finishReason = String(ev.data.finish_reason ?? "");
      // finish_reason tool_calls only ends one LLM round — the following
      // tool_start/tool_result belong to the just-ended round, and the next
      // content_delta opens a new round bubble.
      if (finishReason === "tool_calls") {
        ctx.boundary = true;
        // Tools are about to execute (TUI PreparingTools).
        s.setAgentPhase({ phase: "preparing_tools" });
      } else if (finishReason === "length" || finishReason === "max_tokens") {
        // The provider hit the token budget — with reasoning models the
        // budget is often consumed by reasoning_content first, so the answer
        // can come back empty ("long reasoning, then nothing happens").
        // Surface it instead of ending the turn silently.
        toast.warning("Response truncated — max tokens reached", {
          description:
            "The token budget ran out (often from long reasoning). Resend, or raise max_tokens.",
        });
      }
      break; // finalization handled by the finally block
    }
    case "turn_error": {
      const message = String(ev.data.message ?? "turn failed");
      s.setError({ message, kind: "upstream" });
      s.setAgentPhase({ phase: "error" });
      useSessionManager.getState().setStatus(sessionId, "error");
      break;
    }
    case "turn_context": {
      // Inspector data for the completed turn — store for InspectorPanel.
      s.setTurnContext(ev.data as unknown as import("../state/sessionStore").TurnContextData);
      break;
    }
    case "usage_update": {
      // Live context-occupancy feed (prompt tokens of the last LLM call).
      const promptTokens = Number(ev.data.prompt_tokens);
      if (Number.isFinite(promptTokens) && promptTokens >= 0) {
        s.setContextTokens(promptTokens);
      }
      break;
    }
    case "save":
      break; // daemon persisted; nothing to do client-side
    case "phase_changed": {
      // Daemon-truth phase (v2 status contract): authoritative when present.
      // The local derivation above stays as the fallback for older daemons
      // that never send this event; values it can already derive
      // (preparing_tools) simply agree.
      const phase = String(ev.data.phase ?? "");
      if (
        phase === "thinking" ||
        phase === "connecting" ||
        phase === "preparing_tools" ||
        phase === "compacting"
      ) {
        const attempt = Number(ev.data.attempt);
        const maxRetries = Number(ev.data.max_retries);
        s.setAgentPhase({
          phase,
          ...(phase === "connecting" && Number.isFinite(attempt)
            ? {
                attempt,
                maxRetries: Number.isFinite(maxRetries) ? maxRetries : undefined,
              }
            : {}),
        });
      }
      break;
    }
  }
}
