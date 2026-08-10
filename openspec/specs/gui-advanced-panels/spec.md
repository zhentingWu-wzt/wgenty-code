# gui-advanced-panels Specification

## Purpose
TBD - created by archiving change gui-advanced-panels. Update Purpose after archive.
## Requirements
### Requirement: subagent 进度树

系统 SHALL 以树形结构可视化展示 subagent 的层级、运行状态与工具执行进度，状态更新 MUST 及时且不引起界面抖动。

#### Scenario: 查看 subagent 执行过程

- **WHEN** 对话中触发 subagent 执行
- **THEN** 面板展示 subagent 树的层级结构、各节点状态与当前工具调用进度

#### Scenario: 展开节点详情

- **WHEN** 用户展开某个 subagent 节点
- **THEN** 展示该节点的工具调用序列与结果摘要

### Requirement: todos 实时面板

系统 SHALL 实时同步展示当前任务清单（todos）及其状态变更。

#### Scenario: 任务状态同步

- **WHEN** 助手在对话中创建或更新 todos
- **THEN** 面板实时反映任务的新增、进行中与完成状态

### Requirement: 请求上下文透视面板

系统 SHALL 提供请求上下文透视面板，展示 system prompt 来源分层、召回记忆、完整 messages、hook 注入与 token 统计五类数据；数据源复用现有采集层（tui-inspector 成果），数据不可用时 MUST 明确提示而非展示空白。

#### Scenario: 查看透视数据

- **WHEN** 用户打开透视面板的任一 tab
- **THEN** 展示对应类别的上下文数据（prompt 分层/记忆/messages/hooks/token 统计）

#### Scenario: 数据源不可用

- **WHEN** 透视数据采集层未启用或无数据
- **THEN** 面板明确提示数据不可用及原因，不展示空白或报错堆栈

#### Scenario: 敏感内容保护

- **WHEN** 透视数据包含完整 messages 等潜在敏感内容
- **THEN** 详情默认折叠，展开前提示数据敏感性

