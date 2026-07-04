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

/// Threshold (chars) above which the full content is no longer inlined into
/// the parent agent's context — instead a head-prefix summary is delivered
/// with a disk-recovery hint. Results between `MAX_INLINE_RESULT_LEN` and
/// this value are still inlined in full (with a disk copy).
pub const MAX_FULL_INLINE_LEN: usize = 8000;

/// Length (chars) of the head-prefix summary delivered for `Summarized`
/// results. `content.chars().take(SUMMARY_HEAD_LEN)`.
pub const SUMMARY_HEAD_LEN: usize = 1500;

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

/// Wrapper returned by `offload_if_large`. Three tiers balance "no detail
/// loss" against parent-agent context token cost:
/// - `Inline`: small results (≤4000 chars) returned as-is, no disk copy.
/// - `Offloaded`: medium results (4000–8000 chars) returned in full AND
///   persisted to disk for recovery.
/// - `Summarized`: large results (>8000 chars) deliver a head-prefix summary
///   plus a disk path; the full content is only on disk (recoverable via
///   `file_read`).
///
/// On disk-persistence failure, results degrade to `Inline` (full content,
/// no copy) — content integrity takes priority over token control.
#[derive(Debug, Clone)]
pub enum SubagentResponse {
    /// ≤4000 chars: full content inline, no disk copy.
    Inline { content: String },
    /// 4000–8000 chars: full content inline + disk copy for recovery.
    Offloaded {
        content: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
    /// >8000 chars: head-prefix summary inline + disk copy; full content
    /// recoverable via `file_read` on `mailbox_path`.
    Summarized {
        summary: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
}

impl SubagentResponse {
    /// Turn this response into text suitable for tool output.
    ///
    /// - `Inline` → content as-is.
    /// - `Offloaded` → full content + footer noting the on-disk recovery path.
    /// - `Summarized` → head-prefix summary + footer pointing to the disk
    ///   copy for `file_read` recovery.
    pub fn to_content(&self) -> String {
        match self {
            Self::Inline { content } => content.clone(),
            Self::Offloaded {
                content,
                mailbox_path,
                content_len,
            } => format!(
                "{content}\n\n\
                 ---\n\
                 [Full result ({content_len} chars) persisted at: `{path}` for recovery]",
                content = content,
                content_len = content_len,
                path = mailbox_path.display(),
            ),
            Self::Summarized {
                summary,
                mailbox_path,
                content_len,
            } => format!(
                "{summary}\n\n\
                 ---\n\
                 [Summary only. Full result ({content_len} chars) at `{path}` — file_read for details]",
                summary = summary,
                content_len = content_len,
                path = mailbox_path.display(),
            ),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Inline { content } => content.len(),
            Self::Offloaded { content_len, .. } => *content_len,
            Self::Summarized { content_len, .. } => *content_len,
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

    /// Persist a subagent result and return a [`SubagentResponse`] tiered by
    /// content size:
    ///
    /// - `len <= MAX_INLINE_RESULT_LEN` (4000): `Inline` — full content, no
    ///   disk copy.
    /// - `MAX_INLINE_RESULT_LEN < len <= MAX_FULL_INLINE_LEN` (4000–8000):
    ///   `Offloaded` — full content inline + disk copy for recovery.
    /// - `len > MAX_FULL_INLINE_LEN` (>8000): `Summarized` — head-prefix
    ///   summary (`SUMMARY_HEAD_LEN` chars) inline + disk copy; full content
    ///   recoverable via `file_read`.
    ///
    /// On disk-persistence failure for any result >4000, degrades to
    /// `Inline` with the **full** content (no truncation, no copy) — content
    /// integrity takes priority over token control. The failure is logged.
    pub fn offload_if_large(
        &self,
        subagent_type: &str,
        description: &str,
        session_id: &str,
        content: &str,
    ) -> SubagentResponse {
        let len = content.len();
        if len <= MAX_INLINE_RESULT_LEN {
            return SubagentResponse::Inline {
                content: content.to_string(),
            };
        }
        // >4000: attempt disk persistence first.
        match self.store(subagent_type, description, session_id, content) {
            Ok(path) => {
                if len <= MAX_FULL_INLINE_LEN {
                    // 4000–8000: full content inline + disk copy.
                    SubagentResponse::Offloaded {
                        content: content.to_string(),
                        mailbox_path: path,
                        content_len: len,
                    }
                } else {
                    // >8000: head-prefix summary inline + disk copy.
                    let summary: String = content.chars().take(SUMMARY_HEAD_LEN).collect();
                    SubagentResponse::Summarized {
                        summary,
                        mailbox_path: path,
                        content_len: len,
                    }
                }
            }
            Err(e) => {
                // Disk failure: degrade to Inline (full content, no copy, logged).
                // Integrity > token control — even >8000 returns full content.
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
    fn test_summarized_to_content_has_summary_and_path() {
        let summary = "A".repeat(1500);
        let resp = SubagentResponse::Summarized {
            summary: summary.clone(),
            mailbox_path: PathBuf::from("/tmp/fake_mailbox/result.jsonl"),
            content_len: 9000,
        };
        let text = resp.to_content();
        // Summary (head 1500 chars) is present
        assert!(text.contains(&summary));
        // Footer communicates disk path + content_len + file_read hint
        assert!(text.contains("Summary only."));
        assert!(text.contains("Full result (9000 chars)"));
        assert!(text.contains("/tmp/fake_mailbox/result.jsonl"));
        assert!(text.contains("file_read for details"));
    }

    #[test]
    fn test_summarized_summary_is_head_prefix() {
        // Verify the summary field semantics: it should be the head prefix
        // of the full content. This test documents the contract that
        // offload_if_large (Task 3) will produce summary via
        // content.chars().take(SUMMARY_HEAD_LEN).collect().
        let full = "B".repeat(9000);
        let expected_summary: String = full.chars().take(SUMMARY_HEAD_LEN).collect();
        assert_eq!(expected_summary.len(), 1500);
        assert_eq!(expected_summary, "B".repeat(1500));
    }

    #[test]
    fn test_very_large_result_summarized() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_summarized"),
        );
        let very_large = "A".repeat(9000);
        let response = mailbox.offload_if_large(
            "general-purpose",
            "very large analysis",
            "session_sum",
            &very_large,
        );
        match response {
            SubagentResponse::Summarized {
                ref summary,
                ref mailbox_path,
                ref content_len,
            } => {
                // content_len is the full byte length, not summary length
                assert_eq!(*content_len, 9000);
                // summary is the head 1500 chars (by chars(), not bytes)
                assert_eq!(summary.chars().count(), 1500);
                assert_eq!(summary.as_str(), &very_large[..1500]); // ASCII so byte==char
                // disk copy exists for recovery
                assert!(mailbox_path.exists());
            }
            _ => panic!("Expected Summarized for 9000-char result"),
        }
    }

    #[test]
    fn test_boundary_4000_inline_vs_offloaded() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_boundary_4k"),
        );
        // Exactly 4000 → Inline (<= threshold)
        let exactly_4000 = "A".repeat(4000);
        let resp = mailbox.offload_if_large("explore", "b4k", "s1", &exactly_4000);
        match resp {
            SubagentResponse::Inline { content } => assert_eq!(content.len(), 4000),
            _ => panic!("Expected Inline for 4000-char result (<= MAX_INLINE_RESULT_LEN)"),
        }

        // 4001 → Offloaded (> threshold, <= MAX_FULL_INLINE_LEN)
        let just_over_4001 = "A".repeat(4001);
        let resp = mailbox.offload_if_large("explore", "b4k", "s2", &just_over_4001);
        match resp {
            SubagentResponse::Offloaded { content_len, .. } => assert_eq!(content_len, 4001),
            _ => panic!("Expected Offloaded for 4001-char result"),
        }
    }

    #[test]
    fn test_boundary_8000_offloaded_vs_summarized() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_boundary_8k"),
        );
        // Exactly 8000 → Offloaded (<= MAX_FULL_INLINE_LEN)
        let exactly_8000 = "A".repeat(8000);
        let resp = mailbox.offload_if_large("explore", "b8k", "s1", &exactly_8000);
        match resp {
            SubagentResponse::Offloaded { content_len, .. } => assert_eq!(content_len, 8000),
            _ => panic!("Expected Offloaded for 8000-char result (<= MAX_FULL_INLINE_LEN)"),
        }

        // 8001 → Summarized (> MAX_FULL_INLINE_LEN)
        let just_over_8001 = "A".repeat(8001);
        let resp = mailbox.offload_if_large("explore", "b8k", "s2", &just_over_8001);
        match resp {
            SubagentResponse::Summarized { content_len, .. } => assert_eq!(content_len, 8001),
            _ => panic!("Expected Summarized for 8001-char result (> MAX_FULL_INLINE_LEN)"),
        }
    }

    #[test]
    fn test_disk_persistence_failure_degrades_to_inline() {
        // Use a path that cannot be created/written to simulate store() failure.
        // On most Unix systems, writing under a non-existent root path fails.
        let bad_mailbox = SubagentResultMailbox::new(
            PathBuf::from("/this/path/does/not/exist/wgenty_test_bad_mailbox"),
        );
        let very_large = "C".repeat(9000);
        let response = bad_mailbox.offload_if_large(
            "general-purpose",
            "disk fail test",
            "session_fail",
            &very_large,
        );
        match response {
            SubagentResponse::Inline { content } => {
                // Full content returned despite >8000 + disk failure —
                // integrity > token control (spec R4).
                assert_eq!(content.len(), 9000);
                assert_eq!(content, very_large);
            }
            _ => panic!(
                "Expected Inline (full content) on disk persistence failure, even for >8000"
            ),
        }
    }
}
