import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InspectorPanel } from "./InspectorPanel";
import { useSessionManager } from "../../state/sessionManager";
import type { TurnContextData } from "../../state/sessionStore";

function makeTurnContext(
  cachedTokens: number | null,
  sessionCacheTotals?: { prompt_tokens: number; cached_tokens: number; cache_write_tokens: number },
): TurnContextData {
  return {
    layers: [],
    recalled_memories: [],
    new_messages: [],
    reminder: null,
    usage: {
      prompt_tokens: 10000,
      completion_tokens: 500,
      total_tokens: 10500,
      cached_tokens: cachedTokens,
      ...(sessionCacheTotals ? { session_cache_totals: sessionCacheTotals } : {}),
    },
  };
}

function renderWithTurnContext(turnContext: TurnContextData) {
  const id = useSessionManager.getState().createLocalSession("s1");
  useSessionManager.setState({ activeId: id });
  useSessionManager.getState().entries[id].store.setState({ turnContext });
  return render(<InspectorPanel />);
}

describe("InspectorPanel TokensTab", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "unknown",
      modelName: null,
    });
  });

  it("shows cached tokens with percentage of prompt when the gateway reports hits", async () => {
    const user = userEvent.setup();
    renderWithTurnContext(makeTurnContext(8000));

    await user.click(screen.getByRole("button", { name: "Tokens" }));

    expect(screen.getByText("Cached (last call)")).toBeInTheDocument();
    expect(screen.getByText("8,000 (80%)")).toBeInTheDocument();
  });

  it("shows an em dash when the gateway does not report cache hits", async () => {
    const user = userEvent.setup();
    renderWithTurnContext(makeTurnContext(null));

    await user.click(screen.getByRole("button", { name: "Tokens" }));

    expect(screen.getByText("Cached (last call)")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("shows the session cache-economy card with the weighted hit rate", async () => {
    const user = userEvent.setup();
    renderWithTurnContext(
      makeTurnContext(10500, {
        prompt_tokens: 200_000,
        cached_tokens: 185_000,
        cache_write_tokens: 2_500,
      }),
    );

    await user.click(screen.getByRole("button", { name: "Tokens" }));

    expect(screen.getByText("Session cache economy (weighted)")).toBeInTheDocument();
    // Σ185k / Σ200k = 92.5% → rounds to 93% (token-weighted, not the mean
    // of per-call percentages).
    expect(screen.getByText("93% hit")).toBeInTheDocument();
    expect(screen.getByText("185,000")).toBeInTheDocument();
    expect(screen.getByText("2,500")).toBeInTheDocument();
  });

  it("omits the economy card when the daemon does not report totals", async () => {
    const user = userEvent.setup();
    renderWithTurnContext(makeTurnContext(8000));

    await user.click(screen.getByRole("button", { name: "Tokens" }));

    expect(screen.queryByText("Session cache economy (weighted)")).not.toBeInTheDocument();
  });
});
