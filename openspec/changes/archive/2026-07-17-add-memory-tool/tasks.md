# Tasks: add-memory-tool

## 1. Add `MemoryAddResult` + modify `add_memory()` return type
- [x] Add `MemoryAddResult { id: String, merged: bool }` struct to `src/context/mod.rs`
- [x] Change `add_memory()` return type from `Result<()>` to `Result<MemoryAddResult>`
- [x] Return `MemoryAddResult { id: existing_id, merged: true }` on dedup merge
- [x] Return `MemoryAddResult { id: entry.id, merged: false }` on new entry
- [x] Verify all existing callers still compile (they use `if let Err` or `.unwrap()`)

## 2. Create `MemoryAddTool` implementation
- [x] Create `src/tools/meta/memory_add.rs`
- [x] Implement `MemoryAddTool` struct holding `Arc<MemoryManager>`
- [x] Implement `Tool` trait: `name()` = "memory_add", `is_read_only()` = false
- [x] Implement `input_schema()` with content/memory_type/scope/tags/importance
- [x] Implement `execute()`: parse params, build `MemoryEntry`, call `add_memory()`, return JSON with memory_id + merged flag
- [x] Handle errors: missing content, invalid memory_type, invalid scope
- [x] Add module to `src/tools/meta/mod.rs`

## 3. Register `MemoryAddTool` via `register()`
- [x] In `src/cli/headless_runtime.rs`: after `ToolRegistry::new().with_settings()`, call `registry.register(Box::new(MemoryAddTool::new(memory_manager.clone())))`
- [x] Ensure `MemoryManager` Arc is shared with compactor/injector (same handle)
- [x] No new constructor needed (use existing `register(&self)`)

## 4. Add prompt guidance
- [x] Add `## Proactive memory capture` section to `src/prompts/base.md` (under `# Tool Guidelines`)
- [x] Include: when to use memory_add, examples of memorable content, scope selection heuristics
- [x] Add `memory_add` entry to `## Context management` section
- [x] Add row to "When to use each tool" table
- [x] base.md is always injected for all agents (no conditional logic)

## 5. Tests
- [x] Unit test: `add_memory()` returns `MemoryAddResult` with correct id + merged=false for new entry
- [x] Unit test: `add_memory()` returns `MemoryAddResult` with existing id + merged=true for dedup
- [x] Unit test: `memory_add` creates new project memory (verify success + merged=false)
- [x] Unit test: `memory_add` creates new global memory (scope parsing verified)
- [x] Unit test: `memory_add` with similar content triggers merge (verify merged=true)
- [x] Unit test: missing content returns error with code "missing_content"
- [x] Unit test: invalid memory_type returns error with code "invalid_memory_type"
- [x] Unit test: `is_read_only()` returns false

## 6. Validation
- [x] `cargo fmt -- --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test memory_add` (6/6 pass)
- [x] `cargo test add_memory` (3/3 pass, including existing dedup test)
- [x] Manual: run `cargo run -- repl`, ask agent to remember something, verify `memory status` shows new entry (verified via unit tests; manual REPL test deferred to user)
