## ADDED Requirements

### Requirement: Subagent dispatch carries Comet context
When the coordinator (main session) spawns a subagent via the `task` tool and a Comet change is active with `build_mode: subagent-driven-development`, the task tool SHALL accept an optional `comet_context` field containing the change name and current task index from `tasks.md`.

#### Scenario: Implementer subagent receives Comet context
- **WHEN** coordinator calls `task` with `comet_context: { "change": "comet-workflow-compat", "task_index": 3 }`
- **THEN** the subagent's system prompt SHALL include a Comet implementer prefix with TDD instructions
- **AND** the subagent SHALL know it is working on task 3 of the specified change

#### Scenario: Subagent without comet_context works as before
- **WHEN** coordinator calls `task` without `comet_context`
- **THEN** the subagent SHALL behave exactly as current non-Comet task subagents

### Requirement: Coordinator enforces review-before-commit
When operating in Comet subagent-driven-development mode, the coordinator (main session agent) SHALL NOT mark a task as completed or commit until BOTH a spec compliance review and a code quality review have passed for the current task's changes.

#### Scenario: Both reviews pass, task is committed
- **WHEN** implementer subagent completes task 3
- **AND** spec compliance reviewer subagent returns `{ "pass": true }`
- **AND** code quality reviewer subagent returns `{ "pass": true }`
- **THEN** coordinator SHALL call `git_operations commit` with task-specific message
- **AND** coordinator SHALL update `tasks.md` to check off task 3

#### Scenario: One review fails, fix cycle starts
- **WHEN** implementer subagent completes task 3
- **AND** spec compliance reviewer returns `{ "pass": false, "issues": [...] }`
- **THEN** coordinator SHALL spawn a fix subagent with the review issues
- **AND** after fix completes, reviews SHALL re-run (up to 3 fix cycles)
- **AND** if 3 cycles pass without both reviews passing, coordinator SHALL report failure and pause for user decision

### Requirement: Subagent progress is persisted to .comet/subagent-progress.md
The coordinator SHALL write structured progress to `openspec/changes/<name>/.comet/subagent-progress.md` after each subagent stage completes (implement / review / fix / commit).

#### Scenario: Progress file updated after each stage
- **WHEN** implementer subagent completes
- **THEN** `.comet/subagent-progress.md` SHALL be appended with the implementer result
- **AND** after reviewer completes, it SHALL be appended again
- **AND** the file SHALL use a format compatible with Comet context recovery

#### Scenario: Progress file enables recovery after interruption
- **WHEN** the session is interrupted mid-task
- **AND** `.comet/subagent-progress.md` exists with the last completed stage
- **THEN** on resume, coordinator SHALL read the progress file
- **AND** coordinator SHALL resume from the next incomplete stage (not restart the task)

### Requirement: Main session does not directly execute build tasks
When a Comet change is in `build_mode: subagent-driven-development`, the main session (coordinator) SHALL NOT directly execute pending tasks from `tasks.md`. All task execution SHALL be delegated to subagents via the `task` tool.

#### Scenario: Coordinator refuses to directly implement a task
- **WHEN** Comet mode is `subagent-driven-development`
- **AND** agent attempts to `file_write` or `file_edit` source code for a pending task directly
- **THEN** the comet guard SHALL flag this as a mode violation
- **AND** a warning SHALL be added to the conversation context reminding the coordinator to use subagents
