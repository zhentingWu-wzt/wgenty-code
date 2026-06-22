---
change: comet-workflow-compat
design-doc: docs/superpowers/specs/2026-06-22-comet-workflow-compat-design.md
base-ref: 3b60351444a8d4a0704fb9adab58a084c02aec9b
---

# Comet Workflow Compatibility Implementation Plan

**Goal:** Make Wgenty Code fully compatible with the Comet workflow by unifying skill path resolution, completing hook lifecycle coverage, adding a Comet phase guard, extending worktree operations, adding per-tool timeouts, and enabling subagent orchestration with Comet context.

**Architecture:** A new `src/comet/` module reads `.comet.yaml` state and guards tool execution by phase. A unified `SkillRootResolver` eliminates scattered path construction. Hook lifecycle events (SessionStart/End, UserPromptSubmit, Stop, PermissionRequest, Notification) are completed across the TUI app. Worktree operations are added to the existing `git_operations` tool. Configurable timeouts replace the hardcoded `task`/other ternary. Subagent `comet_context` enables implementer-review flow.

**Tech Stack:** Rust (tokio async runtime), serde/serde_json, chrono, tracing, ratatui.

## Global Constraints

- Language: Rust (edition 2021)
- No new external crate dependencies beyond what is already in Cargo.toml
- All hook fires are async (tokio::spawn), non-blocking, with configurable timeout defaulting to 30s
- Malformed .comet.yaml or absent active change must not crash — fall back to allow-all
- Read-only tools always allowed in all phases
- Commit after each task using conventional commits: feat(comet): ...

---

## Task 1 — Skill path compat — Root resolver & wiring

### 1.1 Add `UserClaude` variant to `ExternalSkillSource`
- File: `src/knowledge/external.rs`
- Add `UserClaude { root: PathBuf }` variant with priority_rank=2, label(), root()

### 1.2 Create `src/knowledge/root_resolver.rs`
- `SkillRootResolver::roots()` returning 3 roots in priority: project .wgenty-code/skills, user ~/.wgenty-code/skills, user ~/.claude/skills

### 1.3 Register in `src/knowledge/mod.rs`
- `pub mod root_resolver;` + re-export

### 1.4 Wire into 5 consumers
- `src/daemon/state.rs:145` — replace inline vec![]
- `src/tui/app/mod.rs:155` — replace inline vec![]
- `src/tui/app/event.rs:693` — replace inline vec![]
- `src/tui/completion.rs:76` — replace scan roots
- `src/cli/args.rs:796` — replace skills_dirs

### 1.5 Add startup trace log
- In `App::new()`, after skill registry construction

### 1.6 Unit tests
- `test_roots_returns_three_entries`, `test_roots_priority_order`

---

## Task 2 — Comet module — State, guard, workflow

### 2.1 Create `src/comet/mod.rs`
- Re-exports state, guard, workflow

### 2.2 Create `src/comet/state.rs`
- `CometPhase` enum (Open, Design, Build, Verify, Archive) with serde rename
- `CometState` struct with change_name, phase, workflow, build_mode, isolation
- `CometState::read(working_dir)` — scan openspec/changes/*/.comet.yaml, return first non-archived
- `CometState::phase_instruction()` — returns phase-specific system prompt text
- Manual YAML line parsing (no serde_yaml dependency)

### 2.3 Create `src/comet/guard.rs`
- `CometGuardDecision` struct (blocked, error_message, phase)
- `CometGuard::check(phase, tool_name, args)` — phase restriction matrix
- `CometGuard::is_coordinator_mode(working_dir)` — check build_mode
- `CometGuard::coordinator_reminder()` — static reminder text
- `is_read_only()` / `is_mutating_command()` helpers

### 2.4 Create `src/comet/workflow.rs`
- `ChangeInfo` struct, `active_changes(working_dir)` function

### 2.5 Create `src/comet/protocol.rs`
- Subagent dispatch protocol documentation (implement → review×2 → fix → commit)

### 2.6 Register in `src/lib.rs`
- `pub mod comet;`

### 2.7 Unit tests
- `test_read_no_changes_dir`, `test_read_with_active_change`, `test_file_read_allowed_in_open`, `test_file_write_blocked_in_open`, `test_file_write_allowed_in_build`, `test_file_write_blocked_in_verify`, `test_git_status_allowed_in_all_phases`

---

## Task 3 — Hook lifecycle — Complete all 8 event fire sites

### 3.1 Add `comet_phase` to `HookContext`
- File: `src/hooks/mod.rs`
- New field: `pub comet_phase: Option<String>`
- Update all context builders (pre_tool, post_tool, session_start, session_end)

### 3.2 Add `hook_manager` to `App` struct
- File: `src/tui/app/mod.rs`
- Initialize from settings.hooks in `App::new()`

### 3.3 Fire `SessionStart` at end of `App::new()`

### 3.4 Fire `SessionEnd` after main loop exit, before Ok(())

### 3.5 Fire `UserPromptSubmit` at top of `submit_input()` — after built-in checks, before slash routing

### 3.6 Fire `Stop` on `TurnComplete` and `TurnAborted` event handlers

### 3.7 Add `hook_manager` to `AgentLoop` struct, wire from App

### 3.8 Fire `PermissionRequest` in `execute_tool_with_permission()` — before PermissionRequired event send

### 3.9 Add `notification_context()` builder to `HookManager`

### 3.10 Integration test: verify each hook event fires (manual via TUI)

---

## Task 4 — Comet guard integration into ToolExecutor

### 4.1 Add `comet_state` field to `ToolExecutor`
- File: `src/tools/executor.rs`
- Initialize from `CometState::read()` in `ToolExecutor::new()`

### 4.2 Integrate guard check in `execute_with_hooks()`
- BEFORE PreToolUse hook, check `CometGuard::check()`
- On block: fire Notification hook, return blocked message

### 4.3 Integration test
- Full comet state flow test in `tests/comet_integration_test.rs`

---

## Task 5 — Worktree operations

### 5.1 Update `input_schema()` — add worktree operations to enum

### 5.2 Add match arms in `execute()` — worktree_add, worktree_remove, worktree_list

### 5.3 Implement `worktree_add` — path, branch, base_ref

### 5.4 Implement `worktree_remove` — path, force

### 5.5 Implement `worktree_list` — list all worktrees

### 5.6 Unit test — schema contains worktree operations

---

## Task 6 — Configurable tool timeout

### 6.1 Implement `resolve_tool_timeout(tool_name, args) -> Duration`
- File: `src/tui/agent/core.rs`
- task/delegate → 300s
- execute_command/exec_command → max(args.timeout + 30, 120)
- other → 120s

### 6.2 Replace hardcoded ternary at line 322

### 6.3 Update `execute_command` schema description

### 6.4 Unit tests — 5 timeout scenarios

---

## Task 7 — Subagent Comet context

### 7.1 Add `comet_context` to TaskTool `input_schema()`
- File: `src/tools/meta/task.rs`
- Optional object: { change: string, task_index: integer }

### 7.2 Extract and inject Comet implementer prefix into subagent system prompt

### 7.3 Inject coordinator reminder into prompt assembly
- File: `src/prompts/mod.rs`
- If coordinator mode, push system reminder message

### 7.4 Inject Comet phase instruction into system messages

---

## Task 8 — Final integration & verification

### 8.1 Write integration test — full comet state + guard flow

### 8.2 Run full test suite — cargo test --lib

### 8.3 Manual verification — TUI smoke test with .comet.yaml active
