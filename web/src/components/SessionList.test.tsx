import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SessionList } from "./SessionList";
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

describe("SessionList", () => {
  beforeEach(reset);

  it("renders open sessions with status dot and preview", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("fix bug");
    m.setPreview(a, "reading files…");
    render(<SessionList client={client} />);
    expect(screen.getByText("fix bug")).toBeInTheDocument();
    expect(screen.getByText("reading files…")).toBeInTheDocument();
  });

  it("clicking a session card makes it active", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("first");
    const b = m.createLocalSession("second");
    m.setActive(a);
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByText("second"));
    expect(useSessionManager.getState().activeId).toBe(b);
  });

  it("new-session button creates and activates a session", async () => {
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /new session/i }));
    expect(useSessionManager.getState().order).toHaveLength(1);
  });
});
