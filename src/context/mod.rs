//! Context Module — session persistence, context window management,
//! history tracking, memory storage, and 3-layer compression strategy.
//!
//! Corresponds to harness mechanisms s06+s07: context compression, session
//! persistence, and memory consolidation.

pub mod consolidation;
pub mod history;
pub mod inject;
pub mod memory_session;
pub mod migration;
mod session;
pub mod storage;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use anyhow::Context as _;

pub use consolidation::{
    classify_relation, ConsolidationConfig, ConsolidationEngine, MemoryRelation,
};
pub use history::{HistoryEntry, HistoryFilter, HistoryManager};
pub use memory_session::{
    Session as MemorySession, SessionDiffData, SessionInfo as MemorySessionInfo,
    SessionManager as MemorySessionManager, SessionUiMessage,
};
pub use storage::{Storage, StorageBackend};

/// Runtime parameters for [`MemoryEntry::effective_importance`].
#[derive(Debug, Clone, Copy)]
pub struct EffectiveImportanceCfg {
    pub age_threshold_hours: u64,
    pub staleness_penalty: f32,
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
        let hit_factor = 0.5 + hitrate; // 0.5..1.5 with clamp on hitrate
        let stale_mul = if self.stale_marked_at.is_some() {
            cfg.staleness_penalty
        } else {
            1.0
        };
        self.importance * decay * hit_factor * stale_mul
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

// ── MemoryIndex: TF-IDF inverted index for memory retrieval ──────────

/// In-memory inverted index with TF-IDF weighting for keyword search
/// over the memory corpus. Built lazily on `load()` and kept in sync
/// with `add_memory()` / `consolidate()`.
struct MemoryIndex {
    /// word → [(entry_index, normalized_tf)]
    inverted: HashMap<String, Vec<(usize, f32)>>,
    /// word → inverse document frequency
    idf: HashMap<String, f32>,
    /// total number of indexed entries
    doc_count: usize,
}

impl MemoryIndex {
    fn new() -> Self {
        Self {
            inverted: HashMap::new(),
            idf: HashMap::new(),
            doc_count: 0,
        }
    }

    /// Rebuild the entire index from a slice of MemoryEntry.
    fn rebuild(&mut self, entries: &[MemoryEntry]) {
        self.inverted.clear();
        self.idf.clear();
        self.doc_count = entries.len();

        if entries.is_empty() {
            return;
        }

        // Phase 1: count term frequencies per document.
        for (i, entry) in entries.iter().enumerate() {
            let mut tf_counts: HashMap<String, u32> = HashMap::new();
            for word in entry.content.split_whitespace() {
                if crate::context::ConsolidationEngine::is_meaningful_token(word) {
                    let lower = word.to_lowercase();
                    *tf_counts.entry(lower).or_insert(0) += 1;
                }
            }
            for (word, tf) in tf_counts {
                // Sub-linear TF scaling: 1 + log(tf)
                let tf_norm = 1.0 + (tf as f32).ln();
                self.inverted.entry(word).or_default().push((i, tf_norm));
            }
        }

        // Phase 2: compute IDF = log(N / df).
        let n = self.doc_count as f32;
        for (word, postings) in &self.inverted {
            let df = postings.len() as f32;
            if df > 0.0 {
                self.idf.insert(word.clone(), (n / df).ln());
            }
        }
    }

    /// Search the index for entries matching `query` (whitespace-split,
    /// stop-word filtered). Returns a list of `(entry_index, score)`
    /// sorted by descending TF-IDF score, limited to `top_n`.
    fn search(&self, query: &str, top_n: usize) -> Vec<(usize, f32)> {
        // Tokenize and filter query terms.
        let terms: Vec<String> = query
            .split_whitespace()
            .filter(|w| crate::context::ConsolidationEngine::is_meaningful_token(w))
            .map(|w| w.to_lowercase())
            .collect();

        if terms.is_empty() || self.inverted.is_empty() {
            return Vec::new();
        }

        // Accumulate TF-IDF scores per entry.
        let mut scores: HashMap<usize, f32> = HashMap::new();
        for term in &terms {
            if let Some(postings) = self.inverted.get(term.as_str()) {
                let idf = self.idf.get(term.as_str()).copied().unwrap_or(0.0);
                for &(idx, tf) in postings {
                    *scores.entry(idx).or_insert(0.0) += tf * idf;
                }
            }
        }

        // Sort by score descending, return top N.
        let mut ranked: Vec<(usize, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_n);
        ranked
    }

    /// Add a single entry to the index incrementally.
    fn add_entry(&mut self, entry: &MemoryEntry, idx: usize) {
        self.doc_count += 1;

        let mut tf_counts: HashMap<String, u32> = HashMap::new();
        for word in entry.content.split_whitespace() {
            if crate::context::ConsolidationEngine::is_meaningful_token(word) {
                *tf_counts.entry(word.to_lowercase()).or_insert(0) += 1;
            }
        }

        for (word, tf) in tf_counts {
            let tf_norm = 1.0 + (tf as f32).ln();
            self.inverted
                .entry(word.clone())
                .or_default()
                .push((idx, tf_norm));
            // Recompute IDF for this word (doc_count changed).
            if let Some(postings) = self.inverted.get(&word) {
                let df = postings.len() as f32;
                let n = self.doc_count as f32;
                self.idf.insert(word, (n / df).ln());
            }
        }
    }

    /// Replace the indexed content for the entry at `idx` with `entry`'s
    /// content, in place.
    ///
    /// Removes all stale postings for `idx`, re-indexes from `entry`, and
    /// recomputes IDF. `doc_count` is unchanged because no document is added
    /// or removed. Used by `add_memory` after an in-place merge folds a
    /// near-duplicate into an existing entry, so the TF-IDF index stays
    /// consistent with the merged content.
    fn replace_entry(&mut self, entry: &MemoryEntry, idx: usize) {
        // Drop every posting that referenced the old content at `idx`.
        for postings in self.inverted.values_mut() {
            postings.retain(|(i, _)| *i != idx);
        }
        // Remove words that no longer have any postings.
        self.inverted.retain(|_, postings| !postings.is_empty());

        // Re-add postings from the new (merged) content.
        let mut tf_counts: HashMap<String, u32> = HashMap::new();
        for word in entry.content.split_whitespace() {
            if crate::context::ConsolidationEngine::is_meaningful_token(word) {
                *tf_counts.entry(word.to_lowercase()).or_insert(0) += 1;
            }
        }
        for (word, tf) in tf_counts {
            let tf_norm = 1.0 + (tf as f32).ln();
            self.inverted
                .entry(word.clone())
                .or_default()
                .push((idx, tf_norm));
        }

        // Recompute IDF for every word (doc_count is unchanged).
        let n = self.doc_count as f32;
        for (word, postings) in &self.inverted {
            let df = postings.len() as f32;
            self.idf.insert(word.clone(), (n / df).ln());
        }
        // Drop IDF entries for words that disappeared.
        self.idf.retain(|w, _| self.inverted.contains_key(w));
    }
}

// ── MemoryManager ────────────────────────────────────────────────────

pub struct MemoryManager {
    sessions: Arc<MemorySessionManager>,
    history: Arc<HistoryManager>,
    project_storage: Arc<Storage>,
    global_storage: Arc<Storage>,
    consolidation: Arc<ConsolidationEngine>,
    /// Project-local memories (indexed by TF-IDF for recall).
    memories: Arc<RwLock<Vec<MemoryEntry>>>,
    /// Global memories (injected every turn, not indexed for recall).
    global_memories: Arc<RwLock<Vec<MemoryEntry>>>,
    index: Arc<RwLock<MemoryIndex>>,
    /// Guards `consolidate()` so concurrent `add_memory()` calls wait
    /// until consolidation completes before proceeding.
    consolidating: Arc<AtomicBool>,
    /// Minimum importance required to accept a newly extracted memory.
    write_importance_threshold: f32,
    /// Maximum memories accepted from a single compaction extract.
    max_extract_per_compaction: usize,
    /// ε-greedy exploration rate for recall (0.0 = off).
    exploration_epsilon: f32,
    /// Whether consolidate should mark stale memories.
    staleness_check: bool,
    /// Multiplier applied when a memory is stale-marked.
    staleness_penalty: f32,
    /// Half-life hours for age decay in effective importance.
    age_threshold_hours: u64,
    /// Explicit project root for path-staleness checks (not derived from storage).
    project_root: PathBuf,
    /// Session/process-local ids recently chosen by recall exploration (v1).
    recently_explored: Arc<RwLock<HashSet<String>>>,
}

impl MemoryManager {
    /// Create project and global Storage instances with fallback.
    ///
    /// If the project root equals the home directory, or the project memory
    /// directory cannot be created (e.g. read-only CWD), project_storage
    /// falls back to the global memory directory so memories are not lost.
    fn create_dual_storage(
        project_root: &std::path::Path,
        global_path: PathBuf,
    ) -> (Arc<Storage>, Arc<Storage>) {
        let project_path = crate::utils::project_memory_dir(project_root);

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let cwd_is_home = project_root == home.as_path();

        let effective_project_path = if cwd_is_home {
            tracing::warn!(
                "CWD is the home directory; project memories will be stored in the global pool"
            );
            global_path.clone()
        } else {
            match std::fs::create_dir_all(&project_path) {
                Ok(()) => project_path,
                Err(e) => {
                    tracing::warn!(
                        path = %project_path.display(),
                        error = %e,
                        "Failed to create project memory directory; falling back to global pool"
                    );
                    global_path.clone()
                }
            }
        };

        if let Err(e) = std::fs::create_dir_all(&global_path) {
            tracing::warn!(
                path = %global_path.display(),
                error = %e,
                "Failed to create global memory directory; storage operations may fail later"
            );
        }

        (
            Arc::new(Storage::new(effective_project_path)),
            Arc::new(Storage::new(global_path)),
        )
    }

    pub fn new(project_root: PathBuf) -> Self {
        let (project_storage, global_storage) =
            Self::create_dual_storage(&project_root, crate::utils::global_memory_dir());

        Self {
            sessions: Arc::new(MemorySessionManager::with_project_root(
                project_root.clone(),
            )),
            history: Arc::new(HistoryManager::new()),
            project_storage,
            global_storage,
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root,
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Create a MemoryManager configured from user settings.
    ///
    /// The consolidation thresholds (`max_memories`, `importance_threshold`,
    /// `age_threshold_hours`, etc.) and write-time extract gates are read from
    /// the `storage.memory` section of `settings.json`. Previously these were
    /// hardcoded in `ConsolidationConfig::default()` and could not be tuned
    /// by users.
    ///
    /// `project_root` determines where project-local sessions are stored
    /// (`<project_root>/.wgenty-code/sessions/`).
    pub fn with_settings(settings: &crate::config::Settings, project_root: PathBuf) -> Self {
        let (project_storage, global_storage) =
            Self::create_dual_storage(&project_root, crate::utils::global_memory_dir());

        let consolidation_config =
            ConsolidationConfig::from_memory_settings(&settings.storage.memory);
        let mem = &settings.storage.memory;

        Self {
            sessions: Arc::new(MemorySessionManager::with_project_root(
                project_root.clone(),
            )),
            history: Arc::new(HistoryManager::new()),
            project_storage,
            global_storage,
            consolidation: Arc::new(ConsolidationEngine::new(consolidation_config)),
            memories: Arc::new(RwLock::new(Vec::new())),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: mem.write_importance_threshold,
            max_extract_per_compaction: mem.max_extract_per_compaction,
            exploration_epsilon: mem.exploration_epsilon,
            staleness_check: mem.staleness_check,
            staleness_penalty: mem.staleness_penalty,
            age_threshold_hours: mem.age_threshold_hours,
            project_root,
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Test-only constructor that isolates both project and global memory into
    /// the supplied directories, so unit tests never write to the real
    /// `~/.wgenty-code/memory/` global pool.
    #[cfg(test)]
    pub(crate) fn new_for_test(project_root: PathBuf, global_dir: PathBuf) -> Self {
        let (project_storage, global_storage) =
            Self::create_dual_storage(&project_root, global_dir);
        Self {
            sessions: Arc::new(MemorySessionManager::with_project_root(
                project_root.clone(),
            )),
            history: Arc::new(HistoryManager::new()),
            project_storage,
            global_storage,
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root,
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Write-time importance gate used by compaction extract.
    pub fn write_importance_threshold(&self) -> f32 {
        self.write_importance_threshold
    }

    /// Cap on memories accepted from a single compaction extract.
    pub fn max_extract_per_compaction(&self) -> usize {
        self.max_extract_per_compaction
    }

    /// ε-greedy exploration rate used by recall (0.0 disables exploration).
    pub fn exploration_epsilon(&self) -> f32 {
        self.exploration_epsilon
    }

    /// Builder-style override of exploration epsilon (primarily for tests).
    #[cfg(test)]
    pub(crate) fn with_exploration_epsilon(mut self, epsilon: f32) -> Self {
        self.exploration_epsilon = epsilon;
        self
    }

    /// Whether `id` is in the session-local recently-explored set.
    #[cfg(test)]
    pub(crate) async fn was_recently_explored(&self, id: &str) -> bool {
        self.recently_explored.read().await.contains(id)
    }

    /// Record that exploration selected `id` this session.
    pub(crate) async fn mark_recently_explored(&self, id: &str) {
        self.recently_explored.write().await.insert(id.to_string());
    }

    /// Snapshot of session-local recently-explored ids (one lock per explore pass).
    pub(crate) async fn recently_explored_ids(&self) -> HashSet<String> {
        self.recently_explored.read().await.clone()
    }

    /// Snapshot of project-local memories (for recall exploration candidates).
    pub(crate) async fn project_memories(&self) -> Vec<MemoryEntry> {
        self.memories.read().await.clone()
    }

    /// Whether consolidate should run staleness checks.
    pub fn staleness_check(&self) -> bool {
        self.staleness_check
    }

    /// Penalty multiplier applied to stale-marked memories.
    pub fn staleness_penalty(&self) -> f32 {
        self.staleness_penalty
    }

    /// Runtime cfg for [`MemoryEntry::effective_importance`].
    pub fn effective_importance_cfg(&self) -> EffectiveImportanceCfg {
        EffectiveImportanceCfg {
            age_threshold_hours: self.age_threshold_hours,
            staleness_penalty: self.staleness_penalty,
        }
    }

    pub async fn status(&self) -> anyhow::Result<MemoryStatus> {
        let memories = self.memories.read().await;
        let global = self.global_memories.read().await;
        let project_count = memories.len();
        let global_count = global.len();
        let storage_size = self.project_storage.size().await.unwrap_or(0)
            + self.global_storage.size().await.unwrap_or(0);

        let count_type = |t: MemoryType| {
            memories
                .iter()
                .chain(global.iter())
                .filter(|m| m.memory_type == t)
                .count()
        };

        Ok(MemoryStatus {
            total_memories: project_count + global_count,
            session_count: count_type(MemoryType::Session),
            conversation_count: count_type(MemoryType::Conversation),
            knowledge_count: count_type(MemoryType::Knowledge),
            last_consolidation: self.consolidation.last_consolidation().await,
            storage_size_bytes: storage_size,
            project_count,
            global_count,
        })
    }

    /// Add a memory to the specified scope. Dedup is performed within the
    /// same scope only. Only project memories are indexed for TF-IDF recall.
    pub async fn add_memory(
        &self,
        entry: MemoryEntry,
        scope: MemoryOrigin,
    ) -> anyhow::Result<MemoryAddResult> {
        // Wait if consolidation is in progress to avoid reading
        // transitional state. Use tokio::time::sleep polling so the
        // tokio runtime is not blocked.
        while self.consolidating.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let (storage, memories, is_project) = match scope {
            MemoryOrigin::Project => (&self.project_storage, self.memories.clone(), true),
            MemoryOrigin::Global => (&self.global_storage, self.global_memories.clone(), false),
        };

        let mut mem = memories.write().await;

        // Dedup guard: context compaction often re-extracts the same fact.
        // When Jaccard ≥ 0.6 against a live (non-superseded) same-scope entry,
        // classify the relation and either merge+reinforce, merge+flag, or
        // tombstone-supersede (new standalone; old file retained).
        //
        // Pre-tombstoned incoming entries (e.g. audit import / test fixtures
        // that already carry `superseded_by`) skip classification so they are
        // not merged back into a live near-dup.
        const DEDUP_THRESHOLD: f32 = 0.6;
        if entry.superseded_by.is_none() {
            if let Some(existing_idx) =
                self.consolidation
                    .find_similar(&entry, &mem, DEDUP_THRESHOLD, false)
            {
                match classify_relation(&entry, &mem[existing_idx]) {
                    MemoryRelation::Compatible => {
                        let mut merged =
                            ConsolidationEngine::merge_into(&mem[existing_idx], &entry);
                        merged.reinforce(Utc::now());
                        storage.save_memory(&merged).await?;
                        mem[existing_idx] = merged.clone();
                        if is_project {
                            self.index
                                .write()
                                .await
                                .replace_entry(&merged, existing_idx);
                        }
                        return Ok(MemoryAddResult {
                            id: merged.id.clone(),
                            merged: true,
                        });
                    }
                    MemoryRelation::Ambiguous => {
                        let mut merged =
                            ConsolidationEngine::merge_into(&mem[existing_idx], &entry);
                        merged
                            .metadata
                            .insert("relation_ambiguous".into(), serde_json::Value::Bool(true));
                        storage.save_memory(&merged).await?;
                        mem[existing_idx] = merged.clone();
                        if is_project {
                            self.index
                                .write()
                                .await
                                .replace_entry(&merged, existing_idx);
                        }
                        return Ok(MemoryAddResult {
                            id: merged.id.clone(),
                            merged: true,
                        });
                    }
                    MemoryRelation::Contradicts => {
                        // Tombstone existing (keep base importance); write new standalone.
                        // Do not change existing base importance (design open Q #2).
                        let old_importance = mem[existing_idx].importance;
                        mem[existing_idx].superseded_by = Some(entry.id.clone());
                        debug_assert!(
                            (mem[existing_idx].importance - old_importance).abs() < f32::EPSILON
                        );
                        storage.save_memory(&mem[existing_idx]).await?;
                        // Fall through to insert `entry` as a new live memory.
                    }
                }
            }
        }

        let idx = mem.len();
        mem.push(entry.clone());
        storage.save_memory(&entry).await?;
        // Incrementally update the index for the new entry.
        if is_project {
            self.index.write().await.add_entry(&entry, idx);
        }
        Ok(MemoryAddResult {
            id: entry.id.clone(),
            merged: false,
        })
    }

    pub async fn get_memory(&self, id: &str) -> Option<MemoryEntry> {
        let memories = self.memories.read().await;
        if let Some(m) = memories.iter().find(|m| m.id == id) {
            return Some(m.clone());
        }
        drop(memories);
        let global = self.global_memories.read().await;
        global.iter().find(|m| m.id == id).cloned()
    }

    /// Increment `recall_count` for project memories that were selected for
    /// `<memory-context>` injection and persist each updated entry.
    ///
    /// Count-only: content tokens are unchanged, so the TF-IDF index is not
    /// rebuilt. Lock order matches `add_memory`: wait while consolidating,
    /// then take the project memories write lock and save under that lock.
    /// Unknown ids are skipped (no error).
    pub async fn record_recall_injections(&self, ids: &[&str]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        while self.consolidating.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut mem = self.memories.write().await;
        for id in ids {
            if let Some(entry) = mem.iter_mut().find(|m| m.id == *id) {
                entry.recall_count = entry.recall_count.saturating_add(1);
                self.project_storage.save_memory(entry).await?;
            }
        }
        Ok(())
    }

    pub async fn search_memories(&self, query: &str) -> Vec<MemoryEntry> {
        // Search only project memories via the TF-IDF index. Global memories
        // are injected every turn and are not part of recall. Falls back to
        // substring scan if the index is empty (e.g., before load()).
        //
        // The index read guard is dropped before acquiring the memories read
        // guard so we never hold both locks at once. `add_memory` and
        // `consolidate` acquire the memories *write* lock first and then the
        // index lock; holding an index read guard across the memories read
        // acquisition would invert that order and can deadlock.
        let ranked = {
            let index = self.index.read().await;
            index.search(query, 10)
        };

        let memories = self.memories.read().await;
        if !ranked.is_empty() {
            ranked
                .into_iter()
                .filter_map(|(idx, _score)| memories.get(idx).cloned())
                .collect()
        } else {
            // Graceful degradation: substring fallback when index is cold.
            let query_lower = query.to_lowercase();
            memories
                .iter()
                .filter(|m| {
                    m.content.to_lowercase().contains(&query_lower)
                        || m.tags
                            .iter()
                            .any(|t| t.to_lowercase().contains(&query_lower))
                })
                .cloned()
                .collect()
        }
    }

    pub async fn get_memories_by_type(&self, memory_type: MemoryType) -> Vec<MemoryEntry> {
        let memories = self.memories.read().await;
        let mut result: Vec<MemoryEntry> = memories
            .iter()
            .filter(|m| m.memory_type == memory_type)
            .cloned()
            .collect();
        drop(memories);
        let global = self.global_memories.read().await;
        result.extend(
            global
                .iter()
                .filter(|m| m.memory_type == memory_type)
                .cloned(),
        );
        result
    }

    pub async fn get_important_memories(&self, threshold: f32) -> Vec<MemoryEntry> {
        let memories = self.memories.read().await;
        let mut result: Vec<MemoryEntry> = memories
            .iter()
            .filter(|m| m.importance >= threshold)
            .cloned()
            .collect();
        drop(memories);
        let global = self.global_memories.read().await;
        result.extend(global.iter().filter(|m| m.importance >= threshold).cloned());
        result
    }

    pub async fn clear(&self) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        memories.clear();
        self.project_storage.clear().await?;
        drop(memories);
        let mut global = self.global_memories.write().await;
        global.clear();
        self.global_storage.clear().await?;
        Ok(())
    }

    /// Delete a single memory by id from the specified origin pool.
    ///
    /// Removes the on-disk file (via `Storage::delete_memory`), drops the
    /// entry from the in-memory Vec, and rebuilds the TF-IDF index for
    /// project memories. The index uses positional Vec indices, so any
    /// removal shifts subsequent indices and a full rebuild is the simplest
    /// correct approach (mirrors `load()` / `consolidate()`). Global memories
    /// are not indexed, so no rebuild is needed for them.
    ///
    /// Returns `true` if an entry was found and removed, `false` if the id
    /// was not present (still considered success for idempotency).
    pub async fn delete_memory(&self, origin: MemoryOrigin, id: &str) -> anyhow::Result<bool> {
        match origin {
            MemoryOrigin::Project => {
                self.project_storage.delete_memory(id).await?;
                let mut mem = self.memories.write().await;
                let before = mem.len();
                mem.retain(|m| m.id != id);
                let removed = mem.len() < before;
                if removed {
                    // Rebuild the positional TF-IDF index to match the new Vec
                    // layout; a mid-Vec removal invalidates all higher indices.
                    self.index.write().await.rebuild(&mem);
                }
                Ok(removed)
            }
            MemoryOrigin::Global => {
                self.global_storage.delete_memory(id).await?;
                let mut global = self.global_memories.write().await;
                let before = global.len();
                global.retain(|m| m.id != id);
                Ok(global.len() < before)
            }
        }
    }

    pub async fn export(&self, output: &PathBuf) -> anyhow::Result<()> {
        let memories = self.memories.read().await;
        let content = serde_json::to_string_pretty(&*memories)?;
        tokio::fs::write(output, content).await?;
        Ok(())
    }

    pub async fn import(&self, input: &PathBuf) -> anyhow::Result<()> {
        let content = tokio::fs::read_to_string(input).await?;
        let imported: Vec<MemoryEntry> = serde_json::from_str(&content)?;

        let mut memories = self.memories.write().await;

        // Build a set of existing IDs so we can skip duplicates. Previously
        // importing the same file twice would insert duplicate entries into
        // the Vec (and silently overwrite on disk via save_memory by ID).
        let existing_ids: std::collections::HashSet<String> =
            memories.iter().map(|m| m.id.clone()).collect();

        for entry in &imported {
            if existing_ids.contains(&entry.id) {
                tracing::debug!(id = %entry.id, "skipping duplicate memory during import");
                continue;
            }
            self.project_storage.save_memory(entry).await?;
            memories.push(entry.clone());
        }

        Ok(())
    }

    /// Prune memories using the consolidation retention policy.
    ///
    /// This is the same engine as `dream`, but returns how many entries were
    /// removed so CLI callers can report progress. Both project and global
    /// pools are pruned independently.
    pub async fn prune(&self) -> anyhow::Result<PruneResult> {
        let before_project = self.memories.read().await.len();
        let before_global = self.global_memories.read().await.len();

        // Project pool (uses consolidate lock + index rebuild).
        self.consolidate().await?;

        // Global pool: apply the same consolidation rules, no TF-IDF index.
        let _guard = ConsolidationFileLock::acquire(&self.global_storage)
            .await
            .context("failed to acquire global consolidation lock")?;
        self.consolidating.store(true, Ordering::SeqCst);
        let _consolidating_guard = ConsolidatingGuard {
            flag: self.consolidating.clone(),
        };
        let mut global = self.global_memories.write().await;
        // Global pool: anchor migration only. Path-staleness is project-scoped
        // (codebase grounding against the current project root must not apply
        // to cross-project global memories).
        consolidation::apply_consolidate_prepass(
            &mut global,
            &self.project_root,
            Utc::now(),
            false, // never path-stale global memories
        );
        let consolidated_global = self.consolidation.consolidate(&global).await?;
        self.global_storage.reconcile(&consolidated_global).await?;
        *global = consolidated_global;
        drop(global);

        let after_project = self.memories.read().await.len();
        let after_global = self.global_memories.read().await.len();

        Ok(PruneResult {
            before: before_project + before_global,
            after: after_project + after_global,
            removed: (before_project + before_global).saturating_sub(after_project + after_global),
            project_before: before_project,
            project_after: after_project,
            global_before: before_global,
            global_after: after_global,
        })
    }

    /// List memories with optional filters (for CLI inspection).
    ///
    /// `min_importance` is compared against **effective** importance.
    /// Superseded rows remain listable (effective 0) when no min filter is set.
    pub async fn list_memories(
        &self,
        min_importance: Option<f32>,
        limit: usize,
    ) -> Vec<(MemoryOrigin, MemoryEntry)> {
        let now = Utc::now();
        let cfg = self.effective_importance_cfg();
        let mut out: Vec<(MemoryOrigin, MemoryEntry, f32)> = Vec::new();
        let project = self.memories.read().await;
        for m in project.iter() {
            let eff = m.effective_importance(now, &cfg);
            if min_importance.map(|t| eff >= t).unwrap_or(true) {
                out.push((MemoryOrigin::Project, m.clone(), eff));
            }
        }
        drop(project);
        let global = self.global_memories.read().await;
        for m in global.iter() {
            let eff = m.effective_importance(now, &cfg);
            if min_importance.map(|t| eff >= t).unwrap_or(true) {
                out.push((MemoryOrigin::Global, m.clone(), eff));
            }
        }
        out.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.timestamp.cmp(&a.1.timestamp))
        });
        if limit > 0 {
            out.truncate(limit);
        }
        out.into_iter().map(|(o, m, _)| (o, m)).collect()
    }

    pub async fn consolidate(&self) -> anyhow::Result<()> {
        // Acquire a cross-process advisory lock so that two concurrent
        // `wgenty-code memory dream` invocations (each with its own
        // MemoryManager instance) do not race on the same memory directory.
        // The in-process RwLock only protects within a single process.
        let _guard = ConsolidationFileLock::acquire(&self.project_storage)
            .await
            .context("failed to acquire consolidation lock")?;

        // Signal that consolidation is in progress so concurrent
        // add_memory() calls wait instead of reading transitional state.
        self.consolidating.store(true, Ordering::SeqCst);
        let _consolidating_guard = ConsolidatingGuard {
            flag: self.consolidating.clone(),
        };

        // Hold the write lock for the entire operation to prevent
        // concurrent add_memory() calls from inserting entries that
        // would be overwritten by the stale consolidated result.
        let mut memories = self.memories.write().await;

        // Prepass (LLM-free): anchor last_reinforced_at + optional all-missing
        // path staleness. Must run under the write lock before the engine so
        // retention sees effective importance with stale multipliers.
        consolidation::apply_consolidate_prepass(
            &mut memories,
            &self.project_root,
            Utc::now(),
            self.staleness_check,
        );

        let consolidated = self.consolidation.consolidate(&memories).await?;

        // P0 fix: persist the consolidated result AND remove orphaned
        // on-disk files in one atomic-ish step. Previously only the
        // in-memory Vec was replaced and `save()` (via `save_all()`)
        // wrote new files without deleting the old ones — causing
        // "consolidated away" memories to be resurrected on the next
        // `load_all()`.
        self.project_storage.reconcile(&consolidated).await?;
        // Rebuild the TF-IDF index to match the consolidated Vec. Previously
        // the index kept stale positional postings from the pre-consolidation
        // Vec (which may be shorter after TTL expiry and merging), so
        // `search_memories()` could resolve indices to wrong or missing
        // entries after a consolidation.
        self.index.write().await.rebuild(&consolidated);
        *memories = consolidated;
        Ok(())
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        // Load project memories and index them for TF-IDF recall.
        let project = self.project_storage.load_all().await?;
        self.index.write().await.rebuild(&project);
        *self.memories.write().await = project;

        // Load global memories (no indexing--injected verbatim every turn).
        let global = self.global_storage.load_all().await?;
        *self.global_memories.write().await = global;

        // Recover persisted sessions and history from disk so that
        // previously-saved records remain visible after a restart. Without
        // this, the in-memory HashMap/VecDeque starts empty and `list()`
        // returns nothing even though the files still exist on disk.
        // Failures here are non-fatal: a corrupt session/history file should
        // not block the app from starting.
        if let Err(e) = self.sessions.load_all().await {
            tracing::warn!(error = %e, "Failed to load persisted sessions from disk");
        }
        if let Err(e) = self.history.load().await {
            tracing::warn!(error = %e, "Failed to load command history from disk");
        }

        Ok(())
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let memories = self.memories.read().await;
        self.project_storage.save_all(&memories).await?;
        drop(memories);
        let global = self.global_memories.read().await;
        self.global_storage.save_all(&global).await
    }

    pub fn sessions(&self) -> Arc<MemorySessionManager> {
        self.sessions.clone()
    }
    pub fn history(&self) -> Arc<HistoryManager> {
        self.history.clone()
    }
    /// Returns the project-local storage (backward-compatible alias).
    pub fn storage(&self) -> Arc<Storage> {
        self.project_storage.clone()
    }
    pub fn project_storage(&self) -> Arc<Storage> {
        self.project_storage.clone()
    }
    pub fn global_storage(&self) -> Arc<Storage> {
        self.global_storage.clone()
    }
    pub fn consolidation(&self) -> Arc<ConsolidationEngine> {
        self.consolidation.clone()
    }

    /// Return all global memories (injected every turn without filtering).
    pub async fn global_memories(&self) -> Vec<MemoryEntry> {
        self.global_memories.read().await.clone()
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new(crate::utils::current_project_root())
    }
}

/// Cross-process advisory lock for memory consolidation.
///
/// `MemoryManager::consolidate()` holds an in-process `RwLock`, but that does
/// not protect against two separate `wgenty-code memory dream` processes
/// running concurrently against the same `~/.wgenty-code/memory` directory.
/// This lock uses a lock-file with a PID + timestamp to serialize
/// consolidation across processes.
///
/// Stale locks (older than `STALE_AFTER` or whose PID is no longer alive) are
/// reclaimed so a crashed process does not permanently block consolidation.
struct ConsolidationFileLock {
    lock_path: PathBuf,
}

/// A lock is considered stale after this duration and can be reclaimed.
const LOCK_STALE_AFTER_SECS: i64 = 30 * 60;

impl ConsolidationFileLock {
    async fn acquire(storage: &Storage) -> anyhow::Result<Self> {
        use tokio::io::AsyncWriteExt;

        let lock_path = storage.path().join(".consolidation.lock");

        // Ensure the directory exists.
        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        loop {
            // Atomically create the lock file with create_new(true) so that
            // only one process can hold it at a time.
            let create_result = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .await;

            match create_result {
                Ok(mut file) => {
                    // We created the file — write our PID + timestamp.
                    let pid = std::process::id();
                    let ts = chrono::Utc::now().to_rfc3339();
                    let content = format!("{}\n{}\n", pid, ts);
                    file.write_all(content.as_bytes())
                        .await
                        .context("failed to write consolidation lock file")?;
                    drop(file);
                    return Ok(Self { lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock exists — check if it's stale.
                    if Self::is_stale(&lock_path).await {
                        tracing::warn!("consolidation lock is stale; reclaiming");
                        // Best-effort removal; race is acceptable (worst case
                        // both processes remove then one wins create_new).
                        let _ = tokio::fs::remove_file(&lock_path).await;
                        continue;
                    }
                    // Wait and retry.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Err(e) => {
                    return Err(e).context("failed to create consolidation lock file");
                }
            }
        }
    }

    async fn is_stale(lock_path: &std::path::Path) -> bool {
        let content = match tokio::fs::read_to_string(lock_path).await {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut lines = content.lines();
        let pid_str = lines.next().and_then(|s| s.trim().parse::<u32>().ok());
        let ts_str = lines.next().map(|s| s.trim());

        // If we can read the PID, check liveness portably without pulling in
        // a `libc` dependency: spawn the platform-native `kill -0` (Unix) or
        // `tasklist` filter (Windows). If the check itself fails (e.g. the
        // helper binary is missing), fall through to the timestamp guard so
        // we never block consolidation forever.
        if let Some(pid) = pid_str {
            if Self::pid_alive(pid) {
                return false;
            }
        }

        // PID is dead or unparseable — check timestamp as a secondary guard.
        if let Some(ts) = ts_str {
            if let Ok(lock_time) = chrono::DateTime::parse_from_rfc3339(ts) {
                let lock_time: chrono::DateTime<chrono::Utc> =
                    lock_time.with_timezone(&chrono::Utc);
                let age = (chrono::Utc::now() - lock_time).num_seconds();
                return age > LOCK_STALE_AFTER_SECS;
            }
        }

        // Can't parse anything — treat as stale so we don't block forever.
        true
    }

    /// Check whether a process is alive, portably, without a `libc` dependency.
    ///
    /// Uses the platform-native helper (`kill -0` on Unix, `tasklist` on
    /// Windows). If the helper is unavailable or errors, returns `false` so
    /// the caller falls back to the timestamp-based staleness guard.
    fn pid_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            // On Windows, `tasklist /FI "PID eq <pid>"` lists the process if
            // it is running. This is heavier than Unix `kill -0` but avoids
            // a Win32 API dependency.
            std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                .unwrap_or(false)
        }
    }
}

impl Drop for ConsolidationFileLock {
    fn drop(&mut self) {
        // Best-effort lock removal on drop. Synchronous removal is fine here
        // because this runs at the end of `consolidate()` and must not be
        // skipped even if the async runtime is shutting down.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// RAII guard that resets the `consolidating` flag on drop, ensuring
/// it is always cleared even when `consolidate()` returns early via `?`.
struct ConsolidatingGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for ConsolidatingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_has_decision_variant() {
        // Decision variant is required by the memory system unify plan.
        // This test verifies the variant exists and can be constructed.
        match MemoryType::Decision {
            MemoryType::Decision => {}
            _ => panic!("MemoryType::Decision variant pattern mismatch"),
        }
    }

    #[test]
    fn legacy_memory_json_defaults_feedback_fields() {
        let raw = r#"{
            "id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "memory_type":"Knowledge",
            "content":"old",
            "timestamp":"2020-01-01T00:00:00Z",
            "importance":0.8,
            "tags":[],
            "metadata":{}
        }"#;
        let e: MemoryEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(e.recall_count, 0);
        assert_eq!(e.hit_count, 0);
        assert!(e.last_reinforced_at.is_none());
        assert!(e.superseded_by.is_none());
        assert!(e.stale_marked_at.is_none());
    }

    #[test]
    fn effective_importance_superseded_is_zero() {
        let now = Utc::now();
        let mut e = MemoryEntry::new(MemoryType::Knowledge, "x").with_importance(0.9);
        e.superseded_by = Some("other-id".into());
        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        assert_eq!(e.effective_importance(now, &cfg), 0.0);
    }

    #[test]
    fn effective_importance_never_recalled_hitrate_neutral() {
        // Laplace prior: hitrate = (0+1)/(0+2) = 0.5 → hit_factor = 0.5 + hitrate = 1.0
        let now = Utc::now();
        let mut e = MemoryEntry::new(MemoryType::Knowledge, "x").with_importance(0.8);
        e.timestamp = now;
        e.last_reinforced_at = None;
        e.recall_count = 0;
        e.hit_count = 0;
        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        let eff = e.effective_importance(now, &cfg);
        // decay=1 (now==anchor), stale_mul=1 → effective == base * 1.0
        let expected = 0.8 * 1.0;
        assert!(
            (eff - expected).abs() < 1e-5,
            "expected {expected}, got {eff}"
        );
    }

    #[test]
    fn effective_importance_hit_rate_damping() {
        // High recall / zero hits should damp below never-recalled neutral.
        let now = Utc::now();
        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        let mut neutral = MemoryEntry::new(MemoryType::Knowledge, "n").with_importance(0.8);
        neutral.timestamp = now;
        neutral.last_reinforced_at = None;
        neutral.recall_count = 0;
        neutral.hit_count = 0;

        let mut damped = MemoryEntry::new(MemoryType::Knowledge, "d").with_importance(0.8);
        damped.timestamp = now;
        damped.last_reinforced_at = None;
        damped.recall_count = 10;
        damped.hit_count = 0;

        let eff_neutral = neutral.effective_importance(now, &cfg);
        let eff_damped = damped.effective_importance(now, &cfg);
        assert!(
            eff_damped < eff_neutral,
            "damped ({eff_damped}) should be below neutral ({eff_neutral})"
        );
        // hitrate=(0+1)/(10+2)=1/12 → hit_factor=0.5+1/12 ≈ 0.5833
        let expected_damped = 0.8 * (0.5 + 1.0 / 12.0);
        assert!(
            (eff_damped - expected_damped).abs() < 1e-5,
            "expected ~{expected_damped}, got {eff_damped}"
        );
    }

    #[test]
    fn effective_importance_decays_with_age() {
        let now = Utc::now();
        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        let mut fresh = MemoryEntry::new(MemoryType::Knowledge, "x").with_importance(0.8);
        fresh.timestamp = now;
        fresh.last_reinforced_at = Some(now);

        let mut aged = MemoryEntry::new(MemoryType::Knowledge, "y").with_importance(0.8);
        aged.timestamp = now - chrono::Duration::hours(192); // one Knowledge half-life (48*4)
        aged.last_reinforced_at = Some(aged.timestamp);

        let eff_fresh = fresh.effective_importance(now, &cfg);
        let eff_aged = aged.effective_importance(now, &cfg);
        assert!(
            eff_aged < eff_fresh,
            "aged ({eff_aged}) should be below fresh ({eff_fresh})"
        );
        // At exactly one half-life, decay ≈ 0.5 → effective ≈ 0.8 * 0.5 * 1.0
        let expected_aged = 0.8 * 0.5 * 1.0;
        assert!(
            (eff_aged - expected_aged).abs() < 1e-3,
            "expected ~{expected_aged}, got {eff_aged}"
        );
    }

    #[test]
    fn effective_importance_stale_multiplier() {
        let now = Utc::now();
        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        let mut e = MemoryEntry::new(MemoryType::Knowledge, "x").with_importance(0.8);
        e.timestamp = now;
        e.last_reinforced_at = Some(now);
        let base = e.effective_importance(now, &cfg);
        e.stale_marked_at = Some(now);
        let stale = e.effective_importance(now, &cfg);
        assert!(
            (stale - base * cfg.staleness_penalty).abs() < 1e-5,
            "stale={stale}, base={base}"
        );
    }

    #[test]
    fn reinforce_bumps_hit_and_anchor_without_raising_importance() {
        let t0 = Utc::now();
        let mut e = MemoryEntry::new(MemoryType::Preference, "pref").with_importance(0.4);
        assert_eq!(e.hit_count, 0);
        assert!(e.last_reinforced_at.is_none());
        e.reinforce(t0);
        assert_eq!(e.hit_count, 1);
        assert_eq!(e.last_reinforced_at, Some(t0));
        assert!((e.importance - 0.4).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn import_skips_duplicate_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // Pre-populate with one memory.
        let existing = MemoryEntry::new(MemoryType::Knowledge, "existing");
        let existing_id = existing.id.clone();
        mm.add_memory(existing, MemoryOrigin::Project)
            .await
            .unwrap();

        // Import file contains the same ID + one new entry.
        let new_entry = MemoryEntry::new(MemoryType::Knowledge, "new");
        let mut dup = MemoryEntry::new(MemoryType::Knowledge, "existing");
        dup.id = existing_id.clone();
        let import_data = serde_json::to_string_pretty(&vec![dup, new_entry]).unwrap();

        let import_path = tmp.path().join("import.json");
        tokio::fs::write(&import_path, &import_data).await.unwrap();

        mm.import(&import_path).await.unwrap();

        let memories = mm.memories.read().await;
        // Should have 2 entries total (existing + new), not 3.
        assert_eq!(memories.len(), 2, "duplicate ID should be skipped");
        // Only one entry with the existing ID.
        assert_eq!(memories.iter().filter(|m| m.id == existing_id).count(), 1);
    }

    #[tokio::test]
    async fn status_reports_last_consolidation_after_consolidate() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // Before consolidation, last_consolidation should be None.
        let status = mm.status().await.unwrap();
        assert!(status.last_consolidation.is_none());

        // Add a memory and consolidate.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "test").with_importance(0.8),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.consolidate().await.unwrap();

        // After consolidation, last_consolidation should be Some.
        let status = mm.status().await.unwrap();
        assert!(
            status.last_consolidation.is_some(),
            "last_consolidation should be set after consolidate()"
        );
    }

    #[tokio::test]
    async fn old_json_with_embedding_field_still_deserializes() {
        // After removing the `embedding` field, old memory JSON files that
        // contain `"embedding": null` must still deserialize correctly
        // (serde ignores unknown fields by default).
        let old_json = r#"{
            "id": "legacy-1",
            "memory_type": "Knowledge",
            "content": "legacy memory with embedding field",
            "timestamp": "2024-01-01T00:00:00Z",
            "importance": 0.5,
            "tags": [],
            "metadata": {},
            "embedding": null
        }"#;

        let entry: MemoryEntry = serde_json::from_str(old_json).unwrap();
        assert_eq!(entry.id, "legacy-1");
        assert_eq!(entry.content, "legacy memory with embedding field");
    }

    #[tokio::test]
    async fn with_settings_reads_consolidation_thresholds() {
        use crate::config::{MemorySettings, Settings};

        let mut settings = Settings::default();
        settings.storage.memory = MemorySettings {
            enabled: true,
            path: std::path::PathBuf::from("/tmp/memory.json"),
            consolidation_interval: 48,
            max_memories: 5000,
            importance_threshold: 0.7,
            age_threshold_hours: 12,
            enable_auto_consolidation: false,
            recall_top_n: 5,
            recall_similarity_threshold: 0.3,
            write_importance_threshold: 0.65,
            max_extract_per_compaction: 2,
            exploration_epsilon: 0.15,
            staleness_check: false,
            staleness_penalty: 0.25,
        };

        let mm = MemoryManager::with_settings(&settings, std::path::PathBuf::from("/tmp"));
        let engine = mm.consolidation();
        let config = engine.config();

        assert_eq!(config.max_memories, 5000);
        assert!((config.importance_threshold - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.age_threshold_hours, 12);
        assert_eq!(config.consolidation_interval_hours, 48);
        assert!(!config.enable_auto_consolidation);
        assert!((mm.write_importance_threshold() - 0.65).abs() < f32::EPSILON);
        assert_eq!(mm.max_extract_per_compaction(), 2);
        assert!((mm.exploration_epsilon() - 0.15).abs() < f32::EPSILON);
        assert!(!mm.staleness_check());
        assert!((mm.staleness_penalty() - 0.25).abs() < f32::EPSILON);
        let cfg = mm.effective_importance_cfg();
        assert_eq!(cfg.age_threshold_hours, 12);
        assert!((cfg.staleness_penalty - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn new_for_test_uses_exploration_and_staleness_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));
        assert!((mm.exploration_epsilon() - 0.0).abs() < f32::EPSILON);
        assert!(mm.staleness_check());
        assert!((mm.staleness_penalty() - 0.5).abs() < f32::EPSILON);
        let cfg = mm.effective_importance_cfg();
        assert_eq!(cfg.age_threshold_hours, 48);
        assert!((cfg.staleness_penalty - 0.5).abs() < f32::EPSILON);
    }

    /// Regression test: `MemoryManager::load()` must recover persisted
    /// sessions AND history from disk, not just memories. Previously `load()`
    /// only loaded memories from storage, leaving the in-memory session
    /// HashMap and history VecDeque empty after a restart - making all
    /// historical sessions/history invisible even though the files existed.
    #[tokio::test]
    async fn load_recovers_sessions_and_history_from_disk() {
        use crate::context::history::HistoryType;

        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let history_path = tmp.path().join("history.jsonl");
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();

        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::with_dir(sessions_dir.clone())),
            history: Arc::new(HistoryManager::with_path(history_path.clone())),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // Pre-populate a session and a history entry on disk.
        let session = MemorySession::new(Some("regression-session"));
        let session_id = session.id.clone();
        mm.sessions().save(&session).await.unwrap();
        mm.history()
            .add(HistoryEntry::new(HistoryType::Command, "regression-cmd"))
            .await
            .unwrap();

        // Simulate a restart: a fresh manager pointed at the same dirs.
        let restarted = MemoryManager {
            sessions: Arc::new(MemorySessionManager::with_dir(sessions_dir)),
            history: Arc::new(HistoryManager::with_path(history_path)),
            project_storage: storage,
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // Before load(), the in-memory caches are empty.
        assert!(
            restarted.sessions().list().await.unwrap().is_empty(),
            "fresh manager should have no sessions in memory before load()"
        );
        assert!(
            restarted.history().get_recent(10).await.is_empty(),
            "fresh manager should have no history in memory before load()"
        );

        // load() recovers sessions and history alongside memories.
        restarted.load().await.unwrap();

        let sessions = restarted.sessions().list().await.unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "load() should recover the persisted session"
        );
        assert_eq!(sessions[0].id, session_id);

        let recent = restarted.history().get_recent(10).await;
        assert_eq!(
            recent.len(),
            1,
            "load() should recover the persisted history entry"
        );
        assert_eq!(recent[0].content, "regression-cmd");
    }

    /// Dedup guard: adding a near-duplicate memory (same fact, different type
    /// and terser wording, as happens across compaction rounds) must fold into
    /// the existing entry instead of creating a new file.
    #[tokio::test]
    async fn add_memory_merges_near_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // First extraction: a decision captured during compaction.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Decision, "use JWT for authentication"),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();

        // Second extraction (later compaction round): same fact, different
        // type and terser wording. Without the dedup guard this would create a
        // second file; with it, the existing entry is merged in place.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "use JWT"),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(
            memories.len(),
            1,
            "near-duplicate should merge into the existing entry, not append"
        );
        // Merged content keeps the richer (existing) text.
        assert!(memories[0].content.contains("use JWT for authentication"));
        // The existing id is preserved (file overwritten, no orphan).
        let id = memories[0].id.clone();
        drop(memories);

        // Exactly one memory file on disk.
        let mut files = 0;
        let mut dir = tokio::fs::read_dir(mm.project_storage.path())
            .await
            .unwrap();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            if entry.path().extension().is_some_and(|e| e == "json") {
                files += 1;
            }
        }
        assert_eq!(
            files, 1,
            "only the existing memory file should exist on disk"
        );
        // The persisted file still carries the existing id.
        let on_disk: MemoryEntry = serde_json::from_str(
            &tokio::fs::read_to_string(mm.project_storage.path().join(format!("{id}.json")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(on_disk.id, id);
    }

    #[tokio::test]
    async fn add_memory_returns_result_for_new_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        let entry = MemoryEntry::new(MemoryType::Knowledge, "a brand new fact");
        let expected_id = entry.id.clone();
        let result = mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();

        assert!(!result.merged, "new entry should not be merged");
        assert_eq!(
            result.id, expected_id,
            "returned id should match the new entry's id"
        );
    }

    #[tokio::test]
    async fn add_memory_returns_result_for_merged_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // First entry.
        let existing = MemoryEntry::new(MemoryType::Decision, "use JWT for authentication");
        let existing_id = existing.id.clone();
        mm.add_memory(existing, MemoryOrigin::Project)
            .await
            .unwrap();

        // Similar entry triggers dedup merge.
        let similar = MemoryEntry::new(MemoryType::Knowledge, "use JWT");
        let result = mm.add_memory(similar, MemoryOrigin::Project).await.unwrap();

        assert!(result.merged, "similar entry should be merged");
        assert_eq!(
            result.id, existing_id,
            "returned id should be the existing entry's id, not the new one"
        );
    }

    /// After `consolidate()` drops a stale low-importance entry, the TF-IDF
    /// index must be rebuilt so a surviving memory that shifted position is
    /// still searchable. Previously the index kept stale positional postings
    /// and the survivor would silently disappear from recall results.
    #[tokio::test]
    async fn consolidate_rebuilds_index_so_search_resolves_survivors() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            project_storage: storage.clone(),
            global_storage: Arc::new(crate::context::Storage::new(
                tmp.path().join("global_memory"),
            )),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: true,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        // idx 0: survives consolidation (Knowledge is always kept).
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "alpha beta keepsake"),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        // idx 1: low-importance, old Session memory -> dropped by consolidate
        // (age from decay anchor > age_threshold_hours, effective < threshold).
        // Prefill last_reinforced_at so first-consolidate anchor migration does
        // not reset the age clock to "now".
        let mut stale =
            MemoryEntry::new(MemoryType::Session, "gamma delta transient").with_importance(0.1);
        stale.timestamp = chrono::Utc::now() - chrono::Duration::hours(100);
        stale.last_reinforced_at = Some(stale.timestamp);
        mm.add_memory(stale, MemoryOrigin::Project).await.unwrap();
        // idx 2: survives, but shifts to idx 1 after the stale entry is
        // dropped. Its distinctive token "unobtainium" lets us search for it
        // precisely.
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "unobtainium alpha rare"),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();

        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(
            memories.len(),
            2,
            "stale low-importance Session memory should be dropped"
        );
        drop(memories);

        // Search for the distinctive token of the shifted survivor. Before the
        // index-rebuild fix the TF-IDF posting still pointed at the
        // pre-consolidation idx 2, which is now out of range, so the memory
        // would be silently missing from results.
        let found = mm.search_memories("unobtainium").await;
        assert!(
            found.iter().any(|m| m.content.contains("unobtainium")),
            "survivor that shifted index after consolidation must still be searchable"
        );
    }

    #[tokio::test]
    async fn list_memories_orders_by_effective_not_raw() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let mut high_raw_old =
            MemoryEntry::new(MemoryType::Knowledge, "high-raw-old fact").with_importance(0.95);
        high_raw_old.timestamp = Utc::now() - chrono::Duration::hours(800);
        high_raw_old.last_reinforced_at = Some(high_raw_old.timestamp);

        let lower_raw_fresh =
            MemoryEntry::new(MemoryType::Knowledge, "lower-raw-fresh fact").with_importance(0.6);

        mm.add_memory(high_raw_old, MemoryOrigin::Project)
            .await
            .unwrap();
        mm.add_memory(lower_raw_fresh, MemoryOrigin::Project)
            .await
            .unwrap();

        let listed = mm.list_memories(None, 0).await;
        assert_eq!(listed.len(), 2);
        assert!(
            listed[0].1.content.contains("lower-raw-fresh"),
            "fresh lower raw should rank first by effective: {:?}",
            listed
                .iter()
                .map(|(_, m)| m.content.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            listed[1].1.content.contains("high-raw-old"),
            "decayed high raw should rank second"
        );
    }

    #[tokio::test]
    async fn list_memories_keeps_superseded_but_filters_by_effective() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let mut live =
            MemoryEntry::new(MemoryType::Knowledge, "live listable").with_importance(0.8);
        live.id = "live-list".into();

        let mut tomb =
            MemoryEntry::new(MemoryType::Knowledge, "tomb listable").with_importance(0.99);
        tomb.id = "tomb-list".into();
        tomb.superseded_by = Some("live-list".into());

        mm.add_memory(live, MemoryOrigin::Project).await.unwrap();
        mm.add_memory(tomb, MemoryOrigin::Project).await.unwrap();

        // No min filter: superseded remains listable for audit.
        let all = mm.list_memories(None, 0).await;
        assert_eq!(all.len(), 2);
        assert!(
            all.iter().any(|(_, m)| m.content.contains("tomb listable")),
            "superseded rows remain listable"
        );
        // Live (effective ~0.8) first; tomb (effective 0) last.
        assert!(all[0].1.content.contains("live listable"));
        assert!(all[1].1.content.contains("tomb listable"));

        // min_importance compares against effective → tomb (0) filtered out.
        let filtered = mm.list_memories(Some(0.1), 0).await;
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].1.content.contains("live listable"));
        assert!(
            !filtered
                .iter()
                .any(|(_, m)| m.content.contains("tomb listable")),
            "superseded effective 0 must fail min_importance filter"
        );
    }

    /// Contradicts path: old content is tombstoned (file retained), new stands
    /// alone; recall must not surface the superseded wording.
    #[tokio::test]
    async fn add_memory_contradicts_supersedes_and_recall_excludes_old() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        // Phrases share enough meaningful tokens for Jaccard >= 0.6 (gate), plus a
        // state-change marker so classify_relation → Contradicts. Short gold
        // pairs like "auth bug exists"/"auth bug fixed" are covered at the
        // classify_relation unit level (their Jaccard is only 0.5).
        let existing = MemoryEntry::new(
            MemoryType::Knowledge,
            "the auth module login bug exists in codebase today",
        )
        .with_importance(0.8);
        let old_id = existing.id.clone();
        let old_importance = existing.importance;
        mm.add_memory(existing, MemoryOrigin::Project)
            .await
            .unwrap();

        let incoming = MemoryEntry::new(
            MemoryType::Knowledge,
            "the auth module login bug fixed in codebase today",
        )
        .with_importance(0.7);
        let new_id = incoming.id.clone();
        let result = mm
            .add_memory(incoming, MemoryOrigin::Project)
            .await
            .unwrap();

        assert!(!result.merged, "contradicts must not report merge");
        assert_eq!(result.id, new_id, "memory_id must be the new standalone id");

        let memories = mm.memories.read().await;
        assert_eq!(
            memories.len(),
            2,
            "both tombstone and new entry stay in pool"
        );
        let old = memories
            .iter()
            .find(|m| m.id == old_id)
            .expect("old retained");
        assert_eq!(old.superseded_by.as_deref(), Some(new_id.as_str()));
        assert!(
            (old.importance - old_importance).abs() < f32::EPSILON,
            "Contradicts must not change base importance"
        );
        assert!(
            memories
                .iter()
                .any(|m| m.id == new_id && m.superseded_by.is_none()),
            "new entry is live"
        );
        drop(memories);

        // Superseded JSON file remains on disk (audit).
        let old_path = mm.project_storage.path().join(format!("{old_id}.json"));
        assert!(
            tokio::fs::try_exists(&old_path).await.unwrap(),
            "tombstone file must be retained on disk"
        );
        let on_disk: MemoryEntry =
            serde_json::from_str(&tokio::fs::read_to_string(&old_path).await.unwrap()).unwrap();
        assert_eq!(on_disk.superseded_by.as_deref(), Some(new_id.as_str()));

        let recall = crate::context::inject::MemoryContextInjector::recall(
            "auth module login bug codebase",
            &mm,
            5,
            0.0,
            None,
        )
        .await;
        assert!(
            recall.contains("bug fixed"),
            "new content should be recallable: {recall}"
        );
        assert!(
            !recall.contains("bug exists"),
            "superseded old content must be excluded from recall: {recall}"
        );
    }

    #[tokio::test]
    async fn add_memory_compatible_merges_and_reinforces() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let existing =
            MemoryEntry::new(MemoryType::Knowledge, "use jwt authentication").with_importance(0.6);
        let existing_id = existing.id.clone();
        mm.add_memory(existing, MemoryOrigin::Project)
            .await
            .unwrap();

        let result = mm
            .add_memory(
                MemoryEntry::new(MemoryType::Knowledge, "use jwt"),
                MemoryOrigin::Project,
            )
            .await
            .unwrap();

        assert!(result.merged);
        assert_eq!(result.id, existing_id);

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].id, existing_id);
        assert_eq!(memories[0].hit_count, 1, "Compatible must reinforce");
        assert!(memories[0].last_reinforced_at.is_some());
        assert!(memories[0].content.contains("use jwt authentication"));
    }

    #[tokio::test]
    async fn add_memory_ambiguous_merges_and_flags_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let existing = MemoryEntry::new(
            MemoryType::Preference,
            "prefer postgres database for storage layer",
        );
        let existing_id = existing.id.clone();
        mm.add_memory(existing, MemoryOrigin::Project)
            .await
            .unwrap();

        let result = mm
            .add_memory(
                MemoryEntry::new(
                    MemoryType::Preference,
                    "prefer mysql database for storage layer",
                ),
                MemoryOrigin::Project,
            )
            .await
            .unwrap();

        assert!(result.merged);
        assert_eq!(result.id, existing_id);

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert_eq!(
            memories[0]
                .metadata
                .get("relation_ambiguous")
                .and_then(|v| v.as_bool()),
            Some(true),
            "Ambiguous must flag metadata without LLM"
        );
        assert_eq!(memories[0].hit_count, 0, "Ambiguous must not reinforce");
        assert!(
            memories[0].content.contains("postgres") && memories[0].content.contains("mysql"),
            "both alternatives retained in merged content"
        );
    }

    #[tokio::test]
    async fn add_memory_skips_superseded_as_merge_target() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let mut tomb = MemoryEntry::new(MemoryType::Knowledge, "use jwt authentication legacy")
            .with_importance(0.9);
        tomb.id = "tomb-jwt".into();
        tomb.superseded_by = Some("someone-else".into());
        mm.add_memory(tomb, MemoryOrigin::Project).await.unwrap();

        // Only similar live candidate is the tombstoned one → should NOT merge into it.
        let incoming = MemoryEntry::new(MemoryType::Knowledge, "use jwt authentication");
        let new_id = incoming.id.clone();
        let result = mm
            .add_memory(incoming, MemoryOrigin::Project)
            .await
            .unwrap();

        assert!(!result.merged);
        assert_eq!(result.id, new_id);
        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 2);
        assert!(
            memories
                .iter()
                .any(|m| m.id == "tomb-jwt" && m.superseded_by.is_some()),
            "tombstone unchanged"
        );
        assert!(
            memories
                .iter()
                .any(|m| m.id == new_id && m.superseded_by.is_none()),
            "new live entry inserted"
        );
    }

    #[tokio::test]
    async fn consolidate_anchors_missing_last_reinforced_at() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let entry = MemoryEntry::new(MemoryType::Knowledge, "legacy fact without anchor")
            .with_importance(0.9);
        assert!(entry.last_reinforced_at.is_none());
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert!(
            memories[0].last_reinforced_at.is_some(),
            "first consolidate must anchor last_reinforced_at"
        );
    }

    #[tokio::test]
    async fn consolidate_marks_stale_when_all_extracted_paths_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "logic lives in src/does_not_exist_xyz.rs only",
        )
        .with_importance(0.9);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert!(
            memories[0].stale_marked_at.is_some(),
            "all-missing extractable paths must mark stale"
        );
        // Base importance never multiplied by staleness.
        assert!((memories[0].importance - 0.9).abs() < 1e-5);
        // Effective importance must apply staleness_penalty.
        let cfg = mm.effective_importance_cfg();
        let now = Utc::now();
        let eff = memories[0].effective_importance(now, &cfg);
        let unstale = {
            let mut clone = memories[0].clone();
            clone.stale_marked_at = None;
            clone.effective_importance(now, &cfg)
        };
        assert!(
            (eff - unstale * cfg.staleness_penalty).abs() < 1e-5,
            "effective after mark must be downweighted by staleness_penalty: eff={eff} unstale={unstale}"
        );
    }

    #[tokio::test]
    async fn consolidate_partial_missing_paths_does_not_mark_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Create one real path under the project root.
        let existing = root.join("src/exists_partial.rs");
        tokio::fs::create_dir_all(existing.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&existing, b"fn ok() {}").await.unwrap();

        let mm = MemoryManager::new_for_test(root.to_path_buf(), root.join("global"));
        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "see src/exists_partial.rs and src/does_not_exist_partial.rs",
        )
        .with_importance(0.9);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert!(
            memories[0].stale_marked_at.is_none(),
            "partial missing must NOT mark stale"
        );
    }

    #[tokio::test]
    async fn consolidate_stale_mark_is_idempotent_and_keeps_base_importance() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "only src/does_not_exist_idem.rs remains",
        )
        .with_importance(0.85);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();
        let (first_mark, first_anchor) = {
            let memories = mm.memories.read().await;
            assert_eq!(memories.len(), 1);
            let mark = memories[0].stale_marked_at;
            assert!(mark.is_some());
            assert!((memories[0].importance - 0.85).abs() < 1e-5);
            let anchor = memories[0].last_reinforced_at;
            assert!(anchor.is_some());
            (mark, anchor)
        };

        // Second pass: no-op on mark/anchor, base importance unchanged.
        mm.consolidate().await.unwrap();
        let memories = mm.memories.read().await;
        assert_eq!(memories[0].stale_marked_at, first_mark);
        assert_eq!(
            memories[0].last_reinforced_at, first_anchor,
            "second consolidate must leave existing last_reinforced_at unchanged"
        );
        assert!((memories[0].importance - 0.85).abs() < 1e-5);
    }

    #[tokio::test]
    async fn consolidate_staleness_check_false_skips_mark() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join(".wgenty-code/memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::with_project_root(
                tmp.path().to_path_buf(),
            )),
            history: Arc::new(HistoryManager::new()),
            project_storage: Arc::new(crate::context::Storage::new(memory_dir)),
            global_storage: Arc::new(crate::context::Storage::new(tmp.path().join("global"))),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            global_memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
            write_importance_threshold: 0.6,
            max_extract_per_compaction: 3,
            exploration_epsilon: 0.0,
            staleness_check: false,
            staleness_penalty: 0.5,
            age_threshold_hours: 48,
            project_root: tmp.path().to_path_buf(),
            recently_explored: Arc::new(RwLock::new(HashSet::new())),
        };

        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "points at src/does_not_exist_gated.rs",
        )
        .with_importance(0.9);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();
        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert!(
            memories[0].stale_marked_at.is_none(),
            "staleness_check=false must not mark"
        );
        // Anchor migration still runs.
        assert!(memories[0].last_reinforced_at.is_some());
    }

    #[tokio::test]
    async fn consolidate_remains_llm_free_structural() {
        // consolidate path must not require an LLM client — pure local prepass
        // + ConsolidationEngine (TF-IDF/TTL). If this compiles and runs, the
        // surface stays LLM-free.
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));
        mm.add_memory(
            MemoryEntry::new(MemoryType::Knowledge, "no paths here").with_importance(0.9),
            MemoryOrigin::Project,
        )
        .await
        .unwrap();
        mm.consolidate().await.unwrap();
        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert!(memories[0].stale_marked_at.is_none());
        assert!(memories[0].last_reinforced_at.is_some());
    }

    #[tokio::test]
    async fn consolidate_existing_only_paths_does_not_mark_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let existing_a = root.join("src/exists_a.rs");
        let existing_b = root.join("lib/exists_b.ts");
        tokio::fs::create_dir_all(existing_a.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::create_dir_all(existing_b.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&existing_a, b"fn a() {}").await.unwrap();
        tokio::fs::write(&existing_b, b"export const b = 1;")
            .await
            .unwrap();

        let mm = MemoryManager::new_for_test(root.to_path_buf(), root.join("global"));
        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "see src/exists_a.rs and lib/exists_b.ts",
        )
        .with_importance(0.9);
        mm.add_memory(entry, MemoryOrigin::Project).await.unwrap();
        mm.consolidate().await.unwrap();

        let memories = mm.memories.read().await;
        assert_eq!(memories.len(), 1);
        assert!(
            memories[0].stale_marked_at.is_none(),
            "all-existing extractable paths must NOT mark stale"
        );
    }

    #[tokio::test]
    async fn prune_global_pool_does_not_path_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mm = MemoryManager::new_for_test(root.to_path_buf(), root.join("global"));

        // Global memory points at a missing project-relative path. Global
        // prepass must still anchor but must NOT path-stale.
        let entry = MemoryEntry::new(
            MemoryType::Knowledge,
            "global tip about src/does_not_exist_global_only.rs",
        )
        .with_importance(0.9);
        mm.add_memory(entry, MemoryOrigin::Global).await.unwrap();

        mm.prune().await.unwrap();

        let global = mm.global_memories.read().await;
        assert_eq!(global.len(), 1);
        assert!(
            global[0].last_reinforced_at.is_some(),
            "global prepass still anchors"
        );
        assert!(
            global[0].stale_marked_at.is_none(),
            "global pool must not run path-staleness against project root"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Reinforcement anchors decay — reinforced entries decay less
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn reinforcement_anchors_decay_from_last_reinforce_not_creation() {
        let now = Utc::now();
        let created = now - chrono::Duration::hours(100);

        // Entry 1: last reinforced 100h ago (creation time)
        let mut old = MemoryEntry::new(MemoryType::Knowledge, "old reinforce").with_importance(0.8);
        old.timestamp = created;
        old.last_reinforced_at = Some(created);
        old.hit_count = 10;
        old.recall_count = 20;

        // Entry 2: last reinforced 10h ago (same creation, recent reinforcement)
        let mut recent =
            MemoryEntry::new(MemoryType::Knowledge, "recent reinforce").with_importance(0.8);
        recent.timestamp = created;
        recent.last_reinforced_at = Some(now - chrono::Duration::hours(10));
        recent.hit_count = 10;
        recent.recall_count = 20;

        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };

        let old_eff = old.effective_importance(now, &cfg);
        let recent_eff = recent.effective_importance(now, &cfg);

        // Recent reinforcement → decay from 10h, NOT 100h
        assert!(
            recent_eff > old_eff,
            "recent={recent_eff} should outrank old={old_eff}"
        );
    }

    #[test]
    fn recent_reinforcement_can_surpass_older_higher_base_importance() {
        let now = Utc::now();

        // High base importance (0.9) but never reinforced in 200h
        let mut ancient_high =
            MemoryEntry::new(MemoryType::Knowledge, "ancient 0.9").with_importance(0.9);
        ancient_high.timestamp = now - chrono::Duration::hours(200);
        ancient_high.last_reinforced_at = Some(now - chrono::Duration::hours(200));

        // Lower base importance (0.7) but recently reinforced (5h ago)
        let mut recent_low =
            MemoryEntry::new(MemoryType::Knowledge, "recent 0.7").with_importance(0.7);
        recent_low.timestamp = now - chrono::Duration::hours(5);
        recent_low.last_reinforced_at = Some(now - chrono::Duration::hours(5));

        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };

        let ancient_eff = ancient_high.effective_importance(now, &cfg);
        let recent_eff = recent_low.effective_importance(now, &cfg);
        assert!(
            recent_eff > ancient_eff,
            "recent 0.7 (eff={recent_eff}) should outrank ancient 0.9 (eff={ancient_eff})"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Effective importance: staleness × decay × hitrate combined
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn effective_importance_stale_and_old_compounds_downward() {
        let now = Utc::now();

        let mut fresh = MemoryEntry::new(MemoryType::Knowledge, "fresh").with_importance(0.8);
        fresh.timestamp = now - chrono::Duration::hours(10);
        fresh.last_reinforced_at = Some(now - chrono::Duration::hours(10));

        // Stale + 300h old → massive decay + staleness penalty
        let mut stale_old =
            MemoryEntry::new(MemoryType::Knowledge, "stale and old").with_importance(0.8);
        stale_old.timestamp = now - chrono::Duration::hours(300);
        stale_old.last_reinforced_at = Some(now - chrono::Duration::hours(300));
        stale_old.stale_marked_at = Some(now - chrono::Duration::hours(100));

        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };

        let fresh_eff = fresh.effective_importance(now, &cfg);
        let stale_eff = stale_old.effective_importance(now, &cfg);

        // stale + old should be dramatically lower than fresh
        assert!(
            stale_eff < fresh_eff * 0.4,
            "stale+old eff={stale_eff} should be << fresh eff={fresh_eff}"
        );
    }

    #[test]
    fn effective_importance_superseded_always_zero() {
        let now = Utc::now();
        let mut mem = MemoryEntry::new(MemoryType::Knowledge, "superseded").with_importance(1.0);
        mem.timestamp = now;
        mem.last_reinforced_at = Some(now);
        mem.hit_count = 100;
        mem.recall_count = 100;
        mem.superseded_by = Some("newer-version".to_string());

        let cfg = EffectiveImportanceCfg {
            age_threshold_hours: 48,
            staleness_penalty: 0.5,
        };
        assert_eq!(
            mem.effective_importance(now, &cfg),
            0.0,
            "superseded entries must have effective_importance = 0"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  search_memories scope separation (project only, not global)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn search_memories_excludes_global_pool() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let proj = MemoryEntry::new(MemoryType::Knowledge, "project specific git workflow")
            .with_importance(0.9);
        mm.add_memory(proj, MemoryOrigin::Project).await.unwrap();

        let glob = MemoryEntry::new(MemoryType::Preference, "user prefers dark theme")
            .with_importance(0.9);
        mm.add_memory(glob, MemoryOrigin::Global).await.unwrap();

        // Ensure index is rebuilt before searching
        mm.consolidate().await.unwrap();

        let results = mm.search_memories("git workflow").await;
        let has_project = results.iter().any(|r| r.content.contains("git workflow"));
        let has_global = results.iter().any(|r| r.content.contains("dark theme"));
        assert!(has_project, "project memory should be searchable");
        assert!(
            !has_global,
            "global memory should NOT appear in search results"
        );
    }

    #[tokio::test]
    async fn search_memories_respects_superseded_exclusion() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let v1 = MemoryEntry::new(MemoryType::Knowledge, "API base URL is v1.example.com")
            .with_importance(0.7);
        mm.add_memory(v1, MemoryOrigin::Project).await.unwrap();

        // Get v1's id to use as superseded_by reference on v2
        let v1_id = {
            let memories = mm.memories.read().await;
            memories.first().unwrap().id.clone()
        };

        let mut v2 = MemoryEntry::new(MemoryType::Knowledge, "API base URL is v2.example.com")
            .with_importance(0.8);
        v2.superseded_by = Some(v1_id);
        mm.add_memory(v2, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();

        let results = mm.search_memories("API base URL").await;
        // All results should be active (not superseded)
        let memories = mm.memories.read().await;
        for r in &results {
            let entry = memories.iter().find(|m| m.id == r.id).unwrap();
            assert!(
                entry.superseded_by.is_none(),
                "superseded entry id={} should not appear in search results",
                entry.id
            );
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  IDF weighting — common terms demoted in ranking
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn idf_weighting_demotes_common_terms_in_ranking() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        // Add 5 memories sharing the word "project" → high DF → low IDF
        for i in 0..5 {
            let text = format!("Project item {i} — shared-term project setup phase");
            let mem = MemoryEntry::new(MemoryType::Knowledge, &text).with_importance(0.8);
            mm.add_memory(mem, MemoryOrigin::Project).await.unwrap();
        }

        // One memory with a rare, distinctive term
        let rare = MemoryEntry::new(
            MemoryType::Knowledge,
            "Ziggurat architectural pattern is a rare Mesopotamian design term",
        )
        .with_importance(0.8);
        mm.add_memory(rare, MemoryOrigin::Project).await.unwrap();

        mm.consolidate().await.unwrap();

        let results = mm.search_memories("Ziggurat").await;
        assert!(!results.is_empty(), "should find at least the rare memory");
        let top = &results[0];
        assert!(
            top.content.contains("Ziggurat"),
            "rare term should rank highest, got: {}",
            &top.content
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Prune full lifecycle: aged low-effective memories removed
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn prune_removes_aged_low_importance_session_memories() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));
        let now = Utc::now();

        // Fresh + high importance → should survive
        let fresh_high = MemoryEntry::new(
            MemoryType::Knowledge,
            "Fresh high-importance knowledge about the codebase",
        )
        .with_importance(0.9);
        mm.add_memory(fresh_high, MemoryOrigin::Project)
            .await
            .unwrap();

        // Old + low importance + Session type → should be removed
        let mut old_session = MemoryEntry::new(
            MemoryType::Session,
            "Old session memory, low importance temporary note",
        )
        .with_importance(0.4);
        old_session.timestamp = now - chrono::Duration::hours(100);
        old_session.last_reinforced_at = Some(now - chrono::Duration::hours(100));
        mm.add_memory(old_session, MemoryOrigin::Project)
            .await
            .unwrap();

        mm.prune().await.unwrap();

        let mems = mm.memories.read().await;
        let contents: Vec<&str> = mems.iter().map(|m| m.content.as_str()).collect();
        assert!(
            contents.iter().any(|c| c.contains("Fresh high")),
            "fresh high-importance should survive prune"
        );
        assert!(
            !contents.iter().any(|c| c.contains("Old session")),
            "old low-importance session should be removed by prune"
        );
    }

    #[tokio::test]
    async fn prune_does_not_remove_global_memories_by_path_staleness() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        // Global memory — should not be checked for path staleness against project
        let mut global = MemoryEntry::new(
            MemoryType::Knowledge,
            "Global preference about project structure conventions",
        )
        .with_importance(0.7);
        global.timestamp = Utc::now() - chrono::Duration::hours(100);
        mm.add_memory(global, MemoryOrigin::Global).await.unwrap();

        mm.prune().await.unwrap();

        let mems = mm.global_memories.read().await;
        assert_eq!(mems.len(), 1, "global memory should survive prune");
        assert!(
            mems[0].stale_marked_at.is_none(),
            "global memory should not be marked stale by path-staleness check"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Supersession chain (A → B → C)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn supersession_chain_corrects_multiple_levels() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = MemoryManager::new_for_test(tmp.path().to_path_buf(), tmp.path().join("global"));

        let v1 =
            MemoryEntry::new(MemoryType::Knowledge, "Config port value: 100").with_importance(0.7);
        mm.add_memory(v1, MemoryOrigin::Project).await.unwrap();

        // Get v1's id
        let v1_id = {
            let mems = mm.memories.read().await;
            mems.first().unwrap().id.clone()
        };

        let mut v2 =
            MemoryEntry::new(MemoryType::Knowledge, "Config port value: 200").with_importance(0.8);
        v2.superseded_by = Some(v1_id.clone());
        mm.add_memory(v2, MemoryOrigin::Project).await.unwrap();

        // Get v2's id
        let v2_id = {
            let mems = mm.memories.read().await;
            mems.iter()
                .find(|m| m.content.contains("200"))
                .unwrap()
                .id
                .clone()
        };

        let mut v3 =
            MemoryEntry::new(MemoryType::Knowledge, "Config port value: 300").with_importance(0.9);
        v3.superseded_by = Some(v2_id);
        mm.add_memory(v3, MemoryOrigin::Project).await.unwrap();

        // Consolidate to remove tombstones
        mm.consolidate().await.unwrap();

        let mems = mm.memories.read().await;
        let active: Vec<&MemoryEntry> = mems.iter().filter(|m| m.superseded_by.is_none()).collect();

        // Only v1 (Config: 100) should survive as active
        assert_eq!(
            active.len(),
            1,
            "only one entry should survive the 3-level chain"
        );
        assert!(
            active[0].content.contains("100"),
            "v1 (config: 100) should be the sole survivor, got: {}",
            active[0].content
        );

        // v2 and v3 should be superseded (tombstones removed via consolidate)
        let superseded_count = mems.iter().filter(|m| m.superseded_by.is_some()).count();
        assert_eq!(superseded_count, 0, "tombstones should be fully cleaned up");
    }
}
