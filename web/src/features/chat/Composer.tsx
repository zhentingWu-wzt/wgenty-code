import { useState, type KeyboardEvent } from "react";
import { Clock, Send, Square, X } from "lucide-react";
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
 *
 * While a turn runs, sends are queued (the parent enqueues them). Queued
 * messages render as an editable list above the input so the user can tweak
 * or remove them before they go out.
 */
export function Composer({ onSend, onStop, onCommand }: ComposerProps) {
  const [text, setText] = useState("");
  const isRunning = useSessionStore((s) => s.isRunning);
  const pendingInputs = useSessionStore((s) => s.pendingInputs);
  const editPendingInput = useSessionStore((s) => s.editPendingInput);
  const removePendingInput = useSessionStore((s) => s.removePendingInput);

  const menuItems = filterSlashCommands(text);

  const send = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    // Exact slash command → open its modal, don't send.
    const cmd = matchSlashCommand(trimmed);
    if (cmd) {
      if (cmd.send) {
        // Skill commands (/skill-name) are agent-side — send as a message.
        onSend(trimmed);
      } else {
        onCommand(cmd);
      }
      setText("");
      return;
    }
    onSend(trimmed);
    setText("");
  };

  const pick = (cmd: SlashCommand) => {
    if (cmd.send) {
      onSend(cmd.name);
    } else {
      onCommand(cmd);
    }
    setText("");
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // While an IME composition is active (e.g. Chinese/Japanese/Korean input),
    // Enter confirms the candidate rather than sending the message.
    if (e.nativeEvent.isComposing) return;
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
          // Cap the menu height (half the dynamic viewport) and scroll
          // inside — with many skills registered the unbounded list could
          // cover the whole screen, hiding the chat behind it.
          className="absolute inset-x-3 bottom-full z-20 mb-1.5 max-h-[50dvh] overflow-y-auto overscroll-contain rounded-md border border-border bg-popover shadow-lg"
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
      {pendingInputs.length > 0 && (
        <div className="mb-2 space-y-1.5">
          <div className="flex items-center gap-1.5 px-0.5 text-[11px] text-muted-foreground">
            <Clock size={12} />
            <span>
              {pendingInputs.length} queued — sends when the current turn finishes
            </span>
          </div>
          {pendingInputs.map((qText, i) => (
            <div
              key={i}
              className="flex items-start gap-2 rounded-md border border-dashed border-border bg-muted/40 px-2 py-1.5"
            >
              <textarea
                className="max-h-32 min-h-[18px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed outline-none"
                value={qText}
                onChange={(e) => editPendingInput(i, e.target.value)}
                rows={1}
                aria-label={`Queued message ${i + 1}`}
              />
              <button
                type="button"
                className="mt-0.5 shrink-0 text-muted-foreground hover:text-destructive"
                onClick={() => removePendingInput(i)}
                aria-label="Remove queued message"
              >
                <X size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2 rounded-lg border border-input bg-card px-3 py-2 focus-within:ring-1 focus-within:ring-ring">
        <textarea
          className="max-h-40 min-h-[20px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed outline-none placeholder:text-muted-foreground"
          placeholder={
            isRunning
              ? "Agent is working… type to queue your next message"
              : "Message the agent (Enter to send, Shift+Enter for newline, / for commands)"
          }
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          rows={3}
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
