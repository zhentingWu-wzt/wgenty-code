//! Session Module - Session management

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
        let sessions_dir = home.join(".claude-code").join("sessions");

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
