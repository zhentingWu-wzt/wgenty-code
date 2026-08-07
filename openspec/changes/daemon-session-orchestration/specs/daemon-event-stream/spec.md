# Spec: daemon-event-stream

daemon 事件分发：会话级 SSE 事件流（序号 + 重放 + 多订阅者）、全局事件总线、permission 事件化审批、替代轮询的变更推送。

## ADDED Requirements

### Requirement: 会话事件流

系统 SHALL 为每个会话提供 SSE 事件流订阅端点，事件携带会话内单调递增序号，支持多个订阅者同时 fan-out；流 MUST 覆盖 turn 输出（文本/推理增量）、工具调用与结果、审批请求与决议、turn 完成/中断/错误等类型。

#### Scenario: 订阅会话事件

- **WHEN** 客户端连接 `GET /sessions/:id/events`
- **THEN** 实时接收该会话的全部事件，多个订阅者收到相同事件序列

#### Scenario: 事件类型完整

- **WHEN** 一个含工具调用与审批的 turn 执行
- **THEN** 事件流依次包含文本增量、工具调用开始/结果、审批请求、审批决议、turn 完成事件

### Requirement: 断线重连与重放

系统 SHALL 为事件流提供有限重放缓冲；订阅时携带 `after=<seq>` 的客户端 MUST 先收到序号之后的缓冲事件再接入实时流；缓冲已淘汰时 MUST 返回明确信号使客户端回退到会话存储全量恢复。

#### Scenario: 断线续传

- **WHEN** 客户端断线后在缓冲窗口内以最后收到的 seq 重连
- **THEN** 先重放错过的事件，随后无缝接入实时事件

#### Scenario: 缓冲淘汰回退

- **WHEN** 客户端重连时携带的 seq 已超出缓冲窗口
- **THEN** daemon 返回明确的失步信号，客户端从会话存储加载全量历史后重新订阅

### Requirement: 全局事件总线

系统 SHALL 提供全局事件流，承载 todos 变更、task-group 结果、背景任务结果、权限模式/模型切换等全局事件，替代各客户端的轮询端点；背景任务结果 MUST 以事件广播给全部订阅者，禁止先到先得的 drain 抢占语义。

#### Scenario: todos 变更推送

- **WHEN** 任一客户端或 agent 更新 todos
- **THEN** 全部订阅客户端通过事件流收到变更，无需轮询

#### Scenario: 背景任务结果广播

- **WHEN** 背景任务产出结果
- **THEN** 所有订阅者均收到该结果事件，不存在结果被单一客户端抢走的情况

### Requirement: 权限审批事件化

系统 SHALL 将权限审批请求作为事件广播到订阅该会话的全部客户端，任一客户端的应答 MUST 全局决议一次；已决议的审批再次被应答 MUST 返回冲突错误；决议结果 MUST 通过事件流通知全部客户端。

#### Scenario: 任一 UI 应答审批

- **WHEN** turn 触发权限审批且多个客户端在线
- **THEN** 全部客户端收到审批请求事件；任一客户端批准后，全部客户端收到决议事件，工具继续执行

#### Scenario: 重复应答被拒绝

- **WHEN** 审批已决议后另一客户端再次应答同一请求
- **THEN** daemon 返回冲突错误，不产生二次效应

### Requirement: daemon 可发现的常驻部署

系统 SHALL 支持 daemon 独立常驻运行，并提供稳定的端口/token 发现机制（发现信息含存活校验所需内容），多个 UI 进程 MUST 能发现并连接同一实例；发现机制 MUST NOT 出现多实例互相覆盖凭据的问题。

#### Scenario: 多 UI 共享 daemon

- **WHEN** daemon 已常驻运行，TUI 与 GUI 先后启动
- **THEN** 两者通过发现机制连接同一实例，各自鉴权成功

#### Scenario: 发现信息失效

- **WHEN** 发现文件指向的 daemon 已退出
- **THEN** UI 校验存活失败，回退到进程内拉起新实例
