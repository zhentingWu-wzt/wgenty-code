# Proposal: Memory TF-IDF Retrieval Pipeline

## Motivation

The memory system's retrieval path has two P1-level deficiencies:

1. **#5 Retrieval quality**: `search_memories()` is a bare `String::contains()` substring scan — O(n) with no weighting, no stemming, and "error_handler" won't match "error-handling". The result is effectively keyword lottery.

2. **#6 Recall timing**: Cross-session memories are recalled exactly once at session startup, keyed on cwd basename. When the conversation topic shifts mid-session, no new memories are retrieved — the agent forgets previous context from prior sessions.

## Goals

- Replace naive substring scan with TF-IDF inverted index for memory retrieval
- Inject relevant cross-session memories at per-turn granularity via smart topic-change detection
- Extract keywords from user message as retrieval query (stop-word filtered, length-weighted)
- Maintain backward compatibility: `search_memories()` API unchanged, `MemoryEntry` struct unchanged, prompt assembly protocol unchanged

## Non-Goals

- No external embedding API or new crate dependency
- No persistent index files — index is rebuilt in-memory on `load()`
- No Chinese/multi-language tokenization (whitespace split only; add later)
- No change to `PromptContext.memories` protocol (remains `Vec<String>`)

## Scope

| Affected | Unaffected |
|----------|------------|
| `src/context/mod.rs` — new `MemoryIndex` struct + TF-IDF retrieval | `src/context/storage.rs` |
| `src/tui/app/mod.rs` — per-turn recall replacing startup-only recall | `src/tui/app/*` other logic |
| `src/config/services.rs` — optional `memory.recall_top_n`, `memory.recall_similarity_threshold` | `src/agent/*` |
|  | `src/prompts/mod.rs` (protocol unchanged) |

## Acceptance Criteria

1. TF-IDF retrieval returns "error_handler" memory when searching "error handling function"
2. Topic switch (e.g., "auth" → "database") triggers new retrieval; repeated same-topic messages don't
3. Session startup performs one initial retrieval (backward-compatible)
4. High-frequency stop words don't inflate TF-IDF scores
5. Very short messages ("ok", "yes") don't trigger retrieval
6. Index stays synchronized after `add_memory()` and `consolidate()`
7. All existing context tests continue to pass
