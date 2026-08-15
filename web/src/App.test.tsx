import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { DaemonClient } from "./api/client";

// The singleton ws push channel is a no-op here: jsdom has no daemon to talk
// to, and the real channel's backoff loop would leave live timers spinning
// after the test.
vi.mock("./api/wsChannel", () => ({
  wsChannel: {
    connect: () => {},
    status: () => "idle" as const,
    subscribeTrace: () => () => {},
    subscribeGlobal: () => () => {},
    subscribeSession: () => ({ unsubscribe: () => {} }),
    onReconnected: () => () => {},
  },
}));

/**
 * Smoke test: the full shell must render with the daemon offline.
 * Regresses the bug where PermissionModal/QuestionModal were mounted outside
 * SessionStoreContext.Provider and threw on first render (white screen).
 * All client methods reject; every caller in the tree handles errors locally.
 */
describe("App", () => {
  beforeEach(() => {
    vi.spyOn(DaemonClient.prototype, "health").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "traceReplay").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listPendingPermissions").mockRejectedValue(
      new Error("offline"),
    );
    vi.spyOn(DaemonClient.prototype, "listWorktrees").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSkills").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSessions").mockRejectedValue(new Error("offline"));
  });

  it("renders the app shell", async () => {
    render(<App />);
    expect(await screen.findByText("wgenty-code")).toBeInTheDocument();
  });
});
