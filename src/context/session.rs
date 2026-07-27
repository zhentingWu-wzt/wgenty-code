//! Session Module - Session management (DEPRECATED: use memory_session.rs)

#![allow(dead_code)] // Deprecated module, kept for backward compatibility
#![allow(unused_imports)]

use crate::api::ChatMessage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

/// Validate a session ID to prevent path traversal attacks.
fn validate_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && !id.starts_with('.')
}

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

    /// Ensure session ID is safe for filesystem use.
    fn check_id(&self, id: &str) -> anyhow::Result<()> {
        if !validate_id(id) {
            anyhow::bail!("Invalid session ID: {id}");
        }
        Ok(())
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
                                    let truncated: String = c.chars().take(80).collect();
                                    format!("{}...", truncated)
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
                    } else {
                        warn!("Skipping corrupt session file: {}", path.display());
                    }
                }
            }
        }

        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
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
            turn_records: Vec::new(),
        };

        self.save(&session)?;

        Ok(session)
    }

    /// Load a session by ID
    pub fn load(&self, id: &str) -> anyhow::Result<Option<Session>> {
        self.check_id(id)?;
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
        self.check_id(&session.id)?;
        std::fs::create_dir_all(&self.sessions_dir)?;

        let path = self.sessions_dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, content)?;

        Ok(())
    }

    /// Delete a session
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.check_id(id)?;
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

/// A single TUI REPL turn record, persisted alongside [`Session`] messages.
///
/// Each entry captures the metadata needed by the `/undo` interactive rollback
/// flow: which checkpoint to rewind to, how many messages belong to this turn,
/// and how many files were edited.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    /// Unique identifier for this turn (e.g. UUID or incrementing counter).
    pub turn_id: String,
    /// ISO-8601 timestamp marking when the turn was created.
    pub created_at: String,
    /// First sentence of the user's input, shown in the turn-picker popup.
    pub user_summary: String,
    /// Identifier of the file checkpoint taken **before** this turn's edits
    /// (pre-edit snapshot).  Used by code-rollback to restore file state.
    pub checkpoint_turn_id: String,
    /// Index into `conversation_history` marking the end of this turn
    /// (i.e. `conversation_history.len()` when the turn completed).
    pub message_end_idx: usize,
    /// Number of files edited during this turn (`0` = pure conversation).
    pub file_count: usize,
    /// Index into `committed_messages` (UI display) at turn completion;
    /// `/undo` syncs the UI by truncating to this index.
    #[serde(default)]
    pub committed_messages_end_idx: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<ChatMessage>,
    /// Per-turn records for `/undo` rollback.  `#[serde(default)]` ensures
    /// old session files (without this field) deserialize to an empty Vec.
    #[serde(default)]
    pub turn_records: Vec<TurnRecord>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Session` serialized to JSON must include the `turn_records` field so
    /// that turn history is persisted alongside conversation messages.
    #[test]
    fn session_serialization_contains_turn_records() {
        // Construct a Session via JSON round-trip so the test compiles even
        // before the `turn_records` field exists (RED → runtime failure).
        let json = r#"{"id":"x","name":"y","created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z","messages":[]}"#;
        let session: Session = serde_json::from_str(json).expect("parse session");
        let serialized = serde_json::to_string(&session).expect("serialize session");
        assert!(
            serialized.contains("\"turn_records\""),
            "serialized session JSON should contain turn_records field, got: {serialized}"
        );
    }

    /// Old session files (saved before `turn_records` existed) must still
    /// deserialize successfully, with `turn_records` defaulting to an empty Vec.
    #[test]
    fn old_session_without_turn_records_deserializes_to_empty_vec() {
        let old_json = r#"{"id":"old","name":"old","created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z","messages":[]}"#;
        let session: Session =
            serde_json::from_str(old_json).expect("old session should deserialize");
        // Inspect via serde_json::Value so the test compiles without the field.
        let value: serde_json::Value =
            serde_json::to_value(&session).expect("convert session to json value");
        let turn_records = value
            .get("turn_records")
            .expect("turn_records field should exist after deserialization");
        assert!(turn_records.is_array(), "turn_records should be an array");
        assert!(
            turn_records.as_array().unwrap().is_empty(),
            "turn_records should default to empty Vec for old sessions"
        );
    }
}
