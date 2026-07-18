//! Session Management - Session lifecycle management

use crate::api::ToolCall;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub project_path: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Model-facing dialogue history (may be compacted). Used for resume/API.
    pub messages: Vec<SessionMessage>,
    /// Human-facing TUI transcript (pre-compact display). Optional for legacy
    /// sessions; when present, the TUI restores this instead of rebuilding from
    /// `messages`. Never sent to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui_messages: Vec<SessionUiMessage>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub status: SessionStatus,
    /// When `Some(n)`, this session was loaded as a lightweight index entry:
    /// `messages` is empty but the real message count is `n`.  Calling
    /// `load(id)` replaces the entry with the fully-deserialized session.
    #[serde(skip)]
    pub lazy_message_count: Option<usize>,
}

impl Session {
    pub fn new(name: Option<&str>) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        Self::with_id(id, name)
    }

    /// Create a session that must use a caller-supplied id (e.g. daemon upsert
    /// via `PUT /sessions/:id` when the file does not yet exist).
    ///
    /// Unlike [`Session::new`], this never regenerates the id. Using `new()` on
    /// the upsert path previously caused each save to write a *different*
    /// `*.json` file while the TUI kept the original path id, flooding the
    /// session panel with duplicate names.
    pub fn with_id(id: impl Into<String>, name: Option<&str>) -> Self {
        let id = id.into();
        let name = name.unwrap_or(&id).to_string();
        Self {
            id: id.clone(),
            name,
            project_path: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            messages: Vec::new(),
            ui_messages: Vec::new(),
            metadata: HashMap::new(),
            status: SessionStatus::Active,
            lazy_message_count: None,
        }
    }

    pub fn with_project(mut self, path: PathBuf) -> Self {
        self.project_path = Some(path);
        self
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        });
        self.updated_at = Utc::now();
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// Persisted TUI chat row (display transcript). Independent of model `messages`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionUiMessage {
    /// `user` | `assistant` | `tool` | `system`
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_args: Option<serde_json::Value>,
    #[serde(default)]
    pub content_collapsed: bool,
    #[serde(default)]
    pub tool_collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_data: Option<SessionDiffData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_metadata: Option<serde_json::Value>,
}

/// Diff payload embedded in [`SessionUiMessage`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDiffData {
    pub file_path: String,
    pub old_content: String,
    pub new_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    /// `#[serde(default)]` for backward compat with legacy session files and
    /// ChatMessage-originated saves where `content` may be absent (e.g.
    /// assistant messages that only carry tool_calls).
    #[serde(default)]
    pub content: String,
    /// Tool call id carried by `role="tool"` result messages. Persisted so a
    /// restored history keeps the assistant `tool_calls` <-> `tool` pairing;
    /// without it the replayed `tool` message is missing `tool_call_id` and
    /// the provider rejects the request (`MissingParameter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Assistant tool calls. Persisted for the same pairing reason as
    /// `tool_call_id`; dropping them orphans the following `tool` results.
    /// Deserialized leniently so a malformed/legacy entry can't block loading.
    #[serde(
        default,
        deserialize_with = "deserialize_tool_calls_lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default = "default_timestamp")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Default timestamp for legacy session files that lack the `timestamp` field
/// on individual messages.
fn default_timestamp() -> DateTime<Utc> {
    Utc::now()
}

/// Lenient deserializer for `tool_calls`: parses strictly when the payload is
/// well-formed, but falls back to `None` for missing/null/malformed entries
/// (e.g. legacy or truncated `{"id":".."}` objects that predate this field).
/// A single bad message must never prevent an entire session from loading.
fn deserialize_tool_calls_lenient<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<ToolCall>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => serde_json::from_value::<Vec<ToolCall>>(v)
            .map(Some)
            .or_else(|_| Ok(None)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SessionStatus {
    #[default]
    Active,
    Paused,
    Archived,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub project_path: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    pub status: SessionStatus,
}

pub struct SessionManager {
    sessions_dir: PathBuf,
    active_session: Arc<RwLock<Option<Session>>>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl SessionManager {
    /// Create a SessionManager using the global sessions directory
    /// (`~/.wgenty-code/sessions/`). Prefer [`with_project_root`](Self::with_project_root)
    /// for production use so sessions are stored per-project.
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let sessions_dir = home.join(".wgenty-code").join("sessions");

        Self {
            sessions_dir,
            active_session: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a SessionManager scoped to a project root.
    ///
    /// Sessions are stored at `<project_root>/.wgenty-code/sessions/`. If that
    /// directory cannot be created (e.g. the project root is unwritable or has
    /// been deleted), storage falls back to the global `~/.wgenty-code/sessions/`
    /// and a warning is logged.
    pub fn with_project_root(project_root: PathBuf) -> Self {
        use crate::utils::{config_dir, project_sessions_dir};

        let project_sessions = project_sessions_dir(&project_root);

        // Try to create the project-local sessions directory. On failure,
        // degrade to the global sessions directory.
        let sessions_dir = if std::fs::create_dir_all(&project_sessions).is_ok() {
            project_sessions
        } else {
            let fallback = config_dir().join("sessions");
            tracing::warn!(
                project_root = %project_root.display(),
                fallback = %fallback.display(),
                "Failed to create project-local sessions directory; falling back to global"
            );
            let _ = std::fs::create_dir_all(&fallback);
            fallback
        };

        Self {
            sessions_dir,
            active_session: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create(&self, name: Option<&str>) -> anyhow::Result<Session> {
        let session = Session::new(name);
        // `save` both persists to disk and updates the in-memory index.
        self.save(&session).await?;
        Ok(session)
    }

    pub async fn load(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let path = self.sessions_dir.join(format!("{}.json", id));

        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let session: Session = serde_json::from_str(&content)?;

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.to_string(), session.clone());

        Ok(Some(session))
    }

    /// Scan the sessions directory and load all persisted sessions into the
    /// in-memory HashMap.
    ///
    /// Previously `list()` only returned sessions already in the HashMap,
    /// which starts empty on every app restart — making all historical
    /// sessions invisible unless their IDs were known and individually
    /// `load(id)`'d. This method makes previously-saved sessions visible
    /// after a restart.
    pub async fn load_all(&self) -> anyhow::Result<usize> {
        if !self.sessions_dir.exists() {
            return Ok(0);
        }

        let mut loaded = 0usize;
        let mut skipped = 0usize;
        let mut dir = tokio::fs::read_dir(&self.sessions_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    match serde_json::from_str::<Session>(&content) {
                        Ok(session) => {
                            let mut sessions = self.sessions.write().await;
                            sessions.entry(session.id.clone()).or_insert(session);
                            loaded += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                file = %path.display(),
                                error = %e,
                                "Skipping malformed session file during load_all"
                            );
                            skipped += 1;
                        }
                    }
                }
            }
        }

        tracing::info!(loaded, skipped, "Session load_all complete");
        Ok(loaded)
    }

    /// Scan the sessions directory and load **metadata only** (id, name,
    /// timestamps, status, message count) without deserializing the full
    /// message history.  This is much faster than `load_all` for directories
    /// with many or large session files.
    ///
    /// Sessions loaded via this method have `lazy_message_count = Some(n)`
    /// and empty `messages`.  Call `load(id)` to hydrate a specific session
    /// on demand.
    pub async fn load_index(&self) -> anyhow::Result<usize> {
        if !self.sessions_dir.exists() {
            return Ok(0);
        }

        let mut loaded = 0usize;
        let mut skipped = 0usize;
        let mut dir = tokio::fs::read_dir(&self.sessions_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(value) => {
                            let id = value
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if id.is_empty() {
                                skipped += 1;
                                continue;
                            }
                            let name = value
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&id)
                                .to_string();
                            let project_path = value
                                .get("project_path")
                                .and_then(|v| v.as_str())
                                .map(PathBuf::from);
                            let created_at = value
                                .get("created_at")
                                .and_then(|v| v.as_str())
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                .map(|dt| dt.with_timezone(&Utc))
                                .unwrap_or_else(Utc::now);
                            let updated_at = value
                                .get("updated_at")
                                .and_then(|v| v.as_str())
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                .map(|dt| dt.with_timezone(&Utc))
                                .unwrap_or_else(Utc::now);
                            let status = value
                                .get("status")
                                .and_then(|v| serde_json::from_value(v.clone()).ok())
                                .unwrap_or_default();
                            let msg_count = value
                                .get("messages")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);

                            let session = Session {
                                id: id.clone(),
                                name,
                                project_path,
                                created_at,
                                updated_at,
                                messages: Vec::new(),
                                ui_messages: Vec::new(),
                                metadata: HashMap::new(),
                                status,
                                lazy_message_count: Some(msg_count),
                            };
                            let mut sessions = self.sessions.write().await;
                            sessions.entry(id).or_insert(session);
                            loaded += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                file = %path.display(),
                                error = %e,
                                "Skipping malformed session file during load_index"
                            );
                            skipped += 1;
                        }
                    }
                }
            }
        }

        tracing::info!(loaded, skipped, "Session load_index complete (lazy)");
        Ok(loaded)
    }

    /// Create a SessionManager with a custom sessions directory (for testing).
    pub fn with_dir(sessions_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            active_session: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn save(&self, session: &Session) -> anyhow::Result<()> {
        self.persist_to_disk(session).await?;

        // Keep the in-memory index coherent with disk so list()/get() reflect
        // the latest name/message_count without waiting for a restart/load_index.
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session.clone());

        Ok(())
    }

    /// Write session JSON to disk without touching the in-memory index.
    /// Used by callers that already hold `sessions` write lock (e.g. add_message).
    async fn persist_to_disk(&self, session: &Session) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.sessions_dir).await?;
        let path = self.sessions_dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)?;
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let path = self.sessions_dir.join(format!("{}.json", id));

        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }

        let mut sessions = self.sessions.write().await;
        sessions.remove(id);

        Ok(())
    }

    pub async fn list(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let sessions = self.sessions.read().await;
        let mut items: Vec<SessionInfo> = sessions
            .values()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                project_path: s.project_path.clone(),
                created_at: s.created_at,
                updated_at: s.updated_at,
                message_count: s.lazy_message_count.unwrap_or(s.messages.len()),
                status: s.status.clone(),
            })
            .collect();
        drop(sessions);
        // Sort by updated_at descending so the most recently active sessions
        // appear first, matching the local SessionManager ordering.
        items.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(items)
    }

    pub async fn get(&self, id: &str) -> Option<Session> {
        // If the session is a lazy-loaded index entry (messages not
        // deserialized), hydrate it from disk on demand.
        let needs_hydrate = {
            let sessions = self.sessions.read().await;
            sessions
                .get(id)
                .map(|s| s.lazy_message_count.is_some())
                .unwrap_or(false)
        };
        if needs_hydrate {
            return self.load(id).await.ok().flatten();
        }
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned()
    }

    pub async fn set_active(&self, session: Session) {
        let mut active = self.active_session.write().await;
        *active = Some(session);
    }

    pub async fn get_active(&self) -> Option<Session> {
        let active = self.active_session.read().await;
        active.clone()
    }

    pub async fn clear_active(&self) {
        let mut active = self.active_session.write().await;
        *active = None;
    }

    pub async fn add_message(&self, id: &str, role: &str, content: &str) -> anyhow::Result<()> {
        // Hydrate lazy-loaded index entries before mutating.
        let needs_hydrate = {
            let sessions = self.sessions.read().await;
            sessions
                .get(id)
                .map(|s| s.lazy_message_count.is_some())
                .unwrap_or(false)
        };
        if needs_hydrate {
            self.load(id).await?;
        }
        // Mutate under the write lock, then persist without re-acquiring it.
        let snapshot = {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(id) {
                session.add_message(role, content);
                Some(session.clone())
            } else {
                None
            }
        };
        if let Some(session) = snapshot {
            self.persist_to_disk(&session).await?;
        }
        Ok(())
    }

    pub async fn archive(&self, id: &str) -> anyhow::Result<()> {
        // Hydrate lazy-loaded index entries before mutating.
        let needs_hydrate = {
            let sessions = self.sessions.read().await;
            sessions
                .get(id)
                .map(|s| s.lazy_message_count.is_some())
                .unwrap_or(false)
        };
        if needs_hydrate {
            self.load(id).await?;
        }
        let snapshot = {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(id) {
                session.status = SessionStatus::Archived;
                Some(session.clone())
            } else {
                None
            }
        };
        if let Some(session) = snapshot {
            self.persist_to_disk(&session).await?;
        }
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Vec<SessionInfo> {
        let query_lower = query.to_lowercase();
        let sessions = self.sessions.read().await;

        sessions
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&query_lower)
                    || s.messages
                        .iter()
                        .any(|m| m.content.to_lowercase().contains(&query_lower))
            })
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                project_path: s.project_path.clone(),
                created_at: s.created_at,
                updated_at: s.updated_at,
                message_count: s.lazy_message_count.unwrap_or(s.messages.len()),
                status: s.status.clone(),
            })
            .collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_all_recovers_persisted_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().to_path_buf());

        // Create and save two sessions.
        let s1 = mgr.create(Some("session-one")).await.unwrap();
        let s2 = mgr.create(Some("session-two")).await.unwrap();

        // Simulate a restart: new manager with the same dir, empty HashMap.
        let restarted = SessionManager::with_dir(tmp.path().to_path_buf());
        let list_before = restarted.list().await.unwrap();
        assert!(
            list_before.is_empty(),
            "fresh manager should have no sessions in memory"
        );

        // load_all should scan the directory and recover both sessions.
        let loaded = restarted.load_all().await.unwrap();
        assert_eq!(loaded, 2);

        let list_after = restarted.list().await.unwrap();
        assert_eq!(list_after.len(), 2);
        let names: Vec<&str> = list_after.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"session-one"));
        assert!(names.contains(&"session-two"));

        // The recovered IDs should match the originals.
        let ids: Vec<&str> = list_after.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&s1.id.as_str()));
        assert!(ids.contains(&s2.id.as_str()));
    }

    #[tokio::test]
    async fn load_all_on_nonexistent_dir_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().join("does-not-exist"));
        let loaded = mgr.load_all().await.unwrap();
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn load_all_handles_legacy_session_format() {
        // Legacy session files (from the old session.rs) only contain
        // id/name/created_at/updated_at/messages, with messages having
        // only role/content. The new Session struct adds status, metadata,
        // project_path, timestamp, etc. - all must be #[serde(default)].
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        let legacy_json = r#"{
            "id": "legacy-001",
            "name": "old session",
            "created_at": "2026-06-01T12:00:00Z",
            "updated_at": "2026-06-01T12:30:00Z",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi there"},
                {"role": "assistant", "tool_calls": [{"id": "tc1"}]}
            ]
        }"#;
        tokio::fs::write(dir.join("legacy-001.json"), legacy_json)
            .await
            .unwrap();

        let mgr = SessionManager::with_dir(dir);
        let loaded = mgr.load_all().await.unwrap();
        assert_eq!(loaded, 1, "legacy session should be loaded");

        let list = mgr.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "legacy-001");
        assert_eq!(list[0].name, "old session");
        assert_eq!(list[0].message_count, 3);
        assert_eq!(list[0].status, SessionStatus::Active);
    }

    #[tokio::test]
    async fn load_index_loads_metadata_without_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().to_path_buf());

        // Create a session with messages.
        let mut session = mgr.create(Some("indexed-session")).await.unwrap();
        session.add_message("user", "hello");
        session.add_message("assistant", "world");
        mgr.save(&session).await.unwrap();

        // Simulate a restart: new manager with the same dir.
        let restarted = SessionManager::with_dir(tmp.path().to_path_buf());
        let loaded = restarted.load_index().await.unwrap();
        assert_eq!(loaded, 1);

        // list() should return the correct message_count even though
        // messages were not deserialized.
        let list = restarted.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "indexed-session");
        assert_eq!(
            list[0].message_count, 2,
            "lazy count should reflect real message count"
        );

        // get() should hydrate the full session on demand.
        let full = restarted.get(&session.id).await.unwrap();
        assert_eq!(
            full.messages.len(),
            2,
            "hydrated session should have full messages"
        );
        assert_eq!(full.messages[0].content, "hello");
        assert!(
            full.lazy_message_count.is_none(),
            "hydrated session should not be lazy"
        );
    }

    #[test]
    fn with_id_preserves_caller_supplied_id() {
        let session = Session::with_id("fixed-id-123", Some("named"));
        assert_eq!(session.id, "fixed-id-123");
        assert_eq!(session.name, "named");
    }

    #[tokio::test]
    async fn save_upsert_reuses_same_id_and_file() {
        // Regression: previously Session::new() on the upsert path minted a new
        // UUID each save, so the same logical session produced many files with
        // the same display name.
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().to_path_buf());
        let fixed_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

        for i in 0..3 {
            let mut session = mgr
                .load(fixed_id)
                .await
                .unwrap()
                .unwrap_or_else(|| Session::with_id(fixed_id, None));
            session.id = fixed_id.to_string();
            session.name = "same-name".to_string();
            session.messages.push(SessionMessage {
                role: "user".to_string(),
                content: format!("msg-{i}"),
                tool_call_id: None,
                tool_calls: None,
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            });
            session.updated_at = Utc::now();
            session.lazy_message_count = None;
            mgr.save(&session).await.unwrap();
        }

        // Exactly one file on disk, named after the fixed id.
        let mut entries = tokio::fs::read_dir(tmp.path()).await.unwrap();
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(files, vec![format!("{fixed_id}.json")]);

        let loaded = mgr.load(fixed_id).await.unwrap().unwrap();
        assert_eq!(loaded.id, fixed_id);
        assert_eq!(loaded.name, "same-name");
        assert_eq!(loaded.messages.len(), 3);

        // In-memory list also has a single entry.
        let list = mgr.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, fixed_id);
        assert_eq!(list[0].message_count, 3);
    }

    #[tokio::test]
    async fn save_updates_in_memory_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().to_path_buf());

        let mut session = Session::with_id("index-id", Some("before"));
        mgr.save(&session).await.unwrap();
        session.name = "after".to_string();
        session.add_message("user", "hi");
        mgr.save(&session).await.unwrap();

        let list = mgr.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "after");
        assert_eq!(list[0].message_count, 1);
    }

    /// Regression: restoring a session must preserve `tool_call_id` and
    /// `tool_calls` so the replayed history keeps the assistant `tool_calls`
    /// <-> `tool` pairing. Previously `SessionMessage` had no such fields, so
    /// serde silently dropped them on save; the restored `role="tool"` message
    /// arrived with `tool_call_id = None`, the provider rejected the next
    /// request with `MissingParameter: missing messages.tool_call_id`.
    ///
    /// This simulates the full wire round-trip the daemon/TUI perform:
    ///   save:  ChatMessage  --serialize--> JSON --deserialize--> SessionMessage
    ///   load:  SessionMessage --serialize--> JSON --deserialize--> ChatMessage
    #[test]
    fn session_message_round_trip_preserves_tool_call_pairing() {
        use crate::api::{ChatMessage, ToolCall, ToolCallFunction};

        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_abc".to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: "file_read".to_string(),
                    arguments: r#"{"path":"x.rs"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let tool_result = ChatMessage::tool("call_abc", "file contents");

        // Save leg: ChatMessage -> SessionMessage (daemon receives the PUT body).
        let saved: Vec<SessionMessage> =
            serde_json::from_str(&serde_json::to_string(&vec![assistant, tool_result]).unwrap())
                .expect("ChatMessage JSON must deserialize into SessionMessage");

        // tool_calls survive on the assistant message.
        assert_eq!(saved[0].role, "assistant");
        let tc = saved[0]
            .tool_calls
            .as_ref()
            .expect("assistant tool_calls must be preserved on save")
            .first()
            .unwrap();
        assert_eq!(tc.id, "call_abc");
        // tool_call_id survives on the tool result message.
        assert_eq!(saved[1].role, "tool");
        assert_eq!(
            saved[1].tool_call_id.as_deref(),
            Some("call_abc"),
            "tool_call_id must be preserved on save"
        );

        // Load leg: SessionMessage -> ChatMessage (TUI decodes the GET response).
        let restored: Vec<ChatMessage> =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap())
                .expect("SessionMessage JSON must deserialize back into ChatMessage");

        // After the full round-trip the pairing is intact and - critically -
        // the `tool` message still carries its `tool_call_id`, so the replayed
        // request is no longer missing the parameter the provider requires.
        assert_eq!(restored[1].role, "tool");
        assert_eq!(
            restored[1].tool_call_id.as_deref(),
            Some("call_abc"),
            "tool_call_id must survive the save+load round-trip"
        );
        assert_eq!(
            restored[0]
                .tool_calls
                .as_ref()
                .and_then(|cs| cs.first())
                .map(|c| c.id.as_str()),
            Some("call_abc"),
            "assistant tool_calls must survive the round-trip"
        );

        // The serialized `tool` message must actually emit `tool_call_id`
        // (not be skipped), which is what the provider validation checks.
        let wire = serde_json::to_string(&restored[1]).unwrap();
        assert!(
            wire.contains("\"tool_call_id\":\"call_abc\""),
            "serialized tool message must include tool_call_id, got: {wire}"
        );
    }

    #[test]
    fn session_ui_messages_round_trip_and_legacy_default() {
        let ui = SessionUiMessage {
            role: "tool".to_string(),
            content: "diff output".to_string(),
            tool_name: Some("apply_patch".to_string()),
            tool_args: Some(serde_json::json!({"path": "a.rs"})),
            content_collapsed: true,
            tool_collapsed: false,
            diff_data: Some(SessionDiffData {
                file_path: "a.rs".to_string(),
                old_content: "old".to_string(),
                new_content: "new".to_string(),
            }),
            tool_metadata: Some(serde_json::json!({"lines": 2})),
        };

        let mut session = Session::with_id("ui-track", Some("ui-track"));
        session.messages.push(SessionMessage {
            role: "user".to_string(),
            content: "compacted model history".to_string(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        });
        session.ui_messages.push(ui.clone());

        let json = serde_json::to_string(&session).expect("serialize session");
        assert!(json.contains("ui_messages"));
        assert!(json.contains("apply_patch"));

        let loaded: Session = serde_json::from_str(&json).expect("deserialize session");
        assert_eq!(loaded.ui_messages.len(), 1);
        assert_eq!(loaded.ui_messages[0], ui);
        assert_eq!(loaded.messages.len(), 1);

        // Legacy files without ui_messages deserialize to empty UI track.
        let legacy = r#"{
            "id":"legacy",
            "name":"legacy",
            "project_path":null,
            "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z",
            "messages":[]
        }"#;
        let legacy_session: Session =
            serde_json::from_str(legacy).expect("legacy session without ui_messages");
        assert!(legacy_session.ui_messages.is_empty());
    }

    #[tokio::test]
    async fn save_load_preserves_ui_messages_independently_of_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = SessionManager::with_dir(tmp.path().to_path_buf());

        let mut session = Session::with_id("dual-track", Some("dual"));
        session.add_message("user", "model-facing only");
        session.ui_messages.push(SessionUiMessage {
            role: "system".to_string(),
            content: "notice kept only on UI track".to_string(),
            tool_name: None,
            tool_args: None,
            content_collapsed: false,
            tool_collapsed: false,
            diff_data: None,
            tool_metadata: None,
        });
        session.ui_messages.push(SessionUiMessage {
            role: "user".to_string(),
            content: "full user text before compact".to_string(),
            tool_name: None,
            tool_args: None,
            content_collapsed: false,
            tool_collapsed: false,
            diff_data: None,
            tool_metadata: None,
        });

        mgr.save(&session).await.unwrap();
        let loaded = mgr.load("dual-track").await.unwrap().unwrap();

        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, "model-facing only");
        assert_eq!(loaded.ui_messages.len(), 2);
        assert_eq!(loaded.ui_messages[0].role, "system");
        assert_eq!(
            loaded.ui_messages[1].content,
            "full user text before compact"
        );
    }
}
