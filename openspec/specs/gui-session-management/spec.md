# gui-session-management Specification

## Purpose
TBD - created by archiving change gui-session-management. Update Purpose after archive.
## Requirements
### Requirement: 会话列表

系统 SHALL 在 GUI 中展示会话列表面板，包含每个会话的标题、最后活跃时间等元信息，并支持新建、切换与删除会话。

#### Scenario: 浏览会话列表

- **WHEN** 用户打开会话面板
- **THEN** 按最近活跃排序展示全部会话及其元信息

#### Scenario: 新建会话

- **WHEN** 用户在会话面板触发新建
- **THEN** 创建空会话并切换为当前对话

#### Scenario: 删除会话

- **WHEN** 用户删除某会话并确认
- **THEN** 该会话从列表移除，若删除的是当前会话则切换到空会话或其他会话

### Requirement: 会话搜索

系统 SHALL 支持按关键词搜索历史会话。

#### Scenario: 关键词搜索

- **WHEN** 用户在搜索框输入关键词
- **THEN** 列表过滤为匹配的会话（标题或内容命中）

### Requirement: 会话恢复

系统 SHALL 在用户切换会话时加载该会话的完整对话历史，并允许在原上下文中继续对话。

#### Scenario: 切换并继续对话

- **WHEN** 用户从列表选择一个历史会话
- **THEN** 对话区加载其历史消息，用户发送新消息后助手基于该会话上下文回复

#### Scenario: 长历史加载

- **WHEN** 目标会话历史较长
- **THEN** 界面先展示最近内容并保持可交互，历史其余部分按需加载

### Requirement: checkpoint 与 undo-turn

系统 SHALL 展示当前会话各 turn 的 checkpoint，允许用户对指定 turn 执行 undo 回滚，且回滚前 MUST 要求用户明确确认。

#### Scenario: 查看 checkpoint

- **WHEN** 用户打开 checkpoint 视图
- **THEN** 展示各 turn 的 checkpoint 列表及对应变更摘要

#### Scenario: 确认后回滚

- **WHEN** 用户对某 turn 执行 undo 并确认
- **THEN** 工作区回滚到该 turn 之前的状态，对话区反映回滚结果

#### Scenario: 取消回滚

- **WHEN** 用户在确认步骤取消
- **THEN** 不发生任何变更

