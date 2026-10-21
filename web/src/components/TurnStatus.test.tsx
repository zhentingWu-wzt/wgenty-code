import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { TurnStatus } from "./TurnStatus";
import { useSessionManager } from "../state/sessionManager";

/**
 * The turn-status strip above the input renders the same phase states the
 * bottom StatusBar used to carry (moved here for visibility): Ready when
 * idle, spinner + phase label while busy, awaiting-decision and error
 * variants.
 */
describe("TurnStatus", () => {
  /** Fresh active session; returns its store. (getState() is a snapshot, so
   *  re-read after createLocalSession replaced the entries object.) */
  function activeSession() {
    const id = useSessionManager.getState().createLocalSession("s1");
    return {
      id,
      store: useSessionManager.getState().entries[id].store,
    };
  }

  it("shows Ready when no turn is active", () => {
    activeSession();
    render(<TurnStatus />);
    expect(screen.getByRole("status")).toHaveTextContent("● Ready");
  });

  it("shows the phase label while a turn runs", () => {
    const { id, store } = activeSession();
    useSessionManager.getState().setStatus(id, "running");
    store.getState().setAgentPhase({ phase: "executing", toolName: "file_read" });
    render(<TurnStatus />);
    expect(screen.getByRole("status")).toHaveTextContent("Executing file_read…");
  });

  it("shows the awaiting-decision variant over the phase", () => {
    const { id, store } = activeSession();
    useSessionManager.getState().setStatus(id, "awaiting_approval");
    store.getState().setAgentPhase({ phase: "executing", toolName: "x" });
    render(<TurnStatus />);
    expect(screen.getByRole("status")).toHaveTextContent("Permission required");
  });

  it("shows the error variant", () => {
    const { id, store } = activeSession();
    useSessionManager.getState().setStatus(id, "error");
    // Error detection follows lastError (persists to the next send), not
    // agentPhase — the runner clears the phase in its finally block.
    store.getState().setError({ message: "boom", kind: "upstream" });
    render(<TurnStatus />);
    expect(screen.getByRole("status")).toHaveTextContent("✗ Error");
  });
});
