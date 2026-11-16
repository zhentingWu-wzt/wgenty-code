import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { DaemonClient } from "./api/client";
import type { GlobalEvent } from "./api/types";
import { useSessionManager } from "./state/sessionManager";
import { useUiStore } from "./state/uiStore";

const wsTest = vi.hoisted(() => ({
  globalHandlers: new Set<(ev: GlobalEvent) => void>(),
}));

vi.mock("./api/wsChannel", () => ({
  wsChannel: {
    connect: () => {},
    status: () => "idle" as const,
    subscribeTrace: () => () => {},
    subscribeGlobal: (handler: (ev: GlobalEvent) => void) => {
      wsTest.globalHandlers.add(handler);
      return () => wsTest.globalHandlers.delete(handler);
    },
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
    contextWindow: null,
  });
  useUiStore.setState({
    openTabs: [],
    activeTabId: null,
    subagentTabs: {},
    previewTabs: {},
    diffTabs: {},
  });
  wsTest.globalHandlers.clear();
}

describe("App welcome screen", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    resetStores();
    vi.spyOn(DaemonClient.prototype, "health").mockRejectedValue(new Error("offline"));
    vi.spyOn(DaemonClient.prototype, "getConfig").mockResolvedValue({
      model: "global-model",
      api_base: "http://localhost",
      max_tokens: 1000,
      timeout: 60,
      streaming: true,
      context_window: 128000,
    });
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
    await waitFor(() => expect(useSessionManager.getState().activeId).not.toBeNull());
    // Welcome is replaced by the chat shell.
    expect(screen.queryByText("Wgenty Code", { selector: "h1" })).not.toBeInTheDocument();

    // Close the only tab → back to the welcome screen (no crash / blank).
    const closeBtn = await screen.findAllByRole("button", { name: /^Close /i });
    await user.click(closeBtn[0]);

    expect(await screen.findByText("Wgenty Code", { selector: "h1" })).toBeInTheDocument();
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
    await waitFor(() => expect(useSessionManager.getState().activeId).toBe("d1"));
    expect(screen.queryByText("Wgenty Code", { selector: "h1" })).not.toBeInTheDocument();

    // Close the only tab → welcome screen, not a blank app.
    const closeBtn = await screen.findAllByRole("button", { name: /^Close /i });
    await user.click(closeBtn[0]);

    expect(await screen.findByText("Wgenty Code", { selector: "h1" })).toBeInTheDocument();
    expect(useSessionManager.getState().activeId).toBeNull();
  });

  it("restores the newest sessions per project, not just the global top 5", async () => {
    // Regression: bootstrap kept only the 5 globally-newest sessions, so an
    // active project could vanish from the left rail after a reload when
    // another project owned all 5 newest sessions.
    vi.spyOn(DaemonClient.prototype, "listProjects").mockRejectedValue(new Error("offline"));
    const mk = (id: string, project: string | null, at: string) => ({
      id,
      name: id,
      project_path: project,
      status: "Active",
      created_at: at,
      updated_at: at,
      message_count: 0,
    });
    vi.spyOn(DaemonClient.prototype, "listSessions").mockResolvedValue([
      mk("main-1", null, "2026-01-05T00:00:00Z"),
      mk("main-2", null, "2026-01-04T00:00:00Z"),
      mk("docs-1", "/repo/docs", "2026-01-03T00:00:00Z"),
      mk("main-3", null, "2026-01-02T00:00:00Z"),
      mk("main-4", null, "2026-01-01T00:00:00Z"),
      mk("docs-2", "/repo/docs", "2025-12-31T00:00:00Z"),
      mk("main-5", null, "2025-12-30T00:00:00Z"),
      mk("main-6", null, "2025-12-29T00:00:00Z"),
    ]);
    vi.spyOn(DaemonClient.prototype, "loadSession").mockImplementation(
      async (id: string) =>
        ({
          id,
          name: id,
          messages: [],
        }) as never,
    );

    render(<App />);

    // Per-project cap of 5: main keeps 5 of its 6, docs keeps both of its 2.
    await waitFor(() => {
      const entries = useSessionManager.getState().entries;
      expect(Object.keys(entries).sort()).toEqual(
        ["main-1", "main-2", "main-3", "main-4", "main-5", "docs-1", "docs-2"].sort(),
      );
    });
  });

  it("routes model_changed broadcasts to the matching session entry", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a", { id: "daemon-a", daemonId: "daemon-a" });
    const b = m.createLocalSession("b", {
      id: "daemon-b",
      daemonId: "daemon-b",
      activate: false,
    });
    m.setSessionModel(a, { provider: "kimi", model: "k2", isOverride: false });
    m.setSessionModel(b, { provider: "kimi", model: "k2", isOverride: false });
    render(<App />);
    await waitFor(() => expect(wsTest.globalHandlers.size).toBeGreaterThan(0));

    act(() => {
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 1,
          kind: "model_changed",
          data: { session_id: "daemon-a", provider: "zhipu", model_name: "glm-5.3" },
        });
      }
    });

    const entries = useSessionManager.getState().entries;
    expect(entries[a].model).toEqual({ provider: "zhipu", model: "glm-5.3", isOverride: true });
    expect(entries[b].model).toEqual({ provider: "kimi", model: "k2", isOverride: false });
  });
});
