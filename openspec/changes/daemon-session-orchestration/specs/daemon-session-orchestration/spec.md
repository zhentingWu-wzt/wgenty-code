# Spec: daemon-session-orchestration

daemon 会话编排的可靠性补强：会话事件流重放/续传/失步信号、审批语义收敛、会话存储版本化、daemon 可发现部署。基于已落地的 `run_session_turn` / `RunRegistry` / `SessionEventHub` 之上。

## ADDED Requirements

### Requirement: 会话事件流重放与续传

系统 SHALL 为每个会话维护定长环形事件缓冲；`GET /sessions/:id/events` 携带 `after=<seq>` 时 MUST 先按序重放缓冲中 seq 之后的事件，再接入实时流。缓冲容量 MUST 可配置且有合理默认值。

#### Scenario: 断线续传

- **WHEN** 客户端断线后在缓冲窗口内以最后收到的 seq 重连（`after=<seq>`）
- **THEN** 先按序重放错过的事件，随后无缝接入实时事件

#### Scenario: 新订阅者仅实时

- **WHEN** 客户端不携带 after 参数订阅
- **THEN** 只接收订阅之后的实时事件（保持现有 live-only 默认行为）

### Requirement: 客户端可感知的失步信号

系统 SHALL 在客户端请求的 seq 已超出缓冲窗口、或该客户端的 broadcast 订阅发生 Lagged 时，向其发送显式失步信号（SyncLost 事件或等价语义），客户端据此回退到 `GET /sessions/:id` 全量恢复。失步 MUST NOT 仅在服务端记录日志。

#### Scenario: 缓冲淘汰失步

- **WHEN** 客户端携带的 seq 已超出缓冲窗口
- **THEN** 客户端收到明确失步信号，执行全量恢复后以最新 seq 重新订阅

#### Scenario: 慢订阅者失步

- **WHEN** 订阅中的客户端消费速度落后导致 Lagged
- **THEN** 该客户端收到失步信号并可触发恢复流程，其他订阅者不受影响

### Requirement: 审批语义收敛

系统 SHALL 统一重复审批应答的返回为 409 冲突；server-side 路径的审批与规则 MUST 归属真实 session，`"default"` 会话硬编码回退 MUST 清零（旧端点路径维持兼容映射）。

#### Scenario: 重复应答返回 409

- **WHEN** 审批（含 ask_user、subagent permission）已决议后另一客户端再次应答同一请求
- **THEN** daemon 返回 409，不产生二次效应

#### Scenario: 多会话审批隔离

- **WHEN** 两个不同会话各自触发权限审批
- **THEN** 各自的审批状态与规则归属各自 session，互不可见、互不干扰

### Requirement: 会话存储版本化

系统 SHALL 为会话持久化增加单调递增版本字段；整体覆盖写 MUST 携带期望版本，版本不匹配 MUST 返回 409 及当前版本。无版本字段的历史会话按 version=0 兼容。

#### Scenario: 冲突写被拒绝

- **WHEN** 两个写入方基于同一旧版本并发写会话
- **THEN** 先写者成功，后写者收到 409 与当前版本号，可重读后重试

#### Scenario: run 期间写保护保持

- **WHEN** 会话存在活跃 run 时的覆盖写
- **THEN** 仍返回 409（既有行为保持），且 run 写盘时版本一并推进

### Requirement: daemon 可发现部署

系统 SHALL 在启动时写入 per-working-dir 发现文件（端口、token、pid、心跳时间戳），退出时清理；UI 进程启动时 MUST 先读发现文件并校验存活（pid/心跳/token 匹配），命中则复用该实例，校验失败 MUST NOT 误连失效实例。

#### Scenario: 多 UI 复用 daemon

- **WHEN** daemon 已常驻运行，TUI 与 GUI 先后启动
- **THEN** 两者通过发现文件连接同一实例并鉴权成功

#### Scenario: 发现文件失效

- **WHEN** 发现文件指向的 daemon 已退出或 token 不匹配
- **THEN** UI 判定失效，回退到现有拉起逻辑，不连接错误实例

### Requirement: 向后兼容

现有端点（server-side run/events/cancel 与旧 chat_stream/tools 路径）MUST 保持原语义可用；TUI 的 client-side 与 server-side 两种模式行为均不变。

#### Scenario: 双模式回归

- **WHEN** 本 change 完成后分别以两种模式运行 TUI
- **THEN** 对话、工具执行、审批等既有功能正常
