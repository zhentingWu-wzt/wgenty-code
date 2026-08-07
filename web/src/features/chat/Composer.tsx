import { useState, type KeyboardEvent } from "react";
import { Send, Square } from "lucide-react";
import { useSessionStore } from "../../state/sessionContext";
import { Button } from "../../components/ui/button";
import {
  filterSlashCommands,
  matchSlashCommand,
  type SlashCommand,
} from "../../components/slashCommands";

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
    <div className="relative border-t border-border bg-background p-3">
      {menuItems.length > 0 && (
        <div
          role="listbox"
          aria-label="Slash commands"
          className="absolute inset-x-3 bottom-full z-20 mb-1.5 overflow-hidden rounded-md border border-border bg-popover shadow-lg"
        >
          {menuItems.map((c) => (
            <button
              key={c.name}
              type="button"
              role="option"
              aria-selected="false"
              className="flex w-full items-baseline gap-2 px-3 py-1.5 text-left text-[13px] text-foreground hover:bg-accent"
              onClick={() => pick(c)}
            >
              <span className="shrink-0 font-mono text-primary">{c.name}</span>
              <span className="text-[12px] text-muted-foreground">{c.description}</span>
            </button>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2 rounded-lg border border-input bg-card px-3 py-2 focus-within:ring-1 focus-within:ring-ring">
        <textarea
          className="max-h-40 min-h-[20px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed outline-none placeholder:text-muted-foreground"
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
          <Button variant="destructive" className="shrink-0" onClick={onStop}>
            <Square size={13} /> Stop
          </Button>
        ) : (
          <Button className="shrink-0" onClick={send} disabled={!text.trim()}>
            <Send size={13} /> Send
          </Button>
        )}
      </div>
    </div>
  );
}
