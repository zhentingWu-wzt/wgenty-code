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
