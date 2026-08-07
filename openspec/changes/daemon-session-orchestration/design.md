# Design: daemon-session-orchestration

## Context

现状（调查结论，均有代码佐证）：

- `chat/stream` 是无会话的 LLM SSE 透传（`src/daemon/handlers.rs:175-283`），agent loop（`src/agent/runtime/loop_.rs` 的 `run_agent_loop`）通过 `LlmPort`/`ToolPort` 抽象跑在客户端进程（TUI 用 `DaemonLlmPort`/`DaemonToolPort` 回连 daemon）
- 唯一成熟的多订阅者推送是 `TRACE_HUB`（`src/teams/trace_sink.rs:55`，broadcast 容量 1024 + 冷启动重放）
- permission 走 `PermissionBridge` 拉模型（`src/teams/permission_bridge.rs`）+ 客户端 500ms 轮询
- 会话存储整体覆盖写无版本（`handlers.rs:902-948`）；审批硬编码 `"default"`（`handlers.rs:637,647,683`）；`background results` drain 抢占语义

约束：TUI 现有行为不能破；新模型与旧端点共存；跨平台。

## Goals / Non-Goals

**Goals:**
- daemon 内按 session 运行 agent loop，turn 状态对多客户端可见
- 会话事件流：序号 + 重放 + 多订阅者 fan-out，断线重连续传
- 事件总线替代全部轮询端点（permission/todos/task-group/背景任务/模式变更）
- 同一会话可被 TUI+GUI+Web 同时观看与操作（任一 UI 可审批、可中断）
- 旧端点与 TUI 现有模式保持兼容

**Non-Goals:**
- TUI 迁移到新模型（后续 change）
- 多 daemon 实例集群/远程访问（仍绑定 127.0.0.1）
- Web 前端实现本身（属后续 change）
- CRDT 式的多端同时编辑同一会话（同一时刻一个 turn，操作互斥）

## Decisions

1. **loop 上收：per-session TurnRunner**
   daemon 为每个活跃 session 持有 TurnRunner：复用 `run_agent_loop`，但 ports 换为进程内实现（LlmPort 直接调 `ApiClient`，ToolPort 直接调 `ToolExecutor`），不再绕 HTTP。turn 输出写入会话事件通道。
   备选（loop 留在客户端、daemon 只做状态同步）被否决——多屏同步要求 turn 只有一份真实来源。

2. **事件通道：每会话 broadcast + 环形缓冲重放**
   仿 `TRACE_HUB`：per-session `broadcast::Sender<SessionEvent>`（事件带 session 内单调 seq）+ 定长环形缓冲；`GET /sessions/:id/events?after=<seq>` SSE 端点，新订阅者先重放缓冲再挂实时流。全局事件（模式/模型变更）走独立全局 hub。
   备选（WebSocket、无限持久化事件日志）被否决——WS 无必要（见探索结论），持久日志超出本 change 范围（重启恢复历史仍走会话存储）。

3. **命令/事件分离的命令模型**
   UI→daemon 一律 HTTP 命令（`POST /sessions/:id/turns` 发起 turn、`POST .../interrupt`、`POST .../permissions/:rid` 应答）；daemon→UI 一律事件流。UI 不持有会话真相，只是事件流的投影 + 命令发送器。

4. **会话并发控制：turn 互斥 + 存储版本化**
   同一 session 同时最多一个进行中的 turn（重复发起返回 409 或排队，design 阶段定）；会话存储写入带版本号，冲突写返回 409 由调用方重读。修掉 `"default"` 硬编码：审批归属请求发起时的真实 session。

5. **兼容策略：新旧并存**
   现有 `chat/stream`、`tools/execute` 保留原语义（无会话代理模式），TUI 继续工作；新端点挂在 `/sessions/:id/` 命名空间下。TUI 迁移另立 change，不在本 change 内做双轨切换。

6. **部署：独立常驻 + 可发现**
   支持 `wgenty-code daemon` 常驻，端口/token 落到 per-working-dir 的发现文件（替代当前单一全局 token 文件互相覆盖的问题）；UI 默认先尝试连接已驻留实例，失败再进程内拉起（保留现有内嵌模式）。

## Risks / Trade-offs

- [loop 上收改动面大，影响 agent runtime 稳定性] → 复用 `run_agent_loop` 不改其核心；TurnRunner 只做 port 接线与事件转发；以现有 agent 测试套件回归
- [事件缓冲容量与内存占用] → 定长环形缓冲（千级事件），超出后客户端重连走会话存储全量恢复
- [新旧两套编排并存期的概念混乱] → 文档明确「新模型为准，旧端点仅兼容」；TUI 迁移 change 完成后标记旧端点 deprecated
- [多 UI 并发命令竞态（两个 UI 同时审批/中断）] → 命令幂等 + 状态机校验（已决议的审批再次应答返回 409），事件流保证全端最终一致
- [daemon 常驻进程的生命周期管理（孤儿进程、端口泄漏）] → 发现文件含 pid + 心跳，UI 启动时校验存活；design 阶段细化

## Open Questions

- turn 重复发起：409 拒绝 vs 排队（倾向 409，简单明确）
- 环形缓冲容量与事件体积上限
- 发现文件位置与格式（per-working-dir vs per-user）
