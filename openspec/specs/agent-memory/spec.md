# agent-memory Specification

## Purpose
TBD - created by archiving change unify-memory-system. Update Purpose after archive.
## Requirements
### Requirement: Memory extraction during compaction

During `do_auto_compact()`, the LLM summarization prompt SHALL be enhanced to request both a conversation summary and extracted memory entries in a single response. The enhanced prompt SHALL request output in JSON format: `{summary: string, memories: [{type: "decision"|"error"|"preference"|"insight"|"knowledge"|"task", content: string, importance: float}]}`. Memory extraction SHALL NOT add an additional LLM call — it reuses the existing compaction LLM call.

#### Scenario: Successful JSON extraction

- **WHEN** compaction fires and the LLM returns valid JSON with `{summary, memories}`
- **THEN** the summary is used to replace conversation history and each memory entry is persisted via `MemoryManager::add_memory()`

#### Scenario: JSON parse failure graceful degradation

- **WHEN** compaction fires and the LLM returns malformed JSON or plain text
- **THEN** the full response is used as summary only (existing behavior), a warning is logged, and no memories are extracted for this compaction cycle

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend. Memories SHALL be physically separated by scope:
- **Project memories** SHALL be stored at `<project_root>/.wgenty-code/memory/<id>.json`
- **Global memories** SHALL be stored at `~/.wgenty-code/memory/<id>.json`

`project_root` SHALL equal the current working directory (CWD), with no upward search for project markers. Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata. `MemoryManager` SHALL track each loaded memory's origin (Project/Global) internally without serializing the origin field. The TF-IDF index SHALL index only project memories so that `search_memories()` naturally returns only project-scoped results. Deduplication SHALL occur within the same scope only; cross-scope duplicates SHALL NOT be merged.

#### Scenario: Project memory persisted to project-local directory

- **WHEN** `MemoryManager::add_memory(entry, Project)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Global memory persisted to global directory

- **WHEN** `MemoryManager::add_memory(entry, Global)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `~/.wgenty-code/memory/<id>.json`

#### Scenario: CWD unavailable degrades to global storage

- **WHEN** the project-local memory directory cannot be created (e.g. CWD deleted or unwritable)
- **THEN** project memories SHALL fall back to the global memory directory and a warning SHALL be logged

#### Scenario: CWD equals home directory

- **WHEN** `project_root` resolves to the user's home directory (project root coincides with global root)
- **THEN** project memories SHALL be written to the global memory directory (merged pool) and a warning SHALL be logged

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load project memories from `<CWD>/.wgenty-code/memory/` and global memories from `~/.wgenty-code/memory/`. `MemoryManager::search_memories(query)` SHALL retrieve only project memories matching the query via the TF-IDF index (global memories are not indexed for recall). Global memories SHALL be injected every turn as a `<global-memory>` block, NOT filtered by the importance threshold. Global memories exceeding a soft cap (default 50) SHALL be truncated to the top entries by importance with a warning logged. The `<global-memory>` block SHALL NOT be injected when no global memories exist.

#### Scenario: Global memories injected every turn

- **WHEN** a turn is processed and global memories exist in `~/.wgenty-code/memory/`
- **THEN** a `<global-memory>` block containing all global memories (sorted by importance, capped at 50) is injected into the system prompt between the Environment and Skills layers

#### Scenario: Project memories recalled by keyword

- **WHEN** a user message is processed and project memories match the extracted keywords with importance >= threshold
- **THEN** a `<memory-context>` block containing the matched project memories is injected (global memories are excluded from this block)

#### Scenario: No global memories

- **WHEN** a turn is processed but no global memories exist
- **THEN** no `<global-memory>` block is injected

#### Scenario: Global memory soft cap exceeded

- **WHEN** more than 50 global memories exist
- **THEN** only the top 50 by importance are injected and a warning is logged

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

### Requirement: Project-local session storage

`SessionManager` SHALL store sessions at `<CWD>/.wgenty-code/sessions/{id}.json` instead of the global `~/.wgenty-code/sessions/`. `SessionManager::list()` SHALL return only sessions belonging to the current project. `project_root` SHALL equal CWD with no upward search. When the project-local sessions directory cannot be created, sessions SHALL fall back to `~/.wgenty-code/sessions/` with a warning logged.

#### Scenario: Session created in project-local directory

- **WHEN** a new session is created via `SessionManager::create()`
- **THEN** the session is persisted at `<CWD>/.wgenty-code/sessions/{id}.json`

#### Scenario: Session list scoped to current project

- **WHEN** `SessionManager::list()` is called
- **THEN** only sessions stored in `<CWD>/.wgenty-code/sessions/` are returned

#### Scenario: Unwritable CWD falls back to global

- **WHEN** the project-local sessions directory cannot be created
- **THEN** sessions are stored in `~/.wgenty-code/sessions/` and a warning is logged

### Requirement: Memory scope classification during compaction

During `do_auto_compact()`, the LLM summarization prompt SHALL request each extracted memory entry to include a `scope` field with value `"project"` or `"global"`. The prompt SHALL instruct the model to classify cross-project preferences and behavioral conventions as `global`, and project-specific decisions/knowledge as `project`. When the `scope` field is absent or unparseable, it SHALL default to `project`. Extracted memories SHALL be persisted via `MemoryManager::add_memory(entry, scope)` to the directory corresponding to their scope.

#### Scenario: Scope classified and routed

- **WHEN** compaction extracts a memory with `scope: "global"`
- **THEN** the memory is stored at `~/.wgenty-code/memory/<id>.json`

#### Scenario: Missing scope defaults to project

- **WHEN** compaction extracts a memory without a `scope` field
- **THEN** the memory is treated as project-scoped and stored at `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Manual memory addition with scope

- **WHEN** a user manually adds a memory via CLI or agent tool with an explicit scope
- **THEN** the memory is stored in the directory corresponding to the specified scope

### Requirement: Legacy data migration to project-local storage

On startup, if legacy sessions exist at `~/.wgenty-code/sessions/` and migration has not been performed (tracked by a `~/.wgenty-code/.migrated-project-local` marker file), `migrate_legacy_sessions()` SHALL move each session to `<project_path>/.wgenty-code/sessions/{id}.json` using the session's `project_path` field. Sessions with `project_path == None` SHALL be moved to the current CWD's project-local directory. Migration SHALL be idempotent: if the target file already exists, the source is skipped. On successful migration of a file, the original SHALL be deleted; on failure, the original SHALL be preserved with a warning. Existing memories at `~/.wgenty-code/memory/` SHALL NOT be migrated--they naturally become global memories.

#### Scenario: Sessions migrated by project_path

- **WHEN** startup detects legacy sessions at `~/.wgenty-code/sessions/` and migration marker is absent
- **THEN** each session is moved to its `project_path`'s `.wgenty-code/sessions/` directory

#### Scenario: Session without project_path migrated to CWD

- **WHEN** a legacy session has `project_path == None`
- **THEN** it is moved to `<CWD>/.wgenty-code/sessions/{id}.json`

#### Scenario: Migration is idempotent

- **WHEN** migration runs and a target file already exists at the destination
- **THEN** the source session is skipped (not overwritten)

#### Scenario: Migration marker prevents re-scan

- **WHEN** the `~/.wgenty-code/.migrated-project-local` marker file exists
- **THEN** session migration is not re-run on subsequent startups

#### Scenario: Existing memories remain global

- **WHEN** startup loads memories after migration
- **THEN** all pre-existing `~/.wgenty-code/memory/*.json` files are loaded as global memories (not moved)

