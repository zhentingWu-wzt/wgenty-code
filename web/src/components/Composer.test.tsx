import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Composer } from "./Composer";
import { SessionStoreContext } from "../state/sessionContext";
import { createSessionStore, type SessionStore } from "../state/sessionStore";
import type { SlashCommand } from "./slashCommands";

function renderComposer(
  onSend: (t: string) => void,
  store: SessionStore = createSessionStore(),
  onCommand: (cmd: SlashCommand) => void = () => {},
) {
  return render(
    <SessionStoreContext.Provider value={store}>
      <Composer onSend={onSend} onCommand={onCommand} />
    </SessionStoreContext.Provider>,
  );
}

describe("Composer", () => {
  it("sends the trimmed message on Enter and clears the input", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend);

    const input = screen.getByRole("textbox");
    await user.type(input, "  hello agent  ");
    await user.keyboard("{Enter}");

    expect(onSend).toHaveBeenCalledWith("hello agent");
    expect(input).toHaveValue("");
  });

  it("inserts a newline on Shift+Enter instead of sending", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend);

    const input = screen.getByRole("textbox");
    await user.type(input, "line one{Shift>}{Enter}{/Shift}line two");

    expect(onSend).not.toHaveBeenCalled();
    expect(input).toHaveValue("line one\nline two");
  });

  it("disables Send when the input is empty or whitespace", async () => {
    const user = userEvent.setup();
    renderComposer(vi.fn());

    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();

    await user.type(screen.getByRole("textbox"), "   ");
    expect(send).toBeDisabled();
  });

  it("shows Stop instead of Send while a turn is running", () => {
    const store = createSessionStore();
    store.getState().setRunning(true);
    renderComposer(vi.fn(), store);

    expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send" })).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeDisabled();
  });
});

describe("Composer slash commands", () => {
  it("shows the command menu while typing a slash prefix", async () => {
    const user = userEvent.setup();
    renderComposer(vi.fn());

    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox"), "/m");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /\/model/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /\/memory/ })).toBeInTheDocument();
  });

  it("Enter on an exact command fires onCommand, not onSend", async () => {
    const onSend = vi.fn();
    const onCommand = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend, undefined, onCommand);

    const input = screen.getByRole("textbox");
    await user.type(input, "/model");
    await user.keyboard("{Enter}");

    expect(onCommand).toHaveBeenCalledWith(expect.objectContaining({ name: "/model" }));
    expect(onSend).not.toHaveBeenCalled();
    expect(input).toHaveValue("");
  });

  it("clicking a menu item fires onCommand and clears the input", async () => {
    const onCommand = vi.fn();
    const user = userEvent.setup();
    renderComposer(vi.fn(), undefined, onCommand);

    const input = screen.getByRole("textbox");
    await user.type(input, "/se");
    await user.click(screen.getByRole("option", { name: /\/sessions/ }));

    expect(onCommand).toHaveBeenCalledWith(expect.objectContaining({ name: "/sessions" }));
    expect(input).toHaveValue("");
  });
});
