import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WelcomeView } from "./WelcomeView";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";

const client = new DaemonClient();

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

describe("WelcomeView", () => {
  beforeEach(reset);
  afterEach(() => vi.unstubAllGlobals());

  it("shows the landing copy and daemon connection status", () => {
    render(<WelcomeView client={client} />);
    expect(screen.getByText("Wgenty Code")).toBeInTheDocument();
    expect(screen.getByText(/Daemon unknown/)).toBeInTheDocument();
  });

  it("new session creates a local session entry", async () => {
    const user = userEvent.setup();
    render(<WelcomeView client={client} />);

    await user.click(screen.getByRole("button", { name: /new session/i }));

    const s = useSessionManager.getState();
    expect(s.order).toHaveLength(1);
    expect(s.entries[s.order[0]].daemonId).toBeNull();
  });

  it("open saved session loads the browser without making a local session", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/v1/sessions") {
          return new Response(
            JSON.stringify([{ id: "d1", name: "d1", status: "Active", message_count: 0 }]),
            { status: 200 },
          );
        }
        return new Response("not found", { status: 404 });
      }),
    );
    const user = userEvent.setup();
    render(<WelcomeView client={client} />);

    await user.click(screen.getByRole("button", { name: /open saved session/i }));
    expect(await screen.findByText("Saved sessions")).toBeInTheDocument();
    expect(useSessionManager.getState().order).toHaveLength(0);
    await waitFor(() => expect(screen.getByText("d1")).toBeInTheDocument());
  });
});
