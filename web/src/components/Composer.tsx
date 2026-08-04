import { useState, type KeyboardEvent } from "react";
import { Send, Square } from "lucide-react";
import { useSessionStore } from "../state/sessionContext";
import { filterSlashCommands, matchSlashCommand, type SlashCommand } from "./slashCommands";

interface ComposerProps {
  onSend: (text: string) => void;
  /** Stop the active server-side run (POST /cancel). */
  onStop: () => void;
  /** Fired when the input is an exact slash command (e.g. /model) — the App
   *  opens the corresponding modal instead of sending a message. */
  onCommand: (cmd: SlashCommand) => void;
}

/**
 * Message input. Enter sends; Shift+Enter inserts a newline. Typing `/` opens
 * the slash-command menu (TUI-style); Enter on an exact command opens its
 * modal via `onCommand` instead of sending.
 */
export function Composer({ onSend, onStop, onCommand }: ComposerProps) {
  const [text, setText] = useState("");
  const isRunning = useSessionStore((s) => s.isRunning);

  const menuItems = filterSlashCommands(text);

  const send = () => {
    const trimmed = text.trim();
    if (!trimmed || isRunning) return;
    // Exact slash command → open its modal, don't send.
    const cmd = matchSlashCommand(trimmed);
    if (cmd) {
      onCommand(cmd);
      setText("");
      return;
    }
    onSend(trimmed);
    setText("");
  };

  const pick = (cmd: SlashCommand) => {
    onCommand(cmd);
    setText("");
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  return (
    <div className="composer">
      {menuItems.length > 0 && (
        <div className="slash-menu" role="listbox" aria-label="Slash commands">
          {menuItems.map((c) => (
            <button
              key={c.name}
              type="button"
              role="option"
              aria-selected="false"
              className="slash-item"
              onClick={() => pick(c)}
            >
              <span className="slash-name">{c.name}</span>
              <span className="slash-desc">{c.description}</span>
            </button>
          ))}
        </div>
      )}
      <textarea
        className="composer-input"
        placeholder={
          isRunning
            ? "Agent is working…"
            : "Message the agent (Enter to send, Shift+Enter for newline, / for commands)"
        }
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        rows={3}
        disabled={isRunning}
      />
      {isRunning ? (
        <button type="button" className="btn btn-danger composer-send" onClick={onStop}>
          <Square size={13} /> Stop
        </button>
      ) : (
        <button
          type="button"
          className="btn btn-primary composer-send"
          onClick={send}
          disabled={!text.trim()}
        >
          <Send size={13} /> Send
        </button>
      )}
    </div>
  );
}
