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
    ///
    /// `explore_draw` is a test hook: `None` draws Bernoulli(`exploration_epsilon`);
    /// `Some(true/false)` forces the exploration branch without RNG.
    pub async fn recall(
        user_input: &str,
        manager: &MemoryManager,
        top_n: usize,
        threshold: f64,
        explore_draw: Option<bool>,
    ) -> String {
        let keywords = extract_keywords(user_input);

        // Don't trigger on very short / empty messages.
        if keywords.len() < 2 {
            return String::new();
        }

        let query = keywords.join(" ");
        let matched = manager.search_memories(&query).await;

        // Filter/sort by effective importance; drop superseded tombstones.
        #[allow(clippy::cast_possible_truncation)]
        // threshold is a small integer; f32 precision is sufficient
        let threshold_f32 = threshold as f32;
        let now = chrono::Utc::now();
        let cfg = manager.effective_importance_cfg();
        let mut scored: Vec<_> = matched
            .into_iter()
            .filter(|m| m.superseded_by.is_none())
            .map(|m| {
                let eff = m.effective_importance(now, &cfg);
                (m, eff)
            })
            .filter(|(_, eff)| *eff >= threshold_f32)
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut top: Vec<_> = scored.into_iter().take(top_n).collect();

        if top.is_empty() {
            return String::new();
        }

        // Optional ε-greedy exploration: replace lowest-ranked injected slot
        // with a cold project memory outside the current top.
        maybe_explore_replace(&mut top, manager, now, &cfg, explore_draw).await;

        // Persist injection frequency for hit-rate damping. Project-only:
        // globals are not part of this block.
        let injected_ids: Vec<&str> = top.iter().map(|(m, _)| m.id.as_str()).collect();
        if let Err(e) = manager.record_recall_injections(&injected_ids).await {
            tracing::warn!(error = %e, "failed to persist recall_count after injection");
        }

        tracing::info!(count = top.len(), "per-turn memory recall triggered");

        let mut block = String::from("<memory-context>\n");
        for (m, eff) in &top {
            block.push_str(&format!(
                "- [{}] {} (importance: {:.1})\n",
                format_memory_type(&m.memory_type),
                m.content,
                eff
            ));
        }
        block.push_str("</memory-context>");

        block
    }

    /// Format global memories into lines for the `<global-memory>` system
    /// prompt block. Returns at most 50 entries (soft cap), sorted by
    /// effective importance descending. Unlike `recall()`, this does NOT
    /// filter by relevance — all global memories are injected every turn.
    pub async fn format_global(manager: &MemoryManager) -> Vec<String> {
        let globals = manager.global_memories().await;
        let now = chrono::Utc::now();
        let cfg = manager.effective_importance_cfg();
        let mut scored: Vec<_> = globals
            .into_iter()
            .filter(|m| m.superseded_by.is_none())
            .map(|m| {
                let eff = m.effective_importance(now, &cfg);
                (m, eff)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        const SOFT_CAP: usize = 50;
        scored
            .into_iter()
            .take(SOFT_CAP)
            .map(|(m, _)| format!("- [{}] {}", format_memory_type(&m.memory_type), m.content))
            .collect()
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
        let memory_block = Self::recall(user_input, manager, top_n, threshold, None).await;
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

/// When exploration is enabled and the draw succeeds, replace the lowest-ranked
/// injected memory with a cold candidate (not superseded, not in top, not
/// recently explored). Prefers low effective importance, then low recall_count.
/// No candidate → leave `top` unchanged (never panics).
async fn maybe_explore_replace(
    top: &mut Vec<(crate::context::MemoryEntry, f32)>,
    manager: &MemoryManager,
    now: chrono::DateTime<chrono::Utc>,
    cfg: &crate::context::EffectiveImportanceCfg,
    explore_draw: Option<bool>,
) {
    if top.is_empty() {
        return;
    }

    let epsilon = manager.exploration_epsilon();
    if epsilon <= 0.0 {
        return;
    }

    let should_explore = match explore_draw {
        Some(forced) => forced,
        None => {
            use rand::Rng;
            rand::thread_rng().gen::<f32>() < epsilon
        }
    };
    if !should_explore {
        return;
    }

    let top_ids: std::collections::HashSet<&str> =
        top.iter().map(|(m, _)| m.id.as_str()).collect();

    let pool = manager.project_memories().await;
    let mut candidates: Vec<(crate::context::MemoryEntry, f32)> = Vec::new();
    for m in pool {
        if m.superseded_by.is_some() {
            continue;
        }
        if top_ids.contains(m.id.as_str()) {
            continue;
        }
        if manager.was_recently_explored(&m.id).await {
            continue;
        }
        let eff = m.effective_importance(now, cfg);
        candidates.push((m, eff));
    }

    if candidates.is_empty() {
        return;
    }

    // Prefer low effective, then low recall_count, then stable id order.
    candidates.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.recall_count.cmp(&b.0.recall_count))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    let (cold, cold_eff) = candidates.into_iter().next().expect("non-empty candidates");

    // Lowest-ranked injected slot is the last element (sorted desc).
    let replaced_id = cold.id.clone();
    if let Some(last) = top.last_mut() {
        *last = (cold, cold_eff);
    }
    manager.mark_recently_explored(&replaced_id).await;
}

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
    use crate::context::{MemoryEntry, MemoryManager, MemoryOrigin, MemoryType};
    use chrono::{Duration, Utc};

    fn make_manager(temp_dir: &tempfile::TempDir) -> MemoryManager {
        MemoryManager::new_for_test(
            temp_dir.path().to_path_buf(),
            temp_dir.path().join("global_memory"),
        )
    }

    async fn setup_manager_with_memories() -> (MemoryManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        // Add memories with matching content for "rust programming"
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "Rust async programming patterns")
                .with_importance(0.9)
                .with_tags(vec!["rust".into(), "async".into()]),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Decision, "Use tokio for async runtime")
                .with_importance(0.8)
                .with_tags(vec!["tokio".into()]),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Insight, "Python is better for data science")
                .with_importance(0.3)
                .with_tags(vec!["python".into()]),
            MemoryOrigin::Project,
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

        let result = MemoryContextInjector::recall("", &mm, 5, 0.5, None).await;
        assert!(result.is_empty(), "empty input should produce empty recall");
    }

    #[tokio::test]
    async fn recall_with_no_matching_memories_returns_empty_string() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);
        mm.load().await.unwrap();

        let result = MemoryContextInjector::recall("completely unrelated query", &mm, 5, 0.5, None).await;
        assert!(
            result.is_empty(),
            "query with no matches should produce empty recall"
        );
    }

    #[tokio::test]
    async fn recall_finds_and_formats_matching_memories() {
        let (mm, _tmp) = setup_manager_with_memories().await;

        let result = MemoryContextInjector::recall("rust async programming", &mm, 5, 0.5, None).await;

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
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "Rust cargo build system")
                .with_importance(0.75),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.load().await.unwrap();

        let result = MemoryContextInjector::recall("rust programming language", &mm, 2, 0.5, None).await;

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
        let result = MemoryContextInjector::recall("rust async programming", &mm, 5, 0.85, None).await;
        assert!(result.contains("Rust async programming patterns"));
        assert!(
            !result.contains("Use tokio"),
            "0.8 < 0.85 threshold, should be excluded"
        );
    }

    #[tokio::test]
    async fn recall_excludes_superseded_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        let mut live = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust async programming patterns live",
        )
        .with_importance(0.7);
        live.id = "live-rust".into();

        let mut old = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust async programming patterns old superseded",
        )
        .with_importance(0.99);
        old.id = "old-rust".into();
        old.superseded_by = Some("live-rust".into());

        mm.add_memory(live, MemoryOrigin::Project).await.unwrap();
        mm.add_memory(old, MemoryOrigin::Project).await.unwrap();
        mm.load().await.unwrap();

        let result =
            MemoryContextInjector::recall("rust async programming patterns", &mm, 5, 0.5, None).await;

        assert!(
            result.contains("patterns live"),
            "live memory should appear in recall: {result}"
        );
        assert!(
            !result.contains("old superseded"),
            "superseded memory must not appear in recall block: {result}"
        );
    }

    #[tokio::test]
    async fn recall_prints_effective_importance() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        // Old anchor → effective well below base importance.
        let mut entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust async programming decayed score",
        )
        .with_importance(0.9);
        entry.timestamp = Utc::now() - Duration::hours(192); // one Knowledge half-life
        entry.last_reinforced_at = Some(entry.timestamp);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();
        mm.load().await.unwrap();

        let result =
            MemoryContextInjector::recall("rust async programming decayed", &mm, 5, 0.3, None).await;
        // One Knowledge half-life → effective ≈ 0.9 * 0.5 = 0.45 → "{:.1}" = "0.5" or "0.4".
        assert!(
            result.contains("(importance: 0.4)") || result.contains("(importance: 0.5)"),
            "recall should print effective (~0.45), not raw 0.9: {result}"
        );
        assert!(
            !result.contains("(importance: 0.9)"),
            "must not print raw base importance when decayed: {result}"
        );
    }

    #[tokio::test]
    async fn recall_increments_and_persists_project_recall_count() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        let mut entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust async programming recall count probe",
        )
        .with_importance(0.9);
        entry.id = "recall-count-probe".into();
        entry.recall_count = 0;
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();
        mm.load().await.unwrap();

        let block = MemoryContextInjector::recall(
            "rust async programming recall count",
            &mm,
            5,
            0.5,
            None,
        )
        .await;
        assert!(
            block.contains("recall count probe"),
            "memory must be injected so recall_count can bump: {block}"
        );

        let in_memory = mm
            .get_memory("recall-count-probe")
            .await
            .expect("memory should still be loaded");
        assert_eq!(
            in_memory.recall_count, 1,
            "injected project memory should bump recall_count in memory"
        );

        // Fresh manager reload from the same project/global dirs proves disk write.
        let reloaded = MemoryManager::new_for_test(
            tmp.path().to_path_buf(),
            tmp.path().join("global_memory"),
        );
        reloaded.load().await.unwrap();
        let from_disk = reloaded
            .get_memory("recall-count-probe")
            .await
            .expect("persisted memory should reload from disk");
        assert_eq!(
            from_disk.recall_count, 1,
            "recall_count bump must be persisted to project storage"
        );
    }

    /// Shared fixture: three live project memories matching "rust programming".
    /// Top-2 by effective importance are high + mid; cold is outside the top.
    async fn setup_exploration_fixture() -> (MemoryManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        let mut high = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust programming high ranked alpha fact",
        )
        .with_importance(0.95);
        high.id = "explore-high".into();

        let mut mid = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust programming mid ranked beta fact",
        )
        .with_importance(0.80);
        mid.id = "explore-mid".into();

        let mut cold = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust programming cold low ranked gamma fact",
        )
        .with_importance(0.55);
        cold.id = "explore-cold".into();
        cold.recall_count = 0;

        mm.add_memory(high, MemoryOrigin::Project).await.unwrap();
        mm.add_memory(mid, MemoryOrigin::Project).await.unwrap();
        mm.add_memory(cold, MemoryOrigin::Project).await.unwrap();
        mm.load().await.unwrap();
        (mm, tmp)
    }

    #[tokio::test]
    async fn recall_exploration_epsilon_zero_never_replaces() {
        let (mm, _tmp) = setup_exploration_fixture().await;
        // Default epsilon is 0. Even a forced explore_draw must not replace.
        let block = MemoryContextInjector::recall(
            "rust programming ranked fact",
            &mm,
            2,
            0.5,
            Some(true),
        )
        .await;

        assert!(
            block.contains("high ranked alpha"),
            "top slot should remain: {block}"
        );
        assert!(
            block.contains("mid ranked beta"),
            "second slot should remain without exploration: {block}"
        );
        assert!(
            !block.contains("cold low ranked gamma"),
            "epsilon=0 must never inject the cold candidate: {block}"
        );
    }

    #[tokio::test]
    async fn recall_exploration_force_draw_replaces_lowest_with_cold() {
        let (mm, _tmp) = setup_exploration_fixture().await;
        let mm = mm.with_exploration_epsilon(1.0);

        let block = MemoryContextInjector::recall(
            "rust programming ranked fact",
            &mm,
            2,
            0.5,
            Some(true),
        )
        .await;

        assert!(
            block.contains("high ranked alpha"),
            "highest-ranked slot must be kept: {block}"
        );
        assert!(
            block.contains("cold low ranked gamma"),
            "forced explore should inject the cold candidate: {block}"
        );
        assert!(
            !block.contains("mid ranked beta"),
            "lowest-ranked top slot should be replaced: {block}"
        );
        assert!(
            mm.was_recently_explored("explore-cold").await,
            "explored cold id should enter the session-local recent set"
        );
    }

    #[tokio::test]
    async fn recall_exploration_no_candidate_keeps_original_top() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp).with_exploration_epsilon(1.0);

        // Exactly top_n live memories → no cold candidate outside the top.
        let mut a = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust programming only alpha slot",
        )
        .with_importance(0.9);
        a.id = "only-a".into();
        let mut b = MemoryEntry::new(
            MemoryType::Knowledge,
            "Rust programming only beta slot",
        )
        .with_importance(0.8);
        b.id = "only-b".into();
        mm.add_memory(a, MemoryOrigin::Project).await.unwrap();
        mm.add_memory(b, MemoryOrigin::Project).await.unwrap();
        mm.load().await.unwrap();

        let block = MemoryContextInjector::recall(
            "rust programming only slot",
            &mm,
            2,
            0.5,
            Some(true),
        )
        .await;

        assert!(
            block.contains("only alpha slot") && block.contains("only beta slot"),
            "with no cold candidate, original top must be kept (no panic): {block}"
        );
    }

    #[tokio::test]
    async fn format_global_orders_by_effective_not_raw_importance() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        let mut high_raw_old = MemoryEntry::new(
            MemoryType::Preference,
            "Always reply in Chinese high-raw-old",
        )
        .with_importance(0.95);
        high_raw_old.timestamp = Utc::now() - Duration::hours(800);
        high_raw_old.last_reinforced_at = Some(high_raw_old.timestamp);

        let lower_raw_fresh = MemoryEntry::new(
            MemoryType::Knowledge,
            "User works on Rust projects lower-raw-fresh",
        )
        .with_importance(0.6);
        // fresh → higher effective than heavily decayed 0.95

        mm.add_memory(high_raw_old, MemoryOrigin::Global)
            .await
            .unwrap();
        mm.add_memory(lower_raw_fresh, MemoryOrigin::Global)
            .await
            .unwrap();
        mm.load().await.unwrap();

        let result = MemoryContextInjector::format_global(&mm).await;
        assert_eq!(result.len(), 2);
        assert!(
            result[0].contains("lower-raw-fresh"),
            "fresh lower raw should rank above decayed high raw: {result:?}"
        );
        assert!(
            result[1].contains("high-raw-old"),
            "decayed high raw should rank second: {result:?}"
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

    // ── format_global() tests ──────────────────────────────────────────

    #[tokio::test]
    async fn format_global_returns_empty_when_no_global_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);
        mm.load().await.unwrap();

        let result = MemoryContextInjector::format_global(&mm).await;
        assert!(result.is_empty(), "no global memories should yield empty");
    }

    #[tokio::test]
    async fn format_global_returns_all_global_memories_sorted_by_importance() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        // Add global memories with varying importance.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Preference, "Always reply in Chinese")
                .with_importance(0.9),
            MemoryOrigin::Global,
        )
        .await
        .unwrap();
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "User works on Rust projects")
                .with_importance(0.5),
            MemoryOrigin::Global,
        )
        .await
        .unwrap();
        // Add a project memory that should NOT appear in global output.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Decision, "Use tokio runtime").with_importance(0.95),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();

        mm.load().await.unwrap();

        let result = MemoryContextInjector::format_global(&mm).await;

        // Should have exactly 2 global memories (not the project one).
        assert_eq!(result.len(), 2, "should return only global memories");

        // Higher importance should come first.
        assert!(
            result[0].contains("Always reply in Chinese"),
            "higher importance global memory should be first: {:?}",
            result
        );
        assert!(
            result[1].contains("User works on Rust projects"),
            "lower importance global memory should be second: {:?}",
            result
        );

        // Each line should be formatted with the memory type prefix.
        assert!(
            result[0].starts_with("- [preference]"),
            "first line should have preference type prefix: {}",
            result[0]
        );

        // Project memory should NOT appear.
        assert!(
            !result.iter().any(|l| l.contains("tokio")),
            "project memories should not appear in global output"
        );
    }

    #[tokio::test]
    async fn format_global_excludes_superseded_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_manager(&tmp);

        let mut live = MemoryEntry::new(
            MemoryType::Preference,
            "Always reply in Chinese live-global",
        )
        .with_importance(0.7);
        live.id = "live-global".into();

        let mut old = MemoryEntry::new(
            MemoryType::Preference,
            "Always reply in Chinese old-superseded-global",
        )
        .with_importance(0.99);
        old.id = "old-global".into();
        old.superseded_by = Some("live-global".into());

        mm.add_memory(live, MemoryOrigin::Global).await.unwrap();
        mm.add_memory(old, MemoryOrigin::Global).await.unwrap();
        mm.load().await.unwrap();

        let result = MemoryContextInjector::format_global(&mm).await;
        assert!(
            result.iter().any(|l| l.contains("live-global")),
            "live global should appear: {result:?}"
        );
        assert!(
            !result.iter().any(|l| l.contains("old-superseded-global")),
            "superseded global must be excluded: {result:?}"
        );
    }
}
