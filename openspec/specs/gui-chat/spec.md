# gui-chat Specification

## Purpose
TBD - created by archiving change gui-desktop-foundation. Update Purpose after archive.
## Requirements
### Requirement: 流式对话

系统 SHALL 通过会话编排命令端点发起 turn，并订阅会话事件流实时渲染增量输出，渲染 MUST 随流式内容平滑更新，不因高频事件导致界面卡死。

#### Scenario: 发送消息并接收流式回复

- **WHEN** 用户在输入区提交消息
- **THEN** 消息出现在对话区，助手回复以流式方式逐段渲染直至完成

#### Scenario: 其他客户端发起的 turn 同样可见

- **WHEN** 同一会话的 turn 由另一客户端（如 TUI）发起
- **THEN** GUI 对话区同样实时渲染该 turn 的输出

#### Scenario: 流式渲染性能

- **WHEN** 助手以高频率输出大量 token
- **THEN** 界面保持可交互（滚动、输入不卡顿），渲染可采用批量/降帧刷新

#### Scenario: 中断生成

- **WHEN** 用户在流式输出过程中请求中断
- **THEN** 当前生成停止，已输出内容保留在对话区

### Requirement: markdown 与富文本渲染

系统 SHALL 将助手消息渲染为 markdown（标题、列表、表格、链接、代码块），代码块 MUST 支持语法高亮。

#### Scenario: markdown 消息渲染

- **WHEN** 助手消息包含 markdown 内容
- **THEN** 对话区以排版后的富文本展示，而非原始标记文本

#### Scenario: 代码块高亮

- **WHEN** 助手消息包含带语言标注的代码块
- **THEN** 代码块以对应语言的语法高亮展示

### Requirement: 工具调用展示

系统 SHALL 在对话流中展示工具调用的名称、关键参数与执行结果（成功/失败），展示形式优于 TUI 的纯文本呈现（如可折叠卡片）。

#### Scenario: 工具执行过程可见

- **WHEN** 助手在对话中调用工具
- **THEN** 对话区展示工具名称、参数摘要与执行状态，完成后展示结果摘要

#### Scenario: 工具结果折叠

- **WHEN** 工具输出内容较长
- **THEN** 默认展示摘要，用户可展开查看完整内容

### Requirement: 权限审批交互

系统 SHALL 将事件流中的审批请求展示为审批界面（工具名、参数、风险信息），用户 MUST 能够经命令通道批准或拒绝；决议结果（无论由哪个客户端应答）MUST 通过事件流同步到本端，审批期间对话流正确挂起与恢复。

#### Scenario: 批准工具执行

- **WHEN** 审批请求到达且用户选择批准
- **THEN** 工具继续执行，对话流恢复，结果正常渲染

#### Scenario: 拒绝工具执行

- **WHEN** 审批请求到达且用户选择拒绝
- **THEN** 工具不执行，拒绝结果反馈给助手，对话继续

#### Scenario: 其他客户端已应答

- **WHEN** 审批请求展示期间另一客户端已完成应答
- **THEN** 本端审批界面按决议事件自动关闭并展示决议结果

### Requirement: 输入区

系统 SHALL 提供支持多行编辑的输入区，支持快捷键提交，并在助手生成期间禁止重复提交。

#### Scenario: 多行输入与提交

- **WHEN** 用户输入多行文本并按下提交快捷键
- **THEN** 消息被发送，输入区清空

#### Scenario: 生成期间禁止重复提交

- **WHEN** 助手正在生成回复
- **THEN** 输入区不可提交新消息（或明确排队），避免并发对话冲突

