---
comet_change: memory-tfidf-recall
role: technical-design
canonical_spec: openspec
---

# Design: Memory TF-IDF Retrieval Pipeline

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                      AFTER (per-turn recall)                      │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  MemoryManager::load()                                            │
│  ├── storage.load_all() → Vec<MemoryEntry>                        │
│  └── MemoryIndex::rebuild(&memories)                              │
│                                                                   │
│  ┌─ Session Startup ────────────────────────────────────────────┐ │
│  │ 1. cwd basename → keywords                                    │ │
│  │ 2. memory_manager.search(keywords) → formatted_lines          │ │
│  │ 3. self.startup_memories = lines                              │ │
│  └──────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  ┌─ Per-Turn (in handle_user_input) ───────────────────────────┐ │
│  │ 1. extract_keywords(user_message) → keywords                  │ │
│  │ 2. topic_overlap(keywords, prev_keywords)                     │ │
│  │ 3. if overlap < threshold AND msg_len > min_len:              │ │
│  │    a. memory_manager.search(keywords) → lines                 │ │
│  │    b. self.startup_memories = merged(startup, per_turn)       │ │
│  │    c. update prompt_context.memories                          │ │
│  │ 4. prev_keywords = keywords                                   │ │
│  └──────────────────────────────────────────────────────────────┘ │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

## Component: `MemoryIndex` (new struct in `src/context/mod.rs`)

```rust
struct MemoryIndex {
    // word → list of (entry_index, term_frequency)
    inverted: HashMap<String, Vec<(usize, f32)>>,
    // word → inverse document frequency
    idf: HashMap<String, f32>,
    // total number of indexed entries
    doc_count: usize,
}
```

### Construction (`rebuild`)

1. Iterate all `MemoryEntry::content`, split whitespace, apply stop-word filter (reuse existing `is_meaningful_token` from consolidation.rs)
2. For each word w in entry i: increment `tf` counter, store in `inverted[w]`
3. After all entries: compute `idf[w] = log(N / df)`, where df = inverted[w].len()
4. Normalize tf per entry: `tf_norm = 1 + log(tf_raw)` for entries with tf > 0

### Retrieval (`search`)

1. Split query, filter stop words → query_terms
2. For each term: look up inverted index, compute `tf_idf = tf × idf`
3. Aggregate scores per entry, sort descending, return top N entry references
4. Fall back to substring scan if index is empty (graceful degradation)

### Synchronization

- `add_memory()`: append single entry to index (O(|words|) per-add)
- `consolidate()`: full `rebuild` after replacement (consolidation is infrequent)
- Lazy: index rebuilt on first `search()` after `load()` if not yet built

## Existing API Compatibility

- `search_memories(query: &str) -> Vec<MemoryEntry>` — signature unchanged; internally dispatches to `MemoryIndex::search` with fallback to substring scan
- `get_important_memories(threshold)` / `get_memories_by_type(type)` — unchanged (these are filter methods, not search)

## Smart Trigger (per-turn, in `app/mod.rs`)

```rust
struct RecallState {
    prev_keywords: HashSet<String>,
    startup_memories: Vec<String>,  // existing field
}
```

### Topic change detection

```
Jaccard_similarity = |current_keywords ∩ prev_keywords| / |current_keywords ∪ prev_keywords|
trigger_retrieval = Jaccard_similarity < RECALL_SIMILARITY_THRESHOLD (default 0.3)
                 && current_keywords.len() >= MIN_KEYWORD_COUNT (default 2)
```

### Keyword extraction (`C2`)

1. Whitespace split user message
2. Filter: stop words, tokens < 3 chars, pure digits
3. Sort by token length descending (longer = more specific)
4. Take top `MAX_KEYWORDS` (default 6)

## Configuration (`MemorySettings` addition)

```rust
pub struct MemorySettings {
    // ... existing fields ...
    /// Top-N memories to inject per recall (default 5)
    #[serde(default = "default_recall_top_n")]
    pub recall_top_n: usize,
    /// Topic similarity threshold for triggering re-retrieval (0.0–1.0)
    #[serde(default = "default_recall_similarity_threshold")]
    pub recall_similarity_threshold: f32,
}
```

## Files Changed

| File | Change |
|------|--------|
| `src/context/mod.rs` | Add `MemoryIndex` struct, TF-IDF retrieval, update `search_memories()`, sync on `add/consolidate` |
| `src/context/consolidation.rs` | Make `is_meaningful_token` pub(crate) so MemoryIndex can reuse it |
| `src/tui/app/mod.rs` | Replace startup-only recall with per-turn smart trigger + initial recall |
| `src/config/services.rs` | Add `recall_top_n`, `recall_similarity_threshold` to `MemorySettings` |

## Trade-offs

- **Memory**: inverted index adds O(unique_words × entries) overhead. With 10000 entries × avg 50 words each ≈ 500K tokens, index is ~2-5 MB. Acceptable for a CLI tool.
- **Rebuild cost**: O(total_tokens) on `load()` and `consolidate()`. With 10K entries, < 10ms.
- **No CJK tokenization**: Chinese/Japanese/Korean queries will degrade to per-character matching (single-char tokens are filtered by length < 3). Mitigated by substring fallback.
