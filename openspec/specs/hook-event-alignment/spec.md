# hook-event-alignment Specification

## Purpose
TBD - created by archiving change cc-ecosystem-compat. Update Purpose after archive.
## Requirements
### Requirement: REQ-HEA-001 — new event types

The `HookEvent` enum SHALL be moved from `src/hooks/` to `src/runtime/event.rs` as part of the generic `EventBus`. The existing event types (PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification, Stop, UserPromptSubmit, PermissionRequest) SHALL be preserved. New event types SHALL be added for Runtime-level events.

#### Scenario: Existing hook events preserved after migration

- **WHEN** hooks are configured with any existing event type (PreToolUse, PostToolUse, etc.)
- **AND** the hooks module has been migrated to `src/runtime/`
- **THEN** all existing event types SHALL fire identically to before the migration

#### Scenario: Runtime events added alongside hook events

- **WHEN** the `EventBus` emits a `RuntimeEvent::StateTransition`
- **THEN** the event SHALL be handled by the `EventBus` subscription system
- **AND** existing hook event types SHALL NOT be affected

### Requirement: REQ-HEA-002 — matcher field

`HookDefinition` MUST MUST 必须支持 `matcher` 字段（`None`/`""` = 全部匹配，`"ToolA|ToolB"` = 管道分隔模式匹配）。

#### Scenario: Empty matcher matches all
- GIVEN hook 的 matcher 为 None 或空字符串
- WHEN 任何工具执行
- THEN hook 被触发

#### Scenario: Pipe-separated matcher
- GIVEN hook 的 matcher = "TaskCreate|TaskUpdate"
- WHEN "TaskCreate" 工具执行 → hook 触发
- WHEN "Read" 工具执行 → hook 不触发

### Requirement: REQ-HEA-003 — variable expansion

Hook 命令执行前MUST MUST 必须展开 `%tool%` 和 `%input%` 变量。

#### Scenario: %tool% expansion
- GIVEN hook 命令 = "echo %tool%"
- WHEN TaskCreate 工具触发 hook
- THEN 展开后命令包含 "TaskCreate"

#### Scenario: %input% expansion with shell escaping
- GIVEN tool_input 包含引号
- WHEN 变量展开执行
- THEN 输入值被 shell-escaped

### Requirement: REQ-HEA-004 — CC hooks format compatibility

`HookManager::from_settings()` SHALL remain compatible with Claude Code hooks format after migration to `src/runtime/`. The `cc_adapter` module SHALL be preserved.

#### Scenario: CC nested array format still parsed after migration

- **WHEN** hooks config uses CC format `{"Stop": [[{"type": "command", "command": "..."}]]}`
- **AND** `HookManager::from_settings()` is called from `src/runtime/`
- **THEN** hooks SHALL be correctly parsed into `Vec<HookDefinition>` with matcher and type fields

### Requirement: REQ-HEA-005 — backward compatibility

All existing hook behavior SHALL be preserved after migration. The `GuardPipeline` (new) SHALL run before `PreToolUse` hooks (existing), matching the current `CometGuard`-before-`PreToolUse` ordering.

#### Scenario: Guard pipeline runs, then PreToolUse hooks

- **WHEN** a tool is about to execute
- **AND** both the `GuardPipeline` and `PreToolUse` hooks are configured
- **THEN** the `GuardPipeline` SHALL evaluate first
- **AND** if the guard allows, `PreToolUse` hooks SHALL then execute
- **AND** this ordering SHALL match the previous comet-guard-before-hooks behavior

