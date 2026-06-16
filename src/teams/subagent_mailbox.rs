//! Subagent Result Mailbox — JSONL storage for large subagent results.
//!
//! When a subagent (task or delegate) produces a result that is too large to
//! pass back inline in the conversation context, the full result is persisted
//! to a JSONL mailbox file. The parent agent receives a compact reference
//! containing a summary and the path to the full result.
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

/// Maximum inline result length (chars) before offloading to mailbox.
pub const MAX_INLINE_RESULT_LEN: usize = 4000;

/// The summary prefix length shown to the parent agent.
const SUMMARY_PREFIX_LEN: usize = 200;

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

/// Wrapper returned by `offload_if_large`. Contains either the full text or a
/// reference + summary when the result was offloaded to disk.
#[derive(Debug, Clone)]
pub enum SubagentResponse {
    /// Result was small enough to return inline.
    Inline { content: String },
    /// Large result stored to disk; parent sees summary + reference.
    Offloaded {
        summary: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
}

impl SubagentResponse {
    /// Turn this response into text suitable for tool output.
    pub fn to_content(&self) -> String {
        match self {
            Self::Inline { content } => content.clone(),
            Self::Offloaded {
                summary,
                mailbox_path,
                content_len,
            } => {
                format!(
                    "Subagent completed successfully.\n\n\
                     **Full result stored at:** `{}`\n\
                     **Result size:** {} chars\n\n\
                     **Summary:**\n{}",
                    mailbox_path.display(),
                    content_len,
                    summary,
                )
            }
        }
    }

    /// Turn this response into a short inline result (for compaction safety).
    /// Truncates Inline results to MAX_INLINE_RESULT_LEN, references path for Offloaded.
    pub fn to_compact(&self) -> String {
        match self {
            Self::Inline { content } => {
                if content.len() <= MAX_INLINE_RESULT_LEN {
                    content.clone()
                } else {
                    format!(
                        "{}…\n\n[truncated: {} total chars]",
                        &content[..MAX_INLINE_RESULT_LEN],
                        content.len()
                    )
                }
            }
            Self::Offloaded {
                summary,
                mailbox_path,
                content_len,
            } => {
                format!(
                    "Subagent completed. Full result ({content_len} chars) at `{path}`.\n{summary}",
                    content_len = content_len,
                    path = mailbox_path.display(),
                    summary = summary,
                )
            }
        }
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

    /// Produce a summary string from the full result content.
    /// Returns the first `SUMMARY_PREFIX_LEN` characters followed by ellipsis.
    pub fn summarize(content: &str) -> String {
        if content.len() <= SUMMARY_PREFIX_LEN {
            content.to_string()
        } else {
            // Try to break at a natural boundary (newline or space).
            let prefix = &content[..SUMMARY_PREFIX_LEN];
            let break_point = prefix
                .rfind('\n')
                .or_else(|| prefix.rfind(' '))
                .unwrap_or(SUMMARY_PREFIX_LEN);
            let truncated = &prefix[..break_point];
            format!("{}…", truncated)
        }
    }

    /// Offload a subagent result if it exceeds the inline threshold.
    /// Returns an `SubagentResponse` with either the full content or a reference.
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
                Ok(path) => {
                    let summary = Self::summarize(content);
                    SubagentResponse::Offloaded {
                        summary,
                        mailbox_path: path,
                        content_len: content.len(),
                    }
                }
                Err(e) => {
                    // Fallback: return truncated inline on I/O error.
                    tracing::warn!(
                        error = %e,
                        "Failed to store subagent result; returning truncated inline"
                    );
                    SubagentResponse::Inline {
                        content: format!(
                            "{}…\n\n[truncated: {} total chars, storage failed: {}]",
                            &content[..content.len().min(MAX_INLINE_RESULT_LEN)],
                            content.len(),
                            e
                        ),
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
    fn test_large_result_offloaded() {
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
                ref summary,
                ref content_len,
                ..
            } => {
                assert_eq!(*content_len, 5000);
                // '…' (U+2026) is 3 bytes in UTF-8.
                assert!(summary.len() <= SUMMARY_PREFIX_LEN + '…'.len_utf8());
            }
            SubagentResponse::Inline { .. } => panic!("Expected Offloaded"),
        }
    }

    #[test]
    fn test_summarize_short_content() {
        let content = "Short result";
        let summary = SubagentResultMailbox::summarize(content);
        assert_eq!(summary, "Short result");
    }

    #[test]
    fn test_summarize_long_content() {
        let content = "A".repeat(500);
        let summary = SubagentResultMailbox::summarize(&content);
        assert!(summary.ends_with('…'));
        // '…' (U+2026) is 3 bytes in UTF-8.
        assert!(summary.len() <= SUMMARY_PREFIX_LEN + '…'.len_utf8());
    }

    #[test]
    fn test_subagent_response_to_content() {
        let inline = SubagentResponse::Inline {
            content: "result".to_string(),
        };
        assert_eq!(inline.to_content(), "result");

        let offloaded = SubagentResponse::Offloaded {
            summary: "summary…".to_string(),
            mailbox_path: PathBuf::from("/tmp/result.jsonl"),
            content_len: 5000,
        };
        let content = offloaded.to_content();
        assert!(content.contains("/tmp/result.jsonl"));
        assert!(content.contains("5000"));
        assert!(content.contains("summary…"));
    }

    #[test]
    fn test_to_compact_truncates_large_inline() {
        let large = "X".repeat(5000);
        let resp = SubagentResponse::Inline { content: large };
        let compact = resp.to_compact();
        assert_eq!(
            compact.len(),
            MAX_INLINE_RESULT_LEN + "…\n\n[truncated: 5000 total chars]".len()
        );
    }
}
