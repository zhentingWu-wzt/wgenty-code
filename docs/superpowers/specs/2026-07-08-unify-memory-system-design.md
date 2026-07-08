---
comet_change: unify-memory-system
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-08-unify-memory-system
status: final
---

# Agent Memory System — Technical Design

## Architecture

```
                      ┌─────────────────────────────────────────────────────┐
                      │                   Agent Loop                         │
                      │                                                      │
                      │  session_start:                                      │
                      │    ④ consolidate (AutoDreamService::check_and_run)   │
                      │       └─ Gate pass? → MemoryManager::consolidate()   │
                      │    ① recall                                         │
                      │       └─ load() → search_memories(cwd) → top N       │
                      │       └─ inject into PromptContext.memories          │
                      │                                                      │
                      │  per_turn:                                           │
                      │    normal dialog (Vec<ChatMessage>, streaming SSE)   │
                      │       ↓                                              │
                      │    needs_compaction()?                               │
                      │       ↓ yes                                          │
                      │    ② compaction + ③ extract                         │
                      │       └─ chat() (non-streaming)                      │
                      │       └─ prompt: "summarize + extract memories as    │
                      │          JSON {summary, memories:[{type,content,     │
                      │          importance}]}"                              │
                      │       └─ parse JSON → summary + add_memory()         │
                      │       └─ fallback: JSON fail → summary only          │
                      │                                                      │
                      └──────────────────────┬──────────────────────────────┘
                                             │
                      ┌──────────────────────▼──────────────────────────────┐
                      │                MemoryManager                          │
                      │                                                      │
                      │  Storage: ~/.wgenty-code/memory/<id>.json            │
                      │  ConsolidationEngine: Jaccard >0.8 dedup + merge     │
                      │  search_memories: keyword match (content + tags)     │
                      │  get_important_memories: importance >= threshold     │
                      └─────────────────────────────────────────────────────┘
```

## Key Design Decisions

### D1: Memory extraction during compaction (not per-turn)

Compaction is the natural extraction point — information is about to be lost, so extraction is salvage. Enhancing the existing LLM call avoids adding latency or cost.

### D2: Non-streaming compaction for JSON integrity

`do_auto_compact()` currently uses `chat_stream_with_plan()` (streaming SSE). Changed to `chat()` (non-streaming) to guarantee complete JSON for dual-output parsing. Normal dialog remains streaming. UX impact: negligible — compaction status bar shows "compacting..." either way.

### D3: One LLM call, dual output

The compaction system prompt is enhanced from "Summarize this conversation" to "Summarize this conversation AND extract key memories as JSON: {summary: string, memories: [{type, content, importance}]}". One round-trip, two products.

### D4: Keyword-based recall (zero-latency)

`MemoryManager::search_memories()` performs case-insensitive substring matching on content and tags. Sufficient for project-name recall. `embedding` field on `MemoryEntry` is reserved for future semantic search upgrade.

### D5: AutoDream reduced to gate trigger

AutoDreamService's 3-gate mechanism (24h + 5 sessions + lock) is preserved but its custom consolidation logic (`analyze_and_consolidate`, custom `MemoryEntry`, `memory.json`/`consolidated_memories.json`) is replaced by delegation to `MemoryManager::consolidate()`.

### D6: ContextWindow removed

`context::ContextWindow` maintains a `VecDeque<ContextEntry>` with token tracking — fully redundant with the existing `conversation_history: Vec<ChatMessage>` and compaction-based eviction.

## Component Changes

### AgentLoop (`src/tui/agent/mod.rs`)

- **New field**: `memory_manager: Arc<MemoryManager>`
- **New parameter**: `AgentLoop::new()` accepts `Arc<MemoryManager>`

### compaction.rs (`src/tui/agent/compaction.rs`)

- **Modified**: `do_auto_compact()` replaces `chat_stream_with_plan()` with non-streaming `chat()`
- **Modified**: Summarization system prompt enhanced to request JSON `{summary, memories}`
- **New**: After receiving response, parse JSON; call `memory_manager.add_memory()` for each entry
- **New**: JSON parse failure → fallback to full text as summary, log warning

### PromptContext (`src/prompts/mod.rs`)

- **New field**: `memories: Vec<String>` (pre-formatted system message lines)
- **New method**: `with_memories()`

### assemble_instructions (`src/prompts/mod.rs`)

- **New**: Between Layer 5 (Environment) and Layer 6 (Skills), inject `context.memories` as a system message when non-empty

### App::spawn_agent_turn (`src/tui/app/turn.rs`)

- **Modified**: Accept and pass `Arc<MemoryManager>` to `AgentLoop::new()`

### App session startup

- **New**: Call `auto_dream_service.check_and_run()` before first turn
- **New**: Call `memory_manager.load()` → `search_memories(cwd_project_name)` → `get_important_memories(0.5)` → populate `PromptContext.memories`

### AutoDreamService (`src/services/auto_dream.rs`)

- **Modified**: `run_consolidation()` delegates to `MemoryManager::consolidate()`
- **Removed**: `load_memories()`, `save_consolidated_memories()`, `analyze_and_consolidate()`
- **Removed**: `services::auto_dream::MemoryEntry` type
- **Removed**: Writes to `memory.json` and `consolidated_memories.json`

### Context module (`src/context/`)

- **Removed**: `context_window.rs` (ContextWindow, ContextManager, ContextEntry, ContextPriority, ContextSource, ContextSummary, ContextStats)
- **Modified**: `mod.rs` — remove `pub mod context_window` and related re-exports

## Data Flow

### Extraction (Producer)

```
do_auto_compact()
  ├─ Save transcript to ~/.wgenty-code/transcripts/
  ├─ split_for_compaction() → (to_summarize, tail)
  ├─ Build enhanced prompt
  │    "Summarize the conversation AND extract key memories as JSON:
  │     {summary: string, memories: [{type, content, importance}]}"
  ├─ client.chat(prompt) ← NON-STREAMING
  ├─ Parse JSON
  │   ├─ Success:
  │   │   ├─ summary → assemble_post_compaction_history()
  │   │   └─ memories[] → memory_manager.add_memory() per entry
  │   └─ Failure:
  │       └─ Full text → summary (existing behavior), log warning
  └─ Fire AppEvent::ContextCompacted
```

### Recall (Consumer)

```
Session startup
  ├─ auto_dream_service.check_and_run()  // P2: consolidate first
  ├─ memory_manager.load()               // Load all from disk
  ├─ memory_manager.search_memories(project_name)
  ├─ Filter: importance >= 0.5
  ├─ Take top N (configurable, default 5)
  ├─ Format as system message lines
  └─ prompt_context.memories = formatted_lines

assemble_instructions()
  ├─ Layer 1..5 (existing)
  ├─ IF context.memories non-empty:
  │     push system("<relevant_memories>\n...\n</relevant_memories>")
  ├─ Layer 6.. (existing)
```

### Consolidation (Maintenance)

```
Session startup (before recall)
  └─ auto_dream_service.check_and_run()
       ├─ Gate 1: hours since last >= 24? → continue
       ├─ Gate 2: new sessions >= 5? → continue
       ├─ Gate 3: no active lock? → continue
       └─ memory_manager.consolidate()
            ├─ Sort by importance desc
            ├─ Filter: should_keep() → importance >= 0.3 or age < 24h or is Knowledge/Preference
            ├─ Dedup: Jaccard similarity > 0.8 → merge group
            ├─ Merge: combine content, take max importance + 0.1, dedup tags
            ├─ Insight: >= 10 of same type → pattern insight (0.7)
            ├─ Insight: >= 3 errors → error pattern (0.8)
            ├─ Cap: max_memories (default 10000)
            └─ Update in-memory + save all
```

## Error Handling

| Scenario | Behavior |
|----------|----------|
| LLM returns non-JSON | Full text used as summary; warning logged; no memory extraction |
| JSON valid but no `memories` key | Summary extracted from `summary` field; no memories extracted |
| `add_memory()` fails (disk full) | Warning logged; compaction continues with summary only |
| `memory_manager.load()` fails | Empty memories; prompt injection skipped; session continues |
| AutoDream lock acquisition fails | Consolidation skipped; retry next session |
| ConsolidationEngine error | Warning logged; memories unchanged |

## Implementation Order

| Phase | Content | Value |
|-------|---------|-------|
| P0 | Producer: AgentLoop holds MemoryManager, compaction extracts memories | Memories start accumulating |
| P1 | Consumer: recall + prompt injection | User sees cross-session memory |
| P2 | Maintenance: AutoDream gate wired + simplified | Memories don't grow unbounded |
| Cleanup | Remove ContextWindow, legacy types, legacy files | Dead code gone |
