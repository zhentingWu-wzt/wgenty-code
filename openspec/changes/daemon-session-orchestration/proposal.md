# Proposal: daemon-session-orchestration

> Comet 批量拆分项 1/5（batch: `.comet/batches/gui-desktop.json`）。路线 B（胖 daemon）的补强 change：合并 feature/web-ui-redesign 后，server-side loop 主体已落地，本 change 收敛为**多 UI 纯视图所需的可靠性缺口补齐**。全部 GUI changes 的前置依赖。

## Why

合并后 daemon 已具备：`run_session_turn` + `RunRegistry`（agent loop 已在 daemon 内运行、同会话重复 run 返回 409）、`SessionEventHub`（事件带 per-session 单调 seq、tokio broadcast fan-out）、permission/ask 事件广播（任一客户端应答、决议广播）、多项目注册表、`web/` React 前端。但纯视图多 UI 仍有五个真实缺口：

1. **会话事件流不可恢复**：`GET /sessions/:id/events` 是 live-only（`run_loop.rs:781`），无 `after=seq` 重放；慢订阅者 Lagged 只在服务端 warn，客户端无失步信号
2. **全局状态仍靠轮询**：todos、task-group、背景任务结果（drain 抢占语义）、模式/模型变更全靠客户端 500ms 轮询
3. **审批语义残留**：重复应答返回 404/`{success:false}` 而非 409；`handlers.rs` 约 10 处 `"default"` 会话硬编码仍在
4. **会话存储无版本控制**：`PUT /sessions/:id` 仍是 last-write-wins
5. **daemon 不可发现**：token 全局单文件、端口不写发现文件、无 pid/存活校验，多 UI 无法可靠复用已驻留实例

## What Changes

- 会话事件流可靠性：per-session 环形缓冲重放 + `after=<seq>` 续传 + 客户端可感知的失步信号（SyncLost 事件或等价语义），失步后客户端回退 `GET /sessions/:id` 全量恢复
- 全局事件总线：todos 变更、task-group 结果、背景任务结果（去除 drain 抢占）、权限模式/模型变更纳入事件推送，替代轮询端点
- 审批语义收敛：重复应答统一返回 409；清理 `"default"` 会话硬编码，审批归属真实 session
- 会话存储版本化：`Session` 增加版本字段，写操作携带期望版本，冲突返回 409（乐观并发）
- daemon 可发现部署：per-working-dir 发现文件（端口 + token + pid/心跳存活校验），UI 启动优先复用已驻留实例
- 向后兼容：现有端点（含 server-side run/events 与旧 chat_stream 路径）行为不变，TUI 两种模式均不受影响

## Capabilities

### New Capabilities

- `daemon-session-orchestration`: daemon 会话编排的可靠性补强——会话事件流重放/续传/失步信号、审批语义收敛（重复应答 409、归属真实 session）、会话存储版本化、daemon 可发现部署（基于已落地的 run_session_turn/RunRegistry 之上）
- `daemon-event-stream`: 全局事件总线——todos 变更、task-group 结果、背景任务结果（去 drain 抢占）、模式/模型变更的事件推送，替代客户端轮询

### Modified Capabilities

（无——`openspec/specs/` 中无对应既有 spec；本 change 在已合并的 server-side loop 代码之上做增量增强，不修改已归档 spec 的需求）

## Impact

- **核心改动**：`src/daemon/run_loop.rs`（重放缓冲、失步信号）、`src/daemon/handlers.rs`（409 语义、`"default"` 清理、版本校验）、`src/daemon/state.rs`（版本字段、全局事件接入）、`src/utils/`（发现文件）
- **客户端**：TUI/Web 可在后续 change 中将轮询切换为事件订阅；本 change 不改客户端默认行为
- **不触碰**：`run_session_turn`/RunRegistry 既有逻辑、`run_agent_loop`、web/ 前端功能
- **后续 change**：gui-desktop-foundation 等 4 个 GUI changes 依赖本 change 的事件可靠性
