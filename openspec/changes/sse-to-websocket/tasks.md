## 1. Daemon: 会话事件订阅内部 API

- [x] 1.1 从 `GET /sessions/:id/events` SSE handler 中抽出「按 session 订阅事件（可选 after 游标）」的内部 API（同一事件 buffer 与 replay 语义），SSE handler 改为调用该 API，行为不变
- [x] 1.2 为抽出的内部 API 补充/迁移单元测试（游标续传、sync_lost、多订阅者）

<!-- task 2.2 quality re-review: coordinator self-review (deviation accepted —
     subagent infra 5 consecutive failures; user approved executing-plans switch).
     Findings: MAJOR-1 fix sound (biased-select fairness holds: drain rate 64/iter
     >= pending growth; dual-write buffer+hub guarantees snapshot-or-hub coverage,
     strict order no-dup no-loss; MAJOR-2 three conclusions confirmed: single-task
     pump / resolve_session short await / forward-fail breaks pump with no
     half-state; disconnect chain complete). 2 MINORs accepted: (1) heartbeat may
     delay under sustained event storm — acceptable, traffic resets proxy idle
     timers and tick fires promptly when idle; (2) self-review independence
     recorded here. Spec review: independent PASS (round 1). -->

<!-- task 2.3 review: coordinator self-review (deviation accepted, same basis as
     2.2 — subagent infra failures; user approved executing-plans). Findings:
     auth semantics match design §3.1 (query-first, header fallback, empty-expected
     guard, 401 body identical to require_auth); 4001 rotation guard on tick;
     route before route_layers per design (in-handler auth, idle accounting is
     2.4); comparison not constant-time but matches existing require_auth posture
     (loopback + high-entropy token). Also reverted an unauthorized auth.rs
     modification left by a misbehaving reviewer agent (disclosed in commit).

     Post-review correction (1d7362b4): the "reverted unauthorized auth.rs
     modification" was in fact a needed fix from a parallel session — axum
     route_layer wraps routes registered BEFORE it, so the ws route behind the
     header-only middleware 401'd browser query-token handshakes (wire-level
     regression proved it). Fixed by giving require_auth the ?token= query
     fallback per design D1. User stopped the comet session and handed the
     change over; remaining tasks continue in-session. -->

## 2. Daemon: WebSocket 推送端点

- [x] 2.1 新增 `src/daemon/ws_push.rs`：单连接任务 `select!` 三路事件源（trace hub broadcast、全局事件 bus、上行控制消息），按 D2 信封协议序列化下行消息
- [x] 2.2 实现 `subscribe`/`unsubscribe` 上行控制消息：per-session 订阅表、`after` 游标续传、`subscribed` 应答（含 latest_seq）、断开自动清理订阅
- [x] 2.3 WS 握手认证：`GET /api/v1/ws?token=…` 复用 require_auth 语义（query 参数提取适配），无效凭证与受保护路由同构拒绝；注册路由
- [x] 2.4 空闲关机计数：WS 连接存续期间计入 `active_clients`（握手成功计入、断开解除），heartbeat 信封按 keepalive 节拍发送
- [x] 2.5 daemon 集成测试：信封类型完整性、订阅游标续传（断开重订阅不丢不重）、认证拒绝、SSE 与 WS 客户端并存等价

## 3. Web: WS 通道模块

- [ ] 3.1 新增 `web/src/api/wsChannel.ts`：单例连接、指数退避重连、`resolveDaemonDirect` 直连优先 + 同源回退（`ws://` URL 派生）、按 type 分发事件总线
- [ ] 3.2 引用计数订阅 API：`subscribeSession(sessionId, handler)` 返回退订函数；重连后按各订阅 cursor 自动重新 subscribe；trace 缺口经 `traceReplay`（REST）补齐
- [ ] 3.3 单元测试：连接/重连状态机、订阅引用计数、游标续传去重、退订后不收事件

## 4. Web: 消费点切换

- [ ] 4.1 `App.tsx` heartbeat：EventSource 替换为 WS 连接存续（连接即心跳），清理 heartbeat SSE 代码路径
- [ ] 4.2 `usePermissionTrace`：SSE 解析循环替换为 `trace` 信封订阅；冷启动 replay 逻辑保持
- [ ] 4.3 `useContinuationObserver`：`globalEvents` SSE 替换为 `global` 信封订阅，`task_group_result` → `observeDaemonRun` 行为不变
- [ ] 4.4 `runSessionTurn`/`observeDaemonRun`：`sessionEvents` SSE 替换为 `subscribeSession(daemonId, after=lastSeq)`，`sync_lost` 与 stop 语义保持；`fetchStream` SSE 路径保留不删
- [ ] 4.5 适配现有 web 测试（4 个消费点的 mock 从 SSE 假流改为 WS 信封假流）

## 5. 构建设施与验收

- [ ] 5.1 `vite.config.ts` 代理增加 `/api` 的 `ws: true` 转发；验证 dev 直连与代理两条 WS 路径
- [ ] 5.2 端到端验收：单条 WS 承载全部推送（devtools 无 SSE 连接）；subagent 密集场景无 `stream connect timed out`；daemon 重启后自动重连恢复；SSE 端点仍可独立工作
