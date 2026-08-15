/**
 * Runs one agent turn for a session as a SERVER-SIDE observer (Change 2 of the
 * server-side agent-loop design). The daemon owns the loop (LLM calls + tool
 * execution + persistence); we POST /run, then subscribe to the SSE event
 * stream and mirror SessionEvents into the session store for rendering.
 *
 * Replaces the old client-side runAgentLoop driver. Closing the browser no
 * longer kills the turn — the daemon keeps running; reconnect on return.
 *
 * This is THE send entry point — App and any future session UI call
 * `runSessionTurn` and nothing else. Module-level (not a component closure):
 * it only touches the session's store and the passed-in client.
 */
import type { DaemonClient } from "../api/client";
import { toast } from "sonner";
import { useSessionManager } from "../state/sessionManager";
import { useDisplayPrefs, type DisplayMode } from "../state/displayPrefs";
import type { SessionStore } from "../state/sessionStore";
import type { SessionEvent, SessionEventKind } from "../api/types";
import type { ToolExecution } from "./types";

/** Pending tool invocation (started but not yet resulted). */
interface PendingTool {
  name: string;
  args: Record<string, unknown>;
  /** Timeline mode: the store message id of the running placeholder. */
  msgId?: string;
}

/** Mutable per-turn render state shared with handleEvent. */
interface RenderCtx {
  mode: DisplayMode;
  /** id of the current assistant bubble (null until the first text arrives). */
  assistantId: string | null;
  /** Current LLM round number (1-based). */
  round: number;
  /** Set when a turn_done(finish_reason=tool_calls) ends an LLM round; the
   *  next text then opens a new bubble with the incremented round. */
  boundary: boolean;
  pendingTools: PendingTool[];
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
      const created = await client.createSession({ name: entry.name });
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
  store.getState().setError(null);
  store.getState().setRunning(true);
  m.setStatus(sessionId, "running");
  m.setPreview(sessionId, "");

  // AbortController lets the Stop button cancel the SSE reader; the actual
  // turn cancellation is POST /cancel (see stopSessionTurn below).
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  // Render state for this turn. `mode` is snapshotted at send time so a
  // mid-turn toggle doesn't scramble an in-flight turn's layout.
  const ctx: RenderCtx = {
    mode: useDisplayPrefs.getState().mode,
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
    // 3. Subscribe BEFORE starting the run. Without `after=` the events
    //    stream is live-only, so subscribing after POST /run can miss the
    //    whole turn — a fast turn finishes in the gap, and the reader would
    //    then wait forever for a turn_done that already fired (the "sent a
    //    message, nothing happens" race). The subscribe itself has a 15s
    //    connect watchdog inside the client; a wedged connection surfaces
    //    as a transport error instead of an eternal hang.
    let events = await client.sessionEvents(daemonId, undefined, abort.signal);
    const { run_id: runId } = await client.runSession(daemonId, text, abort.signal);

    // 4. Read the event stream, mirroring events into the store. Reconnect
    //    with `after=<lastSeq>` when the stream drops mid-turn: the daemon
    //    replays missed events from its per-session buffer (or sends
    //    sync_lost), so a dropped connection no longer leaves the UI stuck
    //    in "running" while the daemon keeps working.
    let lastSeq = 0;
    let turnFinished = false;
    // Consecutive reconnects that closed without a single event — a broken
    // endpoint gets backoff, then a surfaced error instead of a silent spin.
    let eventlessDrops = 0;

    while (!turnFinished) {
      const reader = events.body.getReader();
      let buffer = "";
      let received = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          if (!value) continue;
          buffer += new TextDecoder().decode(value);
          let nl: number;
          while ((nl = buffer.indexOf("\n")) !== -1) {
            const line = buffer.slice(0, nl).trim();
            buffer = buffer.slice(nl + 1);
            if (!line || line.startsWith(":")) continue; // skip SSE comments/keepalives
            const payload = line.startsWith("data: ") ? line.slice(6) : line;
            let ev: SessionEvent;
            try {
              ev = JSON.parse(payload) as SessionEvent;
            } catch {
              continue;
            }
            // Control plane: the replay window no longer covers our cursor.
            // Realign to the daemon's latest seq and keep reading (mirrors
            // the TUI's sync_lost handling).
            if (ev.kind === "sync_lost") {
              const latest = Number(ev.data.latest_seq ?? 0);
              if (latest > lastSeq) lastSeq = latest;
              continue;
            }
            if (ev.run_id !== runId) continue; // stale: an earlier run's events
            if (ev.seq <= lastSeq) continue; // replay-seam duplicate
            lastSeq = ev.seq;
            received += 1;

            handleEvent(ev, store, sessionId, ctx);
            // A turn_done with finish_reason "tool_calls" only ends one LLM
            // round — tool_start/tool_result and further rounds still follow.
            const roundBoundary =
              ev.kind === "turn_done" && ev.data.finish_reason === "tool_calls";
            if ((ev.kind === "turn_done" || ev.kind === "turn_error") && !roundBoundary) {
              turnFinished = true;
            }
          }
          // The daemon keeps the per-session stream open after the turn ends
          // (15s keepalives) — without this break the reader would block on
          // the next read forever and the UI would stay "running" for good.
          if (turnFinished) break;
        }
      } catch (readErr) {
        // User stop unwinds to the outer catch; any other read failure
        // (connection reset mid-turn, daemon restart) falls through to the
        // resubscribe below instead of killing the turn's UI.
        if (abort.signal.aborted) throw readErr;
      } finally {
        reader.cancel().catch(() => {});
      }

      if (turnFinished) break;

      // The stream ended or failed before the turn finished (daemon restart,
      // proxy timeout, network reset). Resubscribe with `after=` and replay
      // what was missed instead of silently stalling.
      eventlessDrops = received === 0 ? eventlessDrops + 1 : 0;
      if (eventlessDrops > 5) {
        throw new Error("session event stream keeps closing without events");
      }
      if (eventlessDrops > 0) {
        await new Promise((resolve) =>
          setTimeout(resolve, Math.min(1000 * 2 ** (eventlessDrops - 1), 15_000)),
        );
      }
      events = await client.sessionEvents(daemonId, lastSeq, abort.signal);
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
    if (m.entries[sessionId]?.store.getState().lastError === null) {
      m.setStatus(sessionId, "idle");
    }
    // Drain the next queued message only on a clean finish — not on error or
    // explicit stop. Mirrors the TUI's pending_inputs / start_next_turn.
    if (outcome === "ok") {
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
  m.setStatus(sessionId, "running");
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  const ctx: RenderCtx = {
    mode: useDisplayPrefs.getState().mode,
    assistantId: null,
    round: 1,
    boundary: false,
    pendingTools: [],
  };

  try {
    let events = await client.sessionEvents(daemonSessionId, undefined, abort.signal);
    let lastSeq = 0;
    /** The run we attached to — taken from the first event seen; later events
     *  from any other run id are ignored. */
    let runId: string | null = null;
    let turnFinished = false;
    let received = 0;
    // Idle guard: the daemon broadcasts task_group_result right before it
    // spawns the continuation, so events should arrive almost immediately.
    // If none do (the run finished in the attach gap, or the broadcast was
    // for another client's claim), don't hold "running" forever — the turn
    // is persisted and shows up via the normal history load.
    const idleTimer = setTimeout(() => {
      if (received === 0) abort.abort();
    }, 20_000);

    try {
      while (!turnFinished) {
        const reader = events.body.getReader();
        let buffer = "";
        try {
          for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            if (!value) continue;
            buffer += new TextDecoder().decode(value);
            let nl: number;
            while ((nl = buffer.indexOf("\n")) !== -1) {
              const line = buffer.slice(0, nl).trim();
              buffer = buffer.slice(nl + 1);
              if (!line || line.startsWith(":")) continue;
              const payload = line.startsWith("data: ") ? line.slice(6) : line;
              let ev: SessionEvent;
              try {
                ev = JSON.parse(payload) as SessionEvent;
              } catch {
                continue;
              }
              if (ev.kind === "sync_lost") {
                const latest = Number(ev.data.latest_seq ?? 0);
                if (latest > lastSeq) lastSeq = latest;
                continue;
              }
              if (runId === null) runId = ev.run_id;
              if (ev.run_id !== runId) continue;
              if (ev.seq <= lastSeq) continue;
              lastSeq = ev.seq;
              received += 1;

              handleEvent(ev, store, sessionId, ctx);
              const roundBoundary =
                ev.kind === "turn_done" && ev.data.finish_reason === "tool_calls";
              if ((ev.kind === "turn_done" || ev.kind === "turn_error") && !roundBoundary) {
                turnFinished = true;
              }
            }
            if (turnFinished) break;
          }
        } catch (readErr) {
          if (abort.signal.aborted) throw readErr;
        } finally {
          reader.cancel().catch(() => {});
        }

        if (turnFinished) break;
        // Dropped mid-turn: resume from the last seen seq (same contract as
        // runSessionTurn).
        events = await client.sessionEvents(daemonSessionId, lastSeq, abort.signal);
      }
    } finally {
      clearTimeout(idleTimer);
    }
  } catch {
    // User stop, idle-guard abort, or transport failure: exit quietly. An
    // observed run is daemon-owned — its lifecycle doesn't depend on us.
  } finally {
    observingRuns.delete(sessionId);
    store.getState().registerAbort(null);
    if (ctx.assistantId) store.getState().finalizeAssistant(ctx.assistantId);
    store.getState().setRunning(false);
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
 * Ensure a text target bubble exists, splitting LLM rounds in rounds/timeline
 * mode: after a turn_done(tool_calls) boundary, the first text of the next
 * round closes the previous bubble and opens a new one with round+1.
 */
function openBubbleForText(ctx: RenderCtx, store: SessionStore): void {
  if (ctx.mode !== "single" && ctx.boundary) {
    ctx.boundary = false;
    ctx.round += 1;
    if (ctx.assistantId) store.getState().finalizeAssistant(ctx.assistantId);
    ctx.assistantId = store.getState().beginAssistantRound(ctx.round);
  } else if (!ctx.assistantId) {
    ctx.assistantId = store.getState().beginAssistantRound(ctx.round);
  }
}

/** Map a SessionEvent to store mutations (the rendering contract). */
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
      openBubbleForText(ctx, store);
      s.appendAssistant(ctx.assistantId!, { type: "contentDelta", text });
      useSessionManager.getState().setPreview(sessionId, text);
      break;
    }
    case "reasoning_delta": {
      const text = String(ev.data.text ?? "");
      openBubbleForText(ctx, store);
      s.appendAssistant(ctx.assistantId!, { type: "reasoningDelta", text });
      break;
    }
    case "tool_start": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      if (ctx.mode === "timeline") {
        // Timeline mode: the placeholder appears at its stream position so the
        // user sees the call start (running card) before the result arrives.
        const msgId = store.getState().pushToolStart(name, args);
        ctx.pendingTools.push({ name, args, msgId });
      } else {
        ctx.pendingTools.push({ name, args });
      }
      break;
    }
    case "tool_result": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      const content = String(ev.data.content ?? "");
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
      if (ctx.mode === "timeline") {
        if (pending?.msgId) store.getState().completeTool(pending.msgId, exec);
      } else {
        if (!ctx.assistantId) ctx.assistantId = store.getState().beginAssistantRound(ctx.round);
        store.getState().attachToolExec(ctx.assistantId, exec);
      }
      break;
    }
    case "turn_done":
      // finish_reason tool_calls only ends one LLM round — the following
      // tool_start/tool_result belong to the just-ended round, and the next
      // content_delta opens a new round bubble.
      if (ev.data.finish_reason === "tool_calls") ctx.boundary = true;
      break; // finalization handled by the finally block
    case "turn_error": {
      const message = String(ev.data.message ?? "turn failed");
      s.setError({ message, kind: "upstream" });
      useSessionManager.getState().setStatus(sessionId, "error");
      break;
    }
    case "turn_context": {
      // Inspector data for the completed turn — store for InspectorPanel.
      s.setTurnContext(ev.data as unknown as import("../state/sessionStore").TurnContextData);
      break;
    }
    case "save":
      break; // daemon persisted; nothing to do client-side
  }
}
