## Context

web 客户端当前通过 4 条 SSE 通道消费 daemon 推送：heartbeat（`/client/heartbeat`，App 常驻）、trace（`/subagents/trace/stream`，`usePermissionTrace` 常驻）、全局事件（`/events`，`useContinuationObserver` 常驻）、会话事件（`/sessions/:id/events`，`runSessionTurn`/`observeDaemonRun` 按 run 建立）。浏览器对每 origin 的 HTTP/1.1 连接预算 ~6，subagent 密集场景叠加超限后新连接排队，触发 15s 看门狗报 `stream connect timed out`（详见 proposal.md - Why）。

约束：
- axum 0.7 内置 WebSocket 支持（`axum::extract::ws::WebSocketUpgrade`），无需新依赖
- 浏览器 WebSocket API 不能设置 `Authorization` header
- daemon 仅监听回环、所有端点 bearer token 保护；`/__daemon-info`（vite dev 中间件）已向同机页面暴露 port/token
- SSE 端点过渡期保留，daemon 同时服务 SSE 与 WS 两类客户端
- TUI（reqwest 直连）不受影响，不迁移

## Goals / Non-Goals

**Goals:**

- 单条 WS 连接承载全部推送，常驻连接 4 → 1，per-run 会话事件不再新建传输连接
- 与 SSE 等价的事件语义：全局事件单调序号、trace 事件 redaction、会话事件 replay 游标
- 断线自动重连 + 订阅恢复（cursor 续传），用户无感

**Non-Goals:**

- 不做双向 RPC —— WS 上行仅限订阅控制消息，请求-响应仍走 REST
- 不删除/不改 SSE 端点行为（后续 change 清理）
- 不改 TUI 客户端
- 不做跨 daemon 进程的消息可靠性保证（at-least-onone 语义与 SSE 现状一致：replay + 序号去重）

## Decisions

### D1: 认证用 query parameter 传递 token

`GET /api/v1/ws?token=<bearer>`，复用现有 `require_auth` 中间件语义（提取方式适配 query 参数）。

- 备选 `Sec-WebSocket-Protocol`：浏览器可传但语义是子协议协商，协议字符串常被代理/中间件改写，坑多
- 备选首消息认证：握手成功后才验 token，需要应用层超时窗口，且违反 spec 的「升级请求即拒绝」场景
- 选 query param 的理由：与现有「token 经 `/__daemon-info` 下发到同机页面」的信任边界一致（token 本就不离开回环机器）；axum 层可复用 auth 中间件；vite 代理与直连两条路径行为一致

### D2: 信封协议 —— type 字段区分，四类下行 + 两类上行

下行（server → client）：

```jsonc
{ "type": "heartbeat" }
{ "type": "trace",  "event": TraceEvent }
{ "type": "global", "event": GlobalEvent }        // 含 seq
{ "type": "session","session_id": "...", "event": SessionEvent } // 含 seq
{ "type": "subscribed", "session_id": "...", "latest_seq": 42 }  // subscribe 应答
```

上行（client → server）：

```jsonc
{ "op": "subscribe",   "session_id": "...", "after": 42 }  // after 可选游标
{ "op": "unsubscribe", "session_id": "..." }
```

- 备选：每通道独立 WS 连接 —— 直接违背单连接目标
- 备选：订阅语义放进握手 query —— 无法中途增删订阅，per-run 场景必须重建连接
- 单条连接 + 控制消息是最小协议面，且 subscribe 应答携带 `latest_seq` 让客户端能对齐游标

### D3: 服务端单任务汇聚，订阅表驱动

新模块 `src/daemon/ws_push.rs`：每个 WS 连接一个 tokio 任务，内部 `select!` 三路事件源 —— trace hub（broadcast）、全局事件 bus、上行控制消息；per-session 订阅持有 `HashMap<session_id, session_event_receiver>`。会话事件的按 session 订阅能力从现有 SSE handler（`/sessions/:id/events`）的实现中抽出内部 API 复用（同一事件 buffer、同一 replay 语义），SSE handler 改为调用该 API。

- 慢订阅者策略沿用 trace hub 的 drop-oldest broadcast 语义；session 事件通道有 replay 端点兜底，与 SSE 现状一致

### D4: web 侧单例 WS 通道 + 引用计数订阅

新模块 `web/src/api/wsChannel.ts`：模块级单例连接（指数退避重连，与现有 SSE 重连纪律一致），按 `type` 分发到各 hook；`subscribeSession(sessionId, handler)` 返回退订函数，内部引用计数——多个消费点订阅同一 session 只占一份服务端订阅。重连成功后自动按各订阅保存的 cursor 重新 subscribe，随后执行一次 `traceReplay`（REST，复用现有冷启动恢复）补齐 trace 缺口。

消费点改造（行为不变，仅换数据源）：
- `App.tsx` heartbeat：连接存在即等效心跳（daemon `active_clients` 计入 WS）
- `usePermissionTrace`：`trace` 信封替代 SSE 解析循环；replay 逻辑不变
- `useContinuationObserver`：`global` 信封替代
- `runSessionTurn`/`observeDaemonRun`：`subscribeSession(daemonId, after=lastSeq)` 替代 `sessionEvents` SSE；`sync_lost` 语义保留

### D5: 直连优先，vite 代理兜底

WS URL 解析复用 `resolveDaemonDirect`（`ws://127.0.0.1:<port>/api/v1/ws?token=…`）；失败回退同源 `ws://<page-origin>/api/v1/ws`，vite 代理增加 `ws: true` 转发。与现有 SSE 的直连/回退策略对称。

## Risks / Trade-offs

- [WS 断线期间事件丢失] → 订阅 cursor 续传 + 全局事件序号去重 + trace replay 端点补齐；与 SSE 断线恢复语义等价
- [双通道过渡期 daemon 推送开销翻倍（SSE+WS 各一份广播）] → broadcast channel 是 clone-and-send，成本可忽略；过渡期有限
- [浏览器 tab 休眠导致 WS 被 OS 挂起] → 重连逻辑以 onclose/onerror 为准，页面唤醒后自动恢复（与 SSE 现状相同）
- [query param 中 token 进入 daemon 访问日志] → daemon 仅回环监听且日志默认不含 query string；风险与 `/__daemon-info` 暴露面一致
- [vite dev 代理对 WS 升级的兼容性问题] → `ws: true` 是成熟配置；直连路径（生产模式与多数 dev 场景）不经过代理

## Migration Plan

1. daemon：WS 端点 + 汇聚层上线，SSE 不动（可独立验证）
2. web：`wsChannel` 模块 + 4 个消费点切换，`fetchStream` SSE 路径保留
3. 验证稳定后，后续 change 移除 web SSE 消费路径与 daemon SSE 端点（另行提案）
4. 回滚：web 侧 WS 失败可单点回退到 SSE 消费代码（本 change 内保留全部 SSE 代码路径）
