import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { DaemonClient } from "./api/client";

/**
 * Smoke test: the full shell must render with the daemon offline.
 * Regresses the bug where PermissionModal/QuestionModal were mounted outside
 * SessionStoreContext.Provider and threw on first render (white screen).
 * All client methods reject; every caller in the tree handles errors locally.
 */
describe("App", () => {
  beforeEach(() => {
    vi.spyOn(DaemonClient.prototype, "health").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "traceStream").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listWorktrees").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSkills").mockRejectedValue(new Error("offline"));
  });

  it("renders the app shell", () => {
    render(<App />);
    expect(screen.getByText("wgenty-code")).toBeInTheDocument();
  });
});
