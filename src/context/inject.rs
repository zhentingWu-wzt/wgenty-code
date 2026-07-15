//! Memory Context Injector — cross-session memory recall for daemon/headless paths.
//!
//! Extracts recall logic from TUI's `recall_memories()` into a reusable module.
//! Used by both TUI turn spawning and CLI `run_query` / `run_agent` paths.

use crate::api::ChatMessage;
use crate::context::{MemoryManager, MemoryType};

/// Stateless injector that searches cross-session memories and prepends
/// relevant context to a user message.
pub struct MemoryContextInjector;

impl MemoryContextInjector {
    /// Extract keywords from user input, search relevant memories via TF-IDF,
    /// and return a formatted `<memory-context>` block (or empty string if no
    /// relevant memories were found).
    pub async fn recall(
        user_input: &str,
        manager: &MemoryManager,
        top_n: usize,
        threshold: f64,
    ) -> String {
        let keywords = extract_keywords(user_input);

        // Don't trigger on very short / empty messages.
        if keywords.len() < 2 {
            return String::new();
        }

        let query = keywords.join(" ");
        let matched = manager.search_memories(&query).await;

        // Filter by importance >= threshold, sort descending, take top N.
        #[allow(clippy::cast_possible_truncation)]
        // threshold is a small integer; f32 precision is sufficient
        let threshold_f32 = threshold as f32;
        let mut sorted: Vec<_> = matched
            .into_iter()
            .filter(|m| m.importance >= threshold_f32)
            .collect();
        sorted.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top: Vec<_> = sorted.into_iter().take(top_n).collect();

        if top.is_empty() {
            return String::new();
        }

        tracing::info!(count = top.len(), "per-turn memory recall triggered");

        let mut block = String::from("<memory-context>\n");
        for m in &top {
            block.push_str(&format!(
                "- [{}] {} (importance: {:.1})\n",
                format_memory_type(&m.memory_type),
                m.content,
                m.importance
            ));
        }
        block.push_str("</memory-context>");

        block
    }

    /// Search for relevant memories and prepend a `<memory-context>` block
    /// to the first user message in `messages`. If no memories are found,
    /// `messages` is left unchanged.
    pub async fn inject(
        messages: &mut [ChatMessage],
        manager: &MemoryManager,
        user_input: &str,
        top_n: usize,
        threshold: f64,
    ) {
        let memory_block = Self::recall(user_input, manager, top_n, threshold).await;
        if memory_block.is_empty() {
            return;
        }

        // Find the first user message and prepend the memory context.
        for msg in messages.iter_mut() {
            if msg.role == "user" {
                let original = msg.content.take().unwrap_or_default();
                msg.content = Some(format!("{}\n\n{}", memory_block, original));
                return;
            }
        }
    }
}

// ── Private helpers ─────────────────────────────────────────────────────

/// Extract meaningful keywords from a user message for memory retrieval.
/// Filters stop words and short tokens, then sorts by token length descending
/// (longer = more specific).
fn extract_keywords(msg: &str) -> Vec<String> {
    use crate::context::ConsolidationEngine;
    let mut keywords: Vec<String> = msg
        .split_whitespace()
        .filter(|w| ConsolidationEngine::is_meaningful_token(w))
        .map(|w| w.to_lowercase())
        .collect();
    // Sort by length descending: longer words are more specific.
    keywords.sort_by_key(|b| std::cmp::Reverse(b.len()));
    keywords.dedup();
    // Keep top-N keywords to avoid query noise.
    const MAX_KEYWORDS: usize = 6;
    keywords.truncate(MAX_KEYWORDS);
    keywords
}

/// Format a MemoryType variant as a short human-readable string.
fn format_memory_type(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::Decision => "decision",
        MemoryType::Error => "error",
        MemoryType::Preference => "preference",
        MemoryType::Insight => "insight",
        MemoryType::Knowledge => "knowledge",
        MemoryType::Task => "task",
        MemoryType::Session => "session",
        MemoryType::Conversation => "conversation",
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ChatMessage;
    use crate::context::{
        ConsolidationEngine, HistoryManager, MemoryEntry, MemoryIndex, MemoryManager,
        MemorySessionManager, MemoryType, Storage,
    };
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_manager(temp_dir: &tempfile::TempDir) -> MemoryManager {
        let memory_dir = temp_dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        let storage = Arc::new(Storage::new(memory_dir));
        MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            storage,
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn setup_manager_with_memories() -> (MemoryManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        // Add memories with matching content for "rust programming"
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "Rust async programming patterns")
                .with_importance(0.9)
                .with_tags(vec!["rust".into(), "async".into()]),
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Decision, "Use tokio for async runtime")
                .with_importance(0.8)
                .with_tags(vec!["tokio".into()]),
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Insight, "Python is better for data science")
                .with_importance(0.3)
                .with_tags(vec!["python".into()]),
        )
        .await
        .unwrap();

        // Load the index so search_memories works
        mm.load().await.unwrap();

        (mm, tmp)
    }

    // ── recall() tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn recall_with_empty_input_returns_empty_string() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);
        mm.load().await.unwrap();

        let result = MemoryContextInjector::recall("", &mm, 5, 0.5).await;
        assert!(result.is_empty(), "empty input should produce empty recall");
    }

    #[tokio::test]
    async fn recall_with_no_matching_memories_returns_empty_string() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);
        mm.load().await.unwrap();

        let result = MemoryContextInjector::recall("completely unrelated query", &mm, 5, 0.5).await;
        assert!(
            result.is_empty(),
            "query with no matches should produce empty recall"
        );
    }

    #[tokio::test]
    async fn recall_finds_and_formats_matching_memories() {
        let (mm, _tmp) = setup_manager_with_memories().await;

        let result = MemoryContextInjector::recall("rust async programming", &mm, 5, 0.5).await;

        // Should find the two high-importance rust/tokio memories, not the low-importance python one
        assert!(
            !result.is_empty(),
            "should find matching memories for 'rust async programming'"
        );
        assert!(
            result.contains("<memory-context>"),
            "result should contain <memory-context> block: {}",
            result
        );
        assert!(
            result.contains("</memory-context>"),
            "result should close </memory-context> block"
        );
        // Should include the high-importance knowledge entry
        assert!(
            result.contains("Rust async programming patterns"),
            "should include the rust knowledge entry"
        );
        // Should include the decision entry
        assert!(
            result.contains("Use tokio for async runtime"),
            "should include the tokio decision entry"
        );
        // Should NOT include the low-importance python entry (importance 0.3 < threshold 0.5)
        assert!(
            !result.contains("Python is better"),
            "should NOT include low-importance (< threshold) entry"
        );
    }

    #[tokio::test]
    async fn recall_respects_top_n_limit() {
        let (mm, _tmp) = setup_manager_with_memories().await;

        // Add more memories so we have >2 matching
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "Rust ownership and borrowing")
                .with_importance(0.85),
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "Rust cargo build system")
                .with_importance(0.75),
        )
        .await
        .unwrap();
        mm.load().await.unwrap();

        let result = MemoryContextInjector::recall("rust programming language", &mm, 2, 0.5).await;

        assert!(!result.is_empty());
        // Count lines in the result (minus the <memory-context> wrapper lines)
        let body_lines: Vec<&str> = result
            .lines()
            .filter(|l| {
                !l.contains("<memory-context>")
                    && !l.contains("</memory-context>")
                    && !l.trim().is_empty()
            })
            .collect();
        assert!(
            body_lines.len() <= 2,
            "should respect top_n=2 limit, got {} lines: {:?}",
            body_lines.len(),
            body_lines
        );
    }

    #[tokio::test]
    async fn recall_uses_threshold_for_importance_filtering() {
        let (mm, _tmp) = setup_manager_with_memories().await;

        // With threshold 0.85, only the 0.9 importance entry should pass
        let result = MemoryContextInjector::recall("rust async programming", &mm, 5, 0.85).await;
        assert!(result.contains("Rust async programming patterns"));
        assert!(
            !result.contains("Use tokio"),
            "0.8 < 0.85 threshold, should be excluded"
        );
    }

    // ── inject() tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn inject_prepends_memory_context_to_first_user_message() {
        let (mm, _tmp) = setup_manager_with_memories().await;
        let mut messages = vec![
            ChatMessage::user("help me with rust async"),
            ChatMessage::assistant("Sure, here are some tips..."),
        ];

        MemoryContextInjector::inject(&mut messages, &mm, "help me with rust async", 5, 0.5).await;

        // First message should now contain <memory-context>
        let first_content = messages[0].content.as_ref().unwrap();
        assert!(
            first_content.contains("<memory-context>"),
            "first message should contain <memory-context>, got: {}",
            first_content
        );
        assert!(
            first_content.contains("help me with rust async"),
            "original user content should still be present"
        );
        // Memory context should appear before original content
        let ctx_pos = first_content.find("<memory-context>").unwrap();
        let orig_pos = first_content.find("help me with rust async").unwrap();
        assert!(
            ctx_pos < orig_pos,
            "memory context should come before original content"
        );
        // Second message should be untouched
        assert_eq!(
            messages[1].content.as_ref().unwrap(),
            "Sure, here are some tips..."
        );
    }

    #[tokio::test]
    async fn inject_with_no_memories_leaves_messages_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);
        mm.load().await.unwrap();

        let original_content = "this query will not match anything";
        let mut messages = vec![ChatMessage::user(original_content)];

        MemoryContextInjector::inject(&mut messages, &mm, original_content, 5, 0.5).await;

        // Content should be unchanged
        assert_eq!(messages[0].content.as_ref().unwrap(), original_content);
    }

    #[tokio::test]
    async fn inject_with_empty_messages_does_nothing() {
        let (mm, _tmp) = setup_manager_with_memories().await;
        let mut messages: Vec<ChatMessage> = vec![];

        // Should not panic
        MemoryContextInjector::inject(&mut messages, &mm, "rust async", 5, 0.5).await;

        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn inject_finds_first_user_message_when_system_messages_present() {
        let (mm, _tmp) = setup_manager_with_memories().await;
        let mut messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("help me with rust async"),
        ];

        MemoryContextInjector::inject(&mut messages, &mm, "help me with rust async", 5, 0.5).await;

        // System message should be untouched
        assert_eq!(
            messages[0].content.as_ref().unwrap(),
            "You are a helpful assistant."
        );
        // User message should have memory context
        assert!(messages[1]
            .content
            .as_ref()
            .unwrap()
            .contains("<memory-context>"));
    }
}
