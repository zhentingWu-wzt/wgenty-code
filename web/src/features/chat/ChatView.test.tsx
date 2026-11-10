import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ReasoningBlock } from "./ChatView";

describe("ReasoningBlock", () => {
  it("collapsed header shows only the latest reasoning line", async () => {
    render(<ReasoningBlock text={"earlier thought\nlatest thought"} live={false} />);

    // Latest line is visible as the live tail; earlier lines stay hidden.
    expect(screen.getByText("latest thought")).toBeInTheDocument();
    expect(screen.queryByText("earlier thought")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /reasoning/ })).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    // Expand reveals the full trace (single <pre> text node).
    await userEvent.click(screen.getByRole("button"));
    const pre = document.querySelector("pre");
    expect(pre).not.toBeNull();
    expect(pre).toHaveTextContent("earlier thought");
    expect(pre).toHaveTextContent("latest thought");
  });

  it("tolerates trailing newlines when picking the tail line", () => {
    render(<ReasoningBlock text={"a thought\n\n"} live={false} />);
    expect(screen.getByText("a thought")).toBeInTheDocument();
  });

  it("shows the live thinking pulse while streaming", () => {
    render(<ReasoningBlock text={"partial"} live />);
    expect(screen.getByText("thinking")).toBeInTheDocument();
  });
});
