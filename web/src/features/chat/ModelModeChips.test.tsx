import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DaemonClient } from "../../api/client";
import type { ProvidersResponse } from "../../api/types";
import { useSessionManager } from "../../state/sessionManager";
import { ModelModeChips } from "./ModelModeChips";

const CATALOG: ProvidersResponse = {
  is_override: true,
  providers: [
    {
      key: "zhipu",
      display_name: "Zhipu",
      protocol: "openai",
      models: [
        { name: "glm-old", tier: "medium", configured: true, active: true },
        { name: "vendor/glm-5.3", tier: "heavy", configured: true, active: false },
      ],
    },
    {
      key: "kimi",
      display_name: "Kimi",
      protocol: "anthropic",
      models: [{ name: "k3", tier: "heavy", configured: true, active: false }],
    },
  ],
};

function createClient() {
  return {
    listModels: vi.fn().mockResolvedValue(CATALOG),
    switchModel: vi.fn().mockResolvedValue({
      success: true,
      label: "zhipu/vendor/glm-5.3",
      provider: "zhipu",
      model_name: "vendor/glm-5.3",
    }),
    resetSessionModel: vi.fn().mockResolvedValue({ success: true, session_id: "daemon-a" }),
    setPermissionMode: vi.fn().mockResolvedValue({
      success: true,
      mode: "yolo",
      effective_mode: "yolo",
      is_override: true,
    }),
    resetSessionMode: vi.fn().mockResolvedValue({
      success: true,
      mode: "accept_edits",
      effective_mode: "accept_edits",
      is_override: false,
      reset: true,
    }),
  };
}

type ClientMock = ReturnType<typeof createClient>;

function renderChips(client: ClientMock) {
  return render(<ModelModeChips client={client as unknown as DaemonClient} />);
}

function createSessions() {
  const manager = useSessionManager.getState();
  const a = manager.createLocalSession("a", { id: "local-a", daemonId: "daemon-a" });
  const b = manager.createLocalSession("b", {
    id: "local-b",
    daemonId: "daemon-b",
    activate: false,
  });
  manager.setSessionModel(a, { provider: "zhipu", model: "glm-old", isOverride: true });
  manager.setSessionMode(a, { mode: "normal", isOverride: false });
  manager.setSessionModel(b, { provider: "kimi", model: "k3", isOverride: false });
  manager.setSessionMode(b, { mode: "yolo", isOverride: true });
  return { a, b };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

async function openTargetModel(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
  );
  await user.click(await screen.findByRole("button", { name: /^Zhipu/ }));
  return screen.findByRole("button", { name: /vendor\/glm-5\.3.*heavy/ });
}

describe("ModelModeChips", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "connected",
      contextWindow: null,
    });
  });

  it("selects a provider then a slash-containing model for only the routed active session", async () => {
    const { a, b } = createSessions();
    const client = createClient();
    const user = userEvent.setup();
    renderChips(client);

    const target = await openTargetModel(user);
    expect(screen.getByRole("button", { name: /glm-old.*medium.*active/ })).toBeInTheDocument();
    await user.click(target);

    await waitFor(() =>
      expect(useSessionManager.getState().entries[a].model).toEqual({
        provider: "zhipu",
        model: "vendor/glm-5.3",
        isOverride: true,
      }),
    );
    expect(client.listModels).toHaveBeenCalledWith("daemon-a");
    expect(client.switchModel).toHaveBeenCalledWith({
      provider: "zhipu",
      model: "vendor/glm-5.3",
      sessionId: "daemon-a",
    });
    expect(useSessionManager.getState().entries[b].model).toEqual({
      provider: "kimi",
      model: "k3",
      isOverride: false,
    });
    expect(
      screen.getByRole("button", {
        name: "Session model: zhipu/vendor/glm-5.3 (session override)",
      }),
    ).toBeInTheDocument();
  });

  it("immediately reflects each entry's effective model and mode when active changes", async () => {
    const { b } = createSessions();
    renderChips(createClient());

    expect(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Permission mode: normal" })).toBeInTheDocument();

    await act(async () => useSessionManager.getState().setActive(b));

    expect(screen.getByRole("button", { name: "Session model: kimi/k3" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Permission mode: yolo (session override)" }),
    ).toBeInTheDocument();
  });

  it("resets the model through the daemon event fallback without touching another entry", async () => {
    const { a, b } = createSessions();
    const client = createClient();
    const user = userEvent.setup();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    );
    await user.click(await screen.findByRole("button", { name: "Follow default" }));

    await waitFor(() => expect(client.resetSessionModel).toHaveBeenCalledWith("daemon-a"));
    expect(useSessionManager.getState().entries[a].model).toEqual({
      provider: "zhipu",
      model: "glm-old",
      isOverride: true,
    });

    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: {
          session_id: "daemon-a",
          reset: true,
          provider: null,
          model_name: "k3-default",
        },
      });
    });

    expect(useSessionManager.getState().entries[a].model).toEqual({
      provider: null,
      model: "k3-default",
      isOverride: false,
    });
    expect(useSessionManager.getState().entries[b].model).toEqual({
      provider: "kimi",
      model: "k3",
      isOverride: false,
    });
    expect(screen.getByRole("button", { name: "Session model: k3-default" })).toBeInTheDocument();
  });

  it("sets and resets only the active entry mode using the response fallback", async () => {
    const { a, b } = createSessions();
    const client = createClient();
    const user = userEvent.setup();
    renderChips(client);

    await user.click(screen.getByRole("button", { name: "Permission mode: normal" }));
    await user.click(screen.getByRole("button", { name: "yolo" }));
    await waitFor(() =>
      expect(useSessionManager.getState().entries[a].mode).toEqual({
        mode: "yolo",
        isOverride: true,
      }),
    );
    expect(client.setPermissionMode).toHaveBeenCalledWith("daemon-a", "yolo");

    await user.click(
      screen.getByRole("button", { name: "Permission mode: yolo (session override)" }),
    );
    await user.click(screen.getByRole("button", { name: "Follow default" }));
    await waitFor(() =>
      expect(useSessionManager.getState().entries[a].mode).toEqual({
        mode: "accept_edits",
        isOverride: false,
      }),
    );
    expect(client.resetSessionMode).toHaveBeenCalledWith("daemon-a");
    expect(useSessionManager.getState().entries[b].mode).toEqual({
      mode: "yolo",
      isOverride: true,
    });
    expect(
      screen.getByRole("button", { name: "Permission mode: accept_edits" }),
    ).toBeInTheDocument();
  });

  it("shows a disabled model chip while the active store is running", async () => {
    const { a, b } = createSessions();
    const activeStore = useSessionManager.getState().entries[a].store;
    activeStore.getState().setRunning(true);
    renderChips(createClient());

    // Visible (which model is answering) but not switchable mid-turn.
    expect(screen.getByRole("button", { name: /^Session model:/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Permission mode: normal" })).toBeEnabled();

    act(() => activeStore.getState().setRunning(false));
    expect(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    ).toBeEnabled();

    // Session b is idle: its chip stays enabled even while a runs.
    act(() => activeStore.getState().setRunning(true));
    await act(async () => useSessionManager.getState().setActive(b));
    expect(screen.getByRole("button", { name: "Session model: kimi/k3" })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Permission mode: yolo (session override)" }),
    ).toBeEnabled();
  });

  it("closes an open picker when the session starts running", async () => {
    const { a } = createSessions();
    const client = createClient();
    const user = userEvent.setup();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    );
    expect(await screen.findByRole("button", { name: /^Zhipu/ })).toBeInTheDocument();

    const activeStore = useSessionManager.getState().entries[a].store;
    act(() => activeStore.getState().setRunning(true));
    expect(screen.queryByRole("button", { name: /^Zhipu/ })).not.toBeInTheDocument();
  });

  it("keeps the old model and an actionable error when switching fails", async () => {
    const { a } = createSessions();
    const client = createClient();
    client.switchModel.mockRejectedValueOnce(new Error("boom"));
    const user = userEvent.setup();
    renderChips(client);

    const target = await openTargetModel(user);
    await user.click(target);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Model switch failed: boom. Try again.",
    );
    expect(useSessionManager.getState().entries[a].model).toEqual({
      provider: "zhipu",
      model: "glm-old",
      isOverride: true,
    });
    expect(await screen.findByRole("button", { name: /vendor\/glm-5\.3.*heavy/ })).toBeEnabled();
  });

  it("retries a failed model reset instead of reloading the catalog", async () => {
    const client = createClient();
    client.resetSessionModel.mockRejectedValueOnce(new Error("reset boom"));
    const user = userEvent.setup();
    createSessions();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    );
    await user.click(await screen.findByRole("button", { name: "Follow default" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Model reset failed: reset boom. Try again.",
    );
    expect(client.resetSessionModel).toHaveBeenCalledTimes(1);
    expect(client.listModels).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => expect(client.resetSessionModel).toHaveBeenCalledTimes(2));
    expect(client.listModels).toHaveBeenCalledTimes(1);
  });

  it("retries a failed catalog load without resetting the model", async () => {
    const client = createClient();
    client.listModels.mockRejectedValueOnce(new Error("catalog boom"));
    const user = userEvent.setup();
    createSessions();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Model list failed: catalog boom. Try again.",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => expect(client.listModels).toHaveBeenCalledTimes(2));
    expect(client.resetSessionModel).not.toHaveBeenCalled();
    expect(await screen.findByRole("button", { name: /^Zhipu/ })).toBeInTheDocument();
  });

  it("closes a stale picker and ignores its late catalog response after active changes", async () => {
    const { b } = createSessions();
    const client = createClient();
    const pending = deferred<ProvidersResponse>();
    client.listModels.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Session model: zhipu/glm-old (session override)" }),
    );
    await act(async () => useSessionManager.getState().setActive(b));
    await act(async () => {
      pending.resolve(CATALOG);
      await pending.promise;
    });

    expect(screen.queryByRole("button", { name: /^Zhipu/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Session model: kimi/k3" })).toBeInTheDocument();
    expect(client.listModels).toHaveBeenCalledTimes(1);
    expect(client.switchModel).not.toHaveBeenCalled();
  });

  it("writes a late switch response to its originating entry, not the new active entry", async () => {
    const { a, b } = createSessions();
    const client = createClient();
    const pending = deferred<{
      success: boolean;
      label: string;
      provider: string;
      model_name: string;
    }>();
    client.switchModel.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    renderChips(client);

    const target = await openTargetModel(user);
    await user.click(target);
    await waitFor(() => expect(client.switchModel).toHaveBeenCalledTimes(1));
    await act(async () => useSessionManager.getState().setActive(b));
    await act(async () => {
      pending.resolve({
        success: true,
        label: "zhipu/vendor/glm-5.3",
        provider: "zhipu",
        model_name: "vendor/glm-5.3",
      });
      await pending.promise;
    });

    await waitFor(() =>
      expect(useSessionManager.getState().entries[a].model).toEqual({
        provider: "zhipu",
        model: "vendor/glm-5.3",
        isOverride: true,
      }),
    );
    expect(useSessionManager.getState().entries[b].model).toEqual({
      provider: "kimi",
      model: "k3",
      isOverride: false,
    });
    expect(screen.getByRole("button", { name: "Session model: kimi/k3" })).toBeInTheDocument();
  });

  it("keeps a newer model event when an old switch response arrives after A to B to A", async () => {
    const { a, b } = createSessions();
    const client = createClient();
    const pending = deferred<{
      success: boolean;
      label: string;
      provider: string;
      model_name: string;
    }>();
    client.switchModel.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    renderChips(client);

    const target = await openTargetModel(user);
    await user.click(target);
    await waitFor(() => expect(client.switchModel).toHaveBeenCalledTimes(1));
    await act(async () => useSessionManager.getState().setActive(b));
    await act(async () => useSessionManager.getState().setActive(a));
    act(() => {
      useSessionManager.getState().applyModelChanged({
        seq: 1,
        kind: "model_changed",
        data: {
          session_id: "daemon-a",
          reset: false,
          provider: "kimi",
          model_name: "newer-model",
        },
      });
    });
    await act(async () => {
      pending.resolve({
        success: true,
        label: "zhipu/vendor/glm-5.3",
        provider: "zhipu",
        model_name: "vendor/glm-5.3",
      });
      await pending.promise;
    });

    expect(useSessionManager.getState().entries[a].model).toEqual({
      provider: "kimi",
      model: "newer-model",
      isOverride: true,
    });
    expect(useSessionManager.getState().entries[b].model).toEqual({
      provider: "kimi",
      model: "k3",
      isOverride: false,
    });
  });

  it("keeps a newer mode event when an old set response arrives", async () => {
    const { a } = createSessions();
    const client = createClient();
    const pending = deferred<{
      success: boolean;
      mode: "yolo";
      effective_mode: "yolo";
      is_override: boolean;
    }>();
    client.setPermissionMode.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    renderChips(client);

    await user.click(screen.getByRole("button", { name: "Permission mode: normal" }));
    await user.click(screen.getByRole("button", { name: "yolo" }));
    await waitFor(() => expect(client.setPermissionMode).toHaveBeenCalledTimes(1));
    act(() => {
      useSessionManager.getState().applyModeChanged({
        seq: 1,
        kind: "mode_changed",
        data: {
          session_id: "daemon-a",
          mode: "accept_edits",
          is_override: true,
        },
      });
    });
    await act(async () => {
      pending.resolve({
        success: true,
        mode: "yolo",
        effective_mode: "yolo",
        is_override: true,
      });
      await pending.promise;
    });

    expect(useSessionManager.getState().entries[a].mode).toEqual({
      mode: "accept_edits",
      isOverride: true,
    });
  });

  it("keeps a newer mode event when an old reset response arrives", async () => {
    const { a } = createSessions();
    useSessionManager.getState().setSessionMode(a, { mode: "normal", isOverride: true });
    const client = createClient();
    const pending = deferred<{
      success: boolean;
      mode: "accept_edits";
      effective_mode: "accept_edits";
      is_override: boolean;
      reset: boolean;
    }>();
    client.resetSessionMode.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    renderChips(client);

    await user.click(
      screen.getByRole("button", { name: "Permission mode: normal (session override)" }),
    );
    await user.click(screen.getByRole("button", { name: "Follow default" }));
    await waitFor(() => expect(client.resetSessionMode).toHaveBeenCalledTimes(1));
    act(() => {
      useSessionManager.getState().applyModeChanged({
        seq: 1,
        kind: "mode_changed",
        data: {
          session_id: "daemon-a",
          mode: "yolo",
          is_override: true,
        },
      });
    });
    await act(async () => {
      pending.resolve({
        success: true,
        mode: "accept_edits",
        effective_mode: "accept_edits",
        is_override: false,
        reset: true,
      });
      await pending.promise;
    });

    expect(useSessionManager.getState().entries[a].mode).toEqual({
      mode: "yolo",
      isOverride: true,
    });
  });

  it("renders nothing without an active session", () => {
    const { container } = renderChips(createClient());
    expect(container).toBeEmptyDOMElement();
  });
});
