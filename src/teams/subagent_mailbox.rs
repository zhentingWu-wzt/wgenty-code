//! Subagent Result Mailbox — JSONL storage for subagent result persistence.
//!
//! When a subagent (task or delegate) produces a result, the **full content is
//! always returned to the parent agent inline — never truncated.** For results
//! exceeding the persistence threshold, a copy is additionally stored to a
//! JSONL mailbox file so the full result can be recovered later (e.g. after
//! context compaction) via `file_read`.
//!
//! # Design rationale
//!
//! Subagent results represent meaningful work (code exploration, architecture
//! analysis, multi-step research) and must not be lossy. Previously, large
//! results were replaced with a 200-character summary, forcing the parent to
//! manually `file_read` the full content — and often losing critical detail.
//! The current design preserves the full result inline while keeping a disk
//! copy as a safety net.
//!
//! # File layout
//! ```text
//! ~/.wgenty-code/task_results/
//!   {subagent_name}_{session_id}_{uuid}.jsonl
//! ```
//!
//! Each JSONL line contains a `StoredResult` record with a header line followed
//! by one or more content lines.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Threshold (chars) above which a subagent result is *additionally* persisted
/// to disk for recovery. The full content is always returned to the parent
/// regardless of this threshold — it only controls whether a disk copy is made.
pub const MAX_INLINE_RESULT_LEN: usize = 4000;

/// A stored subagent result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredResult {
    /// Full task description.
    pub description: String,
    /// Subagent type (general-purpose, explore, plan) or "delegate".
    pub subagent_type: String,
    /// Result content (may span multiple JSONL lines for very large results).
    pub content: String,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Session ID that spawned the subagent.
    pub session_id: String,
    /// Whether the result is stored as multiple lines.
    #[serde(default)]
    pub is_multiline: bool,
}

/// Wrapper returned by `offload_if_large`. The full content is **always** carried
/// inline so the parent agent never loses information. When the result exceeds
/// the persistence threshold, a copy is also saved to disk and the path is
/// included for later recovery.
#[derive(Debug, Clone)]
pub enum SubagentResponse {
    /// Result returned inline (not persisted separately).
    Inline { content: String },
    /// Result persisted to disk for recovery; full content still returned inline.
    Offloaded {
        content: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
}

impl SubagentResponse {
    /// Turn this response into text suitable for tool output.
    ///
    /// Always returns the **full** result content — no truncation. For
    /// `Offloaded` results, a short footer notes the on-disk recovery path.
    pub fn to_content(&self) -> String {
        match self {
            Self::Inline { content } => content.clone(),
            Self::Offloaded {
                content,
                mailbox_path,
                content_len,
            } => {
                format!(
                    "{content}\n\n\
                     ---\n\
                     [Full result ({content_len} chars) persisted at: `{path}` for recovery]",
                    content = content,
                    content_len = content_len,
                    path = mailbox_path.display(),
                )
            }
        }
    }

    /// Return the full content without truncation.
    ///
    /// Previously this truncated large inline results for compaction safety.
    /// Subagent results are considered important and must never be truncated,
    /// so this now delegates to [`to_content`](Self::to_content).
    pub fn to_compact(&self) -> String {
        self.to_content()
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Inline { content } => content.len(),
            Self::Offloaded { content_len, .. } => *content_len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Mailbox for persisting and retrieving large subagent results.
#[derive(Clone)]
pub struct SubagentResultMailbox {
    base_dir: PathBuf,
}

impl SubagentResultMailbox {
    /// Create a new mailbox rooted at `base_dir`.
    /// Creates the directory if it doesn't exist.
    pub fn new(base_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&base_dir).ok();
        Self { base_dir }
    }

    /// Create a mailbox at the default location: `~/.wgenty-code/task_results/`.
    pub fn default_location() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".wgenty-code").join("task_results"))
    }

    /// Generate a filename for a stored result.
    fn make_filename(subagent_type: &str, description: &str, session_id: &str) -> String {
        let safe_desc = description
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect::<String>();
        let short_uuid = uuid::Uuid::new_v4().to_string();
        format!(
            "{}_{}_{}_{}.jsonl",
            subagent_type,
            safe_desc,
            &session_id[..session_id.len().min(16)],
            &short_uuid[..8]
        )
    }

    /// Try to store a result to the mailbox. Returns the file path on success.
    pub fn store(
        &self,
        subagent_type: &str,
        description: &str,
        session_id: &str,
        content: &str,
    ) -> std::io::Result<PathBuf> {
        let filename = Self::make_filename(subagent_type, description, session_id);
        let path = self.base_dir.join(&filename);

        let entry = StoredResult {
            description: description.to_string(),
            subagent_type: subagent_type.to_string(),
            content: content.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            session_id: session_id.to_string(),
            is_multiline: false,
        };

        let json = serde_json::to_string(&entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, json + "\n")?;

        tracing::info!(
            path = %path.display(),
            content_len = content.len(),
            "Stored subagent result to mailbox"
        );

        Ok(path)
    }

    /// Read a previously stored result from the given path.
    pub fn read(&self, path: &std::path::Path) -> std::io::Result<StoredResult> {
        let content = std::fs::read_to_string(path)?;
        let entry: StoredResult = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(entry)
    }

    /// Persist a subagent result to disk if it exceeds the persistence threshold.
    ///
    /// The **full content is always returned** to the caller (never truncated).
    /// When the content exceeds `MAX_INLINE_RESULT_LEN`, a copy is additionally
    /// stored to disk so it can be recovered later; the returned
    /// [`SubagentResponse::Offloaded`] variant carries both the full content and
    /// the disk path. If disk persistence fails, the full content is still
    /// returned inline (no data loss — just no recovery copy).
    pub fn offload_if_large(
        &self,
        subagent_type: &str,
        description: &str,
        session_id: &str,
        content: &str,
    ) -> SubagentResponse {
        if content.len() <= MAX_INLINE_RESULT_LEN {
            SubagentResponse::Inline {
                content: content.to_string(),
            }
        } else {
            match self.store(subagent_type, description, session_id, content) {
                Ok(path) => SubagentResponse::Offloaded {
                    content: content.to_string(),
                    mailbox_path: path,
                    content_len: content.len(),
                },
                Err(e) => {
                    // Fallback: return full content inline even if persistence fails.
                    // No truncation — just no recovery copy on disk.
                    tracing::warn!(
                        error = %e,
                        "Failed to persist subagent result; returning full inline (no recovery copy)"
                    );
                    SubagentResponse::Inline {
                        content: content.to_string(),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_result_stays_inline() {
        let mailbox = SubagentResultMailbox::new(std::env::temp_dir().join("wgenty_test_mailbox"));
        let response = mailbox.offload_if_large(
            "explore",
            "find auth functions",
            "session1",
            "Found 3 functions: authenticate, authorize, refresh_token",
        );
        match response {
            SubagentResponse::Inline { content } => {
                assert!(content.contains("authenticate"));
            }
            _ => panic!("Expected Inline"),
        }
    }

    #[test]
    fn test_large_result_offloaded_with_full_content() {
        let mailbox =
            SubagentResultMailbox::new(std::env::temp_dir().join("wgenty_test_mailbox_large"));
        let large_content = "A".repeat(5000);
        let response = mailbox.offload_if_large(
            "general-purpose",
            "large analysis",
            "session2",
            &large_content,
        );
        match response {
            SubagentResponse::Offloaded {
                ref content,
                ref content_len,
                ref mailbox_path,
            } => {
                assert_eq!(*content_len, 5000);
                // Full content is preserved — not truncated to a summary.
                assert_eq!(content.len(), 5000);
                assert_eq!(content.as_str(), large_content.as_str());
                // A recovery copy was stored to disk.
                assert!(mailbox_path.exists());
            }
            _ => panic!("Expected Offloaded"),
        }
    }

    #[test]
    fn test_offloaded_to_content_returns_full_result() {
        let mailbox =
            SubagentResultMailbox::new(std::env::temp_dir().join("wgenty_test_mailbox_content"));
        let large_content = "B".repeat(5000);
        let response =
            mailbox.offload_if_large("explore", "content test", "session3", &large_content);
        let text = response.to_content();
        // Full 5000-char content is present — not a 200-char summary.
        assert!(text.contains(&large_content));
        // Recovery path is mentioned in the footer.
        assert!(text.contains("persisted"));
    }

    #[test]
    fn test_inline_to_content_unchanged() {
        let resp = SubagentResponse::Inline {
            content: "short result".to_string(),
        };
        assert_eq!(resp.to_content(), "short result");
    }

    #[test]
    fn test_to_compact_returns_full_content() {
        let large = "X".repeat(5000);
        let resp = SubagentResponse::Inline {
            content: large.clone(),
        };
        let compact = resp.to_compact();
        // No truncation — full content preserved.
        assert_eq!(compact, large);
    }

    #[test]
    fn test_offloaded_to_compact_returns_full_content() {
        let mailbox =
            SubagentResultMailbox::new(std::env::temp_dir().join("wgenty_test_mailbox_compact"));
        let large_content = "Y".repeat(5000);
        let response = mailbox.offload_if_large("plan", "compact test", "session4", &large_content);
        let compact = response.to_compact();
        // Full content preserved, no truncation.
        assert!(compact.contains(&large_content));
    }
}
