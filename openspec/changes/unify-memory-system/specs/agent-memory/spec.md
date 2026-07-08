# Agent Memory

End-to-end cross-session memory for the agent loop: extraction during compaction, persistent storage, recall at session start, and time-gated consolidation.

## ADDED

### Requirements

### Memory Extraction (Producer)

- **REQ-AM-001**: During `do_auto_compact()`, the LLM summarization prompt SHALL be enhanced to request both a conversation summary and extracted memory entries in a single response.
- **REQ-AM-002**: The enhanced prompt SHALL request output in JSON format: `{summary: string, memories: [{type: "decision"|"error"|"preference"|"insight"|"knowledge"|"task", content: string, importance: float}]}`.
- **REQ-AM-003**: On successful JSON parse, extracted memories SHALL be persisted via `MemoryManager::add_memory()`.
- **REQ-AM-004**: On JSON parse failure, the system SHALL fall back gracefully to using the full response as summary only (existing behavior), log a warning, and skip memory extraction for this compaction cycle.
- **REQ-AM-005**: Memory extraction SHALL NOT add an additional LLM call — it reuses the existing compaction LLM call.

### Memory Storage

- **REQ-AM-006**: All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend (`~/.wgenty-code/memory/<id>.json`).
- **REQ-AM-007**: Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, embedding.
- **REQ-AM-008**: The legacy `services::auto_dream::MemoryEntry` type SHALL be removed.
- **REQ-AM-009**: The legacy `~/.wgenty-code/memory.json` and `~/.wgenty-code/consolidated_memories.json` files SHALL no longer be written.

### Memory Recall (Consumer)

- **REQ-AM-010**: At session startup, `MemoryManager::load()` SHALL load all memories from disk.
- **REQ-AM-011**: At session startup, `MemoryManager::search_memories(cwd_project_name)` SHALL retrieve memories matching the current project.
- **REQ-AM-012**: Memories below importance threshold 0.5 SHALL be filtered out before injection.
- **REQ-AM-013**: Recalled memories SHALL be injected into the prompt as a system message between Layer 5 (Environment) and Layer 6 (Skills).
- **REQ-AM-014**: Memory recall SHALL NOT add latency from LLM calls — it is keyword-based only.

### Memory Consolidation

- **REQ-AM-015**: `AutoDreamService::check_and_run()` SHALL be called at session startup.
- **REQ-AM-016**: The three-gate check (24h since last consolidation, >= 5 new sessions, no active lock) SHALL remain unchanged.
- **REQ-AM-017**: When gates pass, consolidation SHALL delegate to `MemoryManager::consolidate()`, which uses `ConsolidationEngine` for deduplication and filtering.
- **REQ-AM-018**: Consolidation SHALL use ConsolidationEngine's Jaccard similarity (>0.8 threshold) for duplicate detection, importance threshold (0.3) for filtering, and merge logic for similar memories.

### Dead Code Removal

- **REQ-AM-019**: `context::ContextWindow` and `context::ContextManager` SHALL be removed — redundant with `conversation_history: Vec<ChatMessage>`.
- **REQ-AM-020**: `services::auto_dream::MemoryEntry` SHALL be removed — replaced by `context::MemoryEntry`.
- **REQ-AM-021**: `AutoDreamService::load_memories()` and `save_consolidated_memories()` SHALL delegate to `MemoryManager` instead of reading/writing custom files.

## Acceptance Scenarios

1. **Producer**: After a long conversation triggers compaction, `~/.wgenty-code/memory/` contains new JSON files with extracted memories containing correct type, content, and importance.
2. **Recall**: On next session start in the same project, the injected prompt contains a system message with relevant memories from prior sessions.
3. **Consolidation**: After 24h and 5+ sessions, AutoDream triggers and similar memories are merged; memory count decreases.
4. **Graceful degradation**: If the LLM returns malformed JSON during compaction, the summary still works, no memories are extracted, and no crash occurs.
5. **Dead code gone**: `context::ContextWindow`, `auto_dream::MemoryEntry`, `memory.json`, and `consolidated_memories.json` no longer exist in the codebase or are written to disk.
