import { useState, type KeyboardEvent } from "react";
import { Send, Square } from "lucide-react";
import { useSessionStore } from "../state/sessionContext";

interface ComposerProps {
  onSend: (text: string) => void;
}

/** Message input. Enter sends; Shift+Enter inserts a newline. */
export function Composer({ onSend }: ComposerProps) {
  const [text, setText] = useState("");
  const isRunning = useSessionStore((s) => s.isRunning);
  const stopRunning = useSessionStore((s) => s.stopRunning);

  const send = () => {
    const trimmed = text.trim();
    if (!trimmed || isRunning) return;
    onSend(trimmed);
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
      <textarea
        className="composer-input"
        placeholder={
          isRunning
            ? "Agent is working…"
            : "Message the agent (Enter to send, Shift+Enter for newline)"
        }
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        rows={3}
        disabled={isRunning}
      />
      {isRunning ? (
        <button type="button" className="btn btn-danger composer-send" onClick={stopRunning}>
          <Square size={14} /> Stop
        </button>
      ) : (
        <button
          type="button"
          className="btn btn-primary composer-send"
          onClick={send}
          disabled={!text.trim()}
        >
          <Send size={14} /> Send
        </button>
      )}
    </div>
  );
}
