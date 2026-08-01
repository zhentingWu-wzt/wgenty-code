import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SubagentPanel } from "./SubagentPanel";
import { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";

/** Newline-delimited JSON byte stream, closed after the given rows. */
function streamOf(rows: unknown[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const row of rows) controller.enqueue(encoder.encode(JSON.stringify(row) + "\n"));
      controller.close();
    },
  });
}

describe("SubagentPanel", () => {
  afterEach(() => vi.restoreAllMocks());

  it("renders label + current_tool of a realistic trace event for the active session", async () => {
    const sessionId = useSessionManager.getState().createLocalSession("trace-test");
    const base = {
      ts: 1,
      node_id: "n1",
      status: "running",
      elapsed_ms: 10,
      cumulative_tokens: 0,
      kind: "progress",
    };
    vi.spyOn(DaemonClient.prototype, "traceStream").mockResolvedValue({
      body: streamOf([
        // Same shape, but another session — must be filtered out.
        { ...base, session_id: "other-session", label: "should-not-render" },
        {
          ...base,
          session_id: sessionId,
          label: "explore-agent",
          current_tool: "grep",
        },
      ]),
    });

    render(<SubagentPanel client={new DaemonClient()} />);

    expect(await screen.findByText(/explore-agent/)).toBeInTheDocument();
    expect(screen.getByText(/\[grep\]/)).toBeInTheDocument();
    expect(screen.queryByText(/should-not-render/)).not.toBeInTheDocument();
  });
});
