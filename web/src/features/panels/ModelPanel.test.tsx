import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPanel } from "./ModelPanel";
import { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";

const client = new DaemonClient();

interface ModelRouteOptions {
  /** Profiles as key → model_name. */
  profiles: Record<string, string>;
  /** Which profile key is active ("" = none). Mutated on successful switch. */
  active: { key: string };
  /** When set, POST /model/switch responds 400 with this text. */
  switchError?: string;
}

/** Stubbed global fetch: 404s `/__daemon-info` (so authedFetch falls back to
 *  the plain same-origin path) and serves `/api/v1/models` + `/model/switch`,
 *  mirroring the daemon's behavior. */
function stubModelsFetch(opts: ModelRouteOptions) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/__daemon-info") return new Response("not found", { status: 404 });
    if (url === "/api/v1/models") {
      const payload = Object.entries(opts.profiles).map(([key, name]) => ({
        key,
        label: key.toUpperCase(),
        model_name: name,
        active: key === opts.active.key,
      }));
      return Response.json({ profiles: payload });
    }
    if (url === "/api/v1/model/switch" && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as { profile: string };
      if (opts.switchError) return new Response(opts.switchError, { status: 400 });
      if (!(body.profile in opts.profiles)) {
        return new Response(`unknown model profile '${body.profile}'`, { status: 400 });
      }
      opts.active.key = body.profile;
      return Response.json({
        success: true,
        profile: body.profile,
        label: body.profile.toUpperCase(),
        model_name: opts.profiles[body.profile],
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
    modelName: null,
  });
}

describe("ModelPanel", () => {
  beforeEach(resetStores);
  afterEach(() => vi.unstubAllGlobals());

  it("shows a configure hint (not a forever-loading state) when no profiles are declared", async () => {
    vi.stubGlobal("fetch", stubModelsFetch({ profiles: {}, active: { key: "" } }));
    render(<ModelPanel client={client} />);

    expect(await screen.findByText(/No model profiles configured/i)).toBeInTheDocument();
    expect(screen.queryByText(/Loading models/i)).not.toBeInTheDocument();
    expect(screen.getByText(/models\.profiles/)).toBeInTheDocument();
  });

  it("switches to a clicked profile and moves the active marker", async () => {
    const opts = { profiles: { glm: "glm-5.3", fast: "deepseek-chat" }, active: { key: "glm" } };
    const spy = stubModelsFetch(opts);
    vi.stubGlobal("fetch", spy);
    render(<ModelPanel client={client} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: /FAST/ }));

    const post = spy.mock.calls.find(
      (c) => String(c[0]) === "/api/v1/model/switch" && c[1]?.method === "POST",
    );
    expect(JSON.parse(String(post![1]!.body))).toEqual({ profile: "fast" });
    // Store updated for the status bar…
    await waitFor(() => expect(useSessionManager.getState().modelName).toBe("deepseek-chat"));
    // …and the refreshed list re-marks the switched profile as active.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /FAST/ })).toHaveAttribute("disabled"),
    );
    expect(screen.getByRole("button", { name: /GLM/ })).toBeEnabled();
  });

  it("surfaces the daemon's error when a switch fails", async () => {
    vi.stubGlobal(
      "fetch",
      stubModelsFetch({
        profiles: { glm: "glm-5.3", fast: "deepseek-chat" },
        active: { key: "glm" },
        switchError: "unknown model profile 'gone'",
      }),
    );
    render(<ModelPanel client={client} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: /FAST/ }));

    expect(await screen.findByText(/unknown model profile 'gone'/)).toBeInTheDocument();
    // Failed switch must not touch the store.
    expect(useSessionManager.getState().modelName).toBeNull();
  });
});
