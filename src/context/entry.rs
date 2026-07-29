//! Memory entry data model: the core record type and its derived importance.
//!
//! Extracted from `mod.rs`. These types are the public vocabulary of the
//! memory system — `MemoryManager`, the compactor, the recall injector, and
//! tests all speak in terms of [`MemoryEntry`] / [`MemoryType`] /
//! [`MemoryOrigin`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::context::consolidation;

/// Runtime parameters for [`MemoryEntry::effective_importance`].
#[derive(Debug, Clone, Copy)]
pub struct EffectiveImportanceCfg {
    pub age_threshold_hours: u64,
    pub staleness_penalty: f32,
}

/// Controls how a [`MemoryEntry`] is retrieved for prompt injection.
///
/// - `Auto`: Every-turn TF-IDF recall automatically injects this entry
///   (current behaviour, default).  Suitable for short atomic facts (1–3
///   sentences) that the agent should "just know" when processing a related
///   task.
/// - `OnDemand`: This entry is **never** injected automatically.  The agent
///   (or user) must call `memory list` (CLI) or `memory_add` (search) to
///   retrieve it.  Suitable for longer structured documents (design docs,
///   analysis notes, checklists) that would bloat the context window if
///   injected every turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum RetrievalMode {
    #[default]
    Auto,
    OnDemand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub importance: f32,
    pub tags: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    /// Times injected into `<memory-context>`.
    #[serde(default)]
    pub recall_count: u32,
    /// Positive feedback count (Compatible `reinforce`).
    #[serde(default)]
    pub hit_count: u32,
    /// Decay anchor; `None` → use `timestamp` until first consolidate anchors.
    #[serde(default)]
    pub last_reinforced_at: Option<DateTime<Utc>>,
    /// Tombstone target id when this entry is superseded.
    #[serde(default)]
    pub superseded_by: Option<String>,
    /// Idempotent codebase-staleness mark.
    #[serde(default)]
    pub stale_marked_at: Option<DateTime<Utc>>,
    /// Injection strategy: `Auto` (every-turn TF-IDF recall) or `OnDemand`
    /// (explicit search only).  Defaults to `Auto`.
    #[serde(default)]
    pub retrieval_mode: RetrievalMode,
    // Note: the `embedding` field was removed — it was never populated
    // anywhere and inflated every serialized JSON file. Old JSON files
    // containing `"embedding": null` still deserialize correctly because
    // serde ignores unknown fields by default.
}

impl MemoryEntry {
    pub fn new(memory_type: MemoryType, content: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            memory_type,
            content: content.to_string(),
            timestamp: Utc::now(),
            importance: 0.5,
            tags: Vec::new(),
            metadata: HashMap::new(),
            recall_count: 0,
            hit_count: 0,
            last_reinforced_at: None,
            superseded_by: None,
            stale_marked_at: None,
            retrieval_mode: RetrievalMode::Auto,
        }
    }

    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_metadata(mut self, key: &str, value: serde_json::Value) -> Self {
        self.metadata.insert(key.to_string(), value);
        self
    }

    /// Record positive feedback. Does **not** raise base `importance`.
    pub fn reinforce(&mut self, now: DateTime<Utc>) {
        self.hit_count = self.hit_count.saturating_add(1);
        self.last_reinforced_at = Some(now);
    }

    /// Pure effective importance used for recall ranking / retention.
    ///
    /// The raw product `importance × decay × hit_factor × stale_mul` is
    /// clamped to `[0.0, 1.0]` so the value stays a well-formed weight:
    /// ranking is unaffected (monotonic transform), but downstream consumers
    /// (prompt injection labels, threshold comparisons) never see values >1.
    pub fn effective_importance(&self, now: DateTime<Utc>, cfg: &EffectiveImportanceCfg) -> f32 {
        if self.superseded_by.is_some() {
            return 0.0;
        }
        let anchor = self.last_reinforced_at.unwrap_or(self.timestamp);
        let hours = (now - anchor).num_minutes().max(0) as f64 / 60.0;
        let half =
            consolidation::type_half_life_hours(self.memory_type.clone(), cfg.age_threshold_hours)
                .max(1e-6);
        let decay = (-std::f64::consts::LN_2 * hours / half).exp() as f32;
        let hitrate =
            ((self.hit_count as f32 + 1.0) / (self.recall_count as f32 + 2.0)).clamp(0.0, 1.0);
        // Laplace prior hitrate=0.5 maps to neutral 1.0 (neither reward nor penalty).
        // hitrate ∈ [0,1] ⇒ hit_factor ∈ [0.5, 1.5]; clamp makes the bound explicit.
        let hit_factor = (0.5 + hitrate).clamp(0.5, 1.5);
        let stale_mul = if self.stale_marked_at.is_some() {
            cfg.staleness_penalty
        } else {
            1.0
        };
        (self.importance * decay * hit_factor * stale_mul).clamp(0.0, 1.0)
    }
}

/// Result of adding a memory: the stored entry's id and whether it was
/// merged into an existing entry via dedup.
#[derive(Debug, Clone)]
pub struct MemoryAddResult {
    pub id: String,
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MemoryType {
    Session,
    Conversation,
    Knowledge,
    Preference,
    Task,
    Error,
    Insight,
    Decision,
}

/// Memory scope: determines physical storage location. Not serialized--
/// the origin is decided at load time by which Storage the file was read
/// from, and at write time by which Storage the caller routes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOrigin {
    Project,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResult {
    pub before: usize,
    pub after: usize,
    pub removed: usize,
    pub project_before: usize,
    pub project_after: usize,
    pub global_before: usize,
    pub global_after: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStatus {
    pub total_memories: usize,
    pub session_count: usize,
    pub conversation_count: usize,
    pub knowledge_count: usize,
    pub last_consolidation: Option<DateTime<Utc>>,
    pub storage_size_bytes: u64,
    /// Number of memories stored in the project-local pool.
    #[serde(default)]
    pub project_count: usize,
    /// Number of memories stored in the global pool.
    #[serde(default)]
    pub global_count: usize,
}
