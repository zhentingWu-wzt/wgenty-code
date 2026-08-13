# daemon-event-stream Specification

## Purpose
TBD - created by archiving change daemon-session-orchestration. Update Purpose after archive.
## Requirements
### Requirement: 全局事件流

系统 SHALL 提供全局事件订阅端点（SSE），承载 todos 变更、task-group 结果、背景任务结果、权限模式变更、模型切换等全局事件；事件 MUST 携带单调序号以支持客户端去重与续传。多订阅者 MUST 收到相同事件序列。

#### Scenario: todos 变更推送

- **WHEN** 任一客户端或 agent 更新 todos
- **THEN** 全部全局流订阅者收到 todos 变更事件，无需轮询 `GET /todos`

#### Scenario: 模式/模型变更推送

- **WHEN** 任一客户端切换权限模式或模型
- **THEN** 全部订阅者收到对应变更事件，各端界面同步更新

### Requirement: 背景任务结果广播

系统 SHALL 将背景任务结果以事件广播给全部订阅者；MUST 废除先到先得、其他客户端不可见的 drain 抢占语义。结果 MUST 在广播前持久化或保留，使未订阅的客户端后续仍可查询。

#### Scenario: 多端同时收到结果

- **WHEN** 背景任务产出结果且多个客户端在线
- **THEN** 所有订阅者均收到该结果事件，不存在结果被单一客户端抢走

#### Scenario: 离线后查询

- **WHEN** 结果广播时某客户端未在线
- **THEN** 该客户端上线后仍可通过查询端点获取该结果

### Requirement: 轮询端点兼容

全局事件流上线后，现有轮询端点（todos、background/results、tasks/progress 等）MUST 保持可用；除本 change 指定的一处 dogfood 迁移（TUI todos 面板切换为事件订阅，并保留轮询回退）外，其余客户端轮询路径不变，客户端全量迁移属于后续 change。

#### Scenario: 旧客户端不受影响

- **WHEN** 未迁移的客户端继续轮询现有端点
- **THEN** 返回结果与迁移前一致

#### Scenario: dogfood 迁移等价

- **WHEN** TUI todos 面板切换为事件订阅
- **THEN** todos 变更的呈现与轮询模式等价，订阅断开时回退轮询

