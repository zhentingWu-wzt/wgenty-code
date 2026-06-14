# hook-event-alignment

Hook 系统兼容 Claude Code 标准事件类型和配置格式。

## Requirements

- **REQ-HEA-001**: `HookEvent` 必须新增 `Stop`、`UserPromptSubmit`、`PermissionRequest` 三种事件类型
- **REQ-HEA-002**: `HookDefinition` 必须支持 `matcher` 字段（`None`/`""` = 全部匹配，`"ToolA|ToolB"` = 管道分隔模式匹配）
- **REQ-HEA-003**: Hook 命令执行前必须展开 `%tool%` 和 `%input%` 变量
- **REQ-HEA-004**: `HookManager::from_settings()` 必须兼容 Claude Code 的 hooks 配置数组格式（含 `matcher` 和 `type` 字段的嵌套结构）
- **REQ-HEA-005**: 现有事件类型（PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification）继续正常工作
