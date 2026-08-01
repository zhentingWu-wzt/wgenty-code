import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionManager } from "../state/sessionManager";

// Mock the loop: capture the callbacks the runner wires up.
vi.mock("./loop", () => ({
  runAgentLoop: vi.fn(),
}));
import { runAgentLoop } from "./loop";
import { runSessionTurn } from "./sessionRunner";
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

describe("runSessionTurn", () => {
  beforeEach(() => {
    reset();
    vi.clearAllMocks();
  });

  it("marks the session running, streams into its own store, then goes idle", async () => {
    vi.mocked(runAgentLoop).mockImplementation(async ({ callbacks }) => {
      callbacks.onStreamEvent(0, { type: "contentDelta", text: "hi" });
      return "";
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client, id, "hello");
    const e = useSessionManager.getState().entries[id];
    expect(e.status).toBe("idle");
    expect(e.store.getState().messages.some((m) => m.content === "hi")).toBe(true);
    expect(e.store.getState().isRunning).toBe(false);
  });

  it("permission prompt flips status to awaiting_approval during the loop", async () => {
    const id = useSessionManager.getState().createLocalSession("s1");
    vi.mocked(runAgentLoop).mockImplementation(async ({ callbacks }) => {
      void callbacks.onPermissionRequired({
        tool_name: "exec_command",
        reason: "needs approval",
        session_rule: "bash:*",
      });
      // 循环进行中：状态必须是 awaiting_approval
      expect(useSessionManager.getState().entries[id].status).toBe("awaiting_approval");
      return "";
    });
    await runSessionTurn(client, id, "x");
    // 循环结束后归位 idle
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("loop error marks the session error and does not touch other sessions", async () => {
    vi.mocked(runAgentLoop).mockRejectedValue(new Error("stream error: boom"));
    const a = useSessionManager.getState().createLocalSession("a");
    const b = useSessionManager.getState().createLocalSession("b");
    await runSessionTurn(client, a, "x");
    const s = useSessionManager.getState();
    expect(s.entries[a].status).toBe("error");
    expect(s.entries[a].store.getState().lastError?.kind).toBe("upstream");
    expect(s.entries[b].status).toBe("idle");
    expect(s.entries[b].store.getState().lastError).toBeNull();
  });

  it("aborted turns are silent (no error state)", async () => {
    vi.mocked(runAgentLoop).mockRejectedValue(new Error("aborted"));
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client, id, "x");
    const e = useSessionManager.getState().entries[id];
    expect(e.status).toBe("idle");
    expect(e.store.getState().lastError).toBeNull();
  });
});
