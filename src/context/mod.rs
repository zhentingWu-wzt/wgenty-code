//! Context Module — session persistence, context window management,
//! history tracking, memory storage, and 3-layer compression strategy.
//!
//! Corresponds to harness mechanisms s06+s07: context compression, session
//! persistence, and memory consolidation.

pub mod consolidation;
pub mod history;
pub mod inject;
pub mod memory_session;
mod session;
pub mod storage;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use anyhow::Context as _;

pub use consolidation::{ConsolidationConfig, ConsolidationEngine};
pub use history::{HistoryEntry, HistoryFilter, HistoryManager};
pub use memory_session::{
    Session as MemorySession, SessionInfo as MemorySessionInfo,
    SessionManager as MemorySessionManager,
};
pub use storage::{Storage, StorageBackend};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub importance: f32,
    pub tags: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStatus {
    pub total_memories: usize,
    pub session_count: usize,
    pub conversation_count: usize,
    pub knowledge_count: usize,
    pub last_consolidation: Option<DateTime<Utc>>,
    pub storage_size_bytes: u64,
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
    storage: Arc<Storage>,
    consolidation: Arc<ConsolidationEngine>,
    memories: Arc<RwLock<Vec<MemoryEntry>>>,
    index: Arc<RwLock<MemoryIndex>>,
    /// Guards `consolidate()` so concurrent `add_memory()` calls wait
    /// until consolidation completes before proceeding.
    consolidating: Arc<AtomicBool>,
}

impl MemoryManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let memory_path = home.join(".wgenty-code").join("memory");

        if let Err(e) = std::fs::create_dir_all(&memory_path) {
            tracing::warn!(
                path = %memory_path.display(),
                error = %e,
                "Failed to create memory directory; storage operations may fail later"
            );
        }

        Self {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            storage: Arc::new(Storage::new(memory_path)),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a MemoryManager configured from user settings.
    ///
    /// The consolidation thresholds (`max_memories`, `importance_threshold`,
    /// `age_threshold_hours`, etc.) are read from the `storage.memory` section
    /// of `settings.json`. Previously these were hardcoded in
    /// `ConsolidationConfig::default()` and could not be tuned by users.
    pub fn with_settings(settings: &crate::config::Settings) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let memory_path = home.join(".wgenty-code").join("memory");

        if let Err(e) = std::fs::create_dir_all(&memory_path) {
            tracing::warn!(
                path = %memory_path.display(),
                error = %e,
                "Failed to create memory directory; storage operations may fail later"
            );
        }

        let consolidation_config =
            ConsolidationConfig::from_memory_settings(&settings.storage.memory);

        Self {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            storage: Arc::new(Storage::new(memory_path)),
            consolidation: Arc::new(ConsolidationEngine::new(consolidation_config)),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn status(&self) -> anyhow::Result<MemoryStatus> {
        let memories = self.memories.read().await;
        let storage_size = self.storage.size().await.unwrap_or(0);

        Ok(MemoryStatus {
            total_memories: memories.len(),
            session_count: memories
                .iter()
                .filter(|m| m.memory_type == MemoryType::Session)
                .count(),
            conversation_count: memories
                .iter()
                .filter(|m| m.memory_type == MemoryType::Conversation)
                .count(),
            knowledge_count: memories
                .iter()
                .filter(|m| m.memory_type == MemoryType::Knowledge)
                .count(),
            last_consolidation: self.consolidation.last_consolidation().await,
            storage_size_bytes: storage_size,
        })
    }

    pub async fn add_memory(&self, entry: MemoryEntry) -> anyhow::Result<()> {
        // Wait if consolidation is in progress to avoid reading
        // transitional state. Use tokio::time::sleep polling so the
        // tokio runtime is not blocked.
        while self.consolidating.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut memories = self.memories.write().await;

        // Dedup guard: context compaction asks the model to extract memories
        // from the conversation being summarized, so the same fact is often
        // re-extracted across rounds (and may even be tagged with a different
        // `MemoryType` each time). `add_memory` previously appended a fresh
        // entry every time, accumulating near-duplicate files that only
        // consolidation could merge. Instead, fold a sufficiently similar
        // existing entry into the incoming one (type-agnostic, relaxed
        // threshold) and persist under the existing id, leaving no orphan.
        const DEDUP_THRESHOLD: f32 = 0.6;
        if let Some(existing_idx) =
            self.consolidation
                .find_similar(&entry, &memories, DEDUP_THRESHOLD, false)
        {
            let merged = ConsolidationEngine::merge_into(&memories[existing_idx], &entry);
            // Persist under the existing id so the original file is overwritten
            // and no duplicate file is created.
            self.storage.save_memory(&merged).await?;
            memories[existing_idx] = merged.clone();
            // Keep the TF-IDF index in sync with the merged content. The
            // positional idx is unchanged, but the token set changed, so the
            // postings for this idx must be replaced.
            self.index
                .write()
                .await
                .replace_entry(&merged, existing_idx);
            return Ok(());
        }

        let idx = memories.len();
        memories.push(entry.clone());
        self.storage.save_memory(&entry).await?;
        // Incrementally update the index for the new entry.
        self.index.write().await.add_entry(&entry, idx);
        Ok(())
    }

    pub async fn get_memory(&self, id: &str) -> Option<MemoryEntry> {
        let memories = self.memories.read().await;
        memories.iter().find(|m| m.id == id).cloned()
    }

    pub async fn search_memories(&self, query: &str) -> Vec<MemoryEntry> {
        // Try TF-IDF index first. Falls back to substring scan if the index
        // is empty (e.g., before load() was called).
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
        memories
            .iter()
            .filter(|m| m.memory_type == memory_type)
            .cloned()
            .collect()
    }

    pub async fn get_important_memories(&self, threshold: f32) -> Vec<MemoryEntry> {
        let memories = self.memories.read().await;
        memories
            .iter()
            .filter(|m| m.importance >= threshold)
            .cloned()
            .collect()
    }

    pub async fn clear(&self) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        memories.clear();
        self.storage.clear().await?;
        Ok(())
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
            self.storage.save_memory(entry).await?;
            memories.push(entry.clone());
        }

        Ok(())
    }

    pub async fn consolidate(&self) -> anyhow::Result<()> {
        // Acquire a cross-process advisory lock so that two concurrent
        // `wgenty-code memory dream` invocations (each with its own
        // MemoryManager instance) do not race on the same memory directory.
        // The in-process RwLock only protects within a single process.
        let _guard = ConsolidationFileLock::acquire(&self.storage)
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
        let consolidated = self.consolidation.consolidate(&memories).await?;

        // P0 fix: persist the consolidated result AND remove orphaned
        // on-disk files in one atomic-ish step. Previously only the
        // in-memory Vec was replaced and `save()` (via `save_all()`)
        // wrote new files without deleting the old ones — causing
        // "consolidated away" memories to be resurrected on the next
        // `load_all()`.
        self.storage.reconcile(&consolidated).await?;
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
        let memories = self.storage.load_all().await?;
        // Rebuild the TF-IDF index from the loaded entries so that
        // search_memories() can use it immediately.
        self.index.write().await.rebuild(&memories);
        let mut mem = self.memories.write().await;
        *mem = memories;

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
        self.storage.save_all(&memories).await
    }

    pub fn sessions(&self) -> Arc<MemorySessionManager> {
        self.sessions.clone()
    }
    pub fn history(&self) -> Arc<HistoryManager> {
        self.history.clone()
    }
    pub fn storage(&self) -> Arc<Storage> {
        self.storage.clone()
    }
    pub fn consolidation(&self) -> Arc<ConsolidationEngine> {
        self.consolidation.clone()
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
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

    #[tokio::test]
    async fn import_skips_duplicate_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let memory_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&memory_dir).await.unwrap();
        let storage = Arc::new(crate::context::Storage::new(memory_dir));
        let mm = MemoryManager {
            sessions: Arc::new(MemorySessionManager::new()),
            history: Arc::new(HistoryManager::new()),
            storage: storage.clone(),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        };

        // Pre-populate with one memory.
        let existing = MemoryEntry::new(MemoryType::Knowledge, "existing");
        let existing_id = existing.id.clone();
        mm.add_memory(existing).await.unwrap();

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
            storage: storage.clone(),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        };

        // Before consolidation, last_consolidation should be None.
        let status = mm.status().await.unwrap();
        assert!(status.last_consolidation.is_none());

        // Add a memory and consolidate.
        mm.add_memory(MemoryEntry::new(MemoryType::Knowledge, "test").with_importance(0.8))
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
        };

        let mm = MemoryManager::with_settings(&settings);
        let engine = mm.consolidation();
        let config = engine.config();

        assert_eq!(config.max_memories, 5000);
        assert!((config.importance_threshold - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.age_threshold_hours, 12);
        assert_eq!(config.consolidation_interval_hours, 48);
        assert!(!config.enable_auto_consolidation);
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
            storage: storage.clone(),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
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
            storage,
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
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
            storage: storage.clone(),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        };

        // First extraction: a decision captured during compaction.
        mm.add_memory(MemoryEntry::new(
            MemoryType::Decision,
            "use JWT for authentication",
        ))
        .await
        .unwrap();

        // Second extraction (later compaction round): same fact, different
        // type and terser wording. Without the dedup guard this would create a
        // second file; with it, the existing entry is merged in place.
        mm.add_memory(MemoryEntry::new(MemoryType::Knowledge, "use JWT"))
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
        let mut dir = tokio::fs::read_dir(mm.storage.path()).await.unwrap();
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
            &tokio::fs::read_to_string(mm.storage.path().join(format!("{id}.json")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(on_disk.id, id);
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
            storage: storage.clone(),
            consolidation: Arc::new(ConsolidationEngine::new(Default::default())),
            memories: Arc::new(RwLock::new(Vec::new())),
            index: Arc::new(RwLock::new(MemoryIndex::new())),
            consolidating: Arc::new(AtomicBool::new(false)),
        };

        // idx 0: survives consolidation (Knowledge is always kept).
        mm.add_memory(MemoryEntry::new(
            MemoryType::Knowledge,
            "alpha beta keepsake",
        ))
        .await
        .unwrap();
        // idx 1: low-importance, old Session memory -> dropped by consolidate
        // (age > age_threshold_hours=24, importance < 0.3).
        let mut stale =
            MemoryEntry::new(MemoryType::Session, "gamma delta transient").with_importance(0.1);
        stale.timestamp = chrono::Utc::now() - chrono::Duration::hours(100);
        mm.add_memory(stale).await.unwrap();
        // idx 2: survives, but shifts to idx 1 after the stale entry is
        // dropped. Its distinctive token "unobtainium" lets us search for it
        // precisely.
        mm.add_memory(MemoryEntry::new(
            MemoryType::Knowledge,
            "unobtainium alpha rare",
        ))
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
}
