import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPanel } from "./ModelPanel";
import { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";

const client = new DaemonClient();

interface ModelRouteOptions {
  providers: Record<string, string[]>;
  active: { provider: string; model: string };
  switchError?: string;
}

function stubModelsFetch(opts: ModelRouteOptions) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/__daemon-info") return new Response("not found", { status: 404 });
    if (url === "/api/v1/models") {
      return Response.json({
        providers: Object.entries(opts.providers).map(([key, models]) => ({
          key,
          display_name: key.toUpperCase(),
          protocol: "openai",
          models: models.map((name) => ({
            name,
            tier: null,
            configured: true,
            active: key === opts.active.provider && name === opts.active.model,
          })),
        })),
        is_override: false,
      });
    }
    if (url === "/api/v1/model/switch" && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as { provider: string; model: string };
      if (opts.switchError) return new Response(opts.switchError, { status: 400 });
      if (!opts.providers[body.provider]?.includes(body.model)) {
        return new Response(`unknown model '${body.provider}/${body.model}'`, { status: 400 });
      }
      opts.active = { provider: body.provider, model: body.model };
      return Response.json({
        success: true,
        label: `${body.provider.toUpperCase()}/${body.model}`,
        model_name: body.model,
        provider: body.provider,
      });
    }
    return new Response("not found", { status: 404 });
  });
}

function resetStores() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    contextWindow: null,
  });
}

function createActiveEntry() {
  const id = useSessionManager.getState().createLocalSession("active");
  useSessionManager.setState((state) => ({
    entries: {
      ...state.entries,
      [id]: {
        ...state.entries[id],
        model: { provider: "zhipu", model: "glm-5.3", isOverride: false },
      },
    },
  }));
  return id;
}

describe("ModelPanel", () => {
  beforeEach(resetStores);
  afterEach(() => vi.unstubAllGlobals());

  it("shows a configure hint when no provider models are declared", async () => {
    vi.stubGlobal("fetch", stubModelsFetch({ providers: {}, active: { provider: "", model: "" } }));
    render(<ModelPanel client={client} />);

    expect(await screen.findByText(/No models configured/i)).toBeInTheDocument();
    expect(screen.queryByText(/Loading models/i)).not.toBeInTheDocument();
    expect(screen.getByText(/models\.providers/)).toBeInTheDocument();
  });

  it("uses provider/model for a global switch and updates unoverridden active state", async () => {
    const id = createActiveEntry();
    const opts = {
      providers: { zhipu: ["glm-5.3"], deepseek: ["deepseek-chat"] },
      active: { provider: "zhipu", model: "glm-5.3" },
    };
    const spy = stubModelsFetch(opts);
    vi.stubGlobal("fetch", spy);
    render(<ModelPanel client={client} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: /DEEPSEEK/ }));

    const post = spy.mock.calls.find(
      (c) => String(c[0]) === "/api/v1/model/switch" && c[1]?.method === "POST",
    );
    expect(JSON.parse(String(post![1]!.body))).toEqual({
      provider: "deepseek",
      model: "deepseek-chat",
    });
    await waitFor(() =>
      expect(useSessionManager.getState().entries[id].model).toEqual({
        provider: "deepseek",
        model: "deepseek-chat",
        isOverride: false,
      }),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /DEEPSEEK/ })).toHaveAttribute("disabled"),
    );
    expect(screen.getByRole("button", { name: /ZHIPU/ })).toBeEnabled();
  });

  it("surfaces the daemon error without changing session model state", async () => {
    const id = createActiveEntry();
    vi.stubGlobal(
      "fetch",
      stubModelsFetch({
        providers: { zhipu: ["glm-5.3"], deepseek: ["deepseek-chat"] },
        active: { provider: "zhipu", model: "glm-5.3" },
        switchError: "unknown model 'deepseek/gone'",
      }),
    );
    render(<ModelPanel client={client} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: /DEEPSEEK/ }));

    expect(await screen.findByText(/unknown model 'deepseek\/gone'/)).toBeInTheDocument();
    expect(useSessionManager.getState().entries[id].model).toEqual({
      provider: "zhipu",
      model: "glm-5.3",
      isOverride: false,
    });
  });
});
