import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ReasoningBlock } from "./ChatView";

describe("ReasoningBlock", () => {
  it("collapses long reasoning by default and expands on click", async () => {
    render(<ReasoningBlock text={"very long hidden trace"} live={false} />);

    // Header shows the label + char count; the trace itself is hidden.
    expect(screen.getByRole("button", { name: /reasoning/ })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByText(/\d+ chars/)).toBeInTheDocument();
    expect(screen.queryByText("very long hidden trace")).not.toBeInTheDocument();

    // Expand reveals the trace.
    await userEvent.click(screen.getByRole("button"));
    expect(screen.getByRole("button", { name: /reasoning/ })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByText("very long hidden trace")).toBeInTheDocument();
  });

  it("shows the live thinking pulse while streaming", () => {
    render(<ReasoningBlock text={"partial"} live />);
    expect(screen.getByText("thinking")).toBeInTheDocument();
  });
});
