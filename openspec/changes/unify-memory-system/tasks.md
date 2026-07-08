## 1. P0 — Producer: Memory Extraction in Compaction

- [x] 1.1 Add `memory_manager: Arc<MemoryManager>` field to `AgentLoop` struct and `AgentLoop::new()`
- [x] 1.2 Enhance compaction system prompt in `do_auto_compact()` to request dual output (summary + memories) in JSON format
- [x] 1.3 Parse JSON response after receiving summary — extract `memories` array and persist each via `memory_manager.add_memory()`
- [x] 1.4 Implement graceful degradation: on JSON parse failure, use full response as summary only, log warning, skip memory extraction
- [x] 1.5 Update `App::spawn_agent_turn()` and `App::spawn_compact_turn()` to pass `Arc<MemoryManager>` to `AgentLoop::new()`
- [ ] 1.6 Add unit test: verify enhanced prompt includes JSON output format instruction
- [ ] 1.7 Add unit test: verify JSON parse success path calls `add_memory`
- [ ] 1.8 Add unit test: verify JSON parse failure falls back gracefully

## 2. P1 — Consumer: Memory Recall at Session Start

- [x] 2.1 Add `memories: Vec<String>` field to `PromptContext`
- [x] 2.2 Add builder method `PromptContext::with_memories()`
- [x] 2.3 Inject recalled memories as a system message between Layer 5 (Environment) and Layer 6 (Skills) in `assemble_instructions()`
- [x] 2.4 Implement session startup recall: `MemoryManager::load()` → `search_memories(cwd)` → `get_important_memories(0.5)` → take top N → populate `PromptContext.memories`
- [x] 2.5 Wire startup recall into the App initialization path (before first turn is spawned)
- [ ] 2.6 Add unit test: empty memories → no extra system message injected
- [ ] 2.7 Add unit test: non-empty memories → system message appears between Layer 5 and Layer 6

## 3. P2 — Consolidation: AutoDream Gate Trigger

- [x] 3.1 Wire `AutoDreamService::check_and_run()` call into App session startup (before recall, so recall sees consolidated memories)
- [x] 3.2 Simplify `AutoDreamService::run_consolidation()` to delegate to `MemoryManager::consolidate()` instead of `analyze_and_consolidate()`
- [ ] 3.3 Remove `AutoDreamService::load_memories()` and `save_consolidated_memories()` — replaced by MemoryManager
- [ ] 3.4 Remove `services::auto_dream::MemoryEntry` type — use `context::MemoryEntry` throughout
- [ ] 3.5 Clean up AutoDream's legacy file usage: remove writes to `memory.json` and `consolidated_memories.json`
- [ ] 3.6 Add unit test: AutoDream gate passes → `MemoryManager::consolidate()` is called
- [ ] 3.7 Add unit test: AutoDream gate fails (time) → no consolidation

## 4. Dead Code Removal

- [ ] 4.1 Remove `context::context_window` module (ContextWindow, ContextManager, ContextEntry, ContextPriority, ContextSource, ContextSummary, ContextStats)
- [ ] 4.2 Remove `pub mod context_window` from `context/mod.rs`
- [ ] 4.3 Remove `pub use context_window::*` re-exports from `context/mod.rs`
- [ ] 4.4 Verify compilation: `cargo check` passes after all removals
- [ ] 4.5 Verify no remaining references: grep for `ContextWindow`, `ContextManager`, `ContextEntry` across codebase

## 5. Integration Verification

- [ ] 5.1 End-to-end test: spawn agent, trigger compaction, verify memory files appear in `~/.wgenty-code/memory/`
- [ ] 5.2 End-to-end test: session restart with project path, verify recalled memories appear in prompt
- [ ] 5.3 Verify `cargo test` passes for all touched modules
- [ ] 5.4 Verify `cargo clippy` passes with no new warnings
- [ ] 5.5 Manual smoke test: real conversation → compaction → check `~/.wgenty-code/memory/` for extracted memories
