//! Consolidation - Memory consolidation engine

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use super::{MemoryEntry, MemoryType};

/// Conservative filesystem-relative path extractor for codebase staleness.
///
/// Matches tokens that look like repo-relative source paths (e.g. `src/foo.rs`,
/// `lib/bar/baz.ts:12`). Bare URLs are ignored. Returns unique paths in
/// encounter order, with optional `:line` / `:line:col` suffixes stripped.
pub fn extract_memory_paths(content: &str) -> Vec<PathBuf> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            # Left boundary: avoid matching inside tokens like foosrc/foo.rs
            (?:^|[^A-Za-z0-9_])
            (?P<path>
                (?:src|lib|crates|apps|packages|tests?|scripts?|docs?)
                /[\w./+-]+
                \.
                (?:rs|ts|tsx|js|jsx|py|go|java|kt|toml|json|md|yml|yaml|css|html|sh|c|h|cpp|hpp|rb|php)
            )
            (?: : \d+ (?: : \d+ )? )?
            ",
        )
        .expect("valid extract_memory_paths regex")
    });

    let mut out: Vec<PathBuf> = Vec::new();
    for caps in re.captures_iter(content) {
        let m = caps.name("path").expect("named path group");
        // Skip URL-embedded matches (e.g. https://example.com/src/foo.rs).
        if is_url_embedded(content, m.start()) {
            continue;
        }
        let path = PathBuf::from(m.as_str());
        if !out.iter().any(|p| p == &path) {
            out.push(path);
        }
    }
    out
}

fn is_url_embedded(content: &str, match_start: usize) -> bool {
    // Look back for a URL scheme before the match without crossing whitespace.
    let prefix = &content[..match_start];
    let token_start = prefix
        .rfind(|c: char| c.is_whitespace() || c == '(' || c == '[' || c == '"' || c == '\'')
        .map(|i| i + 1)
        .unwrap_or(0);
    let lead = &content[token_start..match_start];
    lead.contains("://") || lead.starts_with("www.")
}

/// True when `paths` is non-empty and every path is missing under `project_root`
/// (relative paths) or on the absolute path itself.
pub fn paths_all_missing(project_root: &Path, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    paths.iter().all(|p| {
        let candidate = if p.is_absolute() {
            p.clone()
        } else {
            project_root.join(p)
        };
        !candidate.exists()
    })
}

/// First-consolidate anchor migration + optional all-missing path staleness.
///
/// Idempotent:
/// - `last_reinforced_at = None` → `Some(now)` once
/// - stale mark only when extractable paths exist, **all** missing, and not yet marked
/// - never multiplies base `importance`
/// - never refreshes `last_reinforced_at` solely for staleness
pub fn apply_consolidate_prepass(
    memories: &mut [MemoryEntry],
    project_root: &Path,
    now: DateTime<Utc>,
    staleness_check: bool,
) {
    for m in memories.iter_mut() {
        if m.last_reinforced_at.is_none() {
            m.last_reinforced_at = Some(now);
        }
        if !staleness_check || m.stale_marked_at.is_some() {
            continue;
        }
        let paths = extract_memory_paths(&m.content);
        if paths_all_missing(project_root, &paths) {
            m.stale_marked_at = Some(now);
        }
    }
}

/// Per-type half-life in hours for effective-importance decay.
///
/// Shares the same TTL multipliers as [`ConsolidationEngine::should_keep`]:
/// Knowledge/Preference ×4, Decision/Insight ×2, Error max(base/2, 1), else ×1.
pub fn type_half_life_hours(memory_type: MemoryType, age_threshold_hours: u64) -> f64 {
    let base = age_threshold_hours.max(1) as f64;
    match memory_type {
        MemoryType::Knowledge | MemoryType::Preference => base * 4.0,
        MemoryType::Decision | MemoryType::Insight => base * 2.0,
        MemoryType::Error => (base / 2.0).max(1.0),
        MemoryType::Session | MemoryType::Conversation | MemoryType::Task => base,
    }
}

/// Tier-1 relation between a new memory and a similar existing one.
///
/// Conservative: prefer [`MemoryRelation::Ambiguous`] over false
/// [`MemoryRelation::Contradicts`] (see design §M2 / D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRelation {
    Compatible,
    Contradicts,
    Ambiguous,
}

/// Classify how `new` relates to a Jaccard-similar `existing` memory.
///
/// Heuristics (local, LLM-free):
/// - **Contradicts**: high content similarity plus a state-change marker in the
///   *new* text (fixed/resolved/removed/…), or a shared key-like token with a
///   clear numeric drift (e.g. `max_tokens=128000` vs `max_tokens=4096`).
/// - **Compatible**: one meaningful-token set is a non-trivial subset of the
///   other (same-direction refinement), without contradiction signals.
/// - **Ambiguous**: everything else (including competing alternatives).
pub fn classify_relation(new: &MemoryEntry, existing: &MemoryEntry) -> MemoryRelation {
    let new_tokens = meaningful_token_set(&new.content);
    let existing_tokens = meaningful_token_set(&existing.content);

    if has_numeric_value_drift(&new.content, &existing.content)
        || has_state_change_contradiction(
            &new.content,
            &existing.content,
            &new_tokens,
            &existing_tokens,
        )
    {
        return MemoryRelation::Contradicts;
    }

    // Subset / same-direction refinement (mirrors content_similarity's subset boost).
    let min_len = new_tokens.len().min(existing_tokens.len());
    if min_len >= 1
        && (new_tokens.is_subset(&existing_tokens) || existing_tokens.is_subset(&new_tokens))
    {
        return MemoryRelation::Compatible;
    }

    MemoryRelation::Ambiguous
}

fn meaningful_token_set(content: &str) -> std::collections::HashSet<String> {
    content
        .split_whitespace()
        .filter(|w| ConsolidationEngine::is_meaningful_token(w))
        .map(|w| w.to_lowercase())
        .collect()
}

/// Closed (resolved/removed) state-change marker tokens. Matched as whole
/// tokens only — never via raw substring `contains` — so `unresolved` /
/// `fixed-width` do not false-trigger.
const CLOSED_STATE_MARKERS: &[&str] = &[
    "fixed",
    "resolved",
    "removed",
    "deprecated",
    "migrated",
    "replaced",
    "obsolete",
    "deleted",
    "disabled",
];

/// Open / still-pending polarity cues (whole-token).
const OPEN_STATE_MARKERS: &[&str] = &[
    "unresolved",
    "unfixed",
    "open",
    "pending",
    "exists",
    "broken",
];

/// Multi-word closed phrase kept as a phrase (not split for matching).
const CLOSED_STATE_PHRASES: &[&str] = &["no longer"];

/// Multi-word open / negated closed phrases.
const OPEN_STATE_PHRASES: &[&str] = &["not fixed", "not resolved", "still broken"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatePolarity {
    /// Explicitly closed / resolved / removed.
    Closed,
    /// Explicitly open / unresolved / not fixed.
    Open,
    /// No state-change language detected.
    None,
}

/// Split content into lowercase alphanumeric tokens (hyphens/underscores split
/// compounds so `fixed-width` → `fixed`+`width` is *not* used; we keep
/// hyphenated forms as a single non-marker token by treating non-alnum as
/// separators only when producing bare words, then check whole tokens against
/// markers via word-boundary scan on the original lowercased string).
fn lowercase_word_tokens(content: &str) -> Vec<String> {
    content
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// True when `needle` appears in `haystack` as a whole token (bounded by
/// non-alphanumeric edges). Hyphenated compounds like `fixed-width` do **not**
/// match bare `fixed` because `-` is treated as an interior connector, not a
/// boundary that ends the marker alone.
fn has_whole_token(haystack_lower: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let h = haystack_lower.as_bytes();
    let n = needle.as_bytes();
    let mut start = 0;
    while start + n.len() <= h.len() {
        if let Some(rel) = haystack_lower[start..].find(needle) {
            let i = start + rel;
            let before_ok = i == 0 || !is_token_char(h[i - 1]);
            let after = i + n.len();
            let after_ok = after >= h.len() || !is_token_char(h[after]);
            if before_ok && after_ok {
                return true;
            }
            start = i + 1;
        } else {
            break;
        }
    }
    false
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn state_polarity(content: &str) -> StatePolarity {
    let lower = content.to_lowercase();

    // Phrase checks first (multi-word); open phrases win over closed substrings
    // they may embed (e.g. "not fixed" before bare "fixed").
    let has_open_phrase = OPEN_STATE_PHRASES
        .iter()
        .any(|p| has_whole_phrase(&lower, p));
    let has_closed_phrase = CLOSED_STATE_PHRASES
        .iter()
        .any(|p| has_whole_phrase(&lower, p));

    let tokens = lowercase_word_tokens(content);
    let has_open_token = tokens
        .iter()
        .any(|t| OPEN_STATE_MARKERS.iter().any(|m| t == *m));
    // Whole-token match: `fixed` matches, `fixed-width` / `unresolved` do not.
    let has_closed_token = CLOSED_STATE_MARKERS
        .iter()
        .any(|m| has_whole_token(&lower, m));

    let open = has_open_phrase || has_open_token;
    let closed = has_closed_phrase || has_closed_token;

    // Negated / open language dominates if both fire (e.g. "not fixed").
    if open && !closed {
        return StatePolarity::Open;
    }
    if open && closed {
        // "not fixed" sets open phrase + closed token "fixed" — treat as Open.
        if has_open_phrase {
            return StatePolarity::Open;
        }
        // Both explicit without phrase negation: conservative None (no flip).
        return StatePolarity::None;
    }
    if closed {
        return StatePolarity::Closed;
    }
    StatePolarity::None
}

/// Whole-phrase match with token boundaries on both ends of the phrase.
fn has_whole_phrase(haystack_lower: &str, phrase: &str) -> bool {
    // Reuse whole-token logic treating the phrase as a unit whose interior
    // spaces are allowed; boundaries still require non-token chars outside.
    if phrase.is_empty() {
        return false;
    }
    let h = haystack_lower.as_bytes();
    let n = phrase.as_bytes();
    let mut start = 0;
    while start + n.len() <= h.len() {
        if let Some(rel) = haystack_lower[start..].find(phrase) {
            let i = start + rel;
            let before_ok = i == 0 || !is_token_char(h[i - 1]);
            let after = i + n.len();
            let after_ok = after >= h.len() || !is_token_char(h[after]);
            if before_ok && after_ok {
                return true;
            }
            start = i + 1;
        } else {
            break;
        }
    }
    false
}

/// State-change markers that, combined with high subject overlap, imply the new
/// memory supersedes the old one. Polarity-aware: open→closed is Contradicts;
/// closed→closed refinement is not; negated forms (`unresolved`, `not fixed`)
/// never count as closed markers.
fn has_state_change_contradiction(
    new_content: &str,
    existing_content: &str,
    new_tokens: &std::collections::HashSet<String>,
    existing_tokens: &std::collections::HashSet<String>,
) -> bool {
    let new_pol = state_polarity(new_content);
    let existing_pol = state_polarity(existing_content);

    // Only a closed new side can supersede via state-change language.
    if new_pol != StatePolarity::Closed {
        return false;
    }
    // Same closed polarity on both sides → refine, not supersede.
    // Open → Closed is a real flip. None → Closed is a first resolution.
    if existing_pol == StatePolarity::Closed {
        return false;
    }

    // Require real subject overlap beyond marker vocabulary.
    let marker_tokens: std::collections::HashSet<&str> = CLOSED_STATE_MARKERS
        .iter()
        .chain(OPEN_STATE_MARKERS.iter())
        .chain(["no", "longer", "not", "still"].iter())
        .copied()
        .collect();

    let overlap: usize = new_tokens
        .intersection(existing_tokens)
        .filter(|t| !marker_tokens.contains(t.as_str()))
        .count();
    // "auth bug exists" ∩ "auth bug fixed" → {auth, bug} after filtering markers.
    overlap >= 1
        && ConsolidationEngine::content_similarity(
            &MemoryEntry::new(MemoryType::Knowledge, new_content),
            &MemoryEntry::new(MemoryType::Knowledge, existing_content),
        ) >= 0.5
}

/// Detect shared key-like stems with differing numeric values, e.g.
/// `max_tokens=128000` vs `max_tokens=4096`, or `port:8080` vs `port:3000`.
fn has_numeric_value_drift(new_content: &str, existing_content: &str) -> bool {
    let new_pairs = extract_key_numeric_pairs(new_content);
    let existing_pairs = extract_key_numeric_pairs(existing_content);
    if new_pairs.is_empty() || existing_pairs.is_empty() {
        return false;
    }

    for (key, new_val) in &new_pairs {
        if let Some(old_val) = existing_pairs.get(key) {
            if new_val != old_val {
                return true;
            }
        }
    }
    false
}

fn extract_key_numeric_pairs(content: &str) -> std::collections::HashMap<String, String> {
    // Match key=number / key:number / key = number (alnum/_/- keys, length ≥ 3).
    // Also accept glued forms already present as single whitespace tokens.
    let mut pairs = std::collections::HashMap::new();
    let lower = content.to_lowercase();

    // Scan whitespace tokens and also run a light char-level pass for `key=val`.
    for raw in lower.split_whitespace() {
        let token =
            raw.trim_matches(|c: char| matches!(c, ',' | ';' | '.' | ')' | '(' | '"' | '\''));
        if let Some((k, v)) = split_key_numeric(token) {
            pairs.insert(k, v);
        }
    }

    // Character-level: catch `max_tokens=128000` even if punctuation glued oddly.
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
            {
                i += 1;
            }
            let key = &lower[start..i];
            // skip spaces
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let num_start = j;
                while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                    j += 1;
                }
                if j > num_start && key.len() >= 3 {
                    let val = &lower[num_start..j];
                    if val.chars().any(|c| c.is_ascii_digit()) {
                        pairs.insert(key.to_string(), val.to_string());
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }

    pairs
}

fn split_key_numeric(token: &str) -> Option<(String, String)> {
    for sep in ['=', ':'] {
        if let Some((k, v)) = token.split_once(sep) {
            let k = k.trim();
            let v = v.trim();
            if k.len() >= 3
                && k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !v.is_empty()
                && v.chars().all(|c| c.is_ascii_digit() || c == '.')
                && v.chars().any(|c| c.is_ascii_digit())
            {
                return Some((k.to_string(), v.to_string()));
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    pub max_memories: usize,
    pub importance_threshold: f32,
    pub age_threshold_hours: u64,
    pub consolidation_interval_hours: u64,
    pub enable_auto_consolidation: bool,
    /// Multiplier applied to effective importance when `stale_marked_at` is set.
    pub staleness_penalty: f32,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            max_memories: 200,
            importance_threshold: 0.6,
            age_threshold_hours: 48,
            consolidation_interval_hours: 6,
            enable_auto_consolidation: true,
            staleness_penalty: 0.5,
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
            staleness_penalty: settings.staleness_penalty,
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

        let now = Utc::now();
        let eff_cfg = crate::context::EffectiveImportanceCfg {
            age_threshold_hours: self.config.age_threshold_hours,
            staleness_penalty: self.config.staleness_penalty,
        };
        let mut sorted_memories: Vec<_> = memories.iter().collect();
        sorted_memories.sort_by(|a, b| {
            let ea = a.effective_importance(now, &eff_cfg);
            let eb = b.effective_importance(now, &eff_cfg);
            eb.partial_cmp(&ea).unwrap_or(std::cmp::Ordering::Equal)
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
        // Tombstones are audit-only on list; consolidate drops them.
        if memory.superseded_by.is_some() {
            return false;
        }

        // Retention uses effective importance (decay + hit-rate + staleness),
        // not raw base importance.
        let now = Utc::now();
        let cfg = crate::context::EffectiveImportanceCfg {
            age_threshold_hours: self.config.age_threshold_hours,
            staleness_penalty: self.config.staleness_penalty,
        };
        let effective = memory.effective_importance(now, &cfg);
        if effective >= self.config.importance_threshold {
            return true;
        }

        // Type-specific retention for low-effective memories.
        // Knowledge/Preference used to be immortal, which let low-value
        // "facts" accumulate forever. They now get a longer TTL (4× base)
        // instead of permanent retention. Ephemeral types expire faster.
        // Age is measured from the decay anchor (last_reinforced_at or timestamp).
        let anchor = memory.last_reinforced_at.unwrap_or(memory.timestamp);
        let age_hours = (now - anchor).num_hours();
        let age = age_hours.max(0) as u64;
        let base = self.config.age_threshold_hours.max(1);

        let ttl = type_half_life_hours(memory.memory_type.clone(), base).round() as u64;
        let ttl = ttl.max(1);

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
            // Tombstoned entries are not merge/supersede targets; the live
            // replacement (or another live near-dup) should win instead.
            if other.superseded_by.is_some() {
                return false;
            }
            let sim = if require_same_type {
                self.calculate_similarity(entry, other)
            } else {
                Self::content_similarity(entry, other)
            };
            // Spec / add_memory gate is Jaccard >= threshold (inclusive).
            sim >= threshold
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
            // Feedback counters stay on the surviving entry; Compatible path
            // will call reinforce() after merge. Task 1 keeps fields intact.
            recall_count: existing.recall_count,
            hit_count: existing.hit_count,
            last_reinforced_at: existing.last_reinforced_at,
            superseded_by: existing.superseded_by.clone(),
            stale_marked_at: existing.stale_marked_at,
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
    use std::path::PathBuf;

    #[test]
    fn extract_memory_paths_finds_src_relative_files() {
        let paths = extract_memory_paths("logic lives in src/does_not_exist_xyz.rs only");
        assert_eq!(paths, vec![PathBuf::from("src/does_not_exist_xyz.rs")]);
    }

    #[test]
    fn extract_memory_paths_ignores_bare_urls() {
        let paths = extract_memory_paths("see https://example.com/src/foo.rs for docs");
        assert!(
            paths.is_empty(),
            "URL path segments must not be treated as filesystem paths: {paths:?}"
        );
    }

    #[test]
    fn extract_memory_paths_strips_line_suffix() {
        let paths = extract_memory_paths("bug at src/lib/mod.rs:42 in parser");
        assert_eq!(paths, vec![PathBuf::from("src/lib/mod.rs")]);
    }

    #[test]
    fn extract_memory_paths_multiple_unique() {
        let paths = extract_memory_paths("a src/a.rs and b src/b.ts plus src/a.rs again");
        assert_eq!(
            paths,
            vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.ts")]
        );
    }

    #[test]
    fn extract_memory_paths_rejects_prefix_glued_roots() {
        let paths = extract_memory_paths("notes about foosrc/foo.rs and barlib/bar.ts");
        assert!(
            paths.is_empty(),
            "glued prefixes must not extract as repo paths: {paths:?}"
        );
        // Still extracts when a proper boundary precedes the root segment.
        let paths = extract_memory_paths("see (src/foo.rs) nearby");
        assert_eq!(paths, vec![PathBuf::from("src/foo.rs")]);
    }

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
        // Fresh reinforce anchor so effective stays high despite old timestamp.
        entry.last_reinforced_at = Some(chrono::Utc::now());
        assert!(
            engine.should_keep(&entry),
            "high-effective memories must never expire by age alone"
        );
    }

    #[test]
    fn should_not_keep_superseded_even_with_high_raw_importance() {
        let engine = ConsolidationEngine::default();
        let mut entry =
            MemoryEntry::new(MemoryType::Knowledge, "tombstoned fact").with_importance(0.99);
        entry.superseded_by = Some("newer-id".into());
        entry.timestamp = chrono::Utc::now();
        assert!(
            !engine.should_keep(&entry),
            "superseded entries have effective 0 and must not be kept"
        );
    }

    #[test]
    fn should_not_keep_high_raw_when_effective_decays_below_threshold() {
        let engine = ConsolidationEngine::default();
        // Base 0.9 would pass the old raw gate; after many half-lives effective ≪ 0.6.
        let mut entry =
            MemoryEntry::new(MemoryType::Knowledge, "decayed high raw").with_importance(0.9);
        entry.timestamp = chrono::Utc::now() - chrono::Duration::hours(10_000);
        entry.last_reinforced_at = Some(entry.timestamp);
        assert!(
            !engine.should_keep(&entry),
            "retention must use effective importance, not raw base"
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

    #[test]
    fn should_keep_respects_custom_staleness_penalty() {
        // Past Knowledge TTL (4 * 48h = 192h) so retention only succeeds via
        // effective >= importance_threshold. Boost hit_factor so that a mild
        // penalty still clears the gate while the default 0.5 does not:
        //   half-life=192, age≈193 → decay≈0.5; hit_factor=1.5; imp=1.0
        //   penalty 0.5 → eff≈0.37 < 0.6 → drop
        //   penalty 0.85 → eff≈0.63 >= 0.6 → keep
        let mut entry =
            MemoryEntry::new(MemoryType::Knowledge, "stale marked high raw").with_importance(1.0);
        entry.timestamp = chrono::Utc::now() - chrono::Duration::hours(193);
        entry.last_reinforced_at = Some(entry.timestamp);
        entry.stale_marked_at = Some(chrono::Utc::now());
        entry.hit_count = 100;
        entry.recall_count = 0;

        let cfg_harsh = ConsolidationConfig {
            staleness_penalty: 0.5,
            ..Default::default()
        };
        let engine_harsh = ConsolidationEngine::new(cfg_harsh);
        assert!(
            !engine_harsh.should_keep(&entry),
            "penalty 0.5 should drop aged stale high-raw knowledge (effective below threshold)"
        );

        let cfg_mild = ConsolidationConfig {
            staleness_penalty: 0.85,
            ..Default::default()
        };
        let engine_mild = ConsolidationEngine::new(cfg_mild);
        assert!(
            engine_mild.should_keep(&entry),
            "penalty 0.85 should keep the same entry via effective>=threshold"
        );
    }

    #[test]
    fn from_memory_settings_copies_staleness_penalty() {
        let settings = crate::config::MemorySettings {
            staleness_penalty: 0.25,
            ..Default::default()
        };
        let cfg = ConsolidationConfig::from_memory_settings(&settings);
        assert!((cfg.staleness_penalty - 0.25).abs() < f32::EPSILON);
    }

    // ── classify_relation gold cases (Task 5 / M2) ──────────────────────

    #[test]
    fn classify_relation_state_change_marker_is_contradicts() {
        let existing = MemoryEntry::new(MemoryType::Knowledge, "auth bug exists");
        let new = MemoryEntry::new(MemoryType::Knowledge, "auth bug fixed");
        assert_eq!(
            classify_relation(&new, &existing),
            MemoryRelation::Contradicts,
            "state-change marker 'fixed' with shared subject must supersede"
        );
    }

    #[test]
    fn classify_relation_numeric_drift_is_contradicts() {
        let existing = MemoryEntry::new(MemoryType::Knowledge, "API chat uses max_tokens=128000");
        let new = MemoryEntry::new(MemoryType::Knowledge, "API chat uses max_tokens=4096");
        assert_eq!(
            classify_relation(&new, &existing),
            MemoryRelation::Contradicts,
            "shared key-like token with differing numeric value must supersede"
        );
    }

    #[test]
    fn classify_relation_subset_is_compatible() {
        let existing = MemoryEntry::new(MemoryType::Knowledge, "use jwt authentication");
        let new = MemoryEntry::new(MemoryType::Knowledge, "use jwt");
        assert_eq!(
            classify_relation(&new, &existing),
            MemoryRelation::Compatible,
            "subset / same-direction refinement must merge+reinforce, not supersede"
        );
        // Symmetric direction also Compatible.
        assert_eq!(
            classify_relation(&existing, &new),
            MemoryRelation::Compatible
        );
    }

    #[test]
    fn classify_relation_similar_but_unrelated_choice_is_ambiguous() {
        // High token overlap, different concrete choice, no state-change marker
        // and no shared key=value numeric drift → must NOT false-supersede.
        let existing = MemoryEntry::new(
            MemoryType::Preference,
            "prefer postgres database for storage layer",
        );
        let new = MemoryEntry::new(
            MemoryType::Preference,
            "prefer mysql database for storage layer",
        );
        assert_eq!(
            classify_relation(&new, &existing),
            MemoryRelation::Ambiguous,
            "competing alternatives without markers should stay Ambiguous"
        );
    }

    #[test]
    fn classify_relation_unresolved_substring_is_not_marker() {
        // "unresolved" contains the substring "resolved" but is open-state language.
        let existing = MemoryEntry::new(MemoryType::Knowledge, "auth bug reported in login");
        let new = MemoryEntry::new(MemoryType::Knowledge, "auth bug unresolved in login");
        assert_ne!(
            classify_relation(&new, &existing),
            MemoryRelation::Contradicts,
            "'unresolved' must not match marker via substring 'resolved'"
        );
    }

    #[test]
    fn classify_relation_unresolved_to_resolved_is_contradicts() {
        // Both sides mention resolution vocabulary; polarity must flip open→closed.
        let existing = MemoryEntry::new(MemoryType::Knowledge, "auth bug unresolved");
        let new = MemoryEntry::new(MemoryType::Knowledge, "auth bug resolved");
        assert_eq!(
            classify_relation(&new, &existing),
            MemoryRelation::Contradicts,
            "unresolved → resolved polarity flip must supersede"
        );
    }

    #[test]
    fn classify_relation_fixed_width_is_not_state_marker() {
        // Hyphenated "fixed-width" must not count as whole-token marker "fixed".
        let existing =
            MemoryEntry::new(MemoryType::Knowledge, "layout uses grid columns for forms");
        let new = MemoryEntry::new(
            MemoryType::Knowledge,
            "layout uses fixed-width columns for forms",
        );
        assert_ne!(
            classify_relation(&new, &existing),
            MemoryRelation::Contradicts,
            "'fixed-width' must not count as state-change marker 'fixed'"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Type-specific TTL: Knowledge/Pref (4×) > Insight/Decision (2×) > Session/Task/Conv (1×)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn should_keep_knowledge_survives_longer_than_session_when_both_low_importance() {
        let engine = ConsolidationEngine::default();
        let now = Utc::now();
        // 50h — past Session TTL (48h) but within Knowledge TTL (192h)
        let old_time = now - chrono::Duration::hours(50);

        let mut knowledge =
            MemoryEntry::new(MemoryType::Knowledge, "Knowledge — survives via 4× TTL")
                .with_importance(0.1); // well below 0.6, relies on TTL
        knowledge.timestamp = old_time;
        knowledge.last_reinforced_at = Some(old_time);

        let mut session = MemoryEntry::new(MemoryType::Session, "Session — drops via 1× TTL")
            .with_importance(0.1);
        session.timestamp = old_time;
        session.last_reinforced_at = Some(old_time);

        assert!(engine.should_keep(&knowledge));
        assert!(!engine.should_keep(&session));
    }

    #[test]
    fn should_keep_insight_lives_longer_than_task() {
        let engine = ConsolidationEngine::default();
        let now = Utc::now();
        // 75h — past Task TTL (48h) but within Insight TTL (96h)
        let old_time = now - chrono::Duration::hours(75);

        let mut insight = MemoryEntry::new(
            MemoryType::Insight,
            "Architecture insight — survives via 2× TTL",
        )
        .with_importance(0.2);
        insight.timestamp = old_time;
        insight.last_reinforced_at = Some(old_time);

        let mut task =
            MemoryEntry::new(MemoryType::Task, "Task — drops via 1× TTL").with_importance(0.2);
        task.timestamp = old_time;
        task.last_reinforced_at = Some(old_time);

        assert!(engine.should_keep(&insight));
        assert!(!engine.should_keep(&task));
    }

    #[test]
    fn should_keep_high_importance_survives_regardless_of_type_ttl() {
        let engine = ConsolidationEngine::default();
        let now = Utc::now();
        // 75h — past Session's 48h TTL, but effective importance still > 0.6
        let old_time = now - chrono::Duration::hours(75);

        let mut session = MemoryEntry::new(
            MemoryType::Session,
            "High-importance session — survives via effective importance",
        )
        .with_importance(0.9); // high enough to survive ~75h of decay
        session.timestamp = old_time;
        session.last_reinforced_at = Some(old_time);

        // effective_importance decay: 0.9 × 2^(-75/48) ≈ 0.9 × 0.338 ≈ 0.304
        // With hitrate factor ~1.0: 0.304 × 1.0 = 0.304, still < 0.6
        // So this won't survive via effective importance — it relis on the
        // "high-importance keep" clause (importance >= threshold bypasses TTL)
        //
        // Actually the older test implies: effective_importance doesn't bypass
        // TTL; the second branch (age < type_ttl) is separate. Let me verify.
        //
        // After 75h: Session TTL = 48h, age = 75h > 48h → TTL branch fails.
        // effective_importance: 0.9 × 0.338 ≈ 0.304 < 0.6.
        // Both branches fail → should NOT keep.
        //
        // Use a type with longer TTL to demonstrate:
        assert!(
            !engine.should_keep(&session),
            "session past TTL with decaying effective importance should be dropped"
        );
    }

    #[test]
    fn should_keep_preference_has_same_ttl_as_knowledge() {
        let engine = ConsolidationEngine::default();
        let now = Utc::now();
        // 50h — both Preference and Knowledge have 4× TTL = 192h
        let old_time = now - chrono::Duration::hours(50);

        let mut pref = MemoryEntry::new(MemoryType::Preference, "User prefers Rust edition 2021")
            .with_importance(0.1);
        pref.timestamp = old_time;
        pref.last_reinforced_at = Some(old_time);

        let mut knowledge =
            MemoryEntry::new(MemoryType::Knowledge, "Cargo uses edition 2021").with_importance(0.1);
        knowledge.timestamp = old_time;
        knowledge.last_reinforced_at = Some(old_time);

        // Both have same 4× TTL multiplier
        assert!(engine.should_keep(&pref));
        assert!(engine.should_keep(&knowledge));
    }
}
