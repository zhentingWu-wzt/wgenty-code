# Tasks: Memory TF-IDF Retrieval Pipeline

- [x] Task 1: Make `is_meaningful_token` reusable — Change visibility from `fn` to `pub(crate) fn` in `src/context/consolidation.rs`; verify existing tests pass with no behavior change.
- [x] Task 2: Add `MemoryIndex` struct with TF-IDF retrieval — Implement `MemoryIndex` (inverted index + IDF computation + TF-IDF ranking) in `src/context/mod.rs`; add `rebuild()`, `search()`, `add_entry()` methods; reuse `is_meaningful_token` for stop-word filtering.
- [x] Task 3: Wire `MemoryIndex` into `MemoryManager` — Add `index: Arc<RwLock<Option<MemoryIndex>>>`; rebuild on `load()` and `consolidate()`; append on `add_memory()`; dispatch `search_memories()` through `MemoryIndex::search()` with substring fallback.
- [x] Task 4: Add per-turn smart recall to TUI app — Add `RecallState`, `extract_keywords()`, `topic_changed()` helpers to `src/tui/app/mod.rs`; in `handle_user_input`, extract keywords, detect topic change and trigger TF-IDF recall; retain startup recall for backward compatibility.
- [x] Task 5: Add recall configuration to `MemorySettings` — Add `recall_top_n: usize` (default 5) and `recall_similarity_threshold: f32` (default 0.3) to `src/config/services.rs`.
- [x] Task 6: Validate end-to-end — Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`; verify all existing 38 context tests pass plus new tests pass.
