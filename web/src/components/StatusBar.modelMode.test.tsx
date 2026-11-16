import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "./StatusBar";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";

vi.mock("../features/files/useWorkspaceRoot", () => ({
  useWorkspaceRoot: () => ({ root: null }),
}));

describe("StatusBar active model and mode", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "connected",
      contextWindow: null,
    });
  });

  it("follows the active entry and writes a mode choice back to that entry", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a", { id: "local-a", daemonId: "daemon-a" });
    const b = m.createLocalSession("b", {
      id: "local-b",
      daemonId: "daemon-b",
      activate: false,
    });
    m.setSessionModel(a, { provider: "zhipu", model: "glm-5.3", isOverride: true });
    m.setSessionMode(a, { mode: "yolo", isOverride: true });
    m.setSessionModel(b, { provider: "kimi", model: "k3", isOverride: false });
    m.setSessionMode(b, { mode: "normal", isOverride: false });
    const client = {
      setPermissionMode: vi.fn().mockResolvedValue({
        success: true,
        mode: "accept_edits",
        effective_mode: "accept_edits",
        is_override: true,
      }),
    } as unknown as DaemonClient;

    render(<StatusBar client={client} onSwitchModel={() => {}} />);
    expect(screen.getByRole("button", { name: "glm-5.3" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "yolo" })).toBeInTheDocument();

    act(() => useSessionManager.getState().setActive(b));
    expect(screen.getByRole("button", { name: "k3" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "normal" })).toBeInTheDocument();

    act(() => useSessionManager.getState().setActive(a));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "yolo" }));
    await user.click(screen.getByRole("button", { name: "accept edits" }));

    expect(client.setPermissionMode).toHaveBeenCalledWith("daemon-a", "accept_edits");
    await waitFor(() =>
      expect(useSessionManager.getState().entries[a].mode).toEqual({
        mode: "accept_edits",
        isOverride: true,
      }),
    );
    expect(useSessionManager.getState().entries[b].mode).toEqual({
      mode: "normal",
      isOverride: false,
    });
  });
});
