## Why

The codebase has four orphaned memory-related modules (compaction, MemoryManager, AutoDreamService, ContextWindow) that were built but never wired together. Only `agent/compaction.rs` is on the hot path — it's the only one that touches conversation history, calls LLM, and has real data flowing through it. The other three are complete but disconnected: MemoryManager has storage, search, and consolidation but no producer; AutoDreamService has a 3-gate trigger but nobody calls `check_and_run()`; ContextWindow is fully redundant with the existing `conversation_history: Vec<ChatMessage>`. This change wires them into a single memory loop anchored on compaction, delivering cross-session memory to users immediately.

## What Changes

- **New**: `do_auto_compact()` produces both a summary (existing) and extracted memories (new) in a single LLM call, persisting them via MemoryManager
- **New**: Session startup injects recalled memories into the prompt between Layer 5 (Environment) and Layer 6 (Skills)
- **New**: Session startup calls `AutoDreamService::check_and_run()` so the 3-gate consolidation actually triggers
- **New**: `AgentLoop` holds `Arc<MemoryManager>`, replacing the orphaned CLI-only usage
- **New**: `PromptContext` gains a `memories` field for recall injection
- **Removed**: `context::ContextWindow` — fully redundant with `conversation_history`
- **Removed**: `services::auto_dream::MemoryEntry` — replaced by the richer `context::MemoryEntry`
- **Removed**: `~/.wgenty-code/memory.json` and `consolidated_memories.json` — replaced by MemoryManager's `~/.wgenty-code/memory/` directory storage
- **Modified**: AutoDreamService simplified to delegate consolidation to MemoryManager's ConsolidationEngine

## Capabilities

### New Capabilities

- `agent-memory`: End-to-end memory lifecycle — extraction during compaction (producer), storage via MemoryManager's per-file Storage backend, recall and injection into prompt at session start, and time-gated consolidation via AutoDreamService

### Modified Capabilities

*(None — this is a new capability; no existing spec requirements are changing)*

## Impact

- Affected code: `src/tui/agent/compaction.rs`, `src/tui/agent/mod.rs`, `src/tui/app/turn.rs`, `src/prompts/mod.rs`, `src/context/context_window.rs` (removal), `src/services/auto_dream.rs`, `src/services/mod.rs`
- No API changes, no new external dependencies
- No breaking changes to user-facing behavior (existing compaction is preserved; new behavior is additive)
