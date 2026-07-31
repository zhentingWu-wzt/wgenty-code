import { useState } from "react";
import { DaemonClient, DaemonError } from "../api/client";
import type { ChatMessage, SessionResponse } from "../api/types";
import { usePolling } from "../hooks/usePolling";
import { useSidebarStore } from "../state/sidebarStore";
import { useChatStore } from "../state/chatStore";
import type { DisplayMessage } from "../state/chatStore";

const POLL_MS = 10000;

/**
 * Session list + search + open/new/delete.
 *
 * Opening a session replaces the chat view with its history (converted to
 * DisplayMessages). New sessions are created on demand; the current in-memory
 * conversation can be saved back with the Save button.
 */
export function SessionsPanel({ client }: { client: DaemonClient }) {
  const sessions = useSidebarStore((s) => s.sessions);
  const setSessions = useSidebarStore((s) => s.setSessions);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Poll the session list so newly saved sessions appear.
  usePolling(
    async () => {
      const res = query
        ? await client.listSessions().then((all) =>
            all.filter((s) => s.name.toLowerCase().includes(query.toLowerCase())),
          )
        : await client.listSessions();
      setSessions(res);
    },
    true,
    POLL_MS,
  );

  const replaceChatWithSession = (session: SessionResponse) => {
    const display: DisplayMessage[] = session.messages.map((m, i) => ({
      id: `loaded-${session.id}-${i}`,
      role: (m.role === "user" ? "user" : m.role === "assistant" ? "assistant" : "tool") as
        | "user"
        | "assistant"
        | "tool",
      content: m.content ?? "",
      ...(m.tool_calls && m.tool_calls.length > 0 ? { toolExecs: [] } : {}),
    }));
    useChatStore.getState().clear();
    // Push loaded messages directly into the store.
    for (const d of display) useChatStore.getState().pushLoadedMessage(d);
    void session;
  };

  const onOpen = async (id: string) => {
    setBusy(id);
    setError(null);
    try {
      const session = await client.loadSession(id);
      replaceChatWithSession(session);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const onDelete = async (id: string) => {
    if (!confirm("Delete this session?")) return;
    setBusy(id);
    setError(null);
    try {
      await client.deleteSession(id);
      setSessions(sessions.filter((s) => s.id !== id));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const onNew = async () => {
    setBusy("new");
    setError(null);
    try {
      await client.createSession({});
      const all = await client.listSessions();
      setSessions(all);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const onSaveCurrent = async () => {
    setBusy("save");
    setError(null);
    try {
      const msgs = useChatStore.getState().messages;
      const wire: ChatMessage[] = toWireForSave(msgs);
      const created = await client.createSession({});
      await client.saveSession(created.id, { messages: wire, ui_messages: [] });
      const all = await client.listSessions();
      setSessions(all);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="sessions-panel">
      <div className="sessions-toolbar">
        <input
          className="sessions-search"
          placeholder="Search sessions…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="button" className="btn btn-xs" onClick={onNew} disabled={busy !== null}>
          New
        </button>
        <button type="button" className="btn btn-xs" onClick={onSaveCurrent} disabled={busy !== null}>
          Save current
        </button>
      </div>
      {error && <div className="panel-error">{error}</div>}
      {sessions.length === 0 ? (
        <div className="panel-empty">No sessions.</div>
      ) : (
        <ul className="session-list">
          {sessions.map((s) => (
            <li key={s.id} className="session-item">
              <div className="session-main">
                <button
                  type="button"
                  className="session-open"
                  onClick={() => onOpen(s.id)}
                  disabled={busy !== null}
                >
                  <span className="session-name">{s.name || "(untitled)"}</span>
                  <span className="session-meta">
                    {s.message_count} msgs · {new Date(s.updated_at).toLocaleDateString()}
                  </span>
                </button>
              </div>
              <button
                type="button"
                className="btn btn-xs btn-danger"
                onClick={() => onDelete(s.id)}
                disabled={busy !== null}
                title="Delete session"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function toWireForSave(display: DisplayMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const m of display) {
    if (m.role === "user") {
      out.push({ role: "user", content: m.content });
    } else if (m.role === "assistant") {
      out.push({
        role: "assistant",
        content: m.content || undefined,
        ...(m.toolExecs && m.toolExecs.length > 0
          ? { tool_calls: m.toolExecs.map((e) => e.call) }
          : {}),
      });
    }
  }
  return out;
}

// Re-export so callers can pattern-match on daemon errors if needed.
export { DaemonError };
