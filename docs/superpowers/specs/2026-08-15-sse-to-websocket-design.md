---
comet_change: sse-to-websocket
role: technical-design
canonical_spec: openspec
---

# Design: SSE → WebSocket 推送通道迁移

- Change: `sse-to-websocket`（canonical spec: `openspec/changes/sse-to-websocket/specs/`）
- 状态: 已确认
- 日期: 2026-08-15

## 1. 背景与目标

web 客户端 4 条 SSE 推送通道（heartbeat / trace / global / per-run session events）在浏览器 per-origin HTTP/1.1 连接预算（~6）下相互挤兑，subagent 密集场景触发 `stream connect timed out`。目标：单条 WebSocket 连接多路复用全部推送，常驻连接 4 → 1，per-run 订阅零新增传输连接。SSE 端点全部保留过渡。

需求事实源是 OpenSpec delta specs（`ws-push-channel` 新能力 + `daemon-event-stream` / `subagent-trace-streaming` 双承载扩展）；本文只定义实现结构。

## 2. 代码事实基础（关键复用点）

- **session events**：daemon 全局单 hub `state.session_event_hub`（broadcast）+ per-session `SessionEventBuffer`（capacity 1024，`events_after()` / `latest_seq()` 已有）。SSE handler（`run_loop.rs::get_session_events`）的订阅模式：先 replay（`plan_catch_up`）→ 订阅 hub → `seq <= seam_seq` 去重 + `session_id` 过滤 → Lagged 时发 `sync_lost`。**WS 订阅完整复用该模式**，无需新事件源
- **trace events**：`trace_hub_subscribe()` 全局 broadcast，drop-oldest；redaction 在 publish 侧已完成
- **空闲关机**：`active_clients`（SSE/请求计入）；idle window 由连接/请求刷新
- **web 直连解析**：`resolveDaemonDirect()`（`/__daemon-info` → `{port, token}`）；SSE 的「直连优先 → vite 代理回退」模式直接平移为 ws URL 派生
- **desktop**：`desktop/src/token-injection.js` 只 patch `window.fetch`；Tauri custom protocol 不转发 WS upgrade

## 3. 协议设计

### 3.1 端点与认证

```
GET /api/v1/ws?token=<daemon-bearer-token>    [Upgrade: websocket]
```

- `require_auth` 语义扩展：token 提取顺序 query `token` → `Authorization` header（后者留给非浏览器客户端）
- 升级失败（401）与受保护路由同构
- `WebSocketUpgrade` 配置 `max_message_size(16 MiB)`：SessionEvent 可携带大 diff，axum 默认值过小；>16MB 视为异常事件，服务端序列化前截断或拒绝（build 时按现有最大事件体积校准）
- 连接关闭码：`4001` = token 失效/轮换（客户端触发 token 刷新后重连）；`1000` 正常关闭；其他按传输错误处理

### 3.2 消息信封（D2）

下行（server → client，JSON text frame）：

| type | 载荷 | 说明 |
|---|---|---|
| `heartbeat` | 无 | 15s 节拍；兼做应用层保活（防代理闲置断连） |
| `trace` | `event: TraceEvent` | 全局 trace 流，客户端自行过滤 session |
| `global` | `event: GlobalEvent`（含 `seq`） | 全局事件总线 |
| `session` | `session_id` + `event: SessionEvent`（含 `seq`） | 仅推给订阅该 session 的连接 |
| `subscribed` | `session_id` + `latest_seq` | subscribe 应答，客户端游标对齐 |

上行（client → server）：

| op | 载荷 | 说明 |
|---|---|---|
| `subscribe` | `session_id` + `after?` | 订阅会话事件；after 为游标（replay 语义同 SSE `?after=`） |
| `unsubscribe` | `session_id` | 退订 |

未识别消息：忽略并 debug 日志（前向兼容）。未知 `session_id`：`subscribed` 应答带 `latest_seq: 0`，不报错（SSE 是 404，但 WS 长连接内报错代价高；客户端发现无事件自会走 `GET /sessions/:id` 恢复路径）。

### 3.3 会话订阅语义

- 每连接订阅表 `HashMap<SessionId, SubState{ seam_seq, pending_replay }>`，上限 **64**（超限回错误信封，连接保持）
- `subscribe` 处理：同步执行 `plan_catch_up(after, buffer)` → replay 帧直接推送 → 发 `subscribed{latest_seq}` → live 事件按 `session_id` 过滤 + `seq <= seam_seq` 去重
- `SyncLost`（游标滑出窗口）：作为 `session` 信封内特殊事件透传（`kind: "sync_lost"`），流保持 —— 客户端按现有契约（全量 GET + 重订阅）恢复
- hub `Lagged`：单连接发 `sync_lost(lagged)` 后跳到 `latest_seq` 继续，连接保持（与 SSE 一致）
- 连接断开：订阅表随任务结束整体释放，无需逐项清理

## 4. daemon 实现结构

新模块 `src/daemon/ws_push.rs`：

```text
ws_handler(State, Query<TokenQuery>, WebSocketUpgrade)
  └─ on_upgrade(16MB) → spawn per-connection task
       select! {
         hub.recv()   → session 事件：按订阅表过滤/去重 → envelope 推送
         trace.recv() → trace 事件：envelope 推送（Lagged 静默丢）
         global.recv()→ global 事件：envelope 推送
         ctrl.recv()  → subscribe/unsubscribe：更新订阅表（replay 同步执行）
         tick(15s)    → heartbeat 信封
       }
```

- 出站统一经 mpsc → 写半 task（序列化阻塞不阻塞 select 分支）
- `active_clients`：升级成功 `fetch_add(1)`，连接任务退出 `fetch_sub(1)`（Drop guard 防泄漏）
- 路由注册：`.route("/api/v1/ws", get(ws_push::ws_handler))`；认证在 handler 内显式执行（query token），不依赖中间件的 header 提取

## 5. web 实现结构

### 5.1 `web/src/api/wsChannel.ts`（新）

```ts
interface WsChannel {
  on<K extends EnvelopeType>(type: K, handler: (env: Envelope[K]) => void): () => void;
  sessionEvents(sessionId: string, after: number, signal?: AbortSignal): AsyncIterableIterator<SessionEvent>;
  readonly status: "connecting" | "open" | "closed";
}
```

- **模块级单例**（每 tab 一条连接，浏览器上下文隔离天然成立）
- 连接状态机：`connecting → open → (drop) → backoff → connecting…`；指数退避 1s→30s 封顶，成功后重置
- URL 派生：`resolveDaemonDirect()` → `ws://127.0.0.1:<port>/api/v1/ws?token=…`；null（desktop）→ `ws://<page-origin>/api/v1/ws`（经 token-injection 的 WS patch 或 vite 代理）
- close 4001 → 先 `read_daemon_token`（desktop）或重新 `resolveDaemonDirect`（browser）刷新 token 再重连
- **sessionEvents 用 async iterator**：内部 pending 队列 + 等待者；断线期间事件自然断流（不伪造），重连后通道自动按保存 cursor resubscribe，恢复事件流；`sync_lost` 作为普通事件 yield（消费端语义不变）
- 重连成功回调：对所有 active session 订阅 resubscribe(after=cursor) + 触发一次 `traceReplay`（REST）补 trace 缺口
- trace/global 消费用 `on()` 回调注册（无序号语义，直接分发）

### 5.2 消费点切换（行为不变，换数据源）

| 消费点 | 现状 | 切换后 |
|---|---|---|
| `App.tsx` heartbeat | EventSource `/client/heartbeat` | 连接存续即保活；onclose 时 UI 状态改为离线 |
| `usePermissionTrace` | `traceStream()` SSE 循环 | `on("trace", …)`；冷启动 replay 逻辑保留 |
| `useContinuationObserver` | `globalEvents()` SSE 循环 | `on("global", …)` |
| `runSessionTurn` / `observeDaemonRun` | `sessionEvents()` SSE + reader 循环 + 断线重订阅 | `for await (… of channel.sessionEvents(id, lastSeq, abort.signal))`；重订阅由通道承担 |

`fetchStream` 与全部 SSE client 方法保留（过渡期兼容 + 回滚路径）。

## 6. desktop 适配

`desktop/src/token-injection.js` 增加 WebSocket 构造器 patch（与 fetch patch 对称）：

- 拦截 `new WebSocket(url)`，url 匹配 `/api/v1/ws`（同源相对形式）→ 改写为 `ws://127.0.0.1:<port>/api/v1/ws?token=<token>`；port/token 经现有 `__TAURI__.core.invoke("read_daemon_token")` 及 host 下发的连接信息获取
- 401 等价物：daemon close 4001 → web 层触发 token 刷新重连（见 5.1）
- Tauri webview 对 `ws://127.0.0.1` 的混合内容策略在 build 阶段实测；若受限 → desktop 平台分支回退 SSE（`getPlatform()` 已有挂点），作为已识别兜底

## 7. 构建设施

- `vite.config.ts`：`proxy["/api"].ws = true`（WS upgrade 转发）；代理注入 token 的 `configure` 回调对 WS 握手同样生效（header 形式，浏览器同源回退路径由代理补 token —— 代理目标 URL 不带 query token）
- 生产 web（非 vite）：直连路径为主；同源部署需反代 WS upgrade（部署文档标注，不在本 change 范围）

## 8. 测试策略

**daemon（集成，tokio-tungstenite）**
- 认证：无 token / 错 token → 升级拒绝（401 同构）
- 信封：连接后收到 heartbeat；触发 run → session 信封（订阅后）；trace/global 事件 → 对应信封
- 订阅：`after` 游标 replay 不丢不重（seam 去重）；unsubscribe 后不收该 session；断开连接不影响其他订阅者；订阅上限 64 拒绝；未知 session 订阅 → `subscribed{latest_seq:0}`
- 并存：同一事件流下 SSE 客户端与 WS 客户端收到等价事件（序号一致）
- 空闲关机：仅 WS 连接存续时 daemon 不退出

**web（vitest）**
- wsChannel：mock WebSocket 实现（open/message/close/close-code-4001）—— 状态机、退避、`on` 分发、sessionEvents 迭代器（事件顺序/游标传参/signal 中止）、重连后 resubscribe 传保存 cursor、引用计数（同 session 两订阅者一次 subscribe）
- 4 个消费点：现有测试的假 SSE 流 → 假通道注入（`usePermissionTrace` / `useContinuationObserver` / `sessionRunner` / App）

**端到端（验收）**
- devtools Network：1 条 WS、0 条 SSE
- subagent 密集场景（≥3 并发 run + task_group_result）无 `stream connect timed out`
- daemon 重启：WS 自动重连 + 订阅恢复 + trace replay 补齐
- desktop shell：token patch 生效，WS 直连成功

## 9. 风险与缓解

| 风险 | 缓解 |
|---|---|
| Tauri webview 限制 ws:// 直连 | build 阶段实测；兜底 = desktop 平台分支回退 SSE（`getPlatform()` 挂点现成） |
| 大事件超 WS 帧限制 | `max_message_size(16MiB)` 显式配置；超限事件截断策略按实测校准 |
| 长连接内存/句柄泄漏 | 每连接任务退出即释放订阅表；`active_clients` Drop guard；集成测试覆盖断开清理 |
| vite 代理 WS 转发异常 | 直连是主路径；代理仅 dev 回退；`ws: true` 为成熟配置 |
| 双通道过渡期推送成本翻倍 | broadcast clone-and-send，开销可忽略；过渡期有限（后续 change 移除 SSE） |

## 10. 回滚

web 侧保留全部 SSE 消费代码路径（`fetchStream` 及 4 个 client 方法不删）；回滚 = 消费点切回 SSE 调用（git revert 单 change 范围）。
