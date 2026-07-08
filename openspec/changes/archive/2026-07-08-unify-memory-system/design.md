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

**Rationale**: Instead of two separate LLM calls (one for summary, one for memory extraction), enhance the existing compaction prompt to return JSON with both fields. Zero added latency, zero added cost.

**Alternatives considered**:
- Two separate calls: Rejected — doubles cost and latency
- Structured output via tool_use: Rejected — the summarization model is called with `plan_mode=true` (no tools), changing this would require tool definitions in the prompt

### Decision 4: AutoDream simplified to delegate to MemoryManager

**Rationale**: AutoDreamService currently has its own MemoryEntry type, its own file format (`memory.json`), and its own consolidation logic (`analyze_and_consolidate`). All of this is inferior to MemoryManager's existing infrastructure (per-file Storage, Jaccard-similarity ConsolidationEngine with merge + insight generation). AutoDream is reduced to its true role: a time-gate trigger.

**Alternatives considered**:
- Keep both systems: Rejected — two sources of truth for memories
- Remove AutoDream entirely: Rejected — the 3-gate trigger (24h + 5 sessions + lock) is valuable

### Decision 5: Remove ContextWindow entirely

**Rationale**: `ContextWindow` maintains a `VecDeque<ContextEntry>` with token counting and priority-based eviction. But the agent loop already has `conversation_history: Vec<ChatMessage>` and compaction already serves as the eviction policy. ContextWindow is a second, unused copy of the same concept.

**Alternatives considered**:
- Wire ContextWindow as a pre-filter before compaction: Rejected — adds complexity without benefit; `needs_compaction()` already checks token count

## Architecture

```
                      ┌─────────────────────────────────┐
                      │         Agent Loop (活路径)       │
                      │                                  │
    会话启动 ─────────┤  ① recall: 检索记忆 → 注入 prompt │
                      │                                  │
                      │  正常对话 (Vec<ChatMessage>)      │
                      │       ↓                          │
                      │  ② compaction (已有，刚修过)       │
                      │       ↓                          │
                      │  ③ extract: 摘要前提取重要条目     │
                      │       ↓                          │
                      │  summarize: 生成压缩摘要          │
                      │                                  │
                      │  ④ consolidate: 门控通过时压缩    │
                      └──────────────┬───────────────────┘
                                     │
                      ┌──────────────▼───────────────────┐
                      │     MemoryManager (唯一存储)       │
                      │  MemoryEntry (typed+importance)   │
                      │  Storage (~/.wgenty-code/memory/) │
                      │  ConsolidationEngine (去重+过滤)   │
                      │  search_memories (关键词→语义)     │
                      └──────────────────────────────────┘
```

### Component Changes

**AgentLoop** gains:
- `memory_manager: Arc<MemoryManager>` field
- In `do_auto_compact()`: after receiving summary, parse memories from the response and call `memory_manager.add_memory()`
- Enhanced compaction prompt: "Summarize the conversation AND extract key information worth long-term memory (decisions, errors, preferences, insights). Output as JSON: {summary: string, memories: [{type, content, importance}]}"

**PromptContext** gains:
- `memories: Vec<String>` field — pre-formatted system message lines for recalled memories

**assemble_instructions()** gains:
- Between Layer 5 (Environment) and Layer 6 (Skills): if `context.memories` is non-empty, inject a system message with recalled memories

**App::spawn_agent_turn()** changes:
- Passes `Arc<MemoryManager>` to AgentLoop (currently not passed)

**App session startup** gains:
- `AutoDreamService::check_and_run()` call before first turn
- `MemoryManager::load()` + `search_memories(cwd)` + `get_important_memories(0.5)` → populate `PromptContext.memories`

**AutoDreamService** changes:
- `run_consolidation()` delegates to `MemoryManager::consolidate()` instead of its own `analyze_and_consolidate()`
- `load_memories()` and `save_consolidated_memories()` removed (replaced by MemoryManager's Storage)

## Risks / Trade-offs

- **[JSON parsing failure in LLM output]** → Mitigation: graceful degradation — if JSON parse fails, use the full response as summary only (existing behavior), log a warning, no memory extraction this cycle
- **[Memory accumulation without bounds]** → Mitigation: ConsolidationEngine's `max_memories` config (default 10000) caps storage; `should_keep()` filters by importance and age
- **[Compaction prompt size increase]** → Mitigation: The enhanced prompt adds ~100 chars; negligible vs. the transcript being summarized (potentially 100K+ chars)
- **[Keyword search recall quality]** → Mitigation: Acceptable for P0/P1; `embedding` field already exists on MemoryEntry for future semantic search upgrade
- **[Startup latency from memory loading]** → Mitigation: MemoryManager loads from disk (per-file JSON), typically <100ms for hundreds of memories; consolidation only runs when gate passes (rarely)

## Migration Plan

1. Add `memory_manager: Arc<MemoryManager>` to AgentLoop (no behavior change yet)
2. Enhance compaction prompt + parse memories from response
3. Add `memories` field to PromptContext + injection in assemble_instructions
4. Wire session startup recall: load → search → populate PromptContext
5. Wire AutoDreamService::check_and_run() into session startup
6. Simplify AutoDreamService to delegate to MemoryManager
7. Remove ContextWindow, auto_dream::MemoryEntry, legacy JSON files
8. All steps are additive until step 7; rollback is `git revert`

## Open Questions

*(None — resolved during the 4-round codebase tracing)*
