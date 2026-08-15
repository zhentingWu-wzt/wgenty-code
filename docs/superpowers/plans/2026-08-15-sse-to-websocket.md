---
change: sse-to-websocket
design-doc: docs/superpowers/specs/2026-08-15-sse-to-websocket-design.md
base-ref: 347128c07177427caa9e59e969a8858fd599e00a
---

# SSE → WebSocket 推送通道迁移 实施计划

> **For agentic workers:** 使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实施。步骤用 `- [ ]` 复选框跟踪进度。

**Goal:** 将 web 客户端的 4 条 SSE 推送通道（heartbeat / trace / global / per-run session events）合并为单条 WebSocket 多路复用连接，常驻连接 4 → 1，per-run 订阅零新增传输连接；SSE 端点全部保留过渡（回滚路径不删）。

**设计文档:** `docs/superpowers/specs/2026-08-15-sse-to-websocket-design.md`；需求事实源为 OpenSpec delta specs（`openspec/changes/sse-to-websocket/specs/`：`ws-push-channel` 新能力 + `daemon-event-stream` / `subagent-trace-streaming` 双承载扩展）。本计划与 `openspec/changes/sse-to-websocket/tasks.md` 的 5 组任务一一对应（tasks.md 原文 17 个复选框），并按依赖排序。

**Tech Stack:** Rust（axum / tokio broadcast+mpsc / tokio-tungstenite）、TypeScript/React（vitest mock WebSocket）、Vite dev proxy。

---

## Global Constraints（关键复用点，实施前必读）

- **不要设计新事件源。** daemon 会话事件复用现有全局单 hub `state.session_event_hub`（`broadcast::Sender<SessionEvent>`）+ 每 session 的 `SessionEventBuffer`（capacity 1024，`events_after()` / `latest_seq()` 已有）+ `plan_catch_up()` + `sync_lost_event()`（均在 `src/daemon/run_loop.rs`）。WS `subscribe` 与 SSE `GET /sessions/:id/events?after=` 走同一 replay 语义（replay → `subscribed{latest_seq}` → live 按 `session_id` 过滤 + `seq <= seam_seq` 去重 → Lagged 发 `sync_lost`）。
- **trace 复用** `crate::teams::trace_sink::trace_hub_subscribe()`（全局 broadcast，drop-oldest，redaction 已在 publish 侧完成）；**global 复用** `state.global_event_hub`（`src/daemon/global_events.rs`）。
- **空闲关机复用** `state.active_clients: ActiveClientTracker`（`register_client` / `unregister_client` / `client_count`，`src/daemon/state.rs`）。WS 升级成功 `fetch_add(1)`，连接任务退出 `fetch_sub(1)`（用 Drop guard 防泄漏）。
- **web 直连解析复用** `resolveDaemonDirect()`（`web/src/api/client.ts`，读 `/__daemon-info` → `{port, token}`），SSE 的「直连优先 → vite 代理回退」模式直接平移到 `ws://` URL 派生。
- **desktop** 只 patch `window.fetch`（`desktop/src/token-injection.js`）；Tauri custom protocol 不转发 WS upgrade，故必须对称新增 `new WebSocket(url)` patch。
- **回滚安全网：** `fetchStream` 与全部 SSE client 方法（`traceStream` / `globalEvents` / `sessionEvents` / `client_heartbeat`）保留不删；回滚 = 4 个消费点切回 SSE 调用（git revert 单 change 范围）。
- 所有 Rust 改动保持命名惯例，提交前过 `cargo fmt -- --check` 与 `cargo clippy --all-targets -- -D warnings`。

---

## 依赖顺序总览

```
组1 内部订阅 API (1.1 → 1.2)
        │
        ▼
组2 WS 推送端点 (2.1 → 2.2 → 2.3 → 2.4 → 2.5)
        │
        ▼
组3 WS 通道模块 (3.1 → 3.2 → 3.3)
        │
        ▼
组4 消费点切换 (4.1, 4.2, 4.3, 4.4 可并行，均依赖组3 → 4.5)
        │
        ▼
组5 构建设施与验收 (5.1 → 5.2)
```

---

## 进度跟踪（与 tasks.md 复选框一一对应）

- [x] 1.1 从 `GET /sessions/:id/events` SSE handler 中抽出「按 session 订阅事件（可选 after 游标）」的内部 API（同一事件 buffer 与 replay 语义），SSE handler 改为调用该 API，行为不变
- [x] 1.2 为抽出的内部 API 补充/迁移单元测试（游标续传、sync_lost、多订阅者）
- [x] 2.1 新增 `src/daemon/ws_push.rs`：单连接任务 `select!` 三路事件源（trace hub broadcast、全局事件 bus、上行控制消息），按 D2 信封协议序列化下行消息
- [x] 2.2 实现 `subscribe`/`unsubscribe` 上行控制消息：per-session 订阅表、`after` 游标续传、`subscribed` 应答（含 latest_seq）、断开自动清理订阅
- [ ] 2.3 WS 握手认证：`GET /api/v1/ws?token=…` 复用 require_auth 语义（query 参数提取适配），无效凭证与受保护路由同构拒绝；注册路由
- [ ] 2.4 空闲关机计数：WS 连接存续期间计入 `active_clients`（握手成功计入、断开解除），heartbeat 信封按 keepalive 节拍发送
- [ ] 2.5 daemon 集成测试：信封类型完整性、订阅游标续传（断开重订阅不丢不重）、认证拒绝、SSE 与 WS 客户端并存等价
- [ ] 3.1 新增 `web/src/api/wsChannel.ts`：单例连接、指数退避重连、`resolveDaemonDirect` 直连优先 + 同源回退（`ws://` URL 派生）、按 type 分发事件总线
- [ ] 3.2 引用计数订阅 API：`subscribeSession(sessionId, handler)` 返回退订函数；重连后按各订阅 cursor 自动重新 subscribe；trace 缺口经 `traceReplay`（REST）补齐
- [ ] 3.3 单元测试：连接/重连状态机、订阅引用计数、游标续传去重、退订后不收事件
- [ ] 4.1 `App.tsx` heartbeat：EventSource 替换为 WS 连接存续（连接即心跳），清理 heartbeat SSE 代码路径
- [ ] 4.2 `usePermissionTrace`：SSE 解析循环替换为 `trace` 信封订阅；冷启动 replay 逻辑保持
- [ ] 4.3 `useContinuationObserver`：`globalEvents` SSE 替换为 `global` 信封订阅，`task_group_result` → `observeDaemonRun` 行为不变
- [ ] 4.4 `runSessionTurn`/`observeDaemonRun`：`sessionEvents` SSE 替换为 `subscribeSession(daemonId, after=lastSeq)`，`sync_lost` 与 stop 语义保持；`fetchStream` SSE 路径保留不删
- [ ] 4.5 适配现有 web 测试（4 个消费点的 mock 从 SSE 假流改为 WS 信封假流）
- [ ] 5.1 `vite.config.ts` 代理增加 `/api` 的 `ws: true` 转发；验证 dev 直连与代理两条 WS 路径
- [ ] 5.2 端到端验收：单条 WS 承载全部推送（devtools 无 SSE 连接）；subagent 密集场景无 `stream connect timed out`；daemon 重启后自动重连恢复；SSE 端点仍可独立工作

---

## 组 1：Daemon 会话事件订阅内部 API

### 任务 1.1：抽出「按 session 订阅事件（可选 after 游标）」的内部 API

**目标:** 从 `GET /sessions/:id/events` SSE handler（`src/daemon/run_loop.rs::get_session_events`）中抽出可复用的订阅原语，SSE handler 改为调用它，行为零变化。抽出物供 WS 端点（任务 2.2）与 SSE 端点共享同一 buffer + replay + seam 去重 + `sync_lost` 语义。

**涉及文件:**
- 修改：`src/daemon/run_loop.rs`（`get_session_events` ~1314-1406、`plan_catch_up` ~1286-1300、`CatchUp` ~1273-1284、`sync_lost_event` ~1302-1312、`SessionEventBuffer` ~71-110）
- 修改：`src/daemon/state.rs`（如需要暴露辅助访问器，`session_event_hub` / `session_buffer` / `session_seq_counter` 已在）
- 测试：`src/daemon/run_loop.rs`（handler 内 `#[cfg(test)]` 单元测试）

**接口设计（新 API 形态，供 2.2 直接调用）:**
- 输入：`session_id`、`after: Option<u64>`、一个接收 `SessionEvent` 的出站 sink（可以是 mpsc sender 或闭包）。
- 输出：`seam_seq`（replay 后的最后 seq）+ 一个 `live` 接收器（或让调用方持有 hub subscription），返回 `CatchUp` 决策结果。
- 关键不变式：**先订阅 hub 再 replay**（`state.session_event_hub.subscribe()` 先于 `plan_catch_up`），live 分支按 `ev.session_id == id && ev.seq > seam_seq` 过滤，`RecvError::Lagged` 时发 `sync_lost`（`reason: "lagged"`）并把 `seam_seq` 跳到 `latest_seq()`。

**验证方式:**
- `cargo test daemon::run_loop --lib`（抽出后现有 SSE handler 单测仍绿）
- `cargo test --test integration daemon_events_replay`（SSE 端到端 replay/去重/sync_lost 行为不变）
- 手动核对：抽出前后 `get_session_events` 对外 HTTP 行为无差异（`?after=` 游标续传、多订阅者、404 未知 session）。

---

### 任务 1.2：为抽出的内部 API 补充/迁移单元测试

**目标:** 把 SSE 单测中覆盖的游标语义迁移/补齐到新内部 API 上，保证 WS 端点将来复用时不重新发明语义。

**涉及文件:**
- 修改：`src/daemon/run_loop.rs`（`#[cfg(test)] mod tests`，新增内部 API 直测）

**验证方式:**
- `cargo test daemon::run_loop --lib` 覆盖：
  - 游标续传：`after=3` 时 replay `seq>3` 后无缝衔接 live，seam 不重不漏；
  - `sync_lost`：`after` 滑出窗口（`after+1 < oldest`）→ `sync_lost{reason:"evicted"}`；buffer 空（daemon 重启后）→ `latest_seq: 0`；
  - 多订阅者：同一 hub 下两个订阅者收到相同事件序列，互相不干扰。

---

## 组 2：Daemon WebSocket 推送端点

### 任务 2.1：新增 `src/daemon/ws_push.rs`（单连接任务 + 信封协议）

**目标:** 新增 WS handler，完成 `GET /api/v1/ws?token=…` 升级后的 per-connection task：`select!` 三路事件源（session hub / trace hub / global bus）+ 上行控制消息 + 15s heartbeat，按设计 D2 信封协议序列化下行消息。

**涉及文件:**
- 新增：`src/daemon/ws_push.rs`
- 修改：`src/daemon/mod.rs`（`mod ws_push;`）
- 修改：`Cargo.toml`（`[dependencies]` 增加 `tokio-tungstenite`，如尚未有）

**信封协议（设计 D2，实现时对齐）:**
- 下行（JSON text frame）：`heartbeat`（无载荷，15s 节拍）、`trace`（`TraceEvent`）、`global`（`GlobalEvent` 含 `seq`）、`session`（`session_id` + `SessionEvent` 含 `seq`）、`subscribed`（`session_id` + `latest_seq`）。
- 上行：`subscribe`（`session_id` + `after?`）、`unsubscribe`（`session_id`）；未识别消息忽略并 debug 日志。
- 出站统一经 mpsc → 写半 task（序列化/发送不阻塞 `select!` 分支）。
- `WebSocketUpgrade` 配置 `max_message_size(16 MiB)`；trace/global 的 `Lagged` 静默丢弃（与 SSE 一致）。

**涉及文件（新增模块骨架）:**
- 新增：`src/daemon/ws_push.rs`（`ws_handler`、`on_upgrade`、`ConnectionLoop`、信封类型定义 + `Serialize`）

**验证方式:**
- `cargo check --all-targets`（编译通过）
- `cargo fmt -- --check` + `cargo clippy --all-targets -- -D warnings`

---

### 任务 2.2：实现 `subscribe`/`unsubscribe` 控制消息（复用内部 API）

**目标:** per-connection 订阅表 `HashMap<SessionId, SubState{ seam_seq, pending_replay }>`（上限 64，超限回错误信封但连接保持）；`subscribe` 同步执行任务 1.1 抽出的 `plan_catch_up(after, buffer)` → replay 帧直推 → 发 `subscribed{latest_seq}` → live 按 `session_id` 过滤 + `seq <= seam_seq` 去重；`unsubscribe` 移除该 session；连接断开时订阅表随任务结束整体释放。未知 `session_id` → `subscribed{latest_seq: 0}`（不报错，与 SSE 404 不同，客户端自行走 `GET /sessions/:id` 恢复）。

**涉及文件:**
- 修改：`src/daemon/ws_push.rs`
- 依赖：`src/daemon/run_loop.rs`（任务 1.1 抽出的 API）

**验证方式:**
- `cargo test daemon::ws_push --lib`（订阅表上限 64、未知 session 应答、退订只影响目标 session）

---

### 任务 2.3：WS 握手认证 + 注册路由

**目标:** `GET /api/v1/ws?token=<bearer>` 复用 `require_auth` 语义，但 token 提取顺序为 query `token` → `Authorization` header（后者留给非浏览器客户端）；无效凭证与受保护路由同构返回 401；token 轮换/失效时服务端以 close code `4001` 关闭存量连接（客户端触发 token 刷新重连）；`1000` 为正常关闭。

**涉及文件:**
- 修改：`src/daemon/ws_push.rs`（handler 内显式执行认证，不依赖中间件的 header-only 提取）
- 修改：`src/daemon/routes.rs`（`.route("/api/v1/ws", get(ws_push::ws_handler))`，加入 protected router）

**验证方式:**
- `cargo test daemon::ws_push --lib`（无 token / 错 token → 拒绝）
- 手动：`curl -i -N -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Key: ..." -H "Sec-WebSocket-Version: 13" "http://127.0.0.1:<port>/api/v1/ws"` → 401 同构；带 `?token=` 后握手 101。

---

### 任务 2.4：空闲关机计数 + heartbeat 保活

**目标:** WS 升级成功 `state.active_clients.register_client()`，连接任务退出（含 panic / 出错路径）经 Drop guard `unregister_client()`；仅 WS 连接存续时 daemon 不退出（对齐 SSE heartbeat 语义）；每 15s 发 `heartbeat` 信封做应用层保活（防代理闲置断连）。

**涉及文件:**
- 修改：`src/daemon/ws_push.rs`
- 参考：`src/daemon/state.rs`（`ActiveClientTracker` ~32-104）、`src/daemon/handlers.rs::client_heartbeat` ~2195-2252

**验证方式:**
- `cargo test daemon::ws_push --lib`（连接建立/断开后 `client_count()` 增/减；Drop guard 不泄漏）
- 手动：仅保持一条 WS 连接、无其他请求，daemon 不进入 idle shutdown（idle window `THIN_CLIENT_IDLE_TIMEOUT_SECS=300`）。

---

### 任务 2.5：daemon 集成测试（tokio-tungstenite）

**目标:** 在真实 Axum router + tokio-tungstenite 客户端上验证：信封类型完整性、订阅游标续传（断开重订阅不丢不重）、认证拒绝、SSE 与 WS 客户端并存等价、空闲关机。

**涉及文件:**
- 修改：`Cargo.toml`（`[dev-dependencies]` 增加 `tokio-tungstenite`）
- 新增：`tests/integration/daemon_ws_push.rs`
- 修改：`tests/integration/main.rs`（`mod daemon_ws_push;`）
- 复用：`tests/integration/daemon_harness.rs`（`spawn_daemon` / `spawn_daemon_custom` / `create_session` / `TEST_TOKEN`）、`tests/integration/daemon_events_replay.rs`（SSE 对照）

**验证方式:**
- `cargo test --test integration daemon_ws_push` 覆盖（对齐设计 §8 测试策略）：
  - 认证：无/错 token 升级拒绝（401 同构）
  - 信封：连接后收到 `heartbeat`；触发 run → `session` 信封（订阅后）；trace/global 事件 → 对应信封
  - 订阅：`after` 游标 replay 不丢不重；`unsubscribe` 后不收该 session；断开不影响其他订阅者；订阅上限 64 拒绝；未知 session → `subscribed{latest_seq:0}`
  - 并存：同一事件流下 SSE 与 WS 客户端收到等价事件（seq 一致）
  - 空闲关机：仅 WS 连接存续时 daemon 不退出
- 回归：`cargo test --test integration daemon_events_replay` 仍绿

---

## 组 3：Web WS 通道模块

### 任务 3.1：新增 `web/src/api/wsChannel.ts`（单例连接 + URL 派生 + 事件分发）

**目标:** 模块级单例（每 tab 一条连接）；连接状态机 `connecting → open → (drop) → backoff → connecting…`（指数退避 1s → 30s 封顶，成功后重置）；URL 派生：`resolveDaemonDirect()` → `ws://127.0.0.1:<port>/api/v1/ws?token=…`，null（desktop）→ `ws://<page-origin>/api/v1/ws`（经 token-injection WS patch 或 vite 代理）；close `4001` → 先刷新 token（desktop `read_daemon_token` / browser 重新 `resolveDaemonDirect`）再重连；按 `type` 分发事件总线（`on(type, handler)` 返回退订函数）。

**涉及文件:**
- 新增：`web/src/api/wsChannel.ts`
- 修改：`web/src/api/client.ts`（复用/导出 `resolveDaemonDirect` 或共享类型）
- 修改：`desktop/src/token-injection.js`（对称新增 `new WebSocket(url)` patch：url 匹配 `/api/v1/ws` → 改写为 `ws://127.0.0.1:<port>/api/v1/ws?token=<token>`，token 经现有 `__TAURI__.core.invoke("read_daemon_token")` 与 host 下发连接信息获取；close 4001 语义由 web 层处理）

**验证方式:**
- `cd web && npx vitest run src/api/wsChannel.test.ts`（mock WebSocket：open/message/close/close-code-4001）
- 手动（desktop）：`getPlatform()` 分支下 `new WebSocket("/api/v1/ws")` 被 patch 为 `ws://127.0.0.1:<port>/api/v1/ws?token=…`；Tauri webview 对 `ws://127.0.0.1` 的混合内容策略实测，受限则回退 SSE（`getPlatform()` 已有挂点）。

---

### 任务 3.2：引用计数订阅 API（resubscribe + trace 缺口补齐）

**目标:** 提供 `subscribeSession(sessionId, handler)` 返回退订函数（同 session 多订阅者一次 subscribe，引用计数到 0 才 `unsubscribe`）；断线期间事件自然断流（不伪造），重连成功回调对全部 active session 按保存 cursor `resubscribe(after=cursor)`，并触发一次 `traceReplay`（REST）补 trace 缺口；`sessionEvents(sessionId, after, signal)` 以 async iterator 暴露（内部 pending 队列 + 等待者），`sync_lost` 作为普通事件 yield（消费端语义不变）。

**涉及文件:**
- 修改：`web/src/api/wsChannel.ts`

**验证方式:**
- `cd web && npx vitest run src/api/wsChannel.test.ts`：
  - 引用计数（同 session 两订阅者一次 subscribe；退订函数只减计数，最后一个退订才发 `unsubscribe`）
  - 重连后 resubscribe 传保存 cursor
  - `sessionEvents` 迭代器：事件顺序 / 游标传参 / `AbortSignal` 中止
  - `sync_lost` 作为普通事件 yield

---

### 任务 3.3：wsChannel 单元测试（状态机 / 引用计数 / 游标去重 / 退订）

**目标:** 把 3.1/3.2 的行为固化为 vitest 用例（mock WebSocket）。

**涉及文件:**
- 新增：`web/src/api/wsChannel.test.ts`
- 新增（如缺 mock 工具）：`web/src/test/mockWebSocket.ts`

**验证方式:**
- `cd web && npx vitest run src/api/wsChannel.test.ts` 覆盖（对齐设计 §8）：
  - 连接/重连状态机 + 退避封顶
  - `on()` 分发（trace/global/heartbeat）
  - `sessionEvents` 迭代器（事件顺序 / 游标传参 / signal 中止）
  - 重连后按保存 cursor resubscribe；游标续传去重
  - 退订后不收该 session 事件

---

## 组 4：Web 消费点切换（行为不变，换数据源）

> 4.1–4.4 相互独立、均依赖组 3；4.5 依赖前四者。`fetchStream` 与全部 SSE client 方法保留不删。

### 任务 4.1：`App.tsx` heartbeat 切换为 WS 连接存续

**目标:** EventSource `/client/heartbeat` 替换为 WS 通道连接存续即保活；`onclose` 时 UI 状态改离线；清理 heartbeat SSE 代码路径。

**涉及文件:**
- 修改：`web/src/App.tsx`
- 参考：`web/src/api/client.ts`（heartbeat client 方法）

**验证方式:**
- `cd web && npx vitest run src/App.test.tsx`（mock 通道注入，连接即保活 / 断连离线）
- 手动：devtools Network 不再出现 `/client/heartbeat` EventSource。

---

### 任务 4.2：`usePermissionTrace` 切换为 `trace` 信封订阅

**目标:** `traceStream()` SSE 解析循环替换为 `on("trace", …)`；冷启动 replay（`traceReplay` REST）逻辑保留，路由分发（`permission_pending` / `permission_resolved` / `question_*` / `progress`）不变。

**涉及文件:**
- 修改：`web/src/hooks/usePermissionTrace.ts`

**验证方式:**
- `cd web && npx vitest run src/hooks/usePermissionTrace.test.ts`（假 SSE 流 → 假通道注入）
- 手动：subagent 权限提示仍实时弹出；trace 树仍更新。

---

### 任务 4.3：`useContinuationObserver` 切换为 `global` 信封订阅

**目标:** `globalEvents()` SSE 解析循环替换为 `on("global", …)`；`task_group_result` → `observeDaemonRun` 行为不变。

**涉及文件:**
- 修改：`web/src/hooks/useContinuationObserver.ts`

**验证方式:**
- `cd web && npx vitest run src/hooks/useContinuationObserver.test.ts`
- 手动：daemon 调度器 claim 的 synthesis continuation 在 web 可见。

---

### 任务 4.4：`runSessionTurn` / `observeDaemonRun` 切换为 `sessionEvents` 迭代器

**目标:** `client.sessionEvents()` SSE + reader 循环 + 断线重订阅，替换为 `for await (… of channel.sessionEvents(daemonId, lastSeq, abort.signal))`；重订阅由通道承担；`sync_lost`（`ev.kind === "sync_lost"` 对齐 `lastSeq`）与 stop 语义保持；`run_id` 过滤 / `seq <= lastSeq` 去重 / `turn_done`+`tool_calls` round boundary 判断逻辑保留；`fetchStream` SSE 路径保留不删。

**涉及文件:**
- 修改：`web/src/agent/sessionRunner.ts`（`runSessionTurn` ~43-227、`observeDaemonRun` ~261-360）

**验证方式:**
- `cd web && npx vitest run src/agent/sessionRunner.test.ts`
- 手动：发消息 → turn 正常渲染；daemon 重启后自动重连恢复事件流；Stop 按钮仍可中止。

---

### 任务 4.5：适配现有 web 测试（SSE 假流 → WS 信封假流）

**目标:** 4 个消费点（`usePermissionTrace` / `useContinuationObserver` / `sessionRunner` / `App`）现有测试的 mock 从 SSE 假流改为 WS 信封假流，断言行为不变。

**涉及文件:**
- 修改：`web/src/App.test.tsx`
- 修改：`web/src/hooks/usePermissionTrace.test.ts`、`web/src/hooks/useContinuationObserver.test.ts`
- 修改：`web/src/agent/sessionRunner.test.ts`
- 修改：`web/src/api/client.test.ts`、`web/src/api/sseParser.test.ts`（SSE 客户端方法仍保留，相关测试继续绿，仅新增 WS 通道 mock 分支）

**验证方式:**
- `cd web && npm test`（全量 vitest run 通过，无回归）

---

## 组 5：构建设施与验收

### 任务 5.1：`vite.config.ts` 代理增加 `/api` 的 `ws: true` 转发

**目标:** 代理对 `/api` 的 WS upgrade 转发（`proxy["/api"].ws = true`）；代理注入 token 的 `configure` 回调对 WS 握手同样生效（header 形式，同源回退路径由代理补 token）；验证 dev 直连与代理两条 WS 路径。

**涉及文件:**
- 修改：`web/vite.config.ts`

**验证方式:**
- 手动：vite dev 下，直连（`/__daemon-info` 返回时 `ws://127.0.0.1:<port>`）与同源代理（`ws://<page-origin>/api/v1/ws`）两条路径均握手成功并收到 `heartbeat`。

---

### 任务 5.2：端到端验收

**目标:** 单条 WS 承载全部推送；subagent 密集场景无 `stream connect timed out`；daemon 重启自动重连恢复；SSE 端点仍可独立工作；desktop shell token patch 生效。

**验证方式（手动，对齐设计 §8 端到端）:**
- devtools Network：1 条 WS、0 条 SSE（heartbeat/trace/global/session 全部消失）
- subagent 密集场景（≥3 并发 run + `task_group_result`）无 `stream connect timed out`
- daemon 重启：WS 自动重连 + 订阅恢复（按 cursor resubscribe）+ trace replay 补齐
- 存量 SSE：另开客户端直连 `GET /events` / `GET /sessions/:id/events` / `GET /subagents/trace/stream` / `GET /client/heartbeat` 均正常（并存等价）
- desktop shell：`new WebSocket` token patch 生效，WS 直连成功（受限则走 `getPlatform()` SSE 回退）

**回滚演练:** 将 4 个消费点切回 SSE 调用（git revert 本 change 的 web 消费点改动），确认旧 SSE 路径仍完整可用（`fetchStream` 与 SSE client 方法未删）。

---

## 完成定义（Definition of Done）

- `cargo test daemon::run_loop --lib`、`cargo test daemon::ws_push --lib`、`cargo test --test integration daemon_ws_push`、`cargo test --test integration daemon_events_replay` 全绿
- `cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings` 通过
- `cd web && npm test` 全绿
- 5.2 端到端验收全部通过；SSE 端点并存可用；回滚演练可执行
