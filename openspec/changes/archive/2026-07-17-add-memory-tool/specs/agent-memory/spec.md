## ADDED Requirements

### Requirement: Proactive memory capture via tool

The system SHALL provide a `memory_add` tool that allows the agent to proactively write a memory entry at any point during a conversation, without waiting for context compaction. The tool SHALL accept parameters: `content` (required string), `memory_type` (enum: Knowledge/Preference/Session/Conversation, default Knowledge), `scope` (enum: project/global, default project), `tags` (optional string array), and `importance` (optional float 0.0-1.0, default 0.5). The tool SHALL delegate to `MemoryManager::add_memory()` for storage, deduplication (0.6 similarity threshold), and scope routing. The tool SHALL declare `is_read_only() = false`. The tool SHALL be available to all agents (root + subagents).

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

#### Scenario: Tool available to all agents

- **WHEN** any agent (root, explore, plan, or general-purpose subagent) inspects its available tools
- **THEN** `memory_add` is in the agent's tool registry (no subagent filtering; dedup prevents duplication)

### Requirement: Prompt guidance for proactive memory capture

The system prompt SHALL include guidance instructing the agent to proactively call `memory_add` when it identifies content worth remembering long-term. The guidance SHALL specify scope selection heuristics: `global` for cross-project insights (user preferences, workflow habits, correction lessons); `project` for project-specific content (architecture decisions, file paths, bug fixes, conventions).

#### Scenario: Guidance present in base instructions

- **WHEN** the system prompt is assembled for the root agent
- **THEN** a paragraph about proactive memory capture is included, mentioning `memory_add` tool, examples of memorable content, and scope selection guidance

#### Scenario: Guidance present for all agents

- **WHEN** the system prompt is assembled for any agent (root or subagent)
- **THEN** the proactive memory capture guidance is included (base.md is always injected)
