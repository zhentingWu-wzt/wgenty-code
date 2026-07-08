# Comet Design Handoff

- Change: unify-memory-system
- Phase: design
- Mode: compact
- Context hash: 628d5f357e25f4a075000bf0d9e1c72e985f864660373f3c208c4c933910734f

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/unify-memory-system/proposal.md

- Source: openspec/changes/unify-memory-system/proposal.md
- Lines: 1-31
- SHA256: 15da10d00aa12b1241dae8c11c0ac31f6ea8d417e0b0502418aafed050c498e7

```md
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
```

## openspec/changes/unify-memory-system/design.md

- Source: openspec/changes/unify-memory-system/design.md
- Lines: 1-175
- SHA256: b6abaf555fbb2a972ccdf8eeee38b5e49b579f1dde1cf82a14de5e9a99150340

[TRUNCATED]

```md
## Context

The codebase contains four memory-related modules built in isolation:

| Module | Status | Role |
|--------|--------|------|
| `agent/compaction.rs` | **Alive** — on hot path | Summarizes conversation history via LLM |
| `context/mod.rs` (MemoryManager) | Orphaned — only CLI commands use it | Storage, search, consolidation |
| `services/auto_dream.rs` | Orphaned — `check_and_run()` never called | Time-gated memory consolidation |
| `context/context_window.rs` | Orphaned — never used | Token-aware sliding window |

The goal is to wire them into a single Agent Loop with four phases: recall → compaction → extract → consolidate.

### Current State Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│  HOT PATH (wired)                                                │
│                                                                  │
│  App::spawn_agent_turn()                                         │
│    └─ AgentLoop::process_input()                                │
│         └─ run_agent_loop()                                      │
│              └─ needs_compaction() → do_auto_compact()          │
│                   ├─ split_for_compaction()                      │
│                   ├─ chat_stream_with_plan (LLM summary)         │
│                   └─ assemble_post_compaction_history()          │
│                                                                  │
│  assemble_instructions() → 8-layer system prompt                 │
│    Layer 1: Base | 1b: Runtime Ctx | 2: Permissions              │
│    Layer 3: Developer | 4: Collaboration                         │
│    Layer 5: Environment ←────────── RECALL INJECTION POINT       │
│    Layer 6: Skills | 7/8: (removed → reminder channel)          │
│                                                                  │
├─────────────────────────────────────────────────────────────────┤
│  COLD PATH (orphaned)                                            │
│                                                                  │
│  MemoryManager::add_memory() ← nobody calls this                 │
│  MemoryManager::search_memories() ← nobody calls this            │
│  MemoryManager::consolidate() ← nobody calls this (except CLI)   │
│  AutoDreamService::check_and_run() ← nobody calls this           │
│  ContextWindow — never wired to anything                         │
└─────────────────────────────────────────────────────────────────┘
```

## Goals / Non-Goals

**Goals:**
1. Wire MemoryManager into AgentLoop so memories are produced during compaction (P0)
2. Inject recalled memories into the system prompt at session start (P1)
3. Trigger AutoDream consolidation at session start so memories don't grow unbounded (P2)
4. Remove dead code: ContextWindow, auto_dream::MemoryEntry, legacy JSON file formats
5. Reuse existing infrastructure — no new pipelines or storage formats

**Non-Goals:**
- Embedding generation and semantic search (P4, deferred)
- `memory_search` tool for agents (P3, deferred to follow-up change)
- Changing compaction threshold logic (already fixed)
- Changing the 8-layer prompt structure (only inserting between layers)

## Decisions

### Decision 1: Extract memories during compaction, not after every turn

**Rationale**: Adding an LLM call after every user turn would double latency and cost. Compaction is the natural extraction point — information is about to be lost, so extraction is salvage. The existing summarization LLM call is enhanced to produce dual output (summary + memories) in one round-trip.

**Alternatives considered**:
- Per-turn extraction: Rejected — too expensive (1 extra LLM call per turn)
- Post-session batch extraction: Rejected — loses context that was already compacted away
- Separate extraction model: Rejected — adds complexity for marginal quality gain

### Decision 2: Keyword-based recall at session start, not semantic search

**Rationale**: Keyword matching on project path/name is zero-latency and sufficient for "project name + tech stack" recall. The `MemoryEntry.embedding` field already exists for future semantic search upgrade.

**Alternatives considered**:
- Full semantic search at session start: Rejected — requires embedding API call, adds latency to startup
- LLM-based relevance ranking: Rejected — same latency concern

### Decision 3: One LLM call, dual output (summary + memories)

```

Full source: openspec/changes/unify-memory-system/design.md

## openspec/changes/unify-memory-system/tasks.md

- Source: openspec/changes/unify-memory-system/tasks.md
- Lines: 1-46
- SHA256: dee7b31d3aa6799f7f49e32e639a0c43a5c29ec338c98144037d1bfc125651a5

```md
## 1. P0 — Producer: Memory Extraction in Compaction

- [ ] 1.1 Add `memory_manager: Arc<MemoryManager>` field to `AgentLoop` struct and `AgentLoop::new()`
- [ ] 1.2 Enhance compaction system prompt in `do_auto_compact()` to request dual output (summary + memories) in JSON format
- [ ] 1.3 Parse JSON response after receiving summary — extract `memories` array and persist each via `memory_manager.add_memory()`
- [ ] 1.4 Implement graceful degradation: on JSON parse failure, use full response as summary only, log warning, skip memory extraction
- [ ] 1.5 Update `App::spawn_agent_turn()` and `App::spawn_compact_turn()` to pass `Arc<MemoryManager>` to `AgentLoop::new()`
- [ ] 1.6 Add unit test: verify enhanced prompt includes JSON output format instruction
- [ ] 1.7 Add unit test: verify JSON parse success path calls `add_memory`
- [ ] 1.8 Add unit test: verify JSON parse failure falls back gracefully

## 2. P1 — Consumer: Memory Recall at Session Start

- [ ] 2.1 Add `memories: Vec<String>` field to `PromptContext`
- [ ] 2.2 Add builder method `PromptContext::with_memories()`
- [ ] 2.3 Inject recalled memories as a system message between Layer 5 (Environment) and Layer 6 (Skills) in `assemble_instructions()`
- [ ] 2.4 Implement session startup recall: `MemoryManager::load()` → `search_memories(cwd)` → `get_important_memories(0.5)` → take top N → populate `PromptContext.memories`
- [ ] 2.5 Wire startup recall into the App initialization path (before first turn is spawned)
- [ ] 2.6 Add unit test: empty memories → no extra system message injected
- [ ] 2.7 Add unit test: non-empty memories → system message appears between Layer 5 and Layer 6

## 3. P2 — Consolidation: AutoDream Gate Trigger

- [ ] 3.1 Wire `AutoDreamService::check_and_run()` call into App session startup (before recall, so recall sees consolidated memories)
- [ ] 3.2 Simplify `AutoDreamService::run_consolidation()` to delegate to `MemoryManager::consolidate()` instead of `analyze_and_consolidate()`
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
```

## openspec/changes/unify-memory-system/specs/agent-memory/spec.md

- Source: openspec/changes/unify-memory-system/specs/agent-memory/spec.md
- Lines: 1-49
- SHA256: c8bccd85e4c21cdbc51c7dcd33dbce4bf0b6482304b0921b1e9dca4401f77a23

```md
# Agent Memory

End-to-end cross-session memory for the agent loop: extraction during compaction, persistent storage, recall at session start, and time-gated consolidation.

## Requirements

### Memory Extraction (Producer)

- **REQ-AM-001**: During `do_auto_compact()`, the LLM summarization prompt SHALL be enhanced to request both a conversation summary and extracted memory entries in a single response.
- **REQ-AM-002**: The enhanced prompt SHALL request output in JSON format: `{summary: string, memories: [{type: "decision"|"error"|"preference"|"insight"|"knowledge"|"task", content: string, importance: float}]}`.
- **REQ-AM-003**: On successful JSON parse, extracted memories SHALL be persisted via `MemoryManager::add_memory()`.
- **REQ-AM-004**: On JSON parse failure, the system SHALL fall back gracefully to using the full response as summary only (existing behavior), log a warning, and skip memory extraction for this compaction cycle.
- **REQ-AM-005**: Memory extraction SHALL NOT add an additional LLM call — it reuses the existing compaction LLM call.

### Memory Storage

- **REQ-AM-006**: All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend (`~/.wgenty-code/memory/<id>.json`).
- **REQ-AM-007**: Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, embedding.
- **REQ-AM-008**: The legacy `services::auto_dream::MemoryEntry` type SHALL be removed.
- **REQ-AM-009**: The legacy `~/.wgenty-code/memory.json` and `~/.wgenty-code/consolidated_memories.json` files SHALL no longer be written.

### Memory Recall (Consumer)

- **REQ-AM-010**: At session startup, `MemoryManager::load()` SHALL load all memories from disk.
- **REQ-AM-011**: At session startup, `MemoryManager::search_memories(cwd_project_name)` SHALL retrieve memories matching the current project.
- **REQ-AM-012**: Memories below importance threshold 0.5 SHALL be filtered out before injection.
- **REQ-AM-013**: Recalled memories SHALL be injected into the prompt as a system message between Layer 5 (Environment) and Layer 6 (Skills).
- **REQ-AM-014**: Memory recall SHALL NOT add latency from LLM calls — it is keyword-based only.

### Memory Consolidation

- **REQ-AM-015**: `AutoDreamService::check_and_run()` SHALL be called at session startup.
- **REQ-AM-016**: The three-gate check (24h since last consolidation, >= 5 new sessions, no active lock) SHALL remain unchanged.
- **REQ-AM-017**: When gates pass, consolidation SHALL delegate to `MemoryManager::consolidate()`, which uses `ConsolidationEngine` for deduplication and filtering.
- **REQ-AM-018**: Consolidation SHALL use ConsolidationEngine's Jaccard similarity (>0.8 threshold) for duplicate detection, importance threshold (0.3) for filtering, and merge logic for similar memories.

### Dead Code Removal

- **REQ-AM-019**: `context::ContextWindow` and `context::ContextManager` SHALL be removed — redundant with `conversation_history: Vec<ChatMessage>`.
- **REQ-AM-020**: `services::auto_dream::MemoryEntry` SHALL be removed — replaced by `context::MemoryEntry`.
- **REQ-AM-021**: `AutoDreamService::load_memories()` and `save_consolidated_memories()` SHALL delegate to `MemoryManager` instead of reading/writing custom files.

## Acceptance Scenarios

1. **Producer**: After a long conversation triggers compaction, `~/.wgenty-code/memory/` contains new JSON files with extracted memories containing correct type, content, and importance.
2. **Recall**: On next session start in the same project, the injected prompt contains a system message with relevant memories from prior sessions.
3. **Consolidation**: After 24h and 5+ sessions, AutoDream triggers and similar memories are merged; memory count decreases.
4. **Graceful degradation**: If the LLM returns malformed JSON during compaction, the summary still works, no memories are extracted, and no crash occurs.
5. **Dead code gone**: `context::ContextWindow`, `auto_dream::MemoryEntry`, `memory.json`, and `consolidated_memories.json` no longer exist in the codebase or are written to disk.
```

