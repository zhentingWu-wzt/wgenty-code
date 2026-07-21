## MODIFIED Requirements

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall, in both TUI/daemon and headless modes. The gate thresholds SHALL be `min_hours = 1` and `min_sessions = 1` (distance from last consolidation >= 1 hour AND >= 1 new session since last consolidation). The session-scan interval throttle (`SESSION_SCAN_INTERVAL_MS = 10 minutes`) SHALL remain. When gates pass, consolidation SHALL delegate to `MemoryManager::consolidate()`, which uses `ConsolidationEngine` for deduplication and filtering. Consolidation SHALL use ConsolidationEngine's Jaccard similarity (>0.8 threshold) for duplicate detection, importance threshold (0.3) for filtering, and merge logic for similar memories.

AutoDream SHALL NOT maintain its own disk-based consolidation lock. Cross-process mutual exclusion SHALL be provided solely by `MemoryManager::consolidate()`'s internal `ConsolidationFileLock` (at `~/.wgenty-code/memory/.consolidation.lock`). AutoDream's in-memory `is_consolidating` flag SHALL be retained only to prevent same-process re-entrancy within `check_and_run()`, and SHALL NOT be persisted to disk. The `last_consolidated_at` timestamp SHALL remain persisted in `~/.wgenty-code/.autodream_state.json` as the time-gate baseline.

`MemoryManager::consolidate()` does not invoke any LLM call -- it is pure local computation (TF-IDF similarity merge, TTL decay, orphan-file reconcile, index rebuild). This is the premise that permits the aggressive 1h/1session gate.

#### Scenario: Consolidation gate passes

- **WHEN** session starts and 1 hour has passed with >= 1 new session and no active consolidation lock held by another process
- **THEN** `MemoryManager::consolidate()` is called, deduplicating and merging similar memories

#### Scenario: Consolidation gate fails on time

- **WHEN** session starts but less than 1 hour has passed since last consolidation
- **THEN** consolidation is skipped and the session continues with existing memories

#### Scenario: Consolidation gate fails on session-scan throttle

- **WHEN** session starts within the 10-minute session-scan interval since the last scan
- **THEN** consolidation is skipped without re-scanning the sessions directory

#### Scenario: Cross-process mutual exclusion via MemoryManager lock

- **WHEN** AutoDream triggers `consolidate()` while a concurrent `memory dream` invocation already holds the `ConsolidationFileLock`
- **THEN** AutoDream's `consolidate()` waits on the same lock (no separate AutoDream lock file is created) and no race occurs

#### Scenario: AutoDream does not write a separate disk lock

- **WHEN** AutoDream runs consolidation
- **THEN** no `~/.wgenty-code/.consolidation.lock` (timestamp lock) file is written; only `~/.wgenty-code/.autodream_state.json` (state) and `~/.wgenty-code/memory/.consolidation.lock` (mm internal lock) are touched

#### Scenario: Headless mode triggers AutoDream startup check

- **WHEN** a headless/CLI session starts
- **THEN** `AutoDreamService::check_and_run()` is invoked (fire-and-forget) before the agent loop, identical to TUI startup

#### Scenario: Daemon mode triggers AutoDream startup check

- **WHEN** a TUI/daemon session starts and the daemon process initializes its `DaemonState`
- **THEN** the daemon constructs `AutoDreamService` (with the daemon's `MemoryManager`) and invokes `check_and_run()` (fire-and-forget), so TUI/daemon mode triggers consolidation at session startup

#### Scenario: TUI app does not directly start AutoDream

- **WHEN** a TUI session starts
- **THEN** the TUI app does NOT construct or invoke `AutoDreamService` itself; AutoDream startup is handled solely by the daemon (avoiding duplicate consolidation triggers)

#### Scenario: Consolidation is LLM-free

- **WHEN** `check_and_run()` gates pass and `consolidate()` runs
- **THEN** no LLM call is made; consolidation completes via local TF-IDF merge, TTL decay, orphan reconcile, and index rebuild

### Requirement: Proactive memory capture via tool

The system SHALL provide a `memory_add` tool that allows the agent to proactively write a memory entry at any point during a conversation, without waiting for context compaction. The tool SHALL accept parameters: `content` (required string), `memory_type` (enum: Knowledge/Preference/Session/Conversation/Task/Error/Insight/Decision, default Knowledge), `scope` (enum: project/global, default project), `tags` (optional string array), and `importance` (optional float 0.0-1.0, default 0.5). The tool SHALL delegate to `MemoryManager::add_memory()` for storage, deduplication (0.6 similarity threshold), and scope routing. The tool SHALL declare `is_read_only() = false`. The tool SHALL be registered in BOTH the headless runtime tool registry AND the daemon tool registry (`daemon/state.rs`), so that it is available to the model in all run modes (TUI/daemon and headless). The tool SHALL be available to all agents (root + subagents); the subagent tool filter (`filter_allowed_tools`) SHALL NOT exclude `memory_add`.

#### Scenario: Agent proactively writes a project memory

- **WHEN** the agent calls `memory_add` with content "note_edit tool uses NoteStore but is registered with store:None, so it doesn't persist", memory_type "Knowledge", scope "project"
- **THEN** `MemoryManager::add_memory()` is called with a `MemoryEntry` of type Knowledge and `MemoryOrigin::Project`, and the memory is saved to `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Agent proactively writes a global memory

- **WHEN** the agent calls `memory_add` with content "Always read actual settings.json before assuming config defaults", scope "global"
- **THEN** `MemoryManager::add_memory()` is called with `MemoryOrigin::Global`, and the memory is saved to `~/.wgenty-code/memory/<id>.json`

#### Scenario: Dedup merges similar memory

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory in the same scope
- **THEN** `MemoryManager::add_memory()` merges the new content into the existing memory entry (updating timestamp/metadata) instead of creating a duplicate, and the tool output indicates a merge occurred

#### Scenario: Tool returns memory_id on success

- **WHEN** `memory_add` succeeds (new or merged)
- **THEN** the tool returns a JSON result containing `success: true`, `memory_id` (the stored entry's UUID), and `merged: boolean` indicating whether it was merged into an existing entry

#### Scenario: Invalid memory_type rejected

- **WHEN** the agent calls `memory_add` with memory_type "InvalidType"
- **THEN** the tool returns an error with code "invalid_memory_type" and does not call `add_memory()`

#### Scenario: Missing content rejected

- **WHEN** the agent calls `memory_add` without the `content` parameter
- **THEN** the tool returns an error with code "missing_content" and does not call `add_memory()`

#### Scenario: Tool registered in daemon registry

- **WHEN** a TUI/daemon session starts and the daemon builds its tool registry
- **THEN** `memory_add` is registered in the daemon tool registry (constructed with the daemon's `MemoryManager`), so the model can call it in TUI/daemon mode

#### Scenario: Tool registered in headless registry

- **WHEN** a headless session starts and builds its tool registry
- **THEN** `memory_add` is registered in the headless tool registry (constructed with the headless `MemoryManager`)

#### Scenario: Tool available to all agents

- **WHEN** any agent (root, explore, plan, or general-purpose subagent) inspects its available tools
- **THEN** `memory_add` is in the agent's tool registry (`filter_allowed_tools` does not exclude it; dedup prevents duplication)
