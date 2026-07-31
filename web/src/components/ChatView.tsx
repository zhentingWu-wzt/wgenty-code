import { useEffect, useRef } from "react";
import { useChatStore } from "../state/chatStore";
import { ToolCallCard } from "./ToolCallCard";

/** Scrolling message list. Auto-scrolls to bottom while streaming. */
export function ChatView() {
  const messages = useChatStore((s) => s.messages);
  const lastError = useChatStore((s) => s.lastError);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, lastError]);

  if (messages.length === 0) {
    return (
      <div className="chat-empty">
        <p>Send a message to start.</p>
        <p className="chat-empty-hint">
          Read-only requests (e.g. “summarize README.md”) need no approval. File-writing requests
          will pop up a permission dialog.
        </p>
      </div>
    );
  }

  return (
    <div className="chat-list">
      {messages.map((m) => (
        <div key={m.id} className={`msg msg-${m.role}`}>
          <div className="msg-role">
            {m.role}
            {m.round && m.round > 1 ? ` · round ${m.round}` : ""}
            {m.streaming ? " · …" : ""}
          </div>
          {m.reasoning && <pre className="msg-reasoning">{m.reasoning}</pre>}
          {m.content && <div className="msg-content">{m.content}</div>}
          {m.toolExecs && m.toolExecs.length > 0 && (
            <div className="msg-tools">
              {m.toolExecs.map((exec, i) => (
                <ToolCallCard key={i} exec={exec} />
              ))}
            </div>
          )}
        </div>
      ))}
      {lastError && <div className="msg-error">{lastError}</div>}
      <div ref={bottomRef} />
    </div>
  );
}
