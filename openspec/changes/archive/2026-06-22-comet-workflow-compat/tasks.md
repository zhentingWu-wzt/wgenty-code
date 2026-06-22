## 1. Skill path compat — Unified root resolver

- [x] 1.1 Add `src/knowledge/root_resolver.rs`: `SkillRootResolver` struct with `roots() -> Vec<ExternalSkillRoot>` that returns project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, user `~/.claude/skills/` in priority order
- [x] 1.2 Wire `SkillRootResolver` into `DaemonState::new()` at `src/daemon/state.rs:145`, replacing the inline `vec!` of roots
- [x] 1.3 Wire `SkillRootResolver` into `App::new()` at `src/tui/app/mod.rs:155`, replacing the inline `vec!` of roots
- [x] 1.4 Wire `SkillRootResolver` into `AppEvent::ConfigChanged` at `src/tui/app/event.rs:693`, replacing the inline `vec!` of roots
- [x] 1.5 Wire `SkillRootResolver` into `CompletionEngine::load()` at `src/tui/completion.rs:76`, adding `~/.claude/skills/` to the scan roots
- [x] 1.6 Wire `SkillRootResolver` into `run_skills()` at `src/cli/args.rs:796`, for CLI `skills list` consistency
- [x] 1.7 Add startup trace log: count of discovered skills and roots scanned (in `App::new()`)

## 2. Hook lifecycle — Complete all 8 event fire sites

- [x] 2.1 Fire `SessionStart` hook at end of `App::new()` in `src/tui/app/mod.rs`, after all initialization is done
- [x] 2.2 Fire `SessionEnd` hook before daemon shutdown in `App::run()` at `src/tui/app/mod.rs` (or in the daemon shutdown path)
- [x] 2.3 Fire `UserPromptSubmit` hook at start of `submit_input()` in `src/tui/app/input.rs`, after built-in command handling but before slash command routing, carrying raw input text as `tool_input`
- [x] 2.4 Fire `Stop` hook on `AppEvent::TurnComplete` and `AppEvent::TurnAborted` in the event handler at `src/tui/app/event.rs`, carrying turn finish/abort reason
- [x] 2.5 Fire `PermissionRequest` hook in `execute_tool_with_permission()` at `src/tui/agent/tool_dispatch.rs:122`, before sending the `PermissionRequired` event
- [x] 2.6 Add `Notification` hook fire in comet guard module (see task 3.4) with subtype `comet_phase_block`
- [x] 2.7 Ensure hook context JSON carries `session_id`, `working_directory`, `timestamp`, and `comet_phase` (when active) in all relevant hook firings

## 3. Comet phase guard — New `src/comet/` module

- [x] 3.1 Create `src/comet/mod.rs`: re-exports `state`, `guard`, `workflow`
- [x] 3.2 Create `src/comet/state.rs`: `CometState` struct with `.read(working_dir) -> Option<CometState>` that scans `openspec/changes/*/.comet.yaml` and returns the first non-archived active change's `phase`, `workflow`, `build_mode`, `isolation`
- [x] 3.3 Create `src/comet/guard.rs`: `CometGuard::check(phase, tool_name, args) -> CometGuardDecision` implementing the phase tool restriction matrix (open/design: no source writes; verify: limited writes; archive: no writes)
- [x] 3.4 Integrate `CometGuard::check()` into `ToolExecutor::execute_with_hooks()` at `src/tools/executor.rs:128`, BEFORE PreToolUse hook firing. On block, fire `Notification` hook with subtype `comet_phase_block`
- [x] 3.5 Add `CometState` reading and phase context injection into agent system messages during prompt assembly in `src/tui/app/mod.rs` or `src/prompts/mod.rs`
- [x] 3.6 Create `src/comet/workflow.rs`: `active_changes() -> Vec<ChangeInfo>` wrapping `openspec changes/*/.comet.yaml` directory scan

## 4. Worktree isolation — Git operations extension

- [x] 4.1 Add `worktree_add` operation to `GitOperationsTool::execute()` at `src/tools/execution/git_operations.rs:106`, accepting `path`, `branch`, optional `base_ref` (default `origin/main`). Execute `git worktree add -b <branch> <path> <base_ref>`
- [x] 4.2 Add `worktree_remove` operation to `GitOperationsTool::execute()`, accepting `path`, optional `force` (boolean). Execute `git worktree remove [--force] <path>`. Without `force`, refuse if worktree has uncommitted changes
- [x] 4.3 Add `worktree_list` operation to `GitOperationsTool::execute()`. Execute `git worktree list`
- [x] 4.4 Update `input_schema()` to include `worktree_add`, `worktree_remove`, `worktree_list` in the `operation` enum and document the new parameters (`base_ref`, `force`)
- [x] 4.5 Ensure all worktree operations run with `current_dir` set to the repository root (use existing `repo_path` logic or resolve to git root from `path`)

## 5. Long command timeout — Configurable per-tool timeout

- [x] 5.1 Create `resolve_tool_timeout(tool_name: &str, args: &serde_json::Value) -> Duration` in `src/tui/agent/core.rs` with logic: task/delegate → 300s, execute_command/exec_command → max(args.timeout + 30, 120), other → 120
- [x] 5.2 Replace hardcoded inline ternary at `src/tui/agent/core.rs:322` with call to `resolve_tool_timeout`
- [x] 5.3 Update `execute_command` `input_schema()` at `src/tools/execution/execute_command.rs:54` to clarify the `timeout` field description: "Timeout in seconds (optional, default: 60, max enforced by agent loop with 30s buffer)"
- [x] 5.4 Add unit test for `resolve_tool_timeout` covering: execute_command with timeout=600 → 630s, execute_command without timeout → 120s, task → 300s, file_read → 120s

## 6. Subagent orchestrator — Comet context and review flow

- [x] 6.1 Add optional `comet_context` parameter to `TaskTool` input schema at `src/tools/meta/task.rs:211`, accepting `{ "change": "<name>", "task_index": <n> }`
- [x] 6.2 When `comet_context` is present, prepend Comet implementer system prompt prefix to the subagent's system prompt, including TDD instructions and change/task context
- [x] 6.3 Add comet guard check in agent loop at `src/tui/agent/core.rs`: when comet `build_mode` is `subagent-driven-development`, inject a system reminder instructing the coordinator NOT to directly execute source-file writes, only to dispatch subagents
- [x] 6.4 Ensure `.comet/subagent-progress.md` can be written by coordinator via existing `file_write` tool (no new tool needed — verify path resolves correctly within `openspec/changes/<name>/.comet/`)
- [x] 6.5 Add Comet subagent dispatch protocol documentation as a section in `src/comet/` or as inline comments documenting the implementer→review×2→fix→commit flow
