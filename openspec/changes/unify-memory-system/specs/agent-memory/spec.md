# Agent Memory

End-to-end cross-session memory for the agent loop: extraction during compaction, persistent storage, recall at session start, and time-gated consolidation.

## ADDED Requirements

### Requirement: Memory extraction during compaction

During `do_auto_compact()`, the LLM summarization prompt SHALL be enhanced to request both a conversation summary and extracted memory entries in a single response. The enhanced prompt SHALL request output in JSON format: `{summary: string, memories: [{type: "decision"|"error"|"preference"|"insight"|"knowledge"|"task", content: string, importance: float}]}`. Memory extraction SHALL NOT add an additional LLM call — it reuses the existing compaction LLM call.

#### Scenario: Successful JSON extraction

- **WHEN** compaction fires and the LLM returns valid JSON with `{summary, memories}`
- **THEN** the summary is used to replace conversation history and each memory entry is persisted via `MemoryManager::add_memory()`

#### Scenario: JSON parse failure graceful degradation

- **WHEN** compaction fires and the LLM returns malformed JSON or plain text
- **THEN** the full response is used as summary only (existing behavior), a warning is logged, and no memories are extracted for this compaction cycle

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend (`~/.wgenty-code/memory/<id>.json`). Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, embedding.

#### Scenario: Memory persisted to disk

- **WHEN** `MemoryManager::add_memory()` is called with a valid MemoryEntry
- **THEN** the entry is saved as `~/.wgenty-code/memory/<id>.json` and loaded on next session startup

#### Scenario: Legacy types removed

- **WHEN** the codebase is inspected after this change
- **THEN** `services::auto_dream::MemoryEntry` type no longer exists and `~/.wgenty-code/memory.json` and `consolidated_memories.json` are no longer written

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load all memories from disk. `MemoryManager::search_memories(cwd_project_name)` SHALL retrieve memories matching the current project using keyword matching. Memories below importance threshold 0.5 SHALL be filtered out before injection. Recalled memories SHALL be injected into the prompt as a system message between Layer 5 (Environment) and Layer 6 (Skills). Memory recall SHALL NOT add latency from LLM calls — it is keyword-based only.

#### Scenario: Relevant memories recalled

- **WHEN** a session starts in a project that had previous conversations
- **THEN** memories matching the project name with importance >= 0.5 appear in the system prompt between Environment and Skills layers

#### Scenario: No memories for new project

- **WHEN** a session starts in a project with no prior memories
- **THEN** no `<relevant_memories>` block is injected into the system prompt

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall. The three-gate check (24h since last consolidation, >= 5 new sessions, no active lock) SHALL remain unchanged. When gates pass, consolidation SHALL delegate to `MemoryManager::consolidate()`, which uses `ConsolidationEngine` for deduplication and filtering. Consolidation SHALL use ConsolidationEngine's Jaccard similarity (>0.8 threshold) for duplicate detection, importance threshold (0.3) for filtering, and merge logic for similar memories.

#### Scenario: Consolidation gate passes

- **WHEN** session starts and 24h have passed with >= 5 new sessions and no active lock
- **THEN** `MemoryManager::consolidate()` is called, deduplicating and merging similar memories

#### Scenario: Consolidation gate fails

- **WHEN** session starts but the time or session count gate has not been met
- **THEN** consolidation is skipped and the session continues with existing memories

### Requirement: Dead code removal

`context::ContextWindow` and `context::ContextManager` SHALL be removed as they are redundant with `conversation_history: Vec<ChatMessage>`. `services::auto_dream::MemoryEntry` SHALL be removed and replaced by `context::MemoryEntry`. `AutoDreamService::load_memories()` and `save_consolidated_memories()` SHALL be removed — their functionality is now delegated to `MemoryManager`.

#### Scenario: ContextWindow module removed

- **WHEN** the codebase is compiled after this change
- **THEN** `context::context_window` module no longer exists and no references to `ContextWindow` or `ContextManager` remain

#### Scenario: AutoDream types cleaned up

- **WHEN** `AutoDreamService` is inspected after this change
- **THEN** `load_memories()`, `save_consolidated_memories()`, `analyze_and_consolidate()`, and the local `MemoryEntry` type are all removed
