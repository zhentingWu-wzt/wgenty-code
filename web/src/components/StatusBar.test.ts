import { describe, expect, it } from "vitest";
import { deriveTurnVisual, formatDuration, phaseLabel } from "./statusPhase";
import type { AgentPhase } from "../state/sessionStore";

/** The phase labels must stay word-for-word aligned with the TUI's
 *  `phase_label` (src/tui/components/status.rs) so both frontends read
 *  identically for the same turn state. */

function phase(p: AgentPhase, toolName?: string) {
  return toolName === undefined ? { phase: p } : { phase: p, toolName };
}

describe("statusPhase labels (TUI-aligned)", () => {
  it("matches the TUI labels for the derivable phases", () => {
    expect(phaseLabel(phase("thinking"))).toBe("Thinking…");
    expect(phaseLabel(phase("streaming"))).toBe("Streaming…");
    expect(phaseLabel(phase("preparing_tools"))).toBe("Preparing tools…");
    expect(phaseLabel(phase("error"))).toBe("Error");
  });

  it("matches the TUI labels for the daemon-truth phases", () => {
    expect(phaseLabel(phase("compacting"))).toBe("Compacting…");
    // Retry info only when there is more than one attempt (TUI parity).
    expect(phaseLabel(phase("connecting"))).toBe("Connecting…");
    expect(phaseLabel({ phase: "connecting", attempt: 2, maxRetries: 3 })).toBe(
      "Connecting (attempt 2/3)…",
    );
    expect(phaseLabel({ phase: "connecting", attempt: 1, maxRetries: 1 })).toBe("Connecting…");
  });

  it("names the executing tool like the TUI, with the subagent alias", () => {
    expect(phaseLabel(phase("executing", "file_read"))).toBe("Executing file_read…");
    expect(phaseLabel(phase("executing", "task"))).toBe("Subagent running…");
    expect(phaseLabel(phase("executing", "delegate"))).toBe("Subagent running…");
  });
});

describe("statusPhase formatDuration (TUI-aligned)", () => {
  it("formats seconds under a minute bare, minutes+seconds beyond", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(42)).toBe("42s");
    expect(formatDuration(59)).toBe("59s");
    expect(formatDuration(60)).toBe("1m0s");
    expect(formatDuration(90)).toBe("1m30s");
    expect(formatDuration(754)).toBe("12m34s");
  });
});

describe("deriveTurnVisual (single source of truth)", () => {
  const input = (over: Partial<Parameters<typeof deriveTurnVisual>[0]>) => ({
    awaitingDecision: false,
    hasError: false,
    isRunning: false,
    agentPhase: null,
    ...over,
  });

  it("each busy phase gets its own hue and matching frame", () => {
    const cases: Array<[AgentPhase, string, string]> = [
      ["thinking", "text-sky-500", "border-sky-500/60"],
      ["connecting", "text-orange-500", "border-orange-500/60"],
      ["streaming", "text-primary", "border-primary/60"],
      ["preparing_tools", "text-violet-500", "border-violet-500/60"],
      ["executing", "text-teal-500", "border-teal-500/60"],
      ["compacting", "text-fuchsia-500", "border-fuchsia-500/60"],
    ];
    for (const [phase, text, frame] of cases) {
      const v = deriveTurnVisual(input({ isRunning: true, agentPhase: { phase } }));
      expect(v.kind).toBe("busy");
      expect(v.text).toContain(text);
      expect(v.frame).toContain(frame);
      // Label hue and frame hue must agree — the input follows the strip.
      expect(v.frame).toContain(frame.split("-")[1] ?? "");
    }
  });

  it("running without a phase yet falls back to brand blue", () => {
    const v = deriveTurnVisual(input({ isRunning: true }));
    expect(v.kind).toBe("busy");
    expect(v.frame).toContain("border-primary/60");
  });

  it("priority: awaiting > error > busy > idle", () => {
    expect(
      deriveTurnVisual(input({ awaitingDecision: true, hasError: true, isRunning: true })).kind,
    ).toBe("awaiting");
    expect(deriveTurnVisual(input({ hasError: true, isRunning: true })).kind).toBe("error");
    expect(deriveTurnVisual(input({ isRunning: true })).kind).toBe("busy");
    expect(deriveTurnVisual(input({})).kind).toBe("idle");
  });

  it("idle is the muted default on both label and frame", () => {
    const v = deriveTurnVisual(input({}));
    expect(v.text).toBe("text-muted-foreground");
    expect(v.frame).toContain("border-input");
  });
});
