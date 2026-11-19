## MODIFIED Requirements

### Requirement: 全局事件流

系统 SHALL 提供全局事件订阅端点，承载 todos 变更、task-group 结果、背景任务结果、权限模式变更、模型切换等全局事件；事件 MUST 携带单调序号以支持客户端去重与续传。多订阅者 MUST 收到相同事件序列。

全局事件流的承载方式 SHALL 为以下二者之一或并存：SSE 端点（`GET /api/v1/events`）与 WebSocket 推送通道（全局事件信封）。任一承载方式下的订阅者 MUST 收到相同事件序列；WebSocket 承载 MUST 不改变事件的序号语义与多订阅者广播语义。

#### Scenario: todos 变更推送

- **WHEN** 任一客户端或 agent 更新 todos
- **THEN** 全部全局流订阅者收到 todos 变更事件，无需轮询 `GET /todos`

#### Scenario: 模式/模型变更推送

- **WHEN** 任一客户端切换权限模式或模型
- **THEN** 全部订阅者收到对应变更事件，各端界面同步更新

#### Scenario: WebSocket 订阅者与 SSE 订阅者等价

- **WHEN** 一个客户端经 SSE 端点订阅、另一个经 WebSocket 推送通道订阅全局事件
- **THEN** 两端收到携带相同序号的相同事件序列
