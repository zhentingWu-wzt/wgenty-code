//! Consolidation - Memory consolidation engine

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{MemoryEntry, MemoryType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    pub max_memories: usize,
    pub importance_threshold: f32,
    pub age_threshold_hours: u64,
    pub consolidation_interval_hours: u64,
    pub enable_auto_consolidation: bool,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            max_memories: 200,
            importance_threshold: 0.6,
            age_threshold_hours: 48,
            consolidation_interval_hours: 6,
            enable_auto_consolidation: true,
        }
    }
}

impl ConsolidationConfig {
    /// Build a `ConsolidationConfig` from user-facing `MemorySettings`.
    ///
    /// This wires the consolidation engine to the `storage.memory` section
    /// of `settings.json` so users can tune consolidation thresholds
    /// without code changes.
    pub fn from_memory_settings(settings: &crate::config::MemorySettings) -> Self {
        Self {
            max_memories: settings.max_memories,
            importance_threshold: settings.importance_threshold,
            age_threshold_hours: settings.age_threshold_hours,
            consolidation_interval_hours: settings.consolidation_interval,
            enable_auto_consolidation: settings.enable_auto_consolidation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    pub memories_before: usize,
    pub memories_after: usize,
    pub memories_consolidated: usize,
    pub memories_removed: usize,
    pub insights_generated: usize,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
}

pub struct ConsolidationEngine {
    config: ConsolidationConfig,
    last_consolidation: Arc<RwLock<Option<DateTime<Utc>>>>,
}

impl ConsolidationEngine {
    pub fn new(config: ConsolidationConfig) -> Self {
        Self {
            config,
            last_consolidation: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn consolidate(&self, memories: &[MemoryEntry]) -> anyhow::Result<Vec<MemoryEntry>> {
        let start = std::time::Instant::now();
        let memories_before = memories.len();

        let mut consolidated = Vec::new();
        let mut to_merge: Vec<&MemoryEntry> = Vec::new();
        let _insights: Vec<String> = Vec::new();

        let mut sorted_memories: Vec<_> = memories.iter().collect();
        sorted_memories.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for memory in sorted_memories {
            if consolidated.len() >= self.config.max_memories {
                break;
            }

            if self.should_keep(memory) {
                if self.is_similar_to_any(memory, &consolidated) {
                    to_merge.push(memory);
                } else {
                    consolidated.push(memory.clone());
                }
            }
        }

        if !to_merge.is_empty() {
            let merged = self.merge_memories(&to_merge);
            consolidated.push(merged);
        }

        let insights_generated = self.extract_insights(memories, &mut consolidated);

        let result = ConsolidationResult {
            memories_before,
            memories_after: consolidated.len(),
            memories_consolidated: to_merge.len(),
            memories_removed: memories_before.saturating_sub(consolidated.len()),
            insights_generated,
            duration_ms: start.elapsed().as_millis() as u64,
            timestamp: Utc::now(),
        };

        tracing::info!(
            memories_before = result.memories_before,
            memories_after = result.memories_after,
            insights = result.insights_generated,
            "consolidation complete"
        );

        // Record the timestamp of this consolidation so that `status()` and
        // `last_consolidation()` can report it. Previously this field was
        // never updated, so `MemoryStatus.last_consolidation` was always None.
        *self.last_consolidation.write().await = Some(result.timestamp);

        Ok(consolidated)
    }

    fn should_keep(&self, memory: &MemoryEntry) -> bool {
        // High-importance memories are always kept regardless of type or age.
        if memory.importance >= self.config.importance_threshold {
            return true;
        }

        // Type-specific retention for low-importance memories.
        // Knowledge/Preference used to be immortal, which let low-value
        // "facts" accumulate forever. They now get a longer TTL (4× base)
        // instead of permanent retention. Ephemeral types expire faster.
        let age_hours = (Utc::now() - memory.timestamp).num_hours();
        let age = age_hours.max(0) as u64;
        let base = self.config.age_threshold_hours.max(1);

        let ttl = match memory.memory_type {
            // Durable but not immortal: low-value knowledge eventually decays.
            MemoryType::Knowledge | MemoryType::Preference => base.saturating_mul(4),
            // Stable decisions / insights last longer than session noise.
            MemoryType::Decision | MemoryType::Insight => base.saturating_mul(2),
            // Errors go stale quickly.
            MemoryType::Error => (base / 2).max(1),
            // Session/task/conversation noise expires at the base TTL.
            MemoryType::Session | MemoryType::Conversation | MemoryType::Task => base,
        };

        age < ttl
    }

    fn is_similar_to_any(&self, memory: &MemoryEntry, others: &[MemoryEntry]) -> bool {
        others
            .iter()
            .any(|other| self.calculate_similarity(memory, other) > 0.8)
    }

    fn calculate_similarity(&self, a: &MemoryEntry, b: &MemoryEntry) -> f32 {
        if a.memory_type != b.memory_type {
            return 0.0;
        }
        Self::content_similarity(a, b)
    }

    /// Type-agnostic Jaccard similarity over meaningful content tokens.
    ///
    /// Unlike `calculate_similarity` (consolidation-time, which requires the
    /// two memories to share a `MemoryType`), this compares pure text overlap.
    /// It is used by `MemoryManager::add_memory` to catch the same fact being
    /// re-extracted across separate compaction rounds even when the model tags
    /// it with a different type (e.g. `Decision` once, `Knowledge` the next).
    ///
    /// A subset relation (one token set entirely contained in the other, with
    /// at least two tokens on the smaller side) is treated as a full match so
    /// that a terse memory such as "use jwt" merges into a richer one such as
    /// "use jwt authentication". The `min_len` guard keeps single-token
    /// memories from over-merging into anything that happens to mention them.
    pub(crate) fn content_similarity(a: &MemoryEntry, b: &MemoryEntry) -> f32 {
        // Tokens are lowercased so similarity is case-insensitive, matching the
        // TF-IDF index (which also lowercases). Previously "Use JWT" and
        // "use jwt" were disjoint token sets and never matched, so the same
        // fact re-extracted with different capitalization would not dedup.
        let a_words: std::collections::HashSet<String> = a
            .content
            .split_whitespace()
            .filter(|w| Self::is_meaningful_token(w))
            .map(|w| w.to_lowercase())
            .collect();
        let b_words: std::collections::HashSet<String> = b
            .content
            .split_whitespace()
            .filter(|w| Self::is_meaningful_token(w))
            .map(|w| w.to_lowercase())
            .collect();

        if a_words.is_empty() || b_words.is_empty() {
            return 0.0;
        }

        let min_len = a_words.len().min(b_words.len());
        if min_len >= 2 && (a_words.is_subset(&b_words) || b_words.is_subset(&a_words)) {
            return 1.0;
        }

        let intersection = a_words.intersection(&b_words).count();
        let union = a_words.union(&b_words).count();
        intersection as f32 / union as f32
    }

    /// Return the index of the first memory in `others` whose similarity to
    /// `entry` exceeds `threshold`.
    ///
    /// When `require_same_type` is `true` only same-type memories are
    /// considered (consolidation-time semantics via `calculate_similarity`);
    /// when `false` the type is ignored (`content_similarity`), which is what
    /// `add_memory` needs to fold cross-type duplicates.
    pub fn find_similar(
        &self,
        entry: &MemoryEntry,
        others: &[MemoryEntry],
        threshold: f32,
        require_same_type: bool,
    ) -> Option<usize> {
        others.iter().position(|other| {
            let sim = if require_same_type {
                self.calculate_similarity(entry, other)
            } else {
                Self::content_similarity(entry, other)
            };
            sim > threshold
        })
    }

    /// Determine whether a whitespace token is meaningful for similarity
    /// comparison.
    ///
    /// Filters out common English stop words and tokens shorter than 3
    /// characters. Previously every token (including "the", "a", "is")
    /// contributed equally to the Jaccard index, inflating similarity
    /// between unrelated memories that happen to share high-frequency words.
    pub(crate) fn is_meaningful_token(token: &str) -> bool {
        const STOP_WORDS: &[&str] = &[
            "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been",
            "being", "to", "of", "in", "on", "at", "by", "for", "with", "from", "as", "into",
            "than", "then", "this", "that", "these", "those", "it", "its", "i", "you", "he", "she",
            "we", "they", "not", "no", "do", "does", "did", "has", "have", "had", "will", "would",
            "can", "could", "should", "may", "might", "must", "if", "so", "up", "out", "about",
        ];

        let lower = token.to_lowercase();
        if lower.len() < 3 {
            return false;
        }
        !STOP_WORDS.contains(&lower.as_str())
    }

    fn merge_memories(&self, memories: &[&MemoryEntry]) -> MemoryEntry {
        let mut combined_content = String::new();
        let mut max_importance: f32 = 0.0;
        let mut all_tags: Vec<String> = Vec::new();
        let mut source_ids: Vec<String> = Vec::new();
        let mut earliest_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut latest_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;

        for memory in memories {
            if !combined_content.is_empty() {
                combined_content.push('\n');
            }
            combined_content.push_str(&memory.content);
            max_importance = max_importance.max(memory.importance);
            all_tags.extend(memory.tags.clone());
            source_ids.push(memory.id.clone());

            earliest_timestamp = Some(
                earliest_timestamp
                    .map_or(memory.timestamp, |earliest| earliest.min(memory.timestamp)),
            );
            latest_timestamp = Some(
                latest_timestamp.map_or(memory.timestamp, |latest| latest.max(memory.timestamp)),
            );
        }

        all_tags.sort();
        all_tags.dedup();

        let merged = MemoryEntry::new(memories[0].memory_type.clone(), &combined_content)
            .with_importance(max_importance + 0.1)
            .with_tags(all_tags);

        // Preserve provenance: record the source memory IDs and the
        // earliest/latest timestamps of the constituent memories so the
        // merged entry remains traceable. Previously all original metadata
        // (IDs, timestamps) was discarded.
        merged
            .with_metadata(
                "merged_from",
                serde_json::Value::Array(
                    source_ids
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            )
            .with_metadata(
                "merged_earliest",
                earliest_timestamp.map_or(serde_json::Value::Null, |t| t.to_rfc3339().into()),
            )
            .with_metadata(
                "merged_latest",
                latest_timestamp.map_or(serde_json::Value::Null, |t| t.to_rfc3339().into()),
            )
    }

    /// Merge `incoming` into `existing`, preserving `existing`'s id and type.
    ///
    /// Used by `MemoryManager::add_memory` to fold a near-duplicate into an
    /// already-stored entry instead of writing a new file. Content is kept as
    /// the richer of the two when one is a substring of the other (so "use
    /// jwt" folds into "use jwt authentication" without duplication),
    /// otherwise the texts are concatenated. Importance takes the max and tags
    /// are unioned. Unlike `merge_memories` (consolidation-time, which mints a
    /// fresh id), this keeps the existing id so `save_memory` overwrites the
    /// original file and no orphaned duplicate is left behind.
    pub fn merge_into(existing: &MemoryEntry, incoming: &MemoryEntry) -> MemoryEntry {
        let combined = if existing.content.contains(incoming.content.as_str()) {
            existing.content.clone()
        } else if incoming.content.contains(existing.content.as_str()) {
            incoming.content.clone()
        } else {
            format!("{}\n{}", existing.content, incoming.content)
        };

        let mut tags = existing.tags.clone();
        tags.extend(incoming.tags.iter().cloned());
        tags.sort();
        tags.dedup();

        let importance = existing.importance.max(incoming.importance).min(1.0);

        MemoryEntry {
            id: existing.id.clone(),
            memory_type: existing.memory_type.clone(),
            content: combined,
            timestamp: existing.timestamp.min(incoming.timestamp),
            importance,
            tags,
            metadata: existing.metadata.clone(),
        }
    }

    fn extract_insights(
        &self,
        memories: &[MemoryEntry],
        _consolidated: &mut Vec<MemoryEntry>,
    ) -> usize {
        // Previously this method generated generic template insights like
        // "Pattern detected: 10 session memories recorded" and persisted
        // them as MemoryEntry(Insight). These boilerplate strings did not
        // encode actual knowledge yet polluted future recall. Now we only
        // log the observations and return the count, without polluting the
        // consolidated memory set.
        let mut insights = 0;

        let mut type_counts: std::collections::HashMap<MemoryType, usize> =
            std::collections::HashMap::new();
        for memory in memories {
            *type_counts.entry(memory.memory_type.clone()).or_insert(0) += 1;
        }

        for (memory_type, count) in type_counts {
            if count >= 10 {
                tracing::info!(
                    type = ?memory_type,
                    count,
                    "consolidation insight: many memories of this type accumulated"
                );
                insights += 1;
            }
        }

        let error_count = memories
            .iter()
            .filter(|m| m.memory_type == MemoryType::Error)
            .count();

        if error_count >= 3 {
            let error_patterns: Vec<String> = memories
                .iter()
                .filter(|m| m.memory_type == MemoryType::Error)
                .take(5)
                .map(|m| m.content.chars().take(100).collect::<String>())
                .collect();

            tracing::warn!(
                count = error_count,
                recent = ?error_patterns,
                "consolidation insight: recurring errors detected"
            );
            insights += 1;
        }

        insights
    }

    pub fn should_consolidate(&self, memory_count: usize) -> bool {
        memory_count >= self.config.max_memories
    }

    pub async fn last_consolidation(&self) -> Option<DateTime<Utc>> {
        *self.last_consolidation.read().await
    }

    pub fn config(&self) -> &ConsolidationConfig {
        &self.config
    }
}

impl Default for ConsolidationEngine {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MemoryEntry;

    #[test]
    fn merge_memories_preserves_provenance() {
        let engine = ConsolidationEngine::default();

        let mut m1 = MemoryEntry::new(MemoryType::Knowledge, "content one");
        m1.id = "src-1".to_string();
        m1.timestamp = chrono::Utc::now() - chrono::Duration::hours(10);

        let mut m2 = MemoryEntry::new(MemoryType::Knowledge, "content two");
        m2.id = "src-2".to_string();
        m2.timestamp = chrono::Utc::now() - chrono::Duration::hours(2);

        let merged = engine.merge_memories(&[&m1, &m2]);

        // merged_from should list both source IDs.
        let merged_from = merged.metadata.get("merged_from").unwrap();
        let ids: Vec<String> = merged_from
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(ids.contains(&"src-1".to_string()));
        assert!(ids.contains(&"src-2".to_string()));

        // merged_earliest should match the older timestamp (m1).
        let earliest = merged
            .metadata
            .get("merged_earliest")
            .unwrap()
            .as_str()
            .unwrap();
        let earliest_ts = chrono::DateTime::parse_from_rfc3339(earliest).unwrap();
        assert_eq!(earliest_ts.with_timezone(&chrono::Utc), m1.timestamp);

        // merged_latest should match the newer timestamp (m2).
        let latest = merged
            .metadata
            .get("merged_latest")
            .unwrap()
            .as_str()
            .unwrap();
        let latest_ts = chrono::DateTime::parse_from_rfc3339(latest).unwrap();
        assert_eq!(latest_ts.with_timezone(&chrono::Utc), m2.timestamp);

        // Content should contain both originals.
        assert!(merged.content.contains("content one"));
        assert!(merged.content.contains("content two"));
    }

    #[test]
    fn similarity_ignores_stop_words_and_short_tokens() {
        let engine = ConsolidationEngine::default();
        // These two entries share only stop words ("the") and a short token
        // ("is") — with stop-word filtering the similarity should be ~0.
        let a = MemoryEntry::new(MemoryType::Knowledge, "the quick brown fox is jumping");
        let b = MemoryEntry::new(MemoryType::Knowledge, "the lazy dog is sleeping");
        let sim = engine.calculate_similarity(&a, &b);
        // "quick", "brown", "fox", "jumping" vs "lazy", "dog", "sleeping"
        // → 0 meaningful token overlap → similarity should be 0.
        assert!(
            sim < 0.1,
            "similarity should be near zero for entries sharing only stop words"
        );
    }

    #[test]
    fn similarity_detects_real_overlap() {
        let engine = ConsolidationEngine::default();
        let a = MemoryEntry::new(MemoryType::Knowledge, "the function returns a value");
        let b = MemoryEntry::new(MemoryType::Knowledge, "the function takes a value");
        let sim = engine.calculate_similarity(&a, &b);
        // Meaningful tokens: {function, returns, value} vs {function, takes, value}
        // intersection=2, union=4 → 0.5
        assert!((sim - 0.5).abs() < 0.01, "expected ~0.5, got {}", sim);
    }

    #[test]
    fn should_keep_recent_low_importance_knowledge() {
        let engine = ConsolidationEngine::default();
        // Knowledge TTL is 4 * age_threshold_hours (default 48h * 4 = 192h).
        let mut entry = MemoryEntry::new(MemoryType::Knowledge, "recent fact").with_importance(0.1);
        entry.timestamp = chrono::Utc::now() - chrono::Duration::hours(24);
        assert!(
            engine.should_keep(&entry),
            "recent low-importance Knowledge should be kept within durable TTL"
        );
    }

    #[test]
    fn should_drop_stale_low_importance_knowledge() {
        let engine = ConsolidationEngine::default();
        // Knowledge TTL is 4 * 48h = 192h. Age well past that.
        let mut entry = MemoryEntry::new(MemoryType::Knowledge, "stale fact").with_importance(0.1);
        entry.timestamp = chrono::Utc::now() - chrono::Duration::hours(300);
        assert!(
            !engine.should_keep(&entry),
            "stale low-importance Knowledge should eventually expire"
        );
    }

    #[test]
    fn should_keep_high_importance_regardless_of_age() {
        let engine = ConsolidationEngine::default();
        let mut entry =
            MemoryEntry::new(MemoryType::Knowledge, "important fact").with_importance(0.9);
        entry.timestamp = chrono::Utc::now() - chrono::Duration::hours(10_000);
        assert!(
            engine.should_keep(&entry),
            "high-importance memories must never expire by age alone"
        );
    }

    #[test]
    fn should_drop_old_low_importance_error() {
        let engine = ConsolidationEngine::default();
        let mut err = MemoryEntry::new(MemoryType::Error, "stale error").with_importance(0.1);
        // Error TTL is age_threshold/2 = 24h with default 48h base. Set age to 30h.
        err.timestamp = chrono::Utc::now() - chrono::Duration::hours(30);
        assert!(
            !engine.should_keep(&err),
            "stale low-importance error should be dropped"
        );
    }

    #[test]
    fn extract_insights_does_not_pollute_consolidated() {
        let engine = ConsolidationEngine::default();
        // Build 10+ Error memories to trigger the insight path.
        let memories: Vec<_> = (0..12)
            .map(|i| {
                MemoryEntry::new(MemoryType::Error, &format!("error {}", i)).with_importance(0.5)
            })
            .collect();

        let mut consolidated = vec![];
        let count = engine.extract_insights(&memories, &mut consolidated);
        assert!(count >= 1, "should detect insight from 12 Error memories");
        assert!(
            consolidated.is_empty(),
            "extract_insights must not push MemoryEntry into consolidated"
        );
    }

    #[test]
    fn content_similarity_subset_treats_as_full_match() {
        // "use jwt" tokens are a subset of "use jwt authentication".
        let a = MemoryEntry::new(MemoryType::Knowledge, "use jwt");
        let b = MemoryEntry::new(MemoryType::Decision, "use jwt authentication");
        // Type-agnostic and subset (min_len >= 2) -> full match.
        assert!(ConsolidationEngine::content_similarity(&a, &b) >= 1.0);
    }

    #[test]
    fn content_similarity_ignores_type() {
        let a = MemoryEntry::new(MemoryType::Decision, "the auth uses jwt tokens");
        let b = MemoryEntry::new(MemoryType::Knowledge, "the auth uses jwt tokens");
        // calculate_similarity would return 0 (different types), but
        // content_similarity is type-agnostic and sees full overlap.
        assert!(
            ConsolidationEngine::content_similarity(&a, &b) >= 0.99,
            "type-agnostic similarity should be ~1.0 for identical content"
        );
    }

    #[test]
    fn content_similarity_is_case_insensitive() {
        // "Use JWT" and "use jwt" must match despite different casing, so the
        // same fact re-extracted with different capitalization still dedups.
        let a = MemoryEntry::new(MemoryType::Knowledge, "Use JWT");
        let b = MemoryEntry::new(MemoryType::Decision, "use jwt");
        assert!(
            ConsolidationEngine::content_similarity(&a, &b) >= 1.0,
            "similarity should be case-insensitive"
        );
    }

    #[test]
    fn calculate_similarity_is_case_insensitive() {
        // Consolidation-time similarity (same type) is also case-insensitive
        // now that it delegates to content_similarity.
        let engine = ConsolidationEngine::default();
        let a = MemoryEntry::new(MemoryType::Knowledge, "Use JWT tokens");
        let b = MemoryEntry::new(MemoryType::Knowledge, "use jwt tokens");
        let sim = engine.calculate_similarity(&a, &b);
        assert!(
            sim >= 0.99,
            "consolidation similarity should be case-insensitive, got {}",
            sim
        );
    }

    #[test]
    fn find_similar_type_agnostic_when_requested() {
        let engine = ConsolidationEngine::default();
        let existing = vec![MemoryEntry::new(MemoryType::Decision, "use jwt auth")];
        let incoming = MemoryEntry::new(MemoryType::Knowledge, "use jwt auth");
        // require_same_type=false -> matches across types.
        assert_eq!(
            engine.find_similar(&incoming, &existing, 0.6, false),
            Some(0)
        );
        // require_same_type=true -> no match (different types).
        assert_eq!(engine.find_similar(&incoming, &existing, 0.6, true), None);
    }

    #[test]
    fn merge_into_preserves_id_and_keeps_richer_content() {
        let mut existing = MemoryEntry::new(MemoryType::Decision, "use jwt auth");
        existing.id = "keep-me".to_string();
        existing.importance = 0.4;
        let incoming =
            MemoryEntry::new(MemoryType::Knowledge, "use jwt auth tokens").with_importance(0.9);

        let merged = ConsolidationEngine::merge_into(&existing, &incoming);

        assert_eq!(merged.id, "keep-me", "existing id must be preserved");
        assert_eq!(
            merged.memory_type,
            MemoryType::Decision,
            "existing type must be preserved"
        );
        // incoming text is richer (superset) -> preferred over concatenation.
        assert_eq!(merged.content, "use jwt auth tokens");
        // importance takes the max.
        assert!((merged.importance - 0.9).abs() < 1e-6);
    }
}
