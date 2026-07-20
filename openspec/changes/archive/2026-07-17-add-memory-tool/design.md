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

**Rationale**: `MemoryManager` is already constructed in the app lifecycle. Cloning the `Arc` into the tool is zero-cost.

### D4: Prompt guidance in `base.md`

Add a new `## Proactive memory capture` section to `src/prompts/base.md` (under `# Tool Guidelines`). Also add `memory_add` to the `## Context management` section and the "When to use each tool" table.

Content:
> When you identify something worth remembering long-term (a lesson learned, architecture decision, user preference, key file path, bug fix), proactively call `memory_add` to persist it immediately. Choose `scope`: `global` for cross-project insights (user preferences, workflow habits, correction lessons); `project` for project-specific content (architecture, paths, conventions). Default to `project` if unsure.

**Rationale**: `base.md` is a compile-time constant always injected for all agents. No conditional logic needed. A dedicated section is clearer than embedding in an existing section.

**Alternative considered**: Inject only the tool description (Layer 7 skills inventory style). Rejected--tool description alone doesn't convey *when* to use it; explicit guidance in base instructions is needed for proactive behavior.

### D5: `memory_add` available to all agents (root + subagents)

`memory_add` is available to ALL agents, including subagents (explore/plan/general-purpose). No filtering in `filter_allowed_tools()` is needed. The shared `ToolRegistry` makes the tool available to all agents via `GuardingToolPort`.

**Rationale**: Subagents often discover valuable insights during exploration. Letting them write memories directly is more efficient than requiring the root agent to relay. The dedup mechanism (0.6 similarity threshold) prevents duplication. The consolidation engine merges similar entries over time.

**Spec change**: Remove the "Tool not available to subagents" scenario. Add a "Tool available to all agents" scenario.

## Risks / Trade-offs

- **[Agent overuses memory_add]** -> Mitigation: prompt guidance emphasizes "worth remembering long-term"; dedup (0.6 threshold) prevents exact duplicates; consolidation engine merges similar entries over time.
- **[Memory store grows unbounded]** -> Mitigation: existing `ConsolidationEngine` TTL + merge handles this; `Knowledge`/`Preference` types are permanent but consolidation merges similar ones.
- **[Agent writes to wrong scope]** -> Mitigation: prompt guidance gives clear heuristics; default is `project` (conservative); user can manually move memories between scopes.
- **[Subagent noise]** -> Mitigation: dedup (0.6 threshold) merges similar memories; consolidation engine cleans up over time; subagent memories go through the same `add_memory()` path.
