//! JSONL mailbox for team communication (s09 team-message protocol).
//!
//! Each team member has a dedicated JSONL inbox file at `.team/inbox/{name}.jsonl`.
//! Messages are appended; reading drains the mailbox (read + truncate).
//! Thread safety is provided by per-mailbox tokio::sync::Mutex.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Message types for team communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TeamMessage {
    /// Direct message to a team member
    Message {
        from: String,
        to: String,
        content: String,
        timestamp: String,
    },
    /// Broadcast to all team members
    Broadcast {
        from: String,
        content: String,
        timestamp: String,
    },
    /// Request to shut down a teammate
    ShutdownRequest { from: String, request_id: String },
    /// Response to a shutdown request
    ShutdownResponse {
        from: String,
        request_id: String,
        approve: bool,
    },
}

/// Configuration for the team
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub team_name: String,
    pub members: Vec<TeamMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub status: String, // "idle", "working", "closed"
}

/// A JSONL mailbox for a team member
pub struct Mailbox {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Mailbox {
    /// Open (or create) a mailbox at the given path
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Send a message to this mailbox (append JSON line)
    pub async fn send(&self, message: &TeamMessage) -> std::io::Result<()> {
        let _guard = self.lock.lock().await;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let json = serde_json::to_string(message)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let line = json + "\n";

        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?
            .write_all(line.as_bytes())
            .await?;

        Ok(())
    }

    /// Read and drain all messages from this mailbox
    pub async fn receive_all(&self) -> std::io::Result<Vec<TeamMessage>> {
        let _guard = self.lock.lock().await;

        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let mut messages = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<TeamMessage>(&line) {
                messages.push(msg);
            }
        }

        // Drain: truncate the file
        fs::write(&self.path, "").await?;

        Ok(messages)
    }

    /// Check if there are pending messages (without draining)
    pub async fn has_messages(&self) -> std::io::Result<bool> {
        let _guard = self.lock.lock().await;

        if !self.path.exists() {
            return Ok(false);
        }

        let metadata = fs::metadata(&self.path).await?;
        Ok(metadata.len() > 0)
    }
}

/// Team manager that coordinates mailboxes for all members
pub struct TeamManager {
    #[allow(dead_code)]
    base_dir: PathBuf,
    config: TeamConfig,
    mailboxes: HashMap<String, Mailbox>,
}

impl TeamManager {
    /// Load team configuration from .team/config.json
    pub fn load(project_root: &std::path::Path) -> Option<Self> {
        let config_path = project_root.join(".team").join("config.json");
        if !config_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&config_path).ok()?;
        let config: TeamConfig = serde_json::from_str(&content).ok()?;

        let base_dir = project_root.join(".team").join("inbox");
        let mut mailboxes = HashMap::new();

        for member in &config.members {
            let mbox_path = base_dir.join(format!("{}.jsonl", member.name));
            mailboxes.insert(member.name.clone(), Mailbox::new(mbox_path));
        }

        Some(Self {
            base_dir,
            config,
            mailboxes,
        })
    }

    /// Send a message to a specific team member's mailbox
    pub async fn send_to(&self, member_name: &str, message: &TeamMessage) -> std::io::Result<()> {
        if let Some(mailbox) = self.mailboxes.get(member_name) {
            mailbox.send(message).await
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Team member '{}' not found", member_name),
            ))
        }
    }

    /// Broadcast a message to all team members
    pub async fn broadcast(&self, from: &str, content: &str) -> Vec<std::io::Result<()>> {
        let msg = TeamMessage::Broadcast {
            from: from.to_string(),
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let mut results = Vec::new();
        for member in &self.config.members {
            if member.name != from {
                results.push(self.send_to(&member.name, &msg).await);
            }
        }
        results
    }

    /// Send direct message
    pub async fn send_message(&self, from: &str, to: &str, content: &str) -> std::io::Result<()> {
        let msg = TeamMessage::Message {
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        self.send_to(to, &msg).await
    }

    /// Read all messages from a member's mailbox (drains the mailbox)
    pub async fn receive(&self, member_name: &str) -> std::io::Result<Vec<TeamMessage>> {
        if let Some(mailbox) = self.mailboxes.get(member_name) {
            mailbox.receive_all().await
        } else {
            Ok(Vec::new())
        }
    }

    /// Get team members
    pub fn members(&self) -> Vec<&TeamMember> {
        self.config.members.iter().collect()
    }

    /// Get team name
    pub fn team_name(&self) -> &str {
        &self.config.team_name
    }

    /// Check if team is configured
    pub fn is_active(&self) -> bool {
        !self.config.members.is_empty()
    }
}
