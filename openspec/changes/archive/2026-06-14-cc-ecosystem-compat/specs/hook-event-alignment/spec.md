# hook-event-alignment

MUST Hook 系统兼容 Claude Code 标准事件类型和配置格式。

## ADDED Requirements

### Requirement: REQ-HEA-001 — new event types

`HookEvent` MUST MUST 必须新增 `Stop`、`UserPromptSubmit`、`PermissionRequest` 三种事件类型。

#### Scenario: Stop event fires on session completion
- GIVEN hooks 配置包含 "Stop" 事件
- WHEN agent session 完成
- THEN Stop hooks 被执行

#### Scenario: UserPromptSubmit event
- GIVEN hooks 配置包含 "UserPromptSubmit" 事件
- WHEN 用户提交 prompt
- THEN UserPromptSubmit hooks 被执行

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

`HookManager::from_settings()` MUST MUST 必须兼容 Claude Code 的 hooks 配置数组格式（含 `matcher` 和 `type` 字段的嵌套结构）。

#### Scenario: CC nested array format parsed
- GIVEN hooks 配置为 CC 嵌套数组格式 `{"Stop": [[{"type": "command", "command": "..."}]]}`
- WHEN `from_settings()` 被调用
- THEN hooks 被正确解析为 `Vec<HookDefinition>`，含 matcher 和 type 字段

### Requirement: REQ-HEA-005 — backward compatibility

MUST 现有事件类型（PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification）继续正常工作。

#### Scenario: Legacy flat format still works
- GIVEN hooks 配置为原有扁平格式
- WHEN `from_settings()` 被调用
- THEN 原有事件类型 hooks 被正确解析和执行
