# Spec: daemon-session-orchestration

daemon 会话编排：per-session TurnRunner（agent loop 上收）、turn 生命周期管理、会话并发控制与存储版本化。

## ADDED Requirements

### Requirement: daemon 内会话编排

系统 SHALL 在 daemon 内为每个活跃会话运行 agent loop（TurnRunner），turn 的执行状态与输出对全部已连接客户端可见；会话的真相（turn 进度、消息历史）MUST 只存在于 daemon，客户端不各自维护副本。

#### Scenario: 发起 turn

- **WHEN** 任一客户端向 `POST /sessions/:id/turns` 提交用户消息
- **THEN** daemon 在该会话上启动 turn 执行 agent loop，turn 输出进入会话事件通道

#### Scenario: 多客户端观察同一 turn

- **WHEN** 一个 turn 进行中且多个客户端订阅该会话
- **THEN** 所有客户端通过事件流看到相同的流式输出与工具执行过程

### Requirement: turn 生命周期管理

系统 SHALL 提供 turn 状态查询与中断能力；任一客户端发起的中断 MUST 对所有观察该会话的客户端生效。

#### Scenario: 中断 turn

- **WHEN** 任一客户端请求中断当前 turn
- **THEN** daemon 停止该 turn，事件流广播中断结果，全部客户端呈现一致状态

#### Scenario: 查询 turn 状态

- **WHEN** 客户端查询会话当前 turn 状态
- **THEN** daemon 返回 idle / running（含当前阶段）等明确状态

### Requirement: 会话并发控制

同一会话同一时刻 MUST 最多一个进行中的 turn；重复发起 MUST 被明确拒绝（409 或等价语义）而非静默互踩。

#### Scenario: 重复发起被拒绝

- **WHEN** 会话已有进行中的 turn，另一客户端再次发起 turn
- **THEN** daemon 返回冲突错误，进行中的 turn 不受影响

### Requirement: 会话存储版本化

系统 SHALL 为会话持久化写入提供并发协调（版本号或等价机制），整体覆盖写 MUST 带版本校验，冲突写 MUST 返回冲突错误而非 last-write-wins。

#### Scenario: 冲突写被拒绝

- **WHEN** 两个写入方基于同一旧版本并发写会话
- **THEN** 先写者成功，后写者收到冲突错误并可重读后重试

### Requirement: 权限归属真实会话

系统 SHALL 修掉审批相关的 `"default"` 会话硬编码，permission 规则与审批记录 MUST 归属发起请求的真实的 session。

#### Scenario: 多会话审批隔离

- **WHEN** 两个不同会话各自触发权限审批
- **THEN** 各自的审批状态与规则互不可见、互不干扰

### Requirement: 向后兼容

现有无会话端点（`chat/stream`、`tools/execute` 等）MUST 保持原语义可用，TUI 现有模式在本 change 完成后行为不变。

#### Scenario: TUI 回归

- **WHEN** 本 change 完成后以现有模式运行 TUI
- **THEN** 对话、工具执行、审批等既有功能正常，不依赖新编排 API
