# Session Management — Design Spec

## Overview

Add full session management to the project: save/restore/named sessions, session list, and search. The daemon serves as the storage layer via REST API; the TypeScript frontend owns conversation state and calls the daemon for persistence.

## Architecture

```
TS Frontend (AgentLoop / useAgent)
    │  save/load via HTTP
    ▼
Daemon REST API (new endpoints)
    │  read/write JSON files
    ▼
~/.claude-code/sessions/{id}.json
```

The daemon's `chat_stream` remains a stateless pass-through. Session persistence is a separate concern from chat streaming.

## Data Model

### Session file: `~/.claude-code/sessions/{id}.json`

```json
{
  "id": "uuid-v4",
  "name": "first user message, truncated to 50 chars",
  "created_at": "2026-05-28T10:30:00Z",
  "updated_at": "2026-05-28T11:00:00Z",
  "messages": [
    {
      "role": "system",
      "content": "You are a coding agent...",
      "reasoning_content": null,
      "tool_calls": null,
      "tool_call_id": null
    },
    {
      "role": "user",
      "content": "帮我重构 session 模块",
      "reasoning_content": null,
      "tool_calls": null,
      "tool_call_id": null
    },
    {
      "role": "assistant",
      "content": "好的，让我先看看...",
      "reasoning_content": "thinking about the approach...",
      "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "file_read", "arguments": "{\"path\":\"src/context/session.rs\"}" } }],
      "tool_call_id": null
    },
    {
      "role": "tool",
      "content": "{\"success\":true,\"content\":\"...\"}",
      "reasoning_content": null,
      "tool_calls": null,
      "tool_call_id": "call_1"
    }
  ]
}
```

The `messages` array matches the TS `ChatMessage` type exactly. `SessionInfo` (for list/search responses) is the same object without the `messages` field.

### Rust side: enhance `src/context/session.rs`

The existing `SessionManager` has a simplified `Message` struct (role + content + timestamp). It must be updated to match the full `ChatMessage` shape. The existing CRUD methods (`create`, `load`, `save`, `delete`, `list`) need minor adjustments for the new fields.

## REST API

| Method | Path | Request Body | Response | Notes |
|--------|------|-------------|----------|-------|
| `GET` | `/api/v1/sessions` | — | `SessionInfo[]` | Sorted by `updated_at` desc |
| `POST` | `/api/v1/sessions` | `{ name?: string }` | `Session` | Creates with empty messages, auto-generates name from first user msg via frontend |
| `GET` | `/api/v1/sessions/:id` | — | `Session` | Full session including `messages` |
| `PUT` | `/api/v1/sessions/:id` | `{ name?: string, messages: ChatMessage[] }` | `Session` | Full replace of messages array |
| `DELETE` | `/api/v1/sessions/:id` | — | `{ success: true }` | |
| `GET` | `/api/v1/sessions/search?q=xxx` | — | `SessionInfo[]` | Substring match on `name` + first user message content |

Implementation notes:
- `PUT` does a full replace of the messages array. No incremental append.
- `PUT` uses upsert semantics: create the session file if it doesn't exist yet. This is necessary because the frontend generates session IDs at mount time and saves via PUT before any POST.
- Search scans all session files, loading only metadata (not full messages). Performance is fine for <1000 sessions.
- Sessions directory is `~/.claude-code/sessions/`.

## TS Frontend Changes

### ApiClient (packages/core/src/client.ts)

New methods:
```typescript
listSessions(): Promise<SessionInfo[]>
createSession(name?: string): Promise<Session>
loadSession(id: string): Promise<Session>
saveSession(id: string, name: string, messages: ChatMessage[]): Promise<void>
deleteSession(id: string): Promise<void>
searchSessions(query: string): Promise<SessionInfo[]>
```

### AgentLoop (packages/core/src/agent-loop.ts)

New methods:
```typescript
loadHistory(messages: ChatMessage[]): void  // replace conversationHistory
getHistory(): ChatMessage[]                  // expose for save
```

### useAgent hook (packages/cli/src/hooks/use-agent.ts)

New state and actions:
```typescript
// State
sessions: SessionInfo[]            // available sessions for modal
sessionListOpen: boolean           // modal visibility
sessionId: string                  // current session ID (uuid)

// Actions
loadSession(id: string): Promise<void>    // fetch + loadHistory + rebuild UIMessage[]
saveCurrentSession(name?): Promise<void>  // getHistory + PUT to daemon
deleteSession(id: string): Promise<void>
openSessionList(): void                     // open modal, refresh list
closeSessionList(): void
rebuildUIMessages(msgs: ChatMessage[]): UIMessage[]  // pure function
```

### rebuildUIMessages: ChatMessage[] → UIMessage[]

When loading a saved session, we need to reconstruct UIMessages from raw ChatMessages. The mapping:

- `{ role: "user", content }` → `UIMessage { role: "user", content }`
- `{ role: "assistant", content, tool_calls: [...] }` → `UIMessage { role: "assistant", content }` + for each tool_call in the array, insert a `UIMessage { role: "tool", toolName, toolArgs, toolPhase: "call" }` BEFORE the corresponding tool result
- `{ role: "assistant", content, tool_calls: null }` → `UIMessage { role: "assistant", content }`
- `{ role: "tool", content, tool_call_id }` → `UIMessage { role: "tool", content, toolPhase: "result", toolSuccess: parse from JSON content }`
- System messages are skipped (not rendered in UI)

Tool result `toolSuccess` is determined by parsing the JSON content: `{ success: true }` → success, otherwise error.

### New component: SessionModal (packages/cli/src/components/session-modal.tsx)

Ink React component — Card-style layout (Variant B):
- Header: "◇ Sessions (N)" with esc hint
- Search input: `/ <query>` with cursor
- Scrollable list: each row shows session name, message count, date, and first-message preview snippet
- Selected row highlighted with blue left border + background
- Footer shortcuts: ↑↓ navigate, ↩ load, ⌫ delete, r rename
- Renders as an overlay on top of chat view

## Auto-Save Strategy

### Save triggers
1. **After each `processInput()` round-trip**: save `conversationHistory` to current session ID via `PUT /api/v1/sessions/:id`
2. **On `reset()`**: save current session before clearing
3. **On process exit**: best-effort save via `SIGINT`/`SIGTERM` handler

### No timers
No `setInterval`-based auto-save. Only save at natural checkpoints (after round-trip, on reset, on exit).

### Deduplication
If messages are identical to last save (same length + same last-message timestamp), skip the PUT. Frontend can track a dirty flag.

## Startup Recovery

1. `app.tsx` mount → `GET /api/v1/sessions` → sorted by `updated_at` desc
2. If the latest session's `updated_at` is within the last 24 hours: auto-load it
3. Otherwise: start a fresh session (new UUID)
4. Auto-loaded sessions use the existing session ID. Fresh sessions get a new UUID.
5. Auto-save always writes to the current session ID (whether auto-loaded or newly created).

## Manual Session Switch Flow

1. User presses shortcut (e.g. `Ctrl+S`) → `openSessionList()`
2. Before opening: save current session
3. Modal appears with list of sessions, focus in search bar
4. User can type to filter/search (substring match on name)
5. ↑↓ arrows to navigate, ↩ to select, ⌫ to delete, `r` to rename, esc to close
6. On selection: `loadSession(id)` → fetch full session → `AgentLoop.loadHistory()` → `rebuildUIMessages()` → close modal
7. If nothing selected (esc): modal closes, current session unchanged

## Session Naming

- **Auto-name**: first user message, truncated to 50 characters. Set during the first `saveCurrentSession()` call.
- **Manual rename**: press `r` in the modal, type new name in search bar (search bar switches to rename mode), confirm with ↩
- **`name` field**: required in session JSON. Never null or empty.

## Edge Cases

| Situation | Handling |
|-----------|----------|
| `sessions/` directory missing | `list()` returns `[]`, `create()` creates directory |
| Corrupt JSON file | Skip that file in `list()`, log warning, other sessions unaffected |
| Disk full on save | Daemon returns 500, frontend shows dismissable error toast |
| Empty session (0 messages) | Don't auto-restore; don't show in list; skip `saveSession` |
| Session ID not found | Daemon returns 404, frontend shows error, keeps current state |
| Network error on load/switch | Show error toast, do not modify current session |
| Deleting current session | Switch to next most recent; if none, create blank session |
| Search with no results | Show "No sessions matching '<query>'" in list area |
| Terminal < 60 columns | Modal uses min-width 60; truncates session names |

## Files Changed

| File | Change |
|------|--------|
| `src/context/session.rs` | Enhance Message model, add search method |
| `src/daemon/models.rs` | Add session request/response types |
| `src/daemon/handlers.rs` | Add session CRUD + search handlers |
| `src/daemon/routes.rs` | Register 6 new routes |
| `src/daemon/state.rs` | Inject SessionManager into DaemonState |
| `packages/core/src/client.ts` | Add 6 session API methods |
| `packages/core/src/agent-loop.ts` | Add loadHistory/getHistory |
| `packages/core/src/types.ts` | Add SessionInfo, Session types |
| `packages/cli/src/hooks/use-agent.ts` | Add session state + actions |
| `packages/cli/src/components/session-modal.tsx` | New component |
| `packages/cli/src/components/app.tsx` | Wire session modal, startup restore, keyboard shortcut |
