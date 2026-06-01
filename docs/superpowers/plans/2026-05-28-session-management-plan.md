# Session Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full session management (save/restore/named sessions, session list, search) to the project via daemon REST API + TS frontend integration.

**Architecture:** Daemon exposes 6 REST endpoints for session CRUD + search, backed by JSON files in `~/.wgenty-code/sessions/`. The TS frontend calls these endpoints from `ApiClient`, and `useAgent` manages session state with auto-save after each round-trip. A new `SessionModal` Ink component provides terminal UI for session browsing.

**Tech Stack:** Rust (axum, serde, tokio), TypeScript (Ink React), JSON file storage

---

### Task 1: Enhance SessionManager data model

**Files:**
- Modify: `src/context/session.rs`

- [ ] **Step 1: Replace the existing Message struct with re-exports from crate::api**

The current `Message` struct (role + content + timestamp) is too simple. We need it to match `ChatMessage` so session JSON is compatible with the TS frontend. Replace the entire file content:

```rust
//! Session Module - Session management

use crate::api::ChatMessage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Session manager
pub struct SessionManager {
    sessions_dir: PathBuf,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let sessions_dir = home.join(".wgenty-code").join("sessions");

        Self { sessions_dir }
    }

    /// List all sessions (returns SessionInfo without messages)
    pub fn list(&self) -> anyhow::Result<Vec<SessionInfo>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        let summary = session
                            .messages
                            .iter()
                            .find(|m| m.role == "user")
                            .and_then(|m| m.content.as_ref())
                            .map(|c| {
                                if c.len() > 80 {
                                    format!("{}...", &c[..80])
                                } else {
                                    c.clone()
                                }
                            });

                        sessions.push(SessionInfo {
                            id: session.id,
                            name: session.name,
                            created_at: session.created_at,
                            updated_at: session.updated_at,
                            message_count: session.messages.len(),
                            summary,
                        });
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    /// Create a new session
    pub fn create(&self, name: Option<&str>) -> anyhow::Result<Session> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let id = uuid::Uuid::new_v4().to_string();
        let session_name = name.unwrap_or(&id).to_string();

        let session = Session {
            id: id.clone(),
            name: session_name,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            messages: Vec::new(),
        };

        self.save(&session)?;

        Ok(session)
    }

    /// Load a session by ID
    pub fn load(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let path = self.sessions_dir.join(format!("{}.json", id));

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let session = serde_json::from_str(&content)?;

        Ok(Some(session))
    }

    /// Save a session (upsert: create file if it doesn't exist)
    pub fn save(&self, session: &Session) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let path = self.sessions_dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, content)?;

        Ok(())
    }

    /// Delete a session
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let path = self.sessions_dir.join(format!("{}.json", id));

        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        Ok(())
    }

    /// Search sessions by name and first user message content
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let all = self.list()?;
        let query_lower = query.to_lowercase();

        Ok(all
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query_lower)
                    || s.summary
                        .as_ref()
                        .map(|sm| sm.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .collect())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check 2>&1 | head -20
```

Expected: compiles successfully (session.rs no longer defines its own `Message`, it uses `crate::api::ChatMessage`).

- [ ] **Step 3: Commit**

```bash
git add src/context/session.rs
git commit -m "refactor: update session model to use ChatMessage, add search and summary fields"
```

---

### Task 2: Add session API models

**Files:**
- Modify: `src/daemon/models.rs`

- [ ] **Step 1: Append session request/response types**

Add these types to the end of `src/daemon/models.rs`:

```rust
// ── Sessions ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SessionInfoResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<crate::api::ChatMessage>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<crate::api::ChatMessage>>,
}

#[derive(Debug, Deserialize)]
pub struct SearchSessionsQuery {
    pub q: String,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check 2>&1 | head -20
```

Expected: compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/models.rs
git commit -m "feat: add session API request/response models"
```

---

### Task 3: Wire SessionManager into DaemonState

**Files:**
- Modify: `src/daemon/state.rs`

- [ ] **Step 1: Add session_manager field and initialize it**

In `src/daemon/state.rs`, add the import and field:

Add import at top:
```rust
use crate::context::session::SessionManager;
```

Add field to `DaemonState` struct, after the `team_manager` field:
```rust
    pub session_manager: SessionManager,
```

In `DaemonState::new()`, add initialization before the closing `Self {`:
```rust
        let session_manager = SessionManager::new();
```

In the `Self {` constructor expression, add:
```rust
            session_manager,
```

Full diff context — the struct becomes:

```rust
pub struct DaemonState {
    pub app_state: AppState,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_executor: ToolExecutor,
    pub task_manager: Arc<TaskManagementTool>,
    pub todo_state: Arc<RwLock<TodoState>>,
    pub skill_loader: Arc<SkillLoader>,
    pub background_manager: Arc<BackgroundManager>,
    pub team_manager: Option<Arc<TeamManager>>,
    pub session_manager: SessionManager,
    sessions: Arc<RwLock<std::collections::HashMap<String, SessionRules>>>,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check 2>&1 | head -20
```

Expected: compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/state.rs
git commit -m "feat: inject SessionManager into DaemonState"
```

---

### Task 4: Add session handlers

**Files:**
- Modify: `src/daemon/handlers.rs`

- [ ] **Step 1: Add session handler functions**

Append these handlers before the final line of `src/daemon/handlers.rs`:

```rust
// ── Sessions ──────────────────────────────────────────────────────────────────

pub async fn list_sessions(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    let sessions = tokio::task::spawn_blocking(move || state.session_manager.list())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionInfoResponse {
                id: s.id,
                name: s.name,
                created_at: s.created_at.to_rfc3339(),
                updated_at: s.updated_at.to_rfc3339(),
                message_count: s.message_count,
                summary: s.summary,
            })
            .collect(),
    ))
}

pub async fn create_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = tokio::task::spawn_blocking(move || state.session_manager.create(body.name.as_deref()))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn get_session(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = tokio::task::spawn_blocking(move || state.session_manager.load(&id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn update_session(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = tokio::task::spawn_blocking(move || {
        // Load existing or create new (upsert)
        let mut session = state
            .session_manager
            .load(&id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .unwrap_or_else(|| crate::context::session::Session {
                id: id.clone(),
                name: String::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                messages: Vec::new(),
            });

        if let Some(name) = &body.name {
            session.name = name.clone();
        }
        if let Some(messages) = body.messages {
            session.messages = messages;
        }
        session.updated_at = chrono::Utc::now();

        state
            .session_manager
            .save(&session)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok::<_, StatusCode>(session)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn delete_session(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let result = tokio::task::spawn_blocking(move || state.session_manager.delete(&id)).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({"success": true})),
        _ => Json(serde_json::json!({"success": false, "error": "Failed to delete session"})),
    }
}

pub async fn search_sessions(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<SearchSessionsQuery>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    let sessions = tokio::task::spawn_blocking(move || state.session_manager.search(&query.q))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionInfoResponse {
                id: s.id,
                name: s.name,
                created_at: s.created_at.to_rfc3339(),
                updated_at: s.updated_at.to_rfc3339(),
                message_count: s.message_count,
                summary: s.summary,
            })
            .collect(),
    ))
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check 2>&1 | head -30
```

Expected: compiles successfully. If there are import errors for `chrono`, add `use chrono::Utc;` at the top of handlers.rs.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/handlers.rs
git commit -m "feat: add session CRUD and search handlers"
```

---

### Task 5: Register session routes

**Files:**
- Modify: `src/daemon/routes.rs`

- [ ] **Step 1: Add session routes to the router**

In `src/daemon/routes.rs`, add these routes inside the `Router::new()` chain, after the MCP route:

```rust
        // Sessions
        .route("/api/v1/sessions", get(handlers::list_sessions))
        .route("/api/v1/sessions", post(handlers::create_session))
        .route("/api/v1/sessions/search", get(handlers::search_sessions))
        .route("/api/v1/sessions/{id}", get(handlers::get_session))
        .route("/api/v1/sessions/{id}", put(handlers::update_session))
        .route("/api/v1/sessions/{id}", delete(handlers::delete_session))
```

Note: The search route must come BEFORE `/{id}` to avoid "search" being matched as an ID. Also import `put` and `delete` from axum if not already imported — change the axum routing import to:

```rust
use axum::{routing::get, routing::post, routing::put, routing::delete, Router};
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check 2>&1 | head -20
```

Expected: compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/routes.rs
git commit -m "feat: register session API routes"
```

---

### Task 6: Add TypeScript types for sessions

**Files:**
- Modify: `packages/core/src/types.ts`

- [ ] **Step 1: Append session types**

Add these types to the end of `packages/core/src/types.ts`:

```typescript
// ── Session types ──────────────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  message_count: number;
  summary?: string | null;
}

export interface Session {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  messages: ChatMessage[];
}
```

- [ ] **Step 2: Verify typecheck**

```bash
npm run -w packages/core typecheck 2>&1
```

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add packages/core/src/types.ts
git commit -m "feat: add SessionInfo and Session types"
```

---

### Task 7: Add session API methods to ApiClient

**Files:**
- Modify: `packages/core/src/client.ts`

- [ ] **Step 1: Import Session types**

Update the import at the top of `client.ts` to include SessionInfo and Session:

```typescript
import type {
  ChatStreamRequest,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  HealthResponse,
  SessionInfo,
  Session,
  ToolInfo,
} from "./types.ts";
```

- [ ] **Step 2: Add session methods to ApiClient class**

Append these methods before the closing `}` of the `ApiClient` class (after the `getTodos` method):

```typescript
  // ── Sessions ──────────────────────────────────────────────────────────────

  async listSessions(): Promise<SessionInfo[]> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`);
    if (!res.ok) return [];
    return res.json();
  }

  async createSession(name?: string): Promise<Session> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    if (!res.ok) throw new Error(`Failed to create session (${res.status})`);
    return res.json();
  }

  async loadSession(id: string): Promise<Session> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`);
    if (!res.ok) throw new Error(`Failed to load session (${res.status})`);
    return res.json();
  }

  async saveSession(
    id: string,
    name: string,
    messages: import("./types.ts").ChatMessage[],
  ): Promise<void> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, messages }),
      },
    );
    if (!res.ok) throw new Error(`Failed to save session (${res.status})`);
  }

  async deleteSession(id: string): Promise<void> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
    if (!res.ok) throw new Error(`Failed to delete session (${res.status})`);
  }

  async searchSessions(query: string): Promise<SessionInfo[]> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/search?q=${encodeURIComponent(query)}`,
    );
    if (!res.ok) return [];
    return res.json();
  }
```

Note: For the `saveSession` method, prefer to import `ChatMessage` at the top of the file instead of using inline import. Update the top import to:

```typescript
import type {
  ChatMessage,
  ChatStreamRequest,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  HealthResponse,
  SessionInfo,
  Session,
  ToolInfo,
} from "./types.ts";
```

And change the signature to `messages: ChatMessage[]`.

- [ ] **Step 3: Verify typecheck**

```bash
npm run -w packages/core typecheck 2>&1
```

Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add packages/core/src/client.ts
git commit -m "feat: add session API methods to ApiClient"
```

---

### Task 8: Add loadHistory/getHistory to AgentLoop

**Files:**
- Modify: `packages/core/src/agent-loop.ts`

- [ ] **Step 1: Add methods to AgentLoop class**

Append these two methods inside the `AgentLoop` class, before the `reset()` method:

```typescript
  /** Replace conversation history entirely (for session restore). */
  loadHistory(messages: ChatMessage[]): void {
    this.roundsSinceTodo = 0;
    this.compactedSummary = "";
    this.conversationHistory = messages;
  }

  /** Expose current conversation history (for session save). */
  getHistory(): ChatMessage[] {
    return this.conversationHistory;
  }
```

- [ ] **Step 2: Verify typecheck**

```bash
npm run -w packages/core typecheck 2>&1
```

Expected: No errors (ChatMessage is already imported).

- [ ] **Step 3: Commit**

```bash
git add packages/core/src/agent-loop.ts
git commit -m "feat: add loadHistory and getHistory to AgentLoop"
```

---

### Task 9: Export session types from core index

**Files:**
- Modify: `packages/core/src/index.ts`

- [ ] **Step 1: Add SessionInfo and Session to exports**

Add to the `types.ts` re-export block:

```typescript
export type {
  ChatMessage,
  ToolCall,
  ToolDefinition,
  ToolInfo,
  StreamChunk,
  StreamChoice,
  Delta,
  StreamToolCall,
  HealthResponse,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  SessionInfo,   // add this
  Session,        // add this
} from "./types.ts";
```

- [ ] **Step 2: Verify typecheck**

```bash
npm run -w packages/core typecheck 2>&1
```

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add packages/core/src/index.ts
git commit -m "feat: export SessionInfo and Session types from core"
```

---

### Task 10: Add session management to useAgent hook

**Files:**
- Modify: `packages/cli/src/hooks/use-agent.ts`

- [ ] **Step 1: Import session types**

Add to the imports at the top:

```typescript
import { ApiClient, AgentLoop } from "@wgenty-code/core";
import type { AgentCallbacks, ToolResult, ChatMessage, SessionInfo } from "@wgenty-code/core";
```

- [ ] **Step 2: Add session state and logic to useAgent**

Add new state variables after the existing `useState` declarations (after `pendingPermission`):

```typescript
  const [sessionId, setSessionId] = useState<string>(() => crypto.randomUUID());
  const [sessionName, setSessionName] = useState<string>("");
  const [sessionListOpen, setSessionListOpen] = useState(false);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [dirty, setDirty] = useState(false);
```

Add session management methods before `sendMessage`:

```typescript
  /** Rebuild UIMessage[] from raw ChatMessage[] for display after loading a session. */
  const rebuildUIMessages = useCallback(
    (msgs: ChatMessage[]): UIMessage[] => {
      const ui: UIMessage[] = [];
      for (const msg of msgs) {
        if (msg.role === "system") continue; // skip system messages in UI

        if (msg.role === "user") {
          ui.push({ id: nextId.current++, role: "user", content: msg.content ?? "" });
        } else if (msg.role === "assistant") {
          ui.push({ id: nextId.current++, role: "assistant", content: msg.content ?? "" });
          // Generate tool call messages from tool_calls array
          if (msg.tool_calls) {
            for (const tc of msg.tool_calls) {
              let args: Record<string, unknown> = {};
              try { args = JSON.parse(tc.function.arguments); } catch { /* ignore */ }
              ui.push({
                id: nextId.current++,
                role: "tool",
                content: formatToolCallSummary(tc.function.name, args),
                toolName: tc.function.name,
                toolArgs: formatToolArgs(args),
                toolPhase: "call",
              });
            }
          }
        } else if (msg.role === "tool") {
          const parsed = tryParseJson(msg.content);
          const success = parsed?.success ?? true;
          const toolName = findToolNameForId(msgs, msg.tool_call_id);
          ui.push({
            id: nextId.current++,
            role: "tool",
            content: msg.content ?? "",
            toolName,
            toolPhase: "result",
            toolSuccess: success,
          });
        }
      }
      return ui;
    },
    [],
  );
```

Add helper functions before `useAgent`:

```typescript
/** Try to parse JSON, return null on failure. */
function tryParseJson(content?: string | null): Record<string, unknown> | null {
  if (!content) return null;
  try { return JSON.parse(content); } catch { return null; }
}

/** Find the tool name for a given tool_call_id from the messages array. */
function findToolNameForId(msgs: ChatMessage[], toolCallId?: string | null): string | undefined {
  if (!toolCallId) return undefined;
  for (const msg of msgs) {
    if (msg.role === "assistant" && msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        if (tc.id === toolCallId) return tc.function.name;
      }
    }
  }
  return undefined;
}
```

Add session actions after `rebuildUIMessages`:

```typescript
  /** Save current session to daemon. */
  const saveCurrentSession = useCallback(async () => {
    if (!agentRef.current) return;
    const history = agentRef.current.getHistory();
    if (history.length <= 1) return; // skip empty (only system prompt)
    try {
      await client.saveSession(sessionId, sessionName || "Untitled", history);
      setDirty(false);
    } catch {
      // silently fail — auto-save is best-effort
    }
  }, [client, sessionId, sessionName]);

  /** Load a session from daemon and restore state. */
  const loadSession = useCallback(
    async (id: string) => {
      try {
        const session = await client.loadSession(id);
        if (!agentRef.current) {
          agentRef.current = new AgentLoop({ client, callbacks });
        }
        agentRef.current.loadHistory(session.messages);
        const ui = rebuildUIMessages(session.messages);
        // Reset nextId to above the max loaded id
        nextId.current = Math.max(0, ...ui.map((m) => m.id)) + 1;
        setMessages(ui);
        setSessionId(session.id);
        setSessionName(session.name);
        setDirty(false);
      } catch {
        // Network error — keep current session
      }
    },
    [client, callbacks, rebuildUIMessages],
  );

  /** Refresh the session list from daemon. */
  const refreshSessions = useCallback(async () => {
    try {
      const list = await client.listSessions();
      setSessions(list);
    } catch {
      // silently fail
    }
  }, [client]);

  /** Open session modal (save current first, refresh list). */
  const openSessionList = useCallback(async () => {
    await saveCurrentSession();
    await refreshSessions();
    setSessionListOpen(true);
  }, [saveCurrentSession, refreshSessions]);

  const closeSessionList = useCallback(() => {
    setSessionListOpen(false);
  }, []);

  /** Delete a session from daemon. */
  const deleteSessionById = useCallback(
    async (id: string) => {
      try {
        await client.deleteSession(id);
        if (id === sessionId) {
          // Deleting current session — load the next most recent or start fresh
          const updated = sessions.filter((s) => s.id !== id);
          if (updated.length > 0) {
            await loadSession(updated[0].id);
          } else {
            reset();
            setSessionId(crypto.randomUUID());
            setSessionName("");
          }
        }
        await refreshSessions();
      } catch {
        // silently fail
      }
    },
    [client, sessionId, sessions, loadSession, refreshSessions, reset],
  );

  /** Rename current session. */
  const renameSession = useCallback(
    async (newName: string) => {
      setSessionName(newName);
      setDirty(true);
    },
    [],
  );
```

- [ ] **Step 3: Integrate auto-save into the sendMessage flow**

Modify `sendMessage` to save after completion. After the existing `setStatus({ type: "idle" });` line (at the end of the try block), add:

```typescript
      // Auto-save after each round-trip
      setDirty(true);
      saveCurrentSession();
```

And update the `reset` callback to save before clearing:

```typescript
  const reset = useCallback(() => {
    saveCurrentSession();
    setMessages([]);
    setStatus({ type: "idle" });
    streamingContentRef.current = "";
    agentRef.current?.reset();
    setSessionId(crypto.randomUUID());
    setSessionName("");
    setDirty(false);
  }, [saveCurrentSession]);
```

- [ ] **Step 4: Update return statement**

Add new values to the return object:

```typescript
  return {
    messages,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
    // Session management
    sessionId,
    sessionName,
    sessionListOpen,
    sessions,
    loadSession,
    saveCurrentSession,
    openSessionList,
    closeSessionList,
    deleteSession: deleteSessionById,
    renameSession,
    rebuildUIMessages,
  };
```

Update the return type by adding these to the `UseAgentReturn` interface or rely on inference.

- [ ] **Step 5: Verify typecheck**

```bash
npm run -w packages/cli typecheck 2>&1
```

Expected: No errors. If there are issues with `crypto.randomUUID()`, use `crypto.randomUUID()` which is available in Node 19+. For older Node, use a fallback.

- [ ] **Step 6: Commit**

```bash
git add packages/cli/src/hooks/use-agent.ts
git commit -m "feat: add session state, auto-save, load/restore to useAgent hook"
```

---

### Task 11: Create SessionModal component

**Files:**
- Create: `packages/cli/src/components/session-modal.tsx`

- [ ] **Step 1: Create the SessionModal component**

Create the file with this content:

```typescript
import React from "react";
import { Box, Text } from "ink";
import { useInput } from "ink";
import type { SessionInfo } from "@wgenty-code/core";

interface Props {
  sessions: SessionInfo[];
  selectedIndex: number;
  searchQuery: string;
  onSelect: (session: SessionInfo) => void;
  onDelete: (id: string) => void;
  onClose: () => void;
  onSearchChange: (query: string) => void;
  onNavigate: (delta: number) => void;
}

export const SessionModal: React.FC<Props> = ({
  sessions,
  selectedIndex,
  searchQuery,
  onSelect,
  onDelete,
  onClose,
  onSearchChange,
  onNavigate,
}) => {
  useInput((input, key) => {
    if (key.escape) {
      onClose();
      return;
    }
    if (key.upArrow) {
      onNavigate(-1);
      return;
    }
    if (key.downArrow) {
      onNavigate(1);
      return;
    }
    if (key.return) {
      const selected = sessions[selectedIndex];
      if (selected) onSelect(selected);
      return;
    }
    if (key.delete || key.backspace) {
      // Delete needs explicit keypress — Backspace in search mode deletes chars
      if (searchQuery === "" && input === "") {
        const selected = sessions[selectedIndex];
        if (selected) onDelete(selected.id);
        return;
      }
    }
    if (input === "r" && searchQuery === "") {
      // Rename mode — not implemented in v1, skip for now
      return;
    }
    // Append to search query
    if (input.length === 1 && !key.ctrl && !key.meta) {
      onSearchChange(searchQuery + input);
    } else if (key.backspace || key.delete) {
      onSearchChange(searchQuery.slice(0, -1));
    }
  });

  const filtered = sessions.filter(
    (s) =>
      searchQuery === "" ||
      s.name.toLowerCase().includes(searchQuery.toLowerCase()),
  );

  const displaySessions = filtered.slice(0, 20); // show at most 20

  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor="blue"
      paddingX={1}
      marginY={1}
    >
      {/* Header */}
      <Box>
        <Text bold color="blue">
          ◇ Sessions ({sessions.length})
        </Text>
        <Text dimColor>  esc to close</Text>
      </Box>

      {/* Search bar */}
      <Box marginTop={1}>
        <Text dimColor>/ </Text>
        <Text>{searchQuery}</Text>
        <Text dimColor>█</Text>
      </Box>

      {/* Divider */}
      <Box>
        <Text dimColor>{"─".repeat(40)}</Text>
      </Box>

      {/* Session list */}
      {displaySessions.length === 0 ? (
        <Box marginY={1}>
          <Text dimColor>
            {searchQuery
              ? `No sessions matching '${searchQuery}'`
              : "No sessions found"}
          </Text>
        </Box>
      ) : (
        displaySessions.map((s, i) => {
          const isSelected = i === selectedIndex;
          const date = new Date(s.updated_at).toLocaleDateString("zh-CN", {
            month: "2-digit",
            day: "2-digit",
          });
          const time = new Date(s.updated_at).toLocaleTimeString("zh-CN", {
            hour: "2-digit",
            minute: "2-digit",
          });

          return (
            <Box key={s.id} marginTop={i > 0 ? 1 : 0}>
              <Text color={isSelected ? "blue" : undefined}>
                {isSelected ? "▸ " : "  "}
              </Text>
              <Box flexDirection="column">
                <Text bold={isSelected} color={isSelected ? "blue" : undefined}>
                  {s.name || "Untitled"}
                </Text>
                <Text dimColor>
                  {s.message_count} messages · {date} {time}
                  {s.summary ? ` · ${s.summary.slice(0, 60)}` : ""}
                </Text>
              </Box>
            </Box>
          );
        })
      )}

      {/* Footer shortcuts */}
      <Box marginTop={1}>
        <Text dimColor>↑↓ navigate  ↩ load  ⌫ delete  esc close</Text>
      </Box>
    </Box>
  );
};
```

- [ ] **Step 2: Verify typecheck**

```bash
npm run -w packages/cli typecheck 2>&1
```

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add packages/cli/src/components/session-modal.tsx
git commit -m "feat: add SessionModal component for terminal UI"
```

---

### Task 12: Wire session modal into App

**Files:**
- Modify: `packages/cli/src/components/app.tsx`

- [ ] **Step 1: Add imports**

Update the ink import line at the top of app.tsx to include `useInput`:
```typescript
import React from "react";
import { Box, Text, useInput } from "ink";
```

Add the new component and type imports after existing imports:
```typescript
import { SessionModal } from "./session-modal.tsx";
import type { SessionInfo } from "@wgenty-code/core";
```

- [ ] **Step 2: Destructure new session values from useAgent**

In `AgentView`, update the destructuring of `useAgent` to include session properties:

```typescript
  const {
    messages,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
    sessionListOpen,
    sessions,
    loadSession,
    openSessionList,
    closeSessionList,
    deleteSession,
    renameSession,
  } = useAgent({ client });
```

- [ ] **Step 3: Add session modal state and startup restore effect**

Add state for modal navigation and search:

```typescript
  const [sessionSearchQuery, setSessionSearchQuery] = React.useState("");
  const [sessionSelectedIndex, setSessionSelectedIndex] = React.useState(0);
```

Add startup restore effect:

```typescript
  // Startup: try to restore the most recent session
  React.useEffect(() => {
    client.listSessions().then((list) => {
      if (list.length > 0) {
        const latest = list[0];
        const updatedAt = new Date(latest.updated_at).getTime();
        const now = Date.now();
        const hoursSinceUpdate = (now - updatedAt) / (1000 * 60 * 60);
        if (hoursSinceUpdate < 24) {
          loadSession(latest.id);
        }
      }
    }).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
```

- [ ] **Step 4: Add session modal callbacks and keyboard shortcut**

Add callbacks before the `modal` variable:

```typescript
  const handleSessionSelect = React.useCallback(
    async (session: SessionInfo) => {
      setSessionListOpen(false);
      setSessionSearchQuery("");
      setSessionSelectedIndex(0);
      await loadSession(session.id);
    },
    [loadSession, setSessionListOpen],
  );

  const handleSessionDelete = React.useCallback(
    async (id: string) => {
      await deleteSession(id);
      setSessionSelectedIndex(0);
    },
    [deleteSession],
  );

  const handleSessionNavigate = React.useCallback(
    (delta: number) => {
      setSessionSelectedIndex((prev) => {
        const filtered = sessions.filter(
          (s) =>
            sessionSearchQuery === "" ||
            s.name.toLowerCase().includes(sessionSearchQuery.toLowerCase()),
        );
        const max = Math.max(0, filtered.length - 1);
        return Math.max(0, Math.min(max, prev + delta));
      });
    },
    [sessions, sessionSearchQuery],
  );
```

- [ ] **Step 5: Add Ctrl+S keyboard shortcut**

In `app.tsx`, there's no global keybinding mechanism. The InputBox component uses `useInput`. Instead, add a `useInput` call in `AgentView` for the session shortcut:

```typescript
  import { useInput } from "ink";

  // ... inside AgentView:
  useInput((input, key) => {
    // Ctrl+S to open session list (only when idle and no modal open)
    if (
      key.ctrl &&
      input === "s" &&
      status.type === "idle" &&
      !pendingPermission &&
      !pendingQuestion &&
      !sessionListOpen
    ) {
      openSessionList();
    }
  });
```

Note: This needs to be called within the `AgentView` component, not the `App` component. The `useInput` from ink is already used in `PermissionModal` and `QuestionModal`, so it's available.

- [ ] **Step 6: Render SessionModal**

Update the `modal` variable rendering logic. Change:

```typescript
  const modal =
    pendingPermission != null ? (
      <PermissionModal ... />
    ) : pendingQuestion != null ? (
      <QuestionModal ... />
    ) : null;
```

To:

```typescript
  const modal = sessionListOpen ? (
    <SessionModal
      sessions={sessions}
      selectedIndex={sessionSelectedIndex}
      searchQuery={sessionSearchQuery}
      onSelect={handleSessionSelect}
      onDelete={handleSessionDelete}
      onClose={() => {
        closeSessionList();
        setSessionSearchQuery("");
        setSessionSelectedIndex(0);
      }}
      onSearchChange={setSessionSearchQuery}
      onNavigate={handleSessionNavigate}
    />
  ) : pendingPermission != null ? (
    <PermissionModal
      reason={pendingPermission.reason}
      sessionRule={pendingPermission.sessionRule}
      onResolve={resolvePermission}
    />
  ) : pendingQuestion != null ? (
    <QuestionModal
      question={pendingQuestion.question}
      options={pendingQuestion.options}
      multiSelect={pendingQuestion.multiSelect}
      onResolve={resolveQuestion}
    />
  ) : null;
```

Also update the keyboard shortcut hint in the header or welcome message to mention `Ctrl+S` for sessions.

- [ ] **Step 7: Update the modal guard for InputBox**

The `InputBox` should be hidden when the session modal is open (same as other modals). Update the condition:

```typescript
      {!modal && (
        <InputBox ... />
      )}
```

This already works since `modal` now includes the session modal.

- [ ] **Step 8: Verify typecheck**

```bash
npm run -w packages/cli typecheck 2>&1
```

Expected: No errors.

- [ ] **Step 9: Start daemon and test manually**

```bash
# Terminal 1: start daemon
cargo run -- daemon --port 8371

# Terminal 2: start CLI
npm run -w packages/cli dev:ink
```

Manual test checklist:
- [ ] Send a message → daemon should create and auto-save session
- [ ] Press Ctrl+S → session modal appears
- [ ] Search/filter sessions by typing
- [ ] ↑↓ navigate, ↩ to load a session
- [ ] esc to close modal without changing session
- [ ] Delete a session with ⌫
- [ ] Restart CLI → recent session auto-restores
- [ ] Check `~/.wgenty-code/sessions/` for JSON files

- [ ] **Step 10: Commit**

```bash
git add packages/cli/src/components/app.tsx
git commit -m "feat: wire session modal, startup restore, and Ctrl+S shortcut into App"
```

---

### Verification Checklist

After all tasks complete:

```bash
# Rust
cargo check
cargo test

# TypeScript
npm run -w packages/core typecheck
npm run -w packages/cli typecheck
```

---

### Risks & Notes

- **`useInput` conflict**: `app.tsx` and `SessionModal` both use `useInput`. In Ink, the most recently mounted component with `useInput` wins. Since `SessionModal` is mounted higher in the tree when open, it captures input. When it unmounts (on close), control returns to `InputBox`. Verify this works correctly in manual testing.
- **`crypto.randomUUID()`**: Available since Node 19.0. The project likely targets Node 18+. If not available, use a UUID v4 polyfill.
- **UIMessage rebuild**: Tool results loaded from history won't have the `toolName` field unless `findToolNameForId` successfully maps the tool_call_id. This depends on the assistant message with matching `tool_calls` being present in the history.
