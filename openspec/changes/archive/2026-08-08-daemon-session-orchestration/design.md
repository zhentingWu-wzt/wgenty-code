# Design: daemon-session-orchestration

## Context

合并 feature/web-ui-redesign 后的现状（均有代码佐证）：

- server-side loop 已落地：`POST /sessions/:id/run` → `run_session_turn()`（`src/daemon/run_loop.rs:895` 起）复用 `run_agent_loop`；`RunRegistry::claim` 实现同会话 run 互斥 409（run_loop.rs:622）
- `SessionEventHub = broadcast::Sender<SessionEvent>`（run_loop.rs:54），事件信封 `{seq, session_id, run_id, kind, data}`，per-session 单调 seq；`GET /sessions/:id/events` SSE fan-out，但 **live-only，无重放**（run_loop.rs:781）；Lagged 仅服务端 warn
- permission/ask 已事件化：`PermissionRequired`/`AskUser` 广播 + 决议广播；重复应答返回 404 或 `{success:false}`；`"default"` 硬编码约 10 处（handlers.rs:485,654,664,699,740,765,1397,1431,1458,1507）
- todos/task-group/背景结果/模式变更仍轮询；背景结果 drain 抢占（handlers.rs:849）
- 会话存储无版本，PUT last-write-wins（仅 run 活跃时 409）；run 内部有 save_gen 但不暴露
- token 全局单文件 `~/.wgenty-code/daemon.token`，端口不写发现文件，无存活校验
- subagent trace SSE 已有冷启动重放 + `since` 参数（handlers.rs:308-366），可作模板

## Goals / Non-Goals

**Goals:**
- 会话事件流：环形缓冲重放 + `after=seq` 续传 + 客户端可感知失步信号
- 全局事件推送替代轮询（todos/task-group/背景结果/模式/模型变更）
- 审批语义：重复应答 409、`"default"` 硬编码清零
- 会话存储乐观并发（版本冲突 409）
- per-working-dir daemon 发现文件（端口/token/pid/心跳），多 UI 复用已驻留实例

**Non-Goals:**
- loop 上收、run 互斥、permission 广播、多项目注册表（合并分支已完成）
- TUI/Web 默认行为切换（轮询→推送的客户端迁移在后续 change）
- 事件持久化日志（重启后恢复仍走会话存储，不做 event sourcing）
- 远程访问/多 daemon 集群（仍 127.0.0.1）

## Decisions

1. **重放：per-session 环形缓冲 + after=seq，复用 trace SSE 模式**
   为每会话维护定长环形缓冲（千级事件）；`GET /sessions/:id/events?after=<seq>` 先重放缓冲再挂实时 broadcast。
   备选（无限事件日志、客户端只全量刷新）被否决——日志超范围，全量刷新对长会话代价高。

2. **失步信号：显式 SyncLost 事件**
   订阅携带的 seq 已淘汰、或运行中 broadcast Lagged 时，向该客户端发送 `SyncLost` 事件（而非仅服务端 warn），客户端收到后走 `GET /sessions/:id` 全量恢复再以最新 seq 重新订阅。语义明确、TUI/GUI/Web 三端共用一套恢复逻辑。

3. **全局事件并入 SessionEventHub 信封**
   扩展 `SessionEventKind`（或等价机制）承载全局事件（session_id 为空或保留字段），新增 `GET /events` 全局流；背景结果改为「事件广播 + 各端独立消费」，废除 drain 抢占。轮询端点保留兼容。
   备选（每类事件独立 SSE 端点）被否决——N 端点等于把轮询换成 N 条流，客户端复杂度不降。

4. **审批语义收敛：409 + 真实 session**
   重复应答统一 409（替换 404/success:false）；handlers 中 `"default"` 回退清零——请求必须携带真实 session_id（旧端点路径内维持兼容映射，新语义仅限 server-side 路径）。

5. **会话存储版本化**
   `Session` 增加 `version`（单调递增）；PUT 携带期望版本，不匹配返回 409 + 当前版本，调用方重读合并重试。run 内部 save_gen 机制保留，与对外版本共存（对外版本在 run 写盘时一并推进）。

6. **发现文件：全局单 daemon + 存活校验**
   沿用已落地的多项目注册表方向：一台机器一个常驻 daemon 服务全部项目。daemon 启动写 `~/.wgenty-code/daemon.json`（port、token、pid、started_at、heartbeat_at），心跳 30s 更新、120s 过期，退出清理；UI 启动流程：读发现文件 → token 匹配 + 心跳未过期 → 复用，否则按现有逻辑拉起。全局 token 文件保留兼容。

## Risks / Trade-offs

- [环形缓冲容量权衡：太小续传窗口短，太大占内存] → 容量做成常量 + 配置项（默认千级），配合 SyncLost 兜底，正确性不依赖缓冲大小
- [全局事件接入 SessionEventHub 导致事件类型膨胀] → kind 枚举按域分组命名，design 阶段定义事件目录
- [版本化改造触碰会话写路径，有回归风险] → 以现有 session 测试 + run 活跃 409 测试回归；版本缺失的历史会话按 version=0 兼容
- [发现文件多进程并发写/残留] → 写入用临时文件 + rename 原子替换；启动时校验 token 不匹配则视为失效
- [轮询→推送期间两套机制并存] → 推送为增量能力，轮询端点不删；客户端迁移另立 change

## Open Questions

- SyncLost 用独立事件类型还是响应级错误（design 阶段按 SSE 客户端解析便利性定）
- 全局事件是否需要独立的 seq 空间（倾向：全局流独立 seq）
- 发现文件心跳更新频率与过期阈值
