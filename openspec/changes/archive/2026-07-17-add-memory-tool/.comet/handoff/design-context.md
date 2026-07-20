# Comet Design Handoff

- Change: add-memory-tool
- Phase: design
- Mode: compact
- Context hash: 91ba230ed05c238ea37dbf7c8951cb318cfc6f40495420c553eb1714cdb32fa9

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/add-memory-tool/proposal.md

- Source: openspec/changes/add-memory-tool/proposal.md
- Lines: 1-29
- SHA256: ab512b7afb417100a40371c60de8a60ac980966b1a05ad1d68347f0ada9dd687

```md
## Why

Memory capture currently relies solely on passive extraction during context compaction--an unreliable, delayed path that only fires when the context window fills up. Valuable insights (lessons, decisions, preferences) identified mid-conversation are lost if compaction doesn't trigger or the LLM doesn't select them. The agent needs a way to proactively and immediately persist memories the moment it recognizes something worth remembering.

## What Changes

- New `memory_add` tool that lets the agent write a memory entry on demand, with parameters for content, memory type, scope (project/global), tags, and importance
- `ToolRegistry` wired to accept a `MemoryManager` handle so `memory_add` can call `add_memory()` directly (same dedup/merge logic as compaction path)
- Prompt layer guidance instructing the agent to proactively call `memory_add` when it identifies memorable content (lessons, architecture decisions, user preferences, key file paths, bug fixes)
- Scope selection guidance: cross-project content (preferences, workflow habits, correction lessons) -> global; project-specific content (architecture, paths, conventions) -> project

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `agent-memory`: Add requirement for proactive memory capture via tool--agent can write memories immediately without waiting for compaction, using the same `MemoryManager::add_memory()` storage/dedup path

## Impact

- `src/tools/meta/memory_add.rs` (new file): `MemoryAddTool` implementing `Tool` trait
- `src/tools/mod.rs`: Register `MemoryAddTool`, pass `MemoryManager` handle into `ToolRegistry`
- `src/context/mod.rs`: Ensure `add_memory()` is accessible from tool layer (may need `Arc<MemoryManager>` sharing)
- `src/prompts/`: Add memory_add guidance to base instructions or a dedicated memory guidance section
- `src/tui/app/mod.rs`: Wire `MemoryManager` from `App` into `ToolRegistry` construction
- No breaking changes to existing memory compaction/consolidation/recall paths
```

## openspec/changes/add-memory-tool/design.md

- Source: openspec/changes/add-memory-tool/design.md
- Lines: 1-108
- SHA256: a322e88467124317117199be699a7f57155af8921f24438a077ff2d4c6371b2c

[TRUNCATED]

```md
---
comet_change: add-memory-tool
role: technical-design
canonical_spec: openspec
---

## Context

The memory system (`src/context/mod.rs`) currently has a single write path: `ApiCompactor::compact()` extracts memories passively during context compaction. This path is unreliable (fires only when context window fills), delayed (memories not available until compaction), and non-selective (LLM decides what's memorable post-hoc). An agent that identifies a valuable insight mid-conversation has no way to persist it immediately.

`MemoryManager` is already `Arc`-friendly (all fields are `Arc<...>`), and `add_memory(entry, origin)` handles dedup (0.6 similarity threshold) + storage (project/global). The tool layer (`src/tools/`) has a `Tool` trait and `ToolRegistry` that registers tools at construction. `ToolRegistry::new()` creates all tools; `register(&self, tool)` allows adding tools after construction. `BASE_INSTRUCTIONS` (`src/prompts/base.md`) is a compile-time constant always injected for all agents.

## Goals / Non-Goals

**Goals:**
- Agent can proactively write a memory entry at any point in a conversation via a `memory_add` tool
- Reuse existing `MemoryManager::add_memory()` for storage, dedup, and scope routing
- Agent specifies scope (project/global) in tool parameters
- Prompt guidance instructs the agent when to use the tool and how to choose scope
- Immediate persistence (no waiting for compaction)
- Tool returns `memory_id` and `merged` flag so the agent knows if dedup occurred

**Non-Goals:**
- Replacing the compaction extraction path (both paths coexist)
- Adding a CLI `memory add` subcommand (separate change if needed)
- Fixing `note_edit` persistence (separate issue, different schema)
- System-level per-turn LLM detection of memorable content (cost too high)

## Decisions

### D1: New `MemoryAddTool` in `src/tools/meta/memory_add.rs`

Implements `Tool` trait. Holds `Arc<MemoryManager>`. Parameters:
- `content` (string, required): the memory content
- `memory_type` (enum: Knowledge/Preference/Session/Conversation, default Knowledge)
- `scope` (enum: project/global, default project)
- `tags` (string array, optional)
- `importance` (float 0.0-1.0, default 0.5)

`is_read_only()` returns `false` (writes to memory store).

**Rationale**: Follows existing tool conventions (like `note_edit` in `meta/`). `Arc<MemoryManager>` is cheap to clone (all fields Arc). Parameters map directly to `MemoryEntry` + `MemoryOrigin`.

**Alternative considered**: Extend `note_edit` to write to memory store. Rejected--`note_edit` uses a different schema (`Note` vs `MemoryEntry`) and doesn't participate in memory recall/injection. Mixing them would conflate two independent systems.

### D1b: Modify `add_memory()` return type to `Result<MemoryAddResult>`

The spec requires the tool to return `{ success, memory_id, merged }`, but `add_memory()` currently returns `Result<()>`. When dedup merges, the stored id is the *existing* entry's id (not the new one), so the tool can't determine the result on its own.

**Decision**: Change `add_memory()` return type from `anyhow::Result<()>` to `anyhow::Result<MemoryAddResult>`:

```rust
pub struct MemoryAddResult {
    pub id: String,      // stored entry's id (existing if merged, new otherwise)
    pub merged: bool,    // true if dedup merged into existing entry
}
```

**Zero-breaking**: All 3 production callers use `if let Err(e) = mm.add_memory(...).await` pattern (only matches Err variant, Ok type irrelevant). 15 test callers use `.await.unwrap()` (works on any `Result<T, E>`). No caller uses the Ok value.

**Rationale**: Single path, no duplication. The alternative (`add_memory_with_result()` delegation) adds boilerplate for no benefit when the signature change is non-breaking.

### D2: Register `MemoryAddTool` via existing `register()` method

Use `ToolRegistry::register()` after construction instead of a new constructor. The `memory_manager` Arc is already in scope at the headless runtime construction site (`src/cli/headless_runtime.rs:235`, `memory_manager` used at line 240).

```rust
let registry = Arc::new(ToolRegistry::new().with_settings(&settings));
registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())));
```

**Rationale**: `register(&self, tool: Box<dyn Tool>)` already exists and takes `&self`. No new constructor needed. Simpler than `new_with_memory()` and avoids API surface growth.

**Alternative considered**: `ToolRegistry::new_with_memory(memory: Arc<MemoryManager>)`. Rejected--unnecessary when `register()` already works. The original concern about `new()` callers was unfounded since `register()` doesn't modify `new()`.

### D3: Register at headless runtime construction site

In `src/cli/headless_runtime.rs`, after `ToolRegistry::new().with_settings(&settings)`, call `registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())))`. The same `Arc<MemoryManager>` is already used by the compactor (line 240)--shared handle, no duplication.

For `src/state/mod.rs` (Default impl) and `src/daemon/state.rs`: these are display-only or daemon contexts. `memory_add` can be registered conditionally if `MemoryManager` is available; otherwise skip (the tool list is for display, not execution).
```

Full source: openspec/changes/add-memory-tool/design.md

## openspec/changes/add-memory-tool/tasks.md

- Source: openspec/changes/add-memory-tool/tasks.md
- Lines: 1-45
- SHA256: 385707b8971ec3e48839861aa8c7a5793ecc87253e7f2627507d1b187f622eb2

```md
# Tasks: add-memory-tool

## 1. Add `MemoryAddResult` + modify `add_memory()` return type
- [ ] Add `MemoryAddResult { id: String, merged: bool }` struct to `src/context/mod.rs`
- [ ] Change `add_memory()` return type from `Result<()>` to `Result<MemoryAddResult>`
- [ ] Return `MemoryAddResult { id: existing_id, merged: true }` on dedup merge
- [ ] Return `MemoryAddResult { id: entry.id, merged: false }` on new entry
- [ ] Verify all existing callers still compile (they use `if let Err` or `.unwrap()`)

## 2. Create `MemoryAddTool` implementation
- [ ] Create `src/tools/meta/memory_add.rs`
- [ ] Implement `MemoryAddTool` struct holding `Arc<MemoryManager>`
- [ ] Implement `Tool` trait: `name()` = "memory_add", `is_read_only()` = false
- [ ] Implement `input_schema()` with content/memory_type/scope/tags/importance
- [ ] Implement `execute()`: parse params, build `MemoryEntry`, call `add_memory()`, return JSON with memory_id + merged flag
- [ ] Handle errors: missing content, invalid memory_type, invalid scope
- [ ] Add module to `src/tools/meta/mod.rs`

## 3. Register `MemoryAddTool` via `register()`
- [ ] In `src/cli/headless_runtime.rs`: after `ToolRegistry::new().with_settings()`, call `registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())))`
- [ ] Ensure `MemoryManager` Arc is shared with compactor/injector (same handle)
- [ ] No new constructor needed (use existing `register(&self)`)

## 4. Add prompt guidance
- [ ] Add `## Proactive memory capture` section to `src/prompts/base.md` (under `# Tool Guidelines`)
- [ ] Include: when to use memory_add, examples of memorable content, scope selection heuristics
- [ ] Add `memory_add` entry to `## Context management` section
- [ ] Add row to "When to use each tool" table
- [ ] base.md is always injected for all agents (no conditional logic)

## 5. Tests
- [ ] Unit test: `add_memory()` returns `MemoryAddResult` with correct id + merged=false for new entry
- [ ] Unit test: `add_memory()` returns `MemoryAddResult` with existing id + merged=true for dedup
- [ ] Unit test: `memory_add` creates new project memory (verify file exists)
- [ ] Unit test: `memory_add` creates new global memory (verify file exists)
- [ ] Unit test: `memory_add` with similar content triggers merge (verify no duplicate + merged flag)
- [ ] Unit test: missing content returns error
- [ ] Unit test: invalid memory_type returns error
- [ ] Unit test: `is_read_only()` returns false

## 6. Validation
- [ ] `cargo fmt -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all`
- [ ] Manual: run `cargo run -- repl`, ask agent to remember something, verify `memory status` shows new entry
```

## openspec/changes/add-memory-tool/specs/agent-memory/spec.md

- Source: openspec/changes/add-memory-tool/specs/agent-memory/spec.md
- Lines: 1-54
- SHA256: 10ed09a17ca63e946533efe21eccb3b636dd5294021b43a93dbd999873d52828

```md
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
```

