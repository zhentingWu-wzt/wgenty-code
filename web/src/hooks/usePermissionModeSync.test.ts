import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { usePermissionModeSync } from "./usePermissionModeSync";
import type { DaemonClient } from "../api/client";
import type { GlobalEvent, ProvidersResponse } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

const wsTest = vi.hoisted(() => ({
  globalHandlers: new Set<(ev: GlobalEvent) => void>(),
}));

vi.mock("../api/wsChannel", () => ({
  wsChannel: {
    subscribeGlobal: (handler: (ev: GlobalEvent) => void) => {
      wsTest.globalHandlers.add(handler);
      return () => wsTest.globalHandlers.delete(handler);
    },
  },
}));

function modelsResponse(provider: string, model: string, isOverride: boolean): ProvidersResponse {
  return {
    providers: [
      {
        key: provider,
        display_name: provider,
        protocol: "openai",
        models: [{ name: model, tier: null, configured: true, active: true }],
      },
    ],
    is_override: isOverride,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function resetManager() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "connected",
    contextWindow: null,
  });
  wsTest.globalHandlers.clear();
}

describe("usePermissionModeSync", () => {
  beforeEach(resetManager);

  it("loads effective model and mode with their real override flags for the active session", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      projectPath: "/repo",
    });
    const client = {
      listModels: vi.fn().mockResolvedValue(modelsResponse("zhipu", "glm-5.3", true)),
      getPermissionMode: vi.fn().mockResolvedValue({
        mode: "normal",
        effective_mode: "normal",
        is_override: true,
      }),
    } as unknown as DaemonClient;

    renderHook(() => usePermissionModeSync(client));

    await waitFor(() => {
      const entry = useSessionManager.getState().entries[id];
      expect(entry.model).toEqual({ provider: "zhipu", model: "glm-5.3", isOverride: true });
      expect(entry.mode).toEqual({ mode: "normal", isOverride: true });
    });
    expect(client.listModels).toHaveBeenCalledWith("daemon-a");
    expect(client.getPermissionMode).toHaveBeenCalledWith("daemon-a");
  });

  it("discards late responses from the previously active session", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      projectPath: "/repo",
    });
    const b = m.createLocalSession("b", {
      id: "local-b",
      daemonId: "daemon-b",
      projectPath: "/repo",
      activate: false,
    });
    const aModels = deferred<ProvidersResponse>();
    const bModels = deferred<ProvidersResponse>();
    const aMode = deferred<{ mode: "normal"; effective_mode: string; is_override: boolean }>();
    const bMode = deferred<{ mode: "yolo"; effective_mode: string; is_override: boolean }>();
    const client = {
      listModels: vi.fn((sessionId: string) =>
        sessionId === "daemon-a" ? aModels.promise : bModels.promise,
      ),
      getPermissionMode: vi.fn((sessionId: string) =>
        sessionId === "daemon-a" ? aMode.promise : bMode.promise,
      ),
    } as unknown as DaemonClient;

    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => expect(client.listModels).toHaveBeenCalledWith("daemon-a"));
    act(() => useSessionManager.getState().setActive(b));
    await waitFor(() => expect(client.listModels).toHaveBeenCalledWith("daemon-b"));

    await act(async () => {
      bModels.resolve(modelsResponse("kimi", "k3", false));
      bMode.resolve({ mode: "yolo", effective_mode: "yolo", is_override: true });
      await Promise.all([bModels.promise, bMode.promise]);
    });
    await waitFor(() => {
      expect(useSessionManager.getState().entries[b].model).toEqual({
        provider: "kimi",
        model: "k3",
        isOverride: false,
      });
    });

    await act(async () => {
      aModels.resolve(modelsResponse("zhipu", "glm-5.2", true));
      aMode.resolve({ mode: "normal", effective_mode: "normal", is_override: false });
      await Promise.all([aModels.promise, aMode.promise]);
    });

    const entries = useSessionManager.getState().entries;
    expect(entries[b].model).toEqual({ provider: "kimi", model: "k3", isOverride: false });
    expect(entries[b].mode).toEqual({ mode: "yolo", isOverride: true });
    expect(entries[a].model).toBeNull();
    expect(entries[a].mode).toBeNull();
  });
});
