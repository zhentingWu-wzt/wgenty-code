# Brainstorm Summary: add-memory-tool

**Change:** add-memory-tool
**Phase:** design
**Date:** 2026-07-17
**Status:** Confirmed

## Context

Designing a `memory_add` tool that lets the agent proactively write memory entries via `MemoryManager::add_memory()`, without waiting for context compaction. All OpenSpec artifacts (proposal, design, spec, tasks) were created in the open phase. This brainstorm verified the design against the actual codebase and resolved 4 design questions.

## Codebase Findings

### Verified structures

| Symbol | Location | Notes |
|--------|----------|-------|
| `MemoryEntry` | `src/context/mod.rs:34` | id (String/UUID), memory_type, content, timestamp, importance (f32), tags, metadata. Builders: `new()`, `with_importance()`, `with_tags()`, `with_metadata()` |
| `MemoryOrigin` | `src/context/mod.rs:93` | enum { Project, Global } |
| `MemoryType` | `src/context/mod.rs:78` | enum { Knowledge, Preference, Session, Conversation } |
| `MemoryManager::add_memory()` | `src/context/mod.rs:412` | Returns `Result<()>`. Dedup at 0.6 threshold. Merge uses existing entry's id. |
| `Tool` trait | `src/tools/mod.rs:41` | `#[async_trait]`, `execute(Value) -> Result<ToolOutput, ToolError>`, `is_read_only()` default false |
| `ToolOutput` | `src/tools/mod.rs:87` | `{ output_type, content, metadata }` |
| `ToolRegistry` | `src/tools/mod.rs:110` | `register(&self, tool: Box<dyn Tool>)` takes `&self` |
| `filter_allowed_tools()` | `src/tools/meta/task.rs:849` | Only filters `task`/`delegate` + `MUTATING_FS_TOOLS` |
| `BASE_INSTRUCTIONS` | `src/prompts/mod.rs` | `include_str!("base.md")`, always injected for all agents |

### `add_memory()` callers (18 total, 3 production)

All production callers use `if let Err(e) = mm.add_memory(...).await` pattern - only matches Err variant, Ok type irrelevant. **Zero-breaking** to change return type.

- `src/tui/agent/adapters.rs:630` - `if let Err(e) =` pattern
- `src/agent/runtime/compactor.rs:341` - `if let Err(e) =` pattern
- `src/services/auto_dream.rs:289` - test, `.await.unwrap()`

### ToolRegistry construction sites (3 production)

- `src/cli/headless_runtime.rs:235` - `memory_manager` in scope (line 240)
- `src/state/mod.rs:196` - Default impl, display only, no MemoryManager
- `src/daemon/state.rs:141` - daemon state

## Resolved Design Questions

### Q1: `add_memory()` return type gap [CONFIRMED: Option A]

**Problem:** Spec requires tool to return `{ success, memory_id, merged }`, but `add_memory()` returns `Ok(())`. When dedup merges, stored id is the existing entry's id, so tool can't know result.

**Decision:** Modify `add_memory()` return type from `Result<()>` to `Result<MemoryAddResult>`. Zero-breaking (all callers use `if let Err` or `.unwrap()`).

```rust
pub struct MemoryAddResult {
    pub id: String,
    pub merged: bool,
}
```

### Q2: ToolRegistry wiring [CONFIRMED: register() after construction]

**Problem:** Design proposed `new_with_memory(memory)` constructor. But `register(&self)` already exists and `memory_manager` is in scope at the headless runtime call site.

**Decision:** Use `register()` after construction. No new constructor needed.

```rust
let registry = Arc::new(ToolRegistry::new().with_settings(&settings));
registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())));
```

### Q3: Subagent access [CONFIRMED: All agents]

**Problem:** Design D5 said "root agent only". But `filter_allowed_tools()` doesn't automatically exclude `memory_add`.

**Decision:** `memory_add` available to ALL agents (root + subagents). No filtering needed. Dedup prevents duplication. Update spec: remove "Tool not available to subagents" scenario, add "Tool available to all agents" scenario.

### Q4: Prompt guidance placement [CONFIRMED: base.md new section]

**Problem:** Design D4 said "base instructions or dedicated section".

**Decision:** New `## Proactive memory capture` section in `src/prompts/base.md`. Also add `memory_add` to `## Context management` section and "When to use each tool" table. base.md is always injected for all agents - no conditional logic needed.

## Design Doc Updates Needed

- D1: Add `MemoryAddResult` struct + `add_memory()` signature change
- D2: Change from `new_with_memory()` to `register()` after construction
- D3: Simplify - register at headless_runtime call site
- D4: Confirmed - new section in base.md
- D5: Change from "root only" to "all agents" + spec update

## Spec Patch Needed

- Remove scenario: "Tool not available to subagents"
- Add scenario: "Tool available to all agents" (root + subagents)
