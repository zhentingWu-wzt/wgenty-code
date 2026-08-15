# Proposal: sse-to-websocket

## Why

浏览器对每个 origin 的 HTTP/1.1 连接数有 ~6 的硬上限，而 web 客户端的每条 SSE 推送流（heartbeat、trace、global events、per-run session events）都常驻占用一个槽位。subagent 密集场景下（多个并发 run 各占一条 `sessionEvents` + 常驻流叠加），新连接被浏览器排队，15s 连接看门狗触发 `stream connect timed out` 硬错误。WebSocket 不受该预算限制，且能将全部推送并入单条连接，从协议层面根除该类故障。

## What Changes

- daemon 新增 WebSocket 推送端点（axum 0.7 内置 `axum::extract::ws`，无新依赖），单条连接多路复用全部推送类型：heartbeat、subagent trace 事件、全局事件（todos/task-group/背景任务结果）、per-session run 事件
- 定义 WS 消息信封与订阅协议：客户端发送 `subscribe(session_id, after_seq)` / `unsubscribe(session_id)` 控制消息，服务端按订阅推送会话事件；全局与 trace 通道默认全量推送
- WS 握手认证：daemon 现有 bearer token 校验延伸到 WS 升级请求（浏览器 WS API 不支持 Authorization header，需选定 query param / `Sec-WebSocket-Protocol` / 首消息认证之一）
- web 新增 WS 客户端模块（连接管理、指数退避重连、重连后重新订阅 + trace replay 补齐），替换 4 个 SSE 消费点：App heartbeat、`usePermissionTrace`、`useContinuationObserver`、`runSessionTurn`/`observeDaemonRun`
- daemon 空闲关机计时器（thin-client idle shutdown）将 WS 连接计入活跃客户端
- vite dev 代理增加 WS 转发配置（`ws: true`）；直连 daemon origin 的 WS URL 解析复用 `/__daemon-info`
- **SSE 端点全部保留**（过渡兼容，daemon 同时服务两类客户端；稳定后另开 change 移除）—— 非 BREAKING

## Capabilities

### New Capabilities

- `ws-push-channel`: WebSocket 多路复用推送通道 —— 单连接信封协议（心跳/trace/全局/会话事件四类消息）、订阅与退订语义、握手认证、断线重连与事件恢复（replay 补齐）

### Modified Capabilities

- `daemon-event-stream`: 全局事件流的承载方式从「SSE 端点」扩展为「SSE 或 WS 推送通道」，事件序号/多订阅者广播语义不变，SSE 端点保留
- `subagent-trace-streaming`: trace 事件的承载方式同样扩展为 SSE 或 WS；live 订阅与 replay 语义不变

## Impact

- **daemon（Rust）**：新增 WS handler 与推送汇聚层（`src/daemon/`），全局事件 bus、trace hub、session event buffer 增加 WS 订阅者接入；`active_clients` 计数纳入 WS；路由注册
- **web（TypeScript）**：新增 WS 客户端模块；`App.tsx`（heartbeat）、`usePermissionTrace.ts`、`useContinuationObserver.ts`、`agent/sessionRunner.ts`（`runSessionTurn`/`observeDaemonRun`）改为消费 WS 事件流；`api/client.ts` 的 `fetchStream` SSE 路径保留但调用点减少
- **构建/开发设施**：`vite.config.ts` 代理增加 ws 转发
- **不受影响**：TUI（reqwest 直连，无浏览器预算限制）、REST API、desktop shell（加载同一 web 前端，自动受益）
- **测试**：WS 协议层（信封/订阅/认证/重连）需新增集成测试；4 个 SSE 消费点的现有 web 测试需适配
