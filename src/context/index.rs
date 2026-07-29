//! In-memory TF-IDF inverted index for memory recall.
//!
//! Extracted from `mod.rs`. The index is `pub(super)` so only `MemoryManager`
//! (and context-internal tests) can construct and query it.

use std::collections::HashMap;

use crate::context::entry::MemoryEntry;
use crate::context::tokenizer;

/// In-memory inverted index with TF-IDF weighting for keyword search
/// over the memory corpus. Built lazily on `load()` and kept in sync
/// with `add_memory()` / `consolidate()`.
pub(super) struct MemoryIndex {
    /// word → [(entry_index, normalized_tf)]
    inverted: HashMap<String, Vec<(usize, f32)>>,
    /// word → inverse document frequency
    idf: HashMap<String, f32>,
    /// total number of indexed entries
    doc_count: usize,
    /// Tokenizer used to split content/query into indexable terms. Centralized
    /// here so CJK text is segmented consistently with similarity & keyword
    /// extraction (see [`crate::context::tokenizer`]).
    tokenizer: Box<dyn tokenizer::Tokenizer>,
}

impl MemoryIndex {
    pub(super) fn new() -> Self {
        Self {
            inverted: HashMap::new(),
            idf: HashMap::new(),
            doc_count: 0,
            tokenizer: tokenizer::build_tokenizer(),
        }
    }

    /// Rebuild the entire index from a slice of MemoryEntry.
    pub(super) fn rebuild(&mut self, entries: &[MemoryEntry]) {
        self.inverted.clear();
        self.idf.clear();
        self.doc_count = entries.len();

        if entries.is_empty() {
            return;
        }

        // Phase 1: count term frequencies per document.
        for (i, entry) in entries.iter().enumerate() {
            let mut tf_counts: HashMap<String, u32> = HashMap::new();
            for word in self.tokenizer.meaningful_tokens(&entry.content) {
                *tf_counts.entry(word).or_insert(0) += 1;
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

    /// Search the index for entries matching `query`. Returns a list of
    /// `(entry_index, score)` sorted by descending TF-IDF score, limited to
    /// `top_n`.
    pub(super) fn search(&self, query: &str, top_n: usize) -> Vec<(usize, f32)> {
        // Tokenize query via the same tokenizer used for indexing.
        let terms = self.tokenizer.meaningful_tokens(query);

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
    pub(super) fn add_entry(&mut self, entry: &MemoryEntry, idx: usize) {
        self.doc_count += 1;

        let mut tf_counts: HashMap<String, u32> = HashMap::new();
        for word in self.tokenizer.meaningful_tokens(&entry.content) {
            *tf_counts.entry(word).or_insert(0) += 1;
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
    pub(super) fn replace_entry(&mut self, entry: &MemoryEntry, idx: usize) {
        // Drop every posting that referenced the old content at `idx`.
        for postings in self.inverted.values_mut() {
            postings.retain(|(i, _)| *i != idx);
        }
        // Remove words that no longer have any postings.
        self.inverted.retain(|_, postings| !postings.is_empty());

        // Re-add postings from the new (merged) content.
        let mut tf_counts: HashMap<String, u32> = HashMap::new();
        for word in self.tokenizer.meaningful_tokens(&entry.content) {
            *tf_counts.entry(word).or_insert(0) += 1;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::entry::{MemoryEntry, MemoryType};

    fn index_with(entries: &[MemoryEntry]) -> MemoryIndex {
        let mut idx = MemoryIndex::new();
        idx.rebuild(entries);
        idx
    }

    #[test]
    fn memory_index_rebuild_and_search_basic_recall() {
        // Locks in current English TF-IDF behavior: a query term that appears
        // in one document but not others should surface that document first,
        // boosted by a high IDF (rare term).
        let entries = vec![
            MemoryEntry::new(MemoryType::Knowledge, "rust async programming patterns"),
            MemoryEntry::new(MemoryType::Knowledge, "python data science notebooks"),
            MemoryEntry::new(MemoryType::Knowledge, "rust ownership and borrowing"),
        ];
        let idx = index_with(&entries);

        let hits = idx.search("rust", 10);
        assert!(!hits.is_empty(), "rust should match two documents");
        // Both rust docs (index 0 and 2) should appear.
        let matched: Vec<usize> = hits.iter().map(|(i, _)| *i).collect();
        assert!(matched.contains(&0) && matched.contains(&2));
        // python doc (index 1) must not match "rust".
        assert!(!matched.contains(&1));
    }

    #[test]
    fn memory_index_search_empty_query_or_empty_index() {
        let entries = vec![MemoryEntry::new(MemoryType::Knowledge, "some content here")];
        let idx = index_with(&entries);

        assert!(idx.search("", 10).is_empty(), "empty query → no hits");
        assert!(
            idx.search("the a an", 10).is_empty(),
            "stop-word-only query → no hits"
        );

        let empty_idx = index_with(&[]);
        assert!(
            empty_idx.search("anything", 10).is_empty(),
            "empty index → no hits"
        );
    }

    #[test]
    fn memory_index_search_respects_top_n() {
        let entries: Vec<MemoryEntry> = (0..5)
            .map(|i| {
                MemoryEntry::new(
                    MemoryType::Knowledge,
                    &format!("shared token doc number {i}"),
                )
            })
            .collect();
        let idx = index_with(&entries);

        let hits = idx.search("shared", 3);
        assert_eq!(hits.len(), 3, "top_n=3 must cap results");
    }

    #[test]
    fn memory_index_add_entry_incremental() {
        // After rebuild, add_entry must keep the index consistent so a search
        // finds the newly added document.
        let mut idx = index_with(&[MemoryEntry::new(MemoryType::Knowledge, "existing document")]);
        assert!(idx.search("kubernetes", 10).is_empty());

        idx.add_entry(
            &MemoryEntry::new(MemoryType::Knowledge, "kubernetes deployment config"),
            1,
        );
        let hits = idx.search("kubernetes", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 1, "newly added entry at index 1 must match");
    }

    #[test]
    fn memory_index_replace_entry_after_merge() {
        // replace_entry is used by add_memory after an in-place merge; the
        // indexed content must reflect the merged text, not the old one.
        let mut idx = index_with(&[MemoryEntry::new(MemoryType::Knowledge, "old content here")]);

        // Old term matches before replace.
        assert!(!idx.search("old", 10).is_empty());
        assert!(idx.search("refreshed", 10).is_empty());

        idx.replace_entry(
            &MemoryEntry::new(MemoryType::Knowledge, "refreshed merged content"),
            0,
        );
        // After replace, the old term is gone and the new term matches.
        assert!(
            idx.search("old", 10).is_empty(),
            "stale posting for old content must be removed"
        );
        assert!(!idx.search("refreshed", 10).is_empty());
    }
}
