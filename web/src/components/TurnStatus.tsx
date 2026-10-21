import { useEffect, useState, useSyncExternalStore } from "react";
import { useSessionManager } from "../state/sessionManager";
import { deriveTurnVisual, formatDuration, phaseLabel, SPINNER, SPINNER_MS } from "./statusPhase";

/**
 * Turn-status strip — the phase segment ONLY (spinner + label + elapsed,
 * awaiting-decision / error variants), pinned directly above the input box
 * where the user's eyes already are. The persistent facts (connection,
 * workspace root, context bar, permission mode, model) stay in the bottom
 * StatusBar; this strip carries the live turn state, mirrored by the
 * Composer's tinted frame. Renders the same labels as
 * src/tui/components/status.rs so both frontends read identically.
 */
export function TurnStatus() {
  const activeStore = useSessionManager((s) =>
    s.activeId ? (s.entries[s.activeId]?.store ?? null) : null,
  );
  const activeStatus = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.status : undefined,
  );
  const isRunning = activeStatus === "running" || activeStatus === "awaiting_approval";

  const agentPhase = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().agentPhase ?? null,
  );
  const hasQuestion = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().pendingQuestion !== null,
  );
  const turnStartedAt = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().turnStartedAt ?? null,
  );
  // Error persists until the next send (lastError semantics) — NOT the
  // agentPhase, which the runner clears in its finally block.
  const hasError = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().lastError !== null,
  );

  // Priority: awaiting decision > error > busy (per-phase hue) > idle.
  const awaitingDecision = activeStatus === "awaiting_approval";
  const visual = deriveTurnVisual({
    awaitingDecision,
    hasError,
    isRunning,
    agentPhase,
  });
  const busy = visual.kind === "busy" && agentPhase !== null;

  // Spinner frames + elapsed seconds advance on one 100ms tick while a turn
  // is busy (spinner needs the cadence; elapsed just rides along). Both are
  // computed in the effect — render stays pure.
  const [tick, setTick] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (!busy || turnStartedAt === null) return;
    const id = setInterval(() => {
      setTick((t) => (t + 1) % 1_000_000);
      setElapsed(Math.max(0, Math.floor((Date.now() - turnStartedAt) / 1000)));
    }, SPINNER_MS);
    return () => clearInterval(id);
  }, [busy, turnStartedAt]);

  return (
    <div
      className="flex h-6 shrink-0 items-center gap-2 border-t border-border bg-background px-3 text-[11px] text-muted-foreground"
      role="status"
      aria-label="Turn status"
    >
      {awaitingDecision ? (
        <span className={visual.text}>{hasQuestion ? "Question" : "Permission required"}</span>
      ) : visual.kind === "error" ? (
        <span className={visual.text}>✗ Error</span>
      ) : busy && agentPhase ? (
        <span className={`flex items-center gap-1 ${visual.text}`}>
          <span aria-hidden className="inline-block w-3 text-center">
            {SPINNER[tick % SPINNER.length]}
          </span>
          {phaseLabel(agentPhase)}
          {turnStartedAt !== null && (
            <span className="text-muted-foreground">({formatDuration(elapsed)})</span>
          )}
        </span>
      ) : (
        <span className={visual.text}>● Ready</span>
      )}
    </div>
  );
}

const noopSubscribe = () => () => {};
