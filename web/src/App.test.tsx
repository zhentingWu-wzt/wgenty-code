import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { DaemonClient } from "./api/client";

/**
 * Minimal EventSource stand-in — jsdom does not provide one. The heartbeat
 * effect only needs construction + property/event hooks + close().
 */
class MockEventSource {
  onmessage: ((e: unknown) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {}
  addEventListener() {}
  close() {}
}

/**
 * Smoke test: the full shell must render with the daemon offline.
 * Regresses the bug where PermissionModal/QuestionModal were mounted outside
 * SessionStoreContext.Provider and threw on first render (white screen).
 * All client methods reject; every caller in the tree handles errors locally.
 */
describe("App", () => {
  beforeEach(() => {
    vi.stubGlobal("EventSource", MockEventSource);
    vi.spyOn(DaemonClient.prototype, "health").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "traceStream").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listWorktrees").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSkills").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSessions").mockRejectedValue(new Error("offline"));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("renders the app shell", async () => {
    render(<App />);
    expect(await screen.findByText("wgenty-code")).toBeInTheDocument();
  });
});
