# Handoff: memory-tfidf-recall (design phase)
Generated-by: comet-handoff.sh
- Mode: full
- Source: openspec/changes/memory-tfidf-recall/proposal.md
- SHA256: a032d1413b1ee8ee470923a5d41890ab8b3fa0a9810799d64954c9f11cde91f2
- Source: openspec/changes/memory-tfidf-recall/design.md
- SHA256: a20780212bd80e3981f52e24c3d39c80742e068e3b4542a92673646804a0efc5
- Source: openspec/changes/memory-tfidf-recall/tasks.md
- SHA256: 1aac16bed2effdb75da682c74fe7e5001fa4b2c88b0ad80b957c3d8a56e7f151

## Change Summary
TF-IDF inverted index for memory retrieval + per-turn smart topic-change recall.

## Key Decisions
- A2 (TF-IDF indexing) + B2 (smart trigger) + C2 (keyword extraction)
- No external dependencies; all std-only
- Index rebuilt in-memory on load(), appended on add_memory()
- Backward compatible: search_memories() API unchanged

## Architecture
- MemoryIndex struct in context/mod.rs
- RecallState in tui/app/mod.rs
- MemorySettings extensions in config/services.rs

## Acceptance Criteria
1. TF-IDF returns error_handler for error handling function query
2. Topic switch triggers re-retrieval; same topic suppresses
3. Session startup performs initial retrieval (backward compat)
4. Stop words don't inflate scores
5. Short messages ("ok", "yes") don't trigger
6. Index syncs after add_memory() / consolidate()
