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
    // No /projects stub → daemon offline path → legacy local session.
    const user = userEvent.setup();
    render(<WelcomeView client={client} />);

    await user.click(screen.getByRole("button", { name: /new session/i }));

    const s = useSessionManager.getState();
    expect(s.order).toHaveLength(1);
    expect(s.entries[s.order[0]].daemonId).toBeNull();
  });

  it("new session creates in the No Project workspace when known", async () => {
    const npPath = "/home/u/.wgenty-code/no-project";
    const createdCalls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url === "/api/v1/projects") {
          return new Response(
            JSON.stringify([
              {
                path: "/repo/wgenty-code",
                name: "wgenty-code",
                is_main: true,
                is_no_project: false,
                is_git_repo: true,
                added_at: "2026-01-01T00:00:00Z",
              },
              {
                path: npPath,
                name: "No Project",
                is_main: false,
                is_no_project: true,
                is_git_repo: false,
                added_at: "2026-01-01T00:00:00Z",
              },
            ]),
            { status: 200 },
          );
        }
        if (url === "/api/v1/sessions" && init?.method === "POST") {
          createdCalls.push(String(init.body));
          return new Response(
            JSON.stringify({
              id: "np-1",
              name: "np-1",
              created_at: "2026-01-01T00:00:00Z",
              updated_at: "2026-01-01T00:00:00Z",
              messages: [],
              ui_messages: [],
            }),
            { status: 200 },
          );
        }
        return new Response("not found", { status: 404 });
      }),
    );

    const user = userEvent.setup();
    render(<WelcomeView client={client} />);
    await user.click(await screen.findByRole("button", { name: /new session/i }));

    const s = useSessionManager.getState();
    expect(s.order).toHaveLength(1);
    const entry = s.entries[s.order[0]];
    expect(entry.daemonId).toBe("np-1");
    expect(entry.projectPath).toBe(npPath);
    expect(createdCalls).toEqual([JSON.stringify({ project_path: npPath })]);
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
