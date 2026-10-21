import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InspectorPanel } from "./InspectorPanel";
import { useSessionManager } from "../../state/sessionManager";
import type { TurnContextData } from "../../state/sessionStore";

function makeTurnContext(cachedTokens: number | null): TurnContextData {
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

    expect(screen.getByText("Cached")).toBeInTheDocument();
    expect(screen.getByText("8,000 (80%)")).toBeInTheDocument();
  });

  it("shows an em dash when the gateway does not report cache hits", async () => {
    const user = userEvent.setup();
    renderWithTurnContext(makeTurnContext(null));

    await user.click(screen.getByRole("button", { name: "Tokens" }));

    expect(screen.getByText("Cached")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });
});
