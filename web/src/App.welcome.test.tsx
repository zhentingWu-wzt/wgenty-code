import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { DaemonClient } from "./api/client";
import { useSessionManager } from "./state/sessionManager";
import { useUiStore } from "./state/uiStore";

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

function resetStores() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
    contextWindow: null,
    permissionMode: null,
  });
  useUiStore.setState({ openTabs: [], activeTabId: null, subagentTabs: {}, previewTabs: {}, diffTabs: {} });
}

describe("App welcome screen", () => {
  beforeEach(() => {
    resetStores();
    vi.spyOn(DaemonClient.prototype, "health").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "traceReplay").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listPendingPermissions").mockRejectedValue(
      new Error("offline"),
    );
    vi.spyOn(DaemonClient.prototype, "listWorktrees").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSkills").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "listSessions").mockRejectedValue(new Error("offline"));
  });

  it("shows the welcome screen after the last tab closes", async () => {
    render(<App />);
    // Bootstrap resolves (daemon offline) → welcome screen.
    expect(await screen.findByText("Wgenty Code", { selector: "h1" })).toBeInTheDocument();

    // Open a session from the welcome screen.
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /new session/i }));
    await waitFor(() =>
      expect(useSessionManager.getState().activeId).not.toBeNull(),
    );
    // Welcome is replaced by the chat shell.
    expect(screen.queryByText("Wgenty Code", { selector: "h1" })).not.toBeInTheDocument();

    // Close the only tab → back to the welcome screen (no crash / blank).
    const closeBtn = await screen.findAllByRole("button", { name: /^Close /i });
    await user.click(closeBtn[0]);

    expect(
      await screen.findByText("Wgenty Code", { selector: "h1" }),
    ).toBeInTheDocument();
    expect(useSessionManager.getState().activeId).toBeNull();
  });

  it("shows the welcome screen after the last tab closes (history restored)", async () => {
    // Regression: bootstrap's restore-success path returned early without
    // setting `booted`, so a later last-tab-close made App render null — a
    // full-screen blank (black in dark theme) instead of the welcome screen.
    vi.spyOn(DaemonClient.prototype, "listSessions").mockResolvedValue([
      {
        id: "d1",
        name: "restored",
        status: "Active",
        created_at: "2025-01-01T00:00:00Z",
        updated_at: "2025-01-01T00:00:00Z",
        message_count: 0,
      },
    ]);
    vi.spyOn(DaemonClient.prototype, "loadSession").mockResolvedValue({
      id: "d1",
      name: "restored",
      messages: [],
    } as never);

    const user = userEvent.setup();
    render(<App />);

    // Session restored + activated → chat shell (no welcome yet).
    await waitFor(() =>
      expect(useSessionManager.getState().activeId).toBe("d1"),
    );
    expect(screen.queryByText("Wgenty Code", { selector: "h1" })).not.toBeInTheDocument();

    // Close the only tab → welcome screen, not a blank app.
    const closeBtn = await screen.findAllByRole("button", { name: /^Close /i });
    await user.click(closeBtn[0]);

    expect(
      await screen.findByText("Wgenty Code", { selector: "h1" }),
    ).toBeInTheDocument();
    expect(useSessionManager.getState().activeId).toBeNull();
  });
});
