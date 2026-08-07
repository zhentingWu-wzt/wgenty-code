# Proposal: daemon-session-orchestration

> Comet 批量拆分项 1/5（batch: `.comet/batches/gui-desktop.json`）。路线 B（胖 daemon）：把 agent loop 与 turn 编排上收到 daemon，使 TUI / GUI / Web 成为可同时接入的纯视图客户端。本 change 是全部 GUI changes 的前置依赖。

## Why

当前 daemon 是「无会话态代理」：`chat/stream` 不认识 session（消息历史由客户端全量上传），agent loop 跑在各 UI 客户端进程内，事件通知除 subagent trace 外全靠客户端轮询（500ms 级）。这导致：多 UI 无法观看同一会话的进行状态、会话文件整体覆盖写（last-write-wins）、permission 审批硬编码 `"default"` 会话、`background results` 先到先得被抢占、全局模式/模型切换无客户端维度。要实现「同一会话，TUI 发消息、Web 看流式输出、GUI 弹审批」的多屏同步，必须把 turn 编排与事件分发上收到 daemon。

## What Changes

- 新增 daemon 会话编排层：agent loop 移入 daemon 按 session 运行（per-session TurnRunner，复用 `run_agent_loop` 与进程内 ports）
- 新增会话级事件流：SSE 订阅端点，事件带单调序号 + 环形缓冲重放（借鉴 `TRACE_HUB` replay 模式），支持断线重连续传与多客户端 fan-out
- 新增 daemon 事件总线：统一承载 turn 事件、todos 变更、permission 请求/决议、task-group 结果、模式/模型变更，替代各客户端轮询端点
- 权限审批事件化：permission 请求广播到订阅该会话的全部 UI，任一 UI 应答即决议；修掉 `"default"` 会话硬编码
- 会话并发控制：per-session turn 互斥/排队，会话存储加版本号或增量追加，杜绝整体覆盖写
- daemon 部署形态：支持独立常驻进程 + 稳定的端口/token 发现机制（多 UI 共享同一实例；进程内嵌模式保留）
- 向后兼容：现有 `chat/stream`、`tools/execute` 等端点保留，TUI 现有模式不受影响；TUI 迁移到新模型为后续 change

## Capabilities

### New Capabilities

- `daemon-session-orchestration`: daemon 会话编排——per-session TurnRunner（agent loop 上收）、turn 生命周期管理（发起/中断/状态查询）、会话并发控制与存储版本化
- `daemon-event-stream`: daemon 事件分发——会话级 SSE 事件流（序号 + 重放 + 多订阅者）、全局事件总线、permission 事件化审批、替代轮询的变更推送

### Modified Capabilities

（无——对现有 specs 是新增能力；旧端点行为保持不变）

## Impact

- **核心改动**：`src/daemon/`（编排层、事件总线、新端点）、`src/agent/runtime/`（loop 以 daemon 内嵌模式运行）
- **客户端**：TUI 暂不改（走兼容层）；GUI/Web 客户端直接基于新事件流模型开发
- **依赖**：无新增外部依赖预期（tokio broadcast 等已在依赖树）
- **后续 change**：gui-desktop-foundation（改为纯视图）等 4 个 GUI changes 依赖本 change；TUI 迁移另立 change
