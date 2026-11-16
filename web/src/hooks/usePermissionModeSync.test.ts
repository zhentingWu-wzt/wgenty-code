import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { usePermissionModeSync } from "./usePermissionModeSync";
import type { DaemonClient } from "../api/client";
import type { GlobalEvent, ProvidersResponse } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

const wsTest = vi.hoisted(() => ({
  globalHandlers: new Set<(ev: GlobalEvent) => void>(),
  reconnectedHandlers: new Set<() => void>(),
}));

vi.mock("../api/wsChannel", () => ({
  wsChannel: {
    subscribeGlobal: (handler: (ev: GlobalEvent) => void) => {
      wsTest.globalHandlers.add(handler);
      return () => wsTest.globalHandlers.delete(handler);
    },
    onReconnected: (handler: () => void) => {
      wsTest.reconnectedHandlers.add(handler);
      return () => wsTest.reconnectedHandlers.delete(handler);
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
  wsTest.reconnectedHandlers.clear();
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

  it("refreshes the current entry after a websocket reconnect while health stays connected", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const client = {
      listModels: vi
        .fn()
        .mockResolvedValueOnce(modelsResponse("zhipu", "before-disconnect", true))
        .mockResolvedValueOnce(modelsResponse("kimi", "after-reconnect", false)),
      getPermissionMode: vi
        .fn()
        .mockResolvedValueOnce({
          mode: "normal",
          effective_mode: "normal",
          is_override: true,
        })
        .mockResolvedValueOnce({
          mode: "yolo",
          effective_mode: "yolo",
          is_override: false,
        }),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => {
      expect(useSessionManager.getState().entries[id].model?.model).toBe("before-disconnect");
      expect(useSessionManager.getState().entries[id].mode?.mode).toBe("normal");
    });
    expect(useSessionManager.getState().connection).toBe("connected");

    act(() => {
      for (const handler of wsTest.reconnectedHandlers) handler();
    });

    await waitFor(() => {
      expect(useSessionManager.getState().entries[id].model).toEqual({
        provider: "kimi",
        model: "after-reconnect",
        isOverride: false,
      });
      expect(useSessionManager.getState().entries[id].mode).toEqual({
        mode: "yolo",
        isOverride: false,
      });
    });
    expect(client.listModels).toHaveBeenCalledTimes(2);
    expect(client.getPermissionMode).toHaveBeenCalledTimes(2);
  });

  it("accepts lower seq values after reconnecting to a new daemon boot", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const client = {
      listModels: vi
        .fn()
        .mockResolvedValueOnce(modelsResponse("kimi", "before-restart", false))
        .mockResolvedValueOnce(modelsResponse("kimi", "reconnect-snapshot", false)),
      getPermissionMode: vi
        .fn()
        .mockResolvedValueOnce({
          mode: "normal",
          effective_mode: "normal",
          is_override: false,
        })
        .mockResolvedValueOnce({
          mode: "normal",
          effective_mode: "normal",
          is_override: false,
        }),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => {
      expect(useSessionManager.getState().entries[id].model?.model).toBe("before-restart");
    });

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 50,
        kind: "model_changed",
        data: { session_id: "daemon-a", provider: "kimi", model_name: "old-boot-model" },
      });
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 50,
          kind: "mode_changed",
          data: { session_id: "daemon-a", mode: "yolo" },
        });
      }
    });
    act(() => {
      for (const handler of wsTest.reconnectedHandlers) handler();
    });
    await waitFor(() => {
      expect(useSessionManager.getState().entries[id].model?.model).toBe("reconnect-snapshot");
    });

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: { session_id: "daemon-a", provider: "kimi", model_name: "new-boot-model" },
      });
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 1,
          kind: "mode_changed",
          data: { session_id: "daemon-a", mode: "accept_edits" },
        });
      }
    });

    const entry = useSessionManager.getState().entries[id];
    expect(entry.model?.model).toBe("new-boot-model");
    expect(entry.mode?.mode).toBe("accept_edits");
  });

  it("resets old boot cursors when websocket reconnects while health is disconnected", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const client = {
      listModels: vi.fn().mockResolvedValue(modelsResponse("kimi", "health-snapshot", false)),
      getPermissionMode: vi.fn().mockResolvedValue({
        mode: "normal",
        effective_mode: "normal",
        is_override: false,
      }),
    } as unknown as DaemonClient;
    useSessionManager.getState().applyModelChanged({
      seq: 50,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "kimi", model_name: "old-boot-model" },
    });
    useSessionManager.getState().applyModeChanged({
      seq: 50,
      kind: "mode_changed",
      data: { session_id: "daemon-a", mode: "yolo" },
    });
    act(() => {
      useSessionManager.setState({ connection: "disconnected" });
    });
    renderHook(() => usePermissionModeSync(client));

    act(() => {
      for (const handler of wsTest.reconnectedHandlers) handler();
    });
    act(() => {
      useSessionManager.getState().setConnection("connected");
    });
    await waitFor(() => {
      expect(useSessionManager.getState().entries[id].model?.model).toBe("health-snapshot");
    });

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: { session_id: "daemon-a", provider: "kimi", model_name: "new-boot-model" },
      });
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 1,
          kind: "mode_changed",
          data: { session_id: "daemon-a", mode: "accept_edits" },
        });
      }
    });

    const entry = useSessionManager.getState().entries[id];
    expect(entry.model?.model).toBe("new-boot-model");
    expect(entry.mode?.mode).toBe("accept_edits");
  });

  it("resets all entry cursors on reconnect without an active session", async () => {
    const manager = useSessionManager.getState();
    const a = manager.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      activate: false,
    });
    const b = manager.createLocalSession("b", {
      id: "local-b",
      daemonId: "daemon-b",
      activate: false,
    });
    for (const daemonId of ["daemon-a", "daemon-b"]) {
      manager.applyModelChanged({
        seq: 50,
        kind: "model_changed",
        data: { session_id: daemonId, provider: "kimi", model_name: "old-boot-model" },
      });
      manager.applyModeChanged({
        seq: 50,
        kind: "mode_changed",
        data: { session_id: daemonId, mode: "yolo" },
      });
    }
    useSessionManager.setState({ connection: "disconnected" });
    const client = {
      listModels: vi.fn().mockResolvedValue(modelsResponse("kimi", "health-snapshot", false)),
      getPermissionMode: vi.fn().mockResolvedValue({
        mode: "normal",
        effective_mode: "normal",
        is_override: false,
      }),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));

    expect(useSessionManager.getState().activeId).toBeNull();
    for (const entry of Object.values(useSessionManager.getState().entries)) {
      expect(entry).toMatchObject({ modelSeq: 50, modeSeq: 50 });
    }
    act(() => {
      for (const handler of wsTest.reconnectedHandlers) handler();
    });

    expect(client.listModels).not.toHaveBeenCalled();
    expect(client.getPermissionMode).not.toHaveBeenCalled();
    for (const entry of Object.values(useSessionManager.getState().entries)) {
      expect(entry).toMatchObject({ modelSeq: 0, modeSeq: 0 });
    }

    act(() => {
      useSessionManager.getState().setActive(a);
      useSessionManager.getState().setConnection("connected");
    });
    await waitFor(() => {
      expect(useSessionManager.getState().entries[a].model?.model).toBe("health-snapshot");
      expect(useSessionManager.getState().entries[a].mode?.mode).toBe("normal");
    });
    expect(client.listModels).toHaveBeenCalledTimes(1);
    expect(client.getPermissionMode).toHaveBeenCalledTimes(1);

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: { session_id: "daemon-a", provider: "kimi", model_name: "new-boot-model" },
      });
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 1,
          kind: "mode_changed",
          data: { session_id: "daemon-a", mode: "accept_edits" },
        });
      }
    });

    const entries = useSessionManager.getState().entries;
    expect(entries[a].model?.model).toBe("new-boot-model");
    expect(entries[a].mode?.mode).toBe("accept_edits");
    expect(entries[a].modelSeq).toBe(1);
    expect(entries[a].modeSeq).toBe(1);
    expect(entries[b].modelSeq).toBe(0);
    expect(entries[b].modeSeq).toBe(0);
  });

  it("does not let an old model GET overwrite a newer event for the same entry", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const models = deferred<ProvidersResponse>();
    const client = {
      listModels: vi.fn().mockReturnValue(models.promise),
      getPermissionMode: vi.fn().mockResolvedValue({
        mode: "normal",
        effective_mode: "normal",
        is_override: false,
      }),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => expect(client.listModels).toHaveBeenCalledWith("daemon-a"));

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: { session_id: "daemon-a", provider: "kimi", model_name: "new-event" },
      });
    });
    await act(async () => {
      models.resolve(modelsResponse("zhipu", "stale-get", true));
      await models.promise;
    });

    expect(useSessionManager.getState().entries[id].model).toEqual({
      provider: "kimi",
      model: "new-event",
      isOverride: true,
    });
  });

  it("does not let an old mode GET overwrite a newer event for the same entry", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const mode = deferred<{ mode: "normal"; effective_mode: string; is_override: boolean }>();
    const client = {
      listModels: vi.fn().mockResolvedValue(modelsResponse("kimi", "k3", false)),
      getPermissionMode: vi.fn().mockReturnValue(mode.promise),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => expect(client.getPermissionMode).toHaveBeenCalledWith("daemon-a"));

    act(() => {
      for (const handler of wsTest.globalHandlers) {
        handler({
          seq: 1,
          kind: "mode_changed",
          data: { session_id: "daemon-a", mode: "yolo", is_override: true },
        });
      }
    });
    await act(async () => {
      mode.resolve({ mode: "normal", effective_mode: "normal", is_override: false });
      await mode.promise;
    });

    expect(useSessionManager.getState().entries[id].mode).toEqual({
      mode: "yolo",
      isOverride: true,
    });
  });

  it("invalidates a pending GET when a global model event reaches an unknown entry", async () => {
    const id = useSessionManager.getState().createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
    });
    const models = deferred<ProvidersResponse>();
    const client = {
      listModels: vi.fn().mockReturnValue(models.promise),
      getPermissionMode: vi.fn().mockResolvedValue({
        mode: "normal",
        effective_mode: "normal",
        is_override: false,
      }),
    } as unknown as DaemonClient;
    renderHook(() => usePermissionModeSync(client));
    await waitFor(() => expect(client.listModels).toHaveBeenCalledTimes(1));

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: { provider: "kimi", model_name: "new-global" },
      });
    });
    await act(async () => {
      models.resolve(modelsResponse("zhipu", "stale-get", false));
      await models.promise;
    });

    const entry = useSessionManager.getState().entries[id];
    expect(entry.model).toBeNull();
    expect(entry.modelSeq).toBe(1);
  });

  it.each(["old-first", "old-last"] as const)(
    "keeps the newest model refresh when the older request resolves %s",
    async (order) => {
      const id = useSessionManager.getState().createLocalSession("a", {
        id: "local-a",
        daemonId: "daemon-a",
      });
      const initial = deferred<ProvidersResponse>();
      const reconnect = deferred<ProvidersResponse>();
      const client = {
        listModels: vi
          .fn()
          .mockReturnValueOnce(initial.promise)
          .mockReturnValueOnce(reconnect.promise),
        getPermissionMode: vi.fn().mockResolvedValue({
          mode: "normal",
          effective_mode: "normal",
          is_override: false,
        }),
      } as unknown as DaemonClient;
      renderHook(() => usePermissionModeSync(client));
      await waitFor(() => expect(client.listModels).toHaveBeenCalledTimes(1));
      act(() => {
        for (const handler of wsTest.reconnectedHandlers) handler();
      });
      await waitFor(() => expect(client.listModels).toHaveBeenCalledTimes(2));

      await act(async () => {
        if (order === "old-first") {
          initial.resolve(modelsResponse("zhipu", "initial-old", true));
          await initial.promise;
          reconnect.resolve(modelsResponse("kimi", "reconnect-new", false));
          await reconnect.promise;
        } else {
          reconnect.resolve(modelsResponse("kimi", "reconnect-new", false));
          await reconnect.promise;
          initial.resolve(modelsResponse("zhipu", "initial-old", true));
          await initial.promise;
        }
      });

      expect(useSessionManager.getState().entries[id].model).toEqual({
        provider: "kimi",
        model: "reconnect-new",
        isOverride: false,
      });
    },
  );

  it.each(["old-first", "old-last"] as const)(
    "keeps the newest mode refresh when the older request resolves %s",
    async (order) => {
      const id = useSessionManager.getState().createLocalSession("a", {
        id: "local-a",
        daemonId: "daemon-a",
      });
      const initial = deferred<{ mode: "normal"; effective_mode: string; is_override: boolean }>();
      const reconnect = deferred<{ mode: "yolo"; effective_mode: string; is_override: boolean }>();
      const client = {
        listModels: vi.fn().mockResolvedValue(modelsResponse("kimi", "k3", false)),
        getPermissionMode: vi
          .fn()
          .mockReturnValueOnce(initial.promise)
          .mockReturnValueOnce(reconnect.promise),
      } as unknown as DaemonClient;
      renderHook(() => usePermissionModeSync(client));
      await waitFor(() => expect(client.getPermissionMode).toHaveBeenCalledTimes(1));
      act(() => {
        for (const handler of wsTest.reconnectedHandlers) handler();
      });
      await waitFor(() => expect(client.getPermissionMode).toHaveBeenCalledTimes(2));

      await act(async () => {
        if (order === "old-first") {
          initial.resolve({ mode: "normal", effective_mode: "normal", is_override: true });
          await initial.promise;
          reconnect.resolve({ mode: "yolo", effective_mode: "yolo", is_override: false });
          await reconnect.promise;
        } else {
          reconnect.resolve({ mode: "yolo", effective_mode: "yolo", is_override: false });
          await reconnect.promise;
          initial.resolve({ mode: "normal", effective_mode: "normal", is_override: true });
          await initial.promise;
        }
      });

      expect(useSessionManager.getState().entries[id].mode).toEqual({
        mode: "yolo",
        isOverride: false,
      });
    },
  );

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
