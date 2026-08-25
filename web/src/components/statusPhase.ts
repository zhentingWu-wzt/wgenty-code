/**
 * TUI-aligned turn-phase helpers for the StatusBar (mirrors
 * src/tui/components/status.rs): the same braille spinner, phase labels,
 * and duration formatting, so both frontends read identically.
 */
import type { AgentPhase, AgentPhaseInfo } from "../state/sessionStore";

/** Same braille frames as the TUI SPINNER. */
export const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/** Same 100ms cadence as the TUI spinner frame clock. */
export const SPINNER_MS = 100;

/** Phase label — word-for-word the TUI's `phase_label` for the same states
 *  (Connecting / Thinking / PreparingTools / Streaming / ExecutingTool /
 *  Compacting / Errored). */
export function phaseLabel(p: AgentPhaseInfo): string {
  switch (p.phase) {
    case "thinking":
      return "Thinking…";
    case "connecting":
      // TUI parity: retry info only when there is more than one attempt.
      return p.maxRetries !== undefined && p.maxRetries > 1
        ? `Connecting (attempt ${p.attempt ?? 1}/${p.maxRetries})…`
        : "Connecting…";
    case "streaming":
      return "Streaming…";
    case "preparing_tools":
      return "Preparing tools…";
    case "compacting":
      return "Compacting…";
    case "error":
      return "Error";
    case "executing":
      return p.toolName === "task" || p.toolName === "delegate"
        ? "Subagent running…"
        : `Executing ${p.toolName}…`;
  }
}

/** Elapsed formatting — same shape as the TUI's `format_duration`
 *  ("42s" → "1m30s"). */
export function formatDuration(secs: number): string {
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m${secs % 60}s`;
}

// ── Turn visual state (single source of truth) ──────────────────────────────

/** Visual treatment of one turn state: label color + input-frame classes. */
export interface TurnVisual {
  /** Which bucket won the priority chain (tests / a11y). */
  kind: "awaiting" | "error" | "busy" | "idle";
  /** Tailwind `text-*` class for the TurnStatus label. */
  text: string;
  /** Tailwind border/ring classes for the Composer input frame. */
  frame: string;
}

/** Per-PHASE colors (every busy phase reads differently at a glance):
 *  thinking sky · connecting orange · streaming brand blue · preparing violet
 *  · executing teal · compacting fuchsia. States outside a running turn:
 *  awaiting warning amber · error danger red · idle muted gray. */
const PHASE_TEXT: Record<AgentPhase, string> = {
  thinking: "text-sky-500 dark:text-sky-400",
  connecting: "text-orange-500 dark:text-orange-400",
  streaming: "text-primary",
  preparing_tools: "text-violet-500 dark:text-violet-400",
  executing: "text-teal-500 dark:text-teal-400",
  compacting: "text-fuchsia-500 dark:text-fuchsia-400",
  error: "text-danger",
};

const IDLE_VISUAL: TurnVisual = {
  kind: "idle",
  text: "text-muted-foreground",
  frame: "border-input focus-within:ring-ring",
};
const RUNNING_VISUAL: TurnVisual = {
  kind: "busy",
  text: "text-primary",
  frame: "border-primary/60 focus-within:ring-primary",
};
const AWAITING_VISUAL: TurnVisual = {
  kind: "awaiting",
  text: "text-warning",
  frame: "border-warning focus-within:ring-warning",
};
const ERROR_VISUAL: TurnVisual = {
  kind: "error",
  text: "text-danger",
  frame: "border-danger focus-within:ring-danger",
};

/** Frame classes matching a phase's hue (input border follows the phase). */
export function phaseFrame(p: AgentPhaseInfo): string {
  switch (p.phase) {
    case "thinking":
      return "border-sky-500/60 focus-within:ring-sky-500";
    case "connecting":
      return "border-orange-500/60 focus-within:ring-orange-500";
    case "streaming":
      return "border-primary/60 focus-within:ring-primary";
    case "preparing_tools":
      return "border-violet-500/60 focus-within:ring-violet-500";
    case "executing":
      return "border-teal-500/60 focus-within:ring-teal-500";
    case "compacting":
      return "border-fuchsia-500/60 focus-within:ring-fuchsia-500";
    case "error":
      return "border-danger focus-within:ring-danger";
  }
}

export interface TurnVisualInput {
  /** A permission/question decision is pending (any prompt source). */
  awaitingDecision: boolean;
  /** The last turn errored; persists until the next send (lastError). */
  hasError: boolean;
  /** A turn is in flight. */
  isRunning: boolean;
  /** Event-derived phase; null when running but no phase event arrived yet. */
  agentPhase: AgentPhaseInfo | null;
}

/** Single source of truth for turn colors — TurnStatus (label) and Composer
 *  (input frame) both consume this, so the frame always follows the strip.
 *  Priority: awaiting decision > error > busy (per-phase hue, brand blue when
 *  the phase is not yet known) > idle. */
export function deriveTurnVisual(i: TurnVisualInput): TurnVisual {
  if (i.awaitingDecision) return AWAITING_VISUAL;
  if (i.hasError) return ERROR_VISUAL;
  if (i.isRunning) {
    if (!i.agentPhase) return RUNNING_VISUAL;
    return { kind: "busy", text: PHASE_TEXT[i.agentPhase.phase], frame: phaseFrame(i.agentPhase) };
  }
  return IDLE_VISUAL;
}
