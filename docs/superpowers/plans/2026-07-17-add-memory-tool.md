---
archived-with: 2026-07-17-add-memory-tool
status: final
---
# Implementation Plan: add-memory-tool

---
change: add-memory-tool
design-doc: openspec/changes/add-memory-tool/design.md
base-ref: 5954129b4cead84e34fcd18f7dc75ae147f084ee
---

## Overview

Add a `memory_add` tool that lets the agent proactively write memory entries via `MemoryManager::add_memory()`, without waiting for context compaction. The tool is available to all agents (root + subagents).

## Prerequisites

- Rust 2021 edition (MSRV 1.75+)
- `cargo build` passes on base-ref `5954129`
- CodeGraph ready for symbol lookup

## Key Design Decisions

- **D1b**: Modify `add_memory()` return type from `Result<()>` to `Result<MemoryAddResult { id, merged }>`. Zero-breaking (all callers use `if let Err` or `.unwrap()`).
- **D2**: Use existing `register(&self)` method, NOT a new constructor. `memory_manager` is in scope at `src/cli/headless_runtime.rs:235`.
- **D5**: `memory_add` available to ALL agents. No subagent filtering needed.
- **D4**: New `## Proactive memory capture` section in `src/prompts/base.md`.

## Tasks

### Task 1: Add `MemoryAddResult` + modify `add_memory()` return type

**Files:** `src/context/mod.rs`

1. Add `MemoryAddResult` struct after `MemoryEntry`:
   ```rust
   /// Result of adding a memory: the stored entry's id and whether it was merged.
   #[derive(Debug, Clone)]
   pub struct MemoryAddResult {
       pub id: String,
       pub merged: bool,
   }
   ```

2. Change `add_memory()` signature from `Result<()>` to `Result<MemoryAddResult>`.

3. In the dedup-merge branch (where `ConsolidationEngine::merge_into` is called): return `Ok(MemoryAddResult { id: existing_entry.id.clone(), merged: true })`.

4. In the new-entry branch: return `Ok(MemoryAddResult { id: entry.id.clone(), merged: false })`.

5. Run `cargo check` to verify all callers still compile (they use `if let Err` or `.unwrap()`, so no changes needed).

**Verification:** `cargo check` passes. Existing tests in `src/context/mod.rs` and `src/context/inject.rs` still compile.

### Task 2: Create `MemoryAddTool` implementation

**Files:** `src/tools/meta/memory_add.rs` (new), `src/tools/meta/mod.rs`

1. Create `src/tools/meta/memory_add.rs` with:
   - `MemoryAddTool` struct holding `Arc<MemoryManager>`
   - `new(memory: Arc<MemoryManager>) -> Self`
   - `Tool` trait impl: `name()` = "memory_add", `is_read_only()` = false
   - `input_schema()`: content (string, required), memory_type (enum, default "Knowledge"), scope (enum, default "project"), tags (array, optional), importance (float, default 0.5)
   - `execute()`: parse params → build `MemoryEntry` → call `add_memory()` → return JSON `{ success: true, memory_id, merged }`
   - Error handling: missing content → `ToolError { code: "missing_content" }`, invalid memory_type → `ToolError { code: "invalid_memory_type" }`

2. Add `pub mod memory_add;` to `src/tools/meta/mod.rs`.

3. Add `pub use memory_add::MemoryAddTool;` to re-export.

**Verification:** `cargo check` passes. Tool compiles and implements `Tool` trait correctly.

### Task 3: Register `MemoryAddTool` via `register()`

**Files:** `src/cli/headless_runtime.rs`

1. After line 235 (`let registry = Arc::new(ToolRegistry::new().with_settings(&settings));`), add:
   ```rust
   registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())));
   ```

2. Add import: `use crate::tools::meta::MemoryAddTool;` (or appropriate path).

3. Verify `memory_manager` is in scope (it's used at line 240 for `ApiCompactor`).

**Verification:** `cargo check` passes. `memory_add` appears in tool list.

### Task 4: Add prompt guidance to `base.md`

**Files:** `src/prompts/base.md`

1. Add new `## Proactive memory capture` section under `# Tool Guidelines` (after `## Context management`):
   ```markdown
   ## Proactive memory capture

   When you identify something worth remembering long-term (a lesson learned, architecture decision, user preference, key file path, bug fix), proactively call `memory_add` to persist it immediately. Choose `scope`: `global` for cross-project insights (user preferences, workflow habits, correction lessons); `project` for project-specific content (architecture, paths, conventions). Default to `project` if unsure.
   ```

2. Add `memory_add` entry to `## Context management` section:
   ```markdown
   - **`memory_add`**: Proactively write a memory entry (lesson, decision, preference) to persistent storage. Specify scope: `project` or `global`.
   ```

3. Add row to "When to use each tool" table:
   ```markdown
   | Save a memory long-term | `memory_add` |
   ```

**Verification:** `cargo check` passes (base.md is `include_str!`).

### Task 5: Unit tests

**Files:** `src/tools/meta/memory_add.rs` (test module), `src/context/mod.rs` (existing test module)

1. Test `add_memory()` returns `MemoryAddResult` with `merged: false` for new entry.
2. Test `add_memory()` returns `MemoryAddResult` with `merged: true` + existing id for dedup.
3. Test `memory_add` tool creates new project memory (verify file exists in `.wgenty-code/memory/`).
4. Test `memory_add` tool creates new global memory (verify file exists in `~/.wgenty-code/memory/`).
5. Test `memory_add` with similar content triggers merge (verify `merged: true` in output).
6. Test missing content returns error with code "missing_content".
7. Test invalid memory_type returns error with code "invalid_memory_type".
8. Test `is_read_only()` returns false.

**Verification:** `cargo test memory_add` passes. `cargo test add_memory` passes.

### Task 6: Validation

1. `cargo fmt -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test --all`
4. Manual: `cargo run -- repl`, ask agent to remember something, verify `memory status` shows new entry.

**Verification:** All CI checks pass.
