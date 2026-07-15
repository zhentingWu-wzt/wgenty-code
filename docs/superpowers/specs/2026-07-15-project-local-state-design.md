---
comet_change: project-local-state
role: technical-design
canonical_spec: openspec
archived-with: TBD
status: final
---

# Project-Local State & Memory Scoping - Technical Design

## Architecture

```
                      ┌─────────────────────────────────────────────────────┐
                      │                   Agent Loop                         │
                      │                                                      │
                      │  session_start:                                      │
                      │    ④ migrate_legacy_sessions()                       │
                      │       └─ scan ~/.wgenty-code/sessions/               │
                      │       └─ route by project_path -> <proj>/.wgenty-…   │
                      │    ① load (dual-source)                              │
                      │       └─ project: <CWD>/.wgenty-code/memory/         │
                      │       └─ global:  ~/.wgenty-code/memory/             │
                      │       └─ TF-IDF index: project memories only         │
                      │    recall:                                           │
                      │       └─ PromptContext.global_memories -> <global-…  │
                      │       └─ PromptContext.memories      -> <relevant_…  │
                      │                                                      │
                      │  per_turn:                                           │
                      │    normal dialog                                     │
                      │       ↓                                              │
                      │    needs_compaction()?                               │
                      │       ↓ yes                                          │
                      │    ② compaction + ③ extract                          │
                      │       └─ prompt: "summarize + extract memories as    │
                      │          JSON {summary, memories:[{type,scope,       │
                      │          content,importance}]}"                      │
                      │       └─ scope absent -> default "project"           │
                      │       └─ add_memory(entry, scope) -> routed Storage  │
                      │                                                      │
                      │    MemoryContextInjector::inject (per-turn recall)   │
                      │       └─ search_memories() -> project memories only  │
                      │       └─ prepend <memory-context> to user message    │
                      └──────────────────────┬──────────────────────────────┘
                                             │
            ┌────────────────────────────────┴────────────────────────────┐
            │                     MemoryManager                            │
            │                                                              │
            │  project_storage: <CWD>/.wgenty-code/memory/<id>.json       │
            │  global_storage:  ~/.wgenty-code/memory/<id>.json           │
            │  memories: Vec<LoadedMemory { entry, origin: MemoryOrigin }> │
            │  index: TF-IDF over project memories ONLY                    │
            │  search_memories(): project only (index-scoped)              │
            │  global_memories(): all global (for every-turn injection)    │
            │  add_memory(entry, scope): routed by scope                   │
            │  dedup: within same scope only                               │
            └────────────────────────────────────────────────────────────┘

  Storage layout:
    ~/.wgenty-code/                    # global (cross-project)
    ├── memory/<id>.json               #   global memories (every-turn inject)
    ├── sessions/                      #   [migration source] emptied after migrate
    ├── history.jsonl                  #   command history (UNCHANGED)
    ├── settings.json, WGENTY.md, rules/, daemon.token  # (UNCHANGED)
    └── .migrated-project-local        #   migration marker

    <CWD>/.wgenty-code/                # project-local
    ├── sessions/{id}.json             #   project sessions
    └── memory/<id>.json               #   project memories (on-demand recall)
```

## Key Design Decisions

### D1: Scope = physical storage location (no schema field)

Memory scope (project vs global) is determined by **which directory the memory file lives in**, not by a field on `MemoryEntry`. `MemoryManager` tracks each loaded memory's origin via an internal `MemoryOrigin` enum that is never serialized.

**Rationale**: Adding a `scope` field to `MemoryEntry` would require `#[serde(default)]` for backward compat and a backfill migration to rewrite every existing memory file. Physical separation has zero serialization risk--existing `~/.wgenty-code/memory/*.json` files naturally become global memories with no rewrite.

**Trade-off**: `MemoryManager` must manage two `Storage` instances and route `add_memory` by scope. Cross-scope queries require explicit merging. This is acceptable because the two scopes have fundamentally different access patterns (project = TF-IDF recall, global = full injection).

### D2: Dual Storage instances (Approach A)

`MemoryManager` holds two independent `Storage` instances (`project_storage` + `global_storage`). The `Storage` type itself is unchanged--it remains a single-directory per-file backend.

**Alternatives rejected**:
- **Approach B** (single Storage + `project/` `global/` subdirs): would require `Storage` to learn subdirectory routing, a deeper change for the same isolation benefit.
- **Approach C** (`MemoryEntry` + `scope` field + single index): largest change, requires schema migration + runtime filtering, weakest isolation (relies on correct filtering).

### D3: Project root = CWD (no upward search)

The project root is `std::env::current_dir()`, with no upward walk for `.git`/`Cargo.toml` markers. This is simple, predictable, and matches the user's mental model (the main worktree's CWD).

**Trade-off**: Running wgenty-code from a subdirectory creates a separate `<subdir>/.wgenty-code/`. The user has explicitly accepted this.

**Fallback**: When CWD is unavailable or the project-local directory cannot be created, session/memory storage degrades to the global `~/.wgenty-code/` with a warning.

### D4: Global memory injected every turn (not TF-IDF filtered)

Global memories are injected as a `<global-memory>` block in the **system prompt** every turn, NOT filtered by the TF-IDF importance threshold. This fulfills the "permanently remembered" semantic.

**Injection point**: `PromptContext` gains a `global_memories: Vec<String>` field. `assemble_instructions` emits a `<global-memory>` block between Layer 5 (Environment) and Layer 6 (Skills), adjacent to the existing `<relevant_memories>` block.

**Soft cap**: default 50 entries by importance. When exceeded, top-50 are injected and a warning is logged.

**Empty block suppression**: when no global memories exist, no `<global-memory>` block is emitted.

### D5: Global memory coexists with rules/*.md (not a replacement)

Two always-injected cross-project channels coexist with distinct roles:

| Channel | Format | Source | Injection point |
|---------|--------|--------|-----------------|
| `rules/*.md` | plain text, manual | `~/.wgenty-code/rules/*.md` | `<system-reminder>` in user message header |
| Global memory | structured JSON, auto + manual | `~/.wgenty-code/memory/<id>.json` | `<global-memory>` block in system prompt |

**Rationale**: rules/*.md is a proven manual-instruction channel; global memory adds structured, auto-extracted, importance-ranked memories with TF-IDF-free guaranteed injection. They serve different workflows (curated static rules vs. evolving structured memory). Merging them would break existing user workflows.

### D6: Compaction scope classification (trust model + default project)

The compaction extraction prompt is enhanced to request a `scope` field (`"project"` or `"global"`) per memory. The model is instructed to classify cross-project preferences/behavioral conventions as `global` and project-specific decisions/knowledge as `project`.

**Reliability**: when `scope` is absent or unparseable, it defaults to `project` (conservative--prevents accidental leakage of project-specific content to the global pool).

**No type constraint**: all `MemoryType` variants (Preference, Decision, Knowledge, etc.) are eligible for either scope. A type-based allowlist was rejected as too rigid--the model is trusted to judge context relevance.

### D7: Three memory injection paths (clarified)

| Path | Block | Location | When | Scope |
|------|-------|----------|------|-------|
| Session-startup recall | `<relevant_memories>` | system prompt (Layer 5-6) | assembly time | project |
| Per-turn recall | `<memory-context>` | first user message | every turn | project |
| Global injection (NEW) | `<global-memory>` | system prompt (Layer 5-6) | assembly time | global |

Project memory recall continues via the existing two paths (TF-IDF indexed). Global memory adds a third path with guaranteed injection.

### D8: Idempotent legacy migration

On startup, if `~/.wgenty-code/sessions/` is non-empty and the `~/.wgenty-code/.migrated-project-local` marker is absent, `migrate_legacy_sessions()` moves each session to `<project_path>/.wgenty-code/sessions/{id}.json` (using the session's `project_path` field; `None` routes to CWD).

- **Idempotent**: target exists -> skip source.
- **Safe**: on copy failure, original preserved + warning logged.
- **Non-atomic but resumable**: a crash mid-migration leaves a mix of migrated/unmigrated files; the next startup continues (marker only written on full completion).
- **Memories NOT migrated**: existing `~/.wgenty-code/memory/*.json` stays in place, naturally becoming global memories.

## Data Model

### MemoryOrigin (new, internal, not serialized)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOrigin {
    Project,
    Global,
}

struct LoadedMemory {
    entry: MemoryEntry,
    origin: MemoryOrigin,
}
```

### MemoryManager (modified)

```rust
pub struct MemoryManager {
    project_storage: Arc<Storage>,   // <CWD>/.wgenty-code/memory/
    global_storage: Arc<Storage>,    // ~/.wgenty-code/memory/
    sessions: Arc<MemorySessionManager>,  // with_project_root(CWD)
    history: Arc<HistoryManager>,    // UNCHANGED (global)
    consolidation: Arc<ConsolidationEngine>,
    memories: Arc<RwLock<Vec<LoadedMemory>>>,  // was Vec<MemoryEntry>
    index: Arc<RwLock<MemoryIndex>>,  // project memories only
    consolidating: Arc<AtomicBool>,
}
```

### PromptContext (modified)

```rust
pub struct PromptContext {
    // ... existing fields ...
    memories: Vec<String>,          // project memories (startup recall)
    global_memories: Vec<String>,   // NEW: global memories (every-turn inject)
}
```

### SessionManager (modified)

```rust
impl SessionManager {
    pub fn with_project_root(project_root: PathBuf) -> Self {
        let sessions_dir = project_root.join(".wgenty-code").join("sessions");
        // ... create dir, fall back to global on failure ...
    }
}
```

## API Changes

### `src/utils/mod.rs` (new functions)

```rust
pub fn project_local_dir(project_root: &Path) -> PathBuf;
pub fn project_memory_dir(project_root: &Path) -> PathBuf;
pub fn project_sessions_dir(project_root: &Path) -> PathBuf;
pub fn global_memory_dir() -> PathBuf;
pub fn current_project_root() -> PathBuf;  // wraps current_dir(), falls back to config_dir()
```

### `MemoryManager`

- `with_settings(settings, project_root)` -- dual Storage init
- `load()` -- dual-source load with origin tagging
- `search_memories(query)` -- project only (index-scoped)
- `global_memories() -> Vec<MemoryEntry>` -- all global memories
- `add_memory(entry, scope)` -- routed by scope; dedup within scope
- `status()` -- report project + global counts separately

### `MemoryContextInjector`

- `inject(messages, manager, ...)` -- unchanged (project recall via search_memories)
- `inject_global(manager) -> String` -- NEW: returns `<global-memory>` block

### `ConsolidationEngine`

- Extraction prompt: add `scope` field to JSON schema
- Parse `scope`, default `project` on absence
- Route extracted memories via `add_memory(entry, scope)`

## Edge Cases

1. **CWD unwritable**: project storage falls back to global directory + warning.
2. **CWD == home directory**: project root coincides with global root. Warn; project memories written to global `memory/` (merged pool). The `MemoryOrigin::Project` tag is still tracked in-memory for correct recall behavior, but files share the global directory.
3. **CWD unavailable** (`current_dir()` fails): fall back to `config_dir()` as project root.
4. **Migration crash**: non-atomic but idempotent; marker only written on completion.
5. **Global memory overflow**: soft cap 50 by importance; excess logged.
6. **Concurrent processes in same project**: same risk as current global storage (file-level). `Arc<RwLock>` protects in-memory state; file writes are last-writer-wins (acceptable, matches current behavior).

## Non-goals

- Command history project-localization (stays global).
- Upward project-root search.
- Subagent independent scope (subagents inherit main agent's project root).
- Cross-project memory sharing.
- Replacing rules/*.md with global memory.

## Open Questions (deferred to build)

- Global memory soft cap exact value (50 default, tune after profiling).
- `<global-memory>` block prompt wording (refine in build).
- Whether `memory status` CLI should show storage paths (likely yes).
