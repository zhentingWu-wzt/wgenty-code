import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useContinuationObserver } from "./useContinuationObserver";
import { observeDaemonRun } from "../agent/sessionRunner";
import type { DaemonClient } from "../api/client";
import type { GlobalEvent } from "../api/types";

// The hook's only job is mapping global events to observeDaemonRun calls;
// the run observation itself is covered by sessionRunner.test.ts.
vi.mock("../agent/sessionRunner", () => ({
  observeDaemonRun: vi.fn(),
}));

// ── Fake ws push channel ─────────────────────────────────────────────────────

const wsTest = vi.hoisted(() => ({
  globalHandlers: new Set<(ev: GlobalEvent) => void>(),
}));

vi.mock("../api/wsChannel", () => ({
  wsChannel: {
    connect: vi.fn(),
    status: () => "open" as const,
    subscribeTrace: () => () => {},
    subscribeGlobal: (handler: (ev: GlobalEvent) => void) => {
      wsTest.globalHandlers.add(handler);
      return () => wsTest.globalHandlers.delete(handler);
    },
    subscribeSession: () => ({ unsubscribe: () => {} }),
    onReconnected: () => () => {},
  },
}));

/** Deliver a global event to every registered subscriber (server → client). */
function serverSendGlobal(ev: GlobalEvent): void {
  for (const h of wsTest.globalHandlers) h(ev);
}

function globalEvent(kind: string, data: Record<string, unknown>): GlobalEvent {
  return { seq: 1, kind, data };
}

const client = {} as DaemonClient;

describe("useContinuationObserver", () => {
  beforeEach(() => {
    wsTest.globalHandlers.clear();
    vi.clearAllMocks();
  });

  afterEach(() => {
    wsTest.globalHandlers.clear();
  });

  it("attaches an observer on task_group_result (top-level session_id)", () => {
    renderHook(() => useContinuationObserver(client));
    serverSendGlobal(globalEvent("task_group_result", { session_id: "daemon-s1" }));
    expect(observeDaemonRun).toHaveBeenCalledWith(client, "daemon-s1");
  });

  it("attaches an observer on background_result (nested result.session_id)", () => {
    renderHook(() => useContinuationObserver(client));
    // Mirrors retain_and_broadcast_background_result's payload: the whole
    // BackgroundResult is nested under data.result, session_id inside it.
    serverSendGlobal(
      globalEvent("background_result", {
        result: { task_id: "bg_1", session_id: "daemon-s1", success: true },
      }),
    );
    expect(observeDaemonRun).toHaveBeenCalledWith(client, "daemon-s1");
  });

  it("ignores a background_result without an owning session", () => {
    renderHook(() => useContinuationObserver(client));
    serverSendGlobal(
      globalEvent("background_result", {
        result: { task_id: "bg_2", success: true },
      }),
    );
    expect(observeDaemonRun).not.toHaveBeenCalled();
  });

  it("does not attach on unrelated global events", () => {
    renderHook(() => useContinuationObserver(client));
    serverSendGlobal(globalEvent("todos_changed", { items: [] }));
    serverSendGlobal(globalEvent("mode_changed", { mode: "yolo" }));
    expect(observeDaemonRun).not.toHaveBeenCalled();
  });

  it("subscribes nothing while the client is null", () => {
    const { rerender } = renderHook(({ c }) => useContinuationObserver(c), {
      initialProps: { c: null as DaemonClient | null },
    });
    expect(wsTest.globalHandlers.size).toBe(0);
    rerender({ c: client });
    expect(wsTest.globalHandlers.size).toBe(1);
  });
});
