# Change 1：服务端 Agent Loop（daemon）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** daemon 新增 `POST /sessions/:id/run`、`POST /sessions/:id/cancel`、`GET /sessions/:id/events`——服务端跑完整 agent turn，curl 可端到端验证。

**Architecture:** 每会话 run registry（单 run + 每 turn CancellationToken + run 锁挡 update_session）；run 任务用现成 `run_agent_loop` + `MutexHistoryStore` + `ApiLlmPort` + 新 RootToolPort（policy → session_rules → PermissionBridge 无超时挂起 → guardian → registry，workdir 取会话绑定）；`DaemonEventSink` 把 `RuntimeEvent` 映射为 `SessionEvent` 经全局 hub 广播（SSE 按 session 过滤）；`SaveSession` 触发整史持久化。

**Tech Stack:** Rust + axum + tokio（broadcast）；cargo test。

**Spec:** `docs/superpowers/specs/2026-08-02-server-side-agent-loop-design.md`（§Change 1）

## Global Constraints

- `cargo clippy --all-targets -- -D warnings` 零 warning；`cargo fmt`；commit 英文 Conventional Commits。
- 每任务单独 commit；`cargo test <模块>` 通过。
- 已有行为零回归：`/chat/stream`、`/tools/execute`、trace SSE、权限流程不动（PermissionBridge 只做增量）。
- `SessionEvent` JSON 信封字段固定为 `{seq, session_id, run_id, kind, data}`——WebSocket 升级只换传输层。
- 关键事实（已核实，直接引用）：
  - `RunLoopArgs { llm, tools, events, history, config, state, stream_style, hooks, system_messages }`（loop_.rs:137），`LoopHooks` 全 Option 可 `Default::default()`，`StreamStyle::default()` 适用。
  - `ApiClient::with_clients(settings, http_client, http_client_stream)`；daemon 构造见 handlers.rs:182-191。
  - `MutexHistoryStore::new(Arc<Mutex<Vec<ChatMessage>>>)` + `handle()`。
  - `SessionMessage` ≠ `ChatMessage`：经 `serde_json::to_value` → `from_value` 互转（memory_session.rs:866-913 有先例）。
  - `sanitize_tool_call_pairing` 是 `crate::api::types` 的 pub 函数（api/types.rs:141）。
  - `PermissionBridge::request_with_timeout`（permission_bridge.rs:91-138）：insert oneshot → 发 pending trace → timeout 等待 → 移除 → 发 resolved trace。
  - `update_session` PUT 处理在 handlers.rs:919-955。

## 文件结构

| 文件 | 职责 |
|:--|:--|
| `src/daemon/run_loop.rs`（新建） | SessionEvent 信封 + hub、DaemonEventSink、RootToolPort、run/cancel/events handlers、持久化桥、run registry 类型（全部单测） |
| `src/daemon/state.rs`（改） | `session_runs: Arc<RwLock<HashMap<String, SessionRun>>>` + `session_event_hub: broadcast::Sender<SessionEvent>` |
| `src/daemon/handlers.rs`（改） | `update_session` 加 run 锁（409） |
| `src/daemon/routes.rs`（改） | 注册 3 条路由 |
| `src/teams/permission_bridge.rs`（改） | 提取 `enqueue` + 新增 `request_indefinite` |

---

### Task 1: SessionEvent 信封 + hub + DaemonEventSink 映射

**Files:**
- Create: `src/daemon/run_loop.rs`（本任务只放事件层）
- Modify: `src/daemon/mod.rs`（`pub(crate) mod run_loop;`）、`src/daemon/state.rs`（hub 字段）

**Interfaces:**
- Produces（Task 4/5 依赖）：
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct SessionEvent {
      pub seq: u64,
      pub session_id: String,
      pub run_id: String,
      pub kind: SessionEventKind,
      pub data: serde_json::Value,
  }
  #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
  #[serde(rename_all = "snake_case")]
  pub enum SessionEventKind {
      ContentDelta, ReasoningDelta, ToolStart, ToolResult,
      TurnDone, TurnError, Save,
  }
  pub type SessionEventHub = tokio::sync::broadcast::Sender<SessionEvent>;
  pub struct DaemonEventSink { session_id, run_id, hub, next_seq: Arc<AtomicU64> }
  impl EventSink for DaemonEventSink { fn emit(&self, ev: RuntimeEvent) }
  ```

- [ ] **Step 1: 写失败测试**

`run_loop.rs` 内 `#[cfg(test)]`：

```rust
#[test]
fn maps_runtime_events_to_session_events() {
    let (hub, mut rx) = tokio::sync::broadcast::channel(16);
    let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
    sink.emit(RuntimeEvent::ContentDelta("hi".into()));
    sink.emit(RuntimeEvent::ReasoningDelta("think".into()));
    sink.emit(RuntimeEvent::ToolStart { name: "file_read".into(), args: serde_json::json!({"path":"a"}) });
    sink.emit(RuntimeEvent::SaveSession);
    let e1 = rx.try_recv().unwrap();
    assert_eq!(e1.kind, SessionEventKind::ContentDelta);
    assert_eq!(e1.data["text"], "hi");
    assert_eq!(e1.seq, 1);
    let e2 = rx.try_recv().unwrap();
    assert_eq!(e2.kind, SessionEventKind::ReasoningDelta);
    let e3 = rx.try_recv().unwrap();
    assert_eq!(e3.kind, SessionEventKind::ToolStart);
    assert_eq!(e3.data["name"], "file_read");
    let e4 = rx.try_recv().unwrap();
    assert_eq!(e4.kind, SessionEventKind::Save);
    // seq 单调递增
    assert_eq!(e4.seq, 4);
}

#[test]
fn unmapped_variants_are_skipped() {
    // Connecting/PreparingTools/StreamDone/CompactionStarted 等不产生事件
    //（v1 不广播连接噪声）；StreamError → TurnError；StreamDone → TurnDone
    let (hub, mut rx) = tokio::sync::broadcast::channel(16);
    let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
    sink.emit(RuntimeEvent::Connecting { attempt: 1, max_retries: 2 });
    sink.emit(RuntimeEvent::StreamDone { finish_reason: "stop".into() });
    sink.emit(RuntimeEvent::StreamError("boom".into()));
    assert!(rx.try_recv().is_ok_and(|e| e.kind == SessionEventKind::TurnDone));
    assert!(rx.try_recv().is_ok_and(|e| e.kind == SessionEventKind::TurnError));
    assert!(rx.try_recv().is_err());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test daemon::run_loop`
Expected: FAIL（类型不存在）

- [ ] **Step 3: 实现**

- `SessionEvent`/`SessionEventKind`/`DaemonEventSink`（`emit` 按上表映射；seq 从 1 起 `fetch_add(1, Ordering::Relaxed)`）
- `DaemonState` 加 `pub session_event_hub: SessionEventHub`（构造处 `broadcast::channel(1024).0`）

- [ ] **Step 4: 验证 + Commit**

Run: `cargo test daemon::run_loop && cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add src/daemon/
git commit -m "feat(daemon): add SessionEvent envelope and runtime event sink"
```

---

### Task 2: PermissionBridge.request_indefinite

**Files:**
- Modify: `src/teams/permission_bridge.rs`

**Interfaces:**
- Consumes: 现有 `request_with_timeout`（:91-138）
- Produces: `pub async fn request_indefinite(&self, approval: StructuredApproval) -> bool`——挂起直到 resolve（无 deadline）；Task 3 的 RootToolPort 只调用它

- [ ] **Step 1: 写失败测试**

沿用该文件现有测试模式（若无则仿写）：

```rust
#[tokio::test]
async fn indefinite_waits_until_resolved() {
    let bridge = PermissionBridge::with_timeout_secs(1); // timeout 与此路径无关
    let approval = StructuredApproval::policy_ask("r1".into(), "sess".into(), "file_edit".into(), "reason".into(), "path:/x".into());
    let other = bridge.clone();
    let waiter = tokio::spawn(async move { other.request_indefinite(approval).await });
    // 等 pending 注册后 resolve；不应在 1s 默认超时时返回
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(!waiter.is_finished()); // 超过了 default_timeout 仍在挂起
    let pending = bridge.pending().await;
    assert!(bridge.resolve(&pending[0].request_id, true).await);
    assert!(waiter.await.unwrap());
}
```

- [ ] **Step 2: 跑测试确认失败 → 实现**

把 `request_with_timeout` 主体拆为私有 `async fn enqueue(&self, approval) -> oneshot::Receiver<bool>`（insert + pending trace），`request_with_timeout` = enqueue + `tokio::time::timeout` + 收尾，`request_indefinite` = enqueue + `rx.await` + 相同收尾（移除 entry + resolved trace）。提取共用 `finish(request_id, result)` 避免重复。

- [ ] **Step 3: 验证 + Commit**

Run: `cargo test teams::permission_bridge && cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add src/teams/permission_bridge.rs
git commit -m "feat(teams): add indefinite permission wait for root server loops"
```

---

### Task 3: RootToolPort

**Files:**
- Modify: `src/daemon/run_loop.rs`（追加）

**Interfaces:**
- Consumes: `ToolPort` trait（`ports.rs:69-77`：`execute(ToolRequest) -> ToolResponse`、`definitions()`）、`ToolPermissionPolicy::from_settings`、`Guardian::classify_risk`、`state.tool_executor` 的共享 session_rules（见 state.rs:162 附近，`ToolExecutor` 的已批准规则集）、Task 2 的 `request_indefinite`
- Produces：
  ```rust
  pub struct RootToolPort { /* 全部 owned */ }
  impl RootToolPort {
      pub fn new(state: &DaemonState, session_id: &str, workdir: Option<PathBuf>) -> Self;
  }
  impl ToolPort for RootToolPort { ... }
  ```

- [ ] **Step 1: 写失败测试**

```rust
// 用 tempdir + ToolRegistry::with_project_root + 内存 DaemonState 不可行（太重），
// 拆成可测的纯决策函数：
// decide(policy_allow: bool, rule_approved: bool, mode_auto: bool) -> Decision
// 以及 execute 的集成测试用 registry + 真实 file_write 落在 workdir：
#[tokio::test]
async fn executes_in_bound_workdir() {
    // tempdir 作为 project root 建 registry；RootToolPort::new_for_test(registry, rules, bridge, workdir)
    // execute file_write {path: "a.txt", content: "x"} → 文件落在 workdir/a.txt
    // policy 需放开 file_write（构造 ToolPermissionPolicy 允许全部，或用 AcceptEdits mode）
}

#[tokio::test]
async fn ask_suspends_until_bridge_resolve() {
    // policy Ask 的工具（如 file_edit 默认策略）→ spawn execute；
    // bridge.pending() 出现 → resolve(true) → execute 完成且文件写入
}
```

- [ ] **Step 2: 实现**

`RootToolPort::execute` 管线（参照 `GuardingToolPort::execute` 与 headless `RegistryToolPort`）：

1. `policy.validate(tool_name, &args)` → Allow：直接执行
2. Ask：session_rules 已含 rule → 执行；root_mode.auto_approves → 执行；否则 `StructuredApproval::policy_ask(uuid, session_id, tool, reason, rule)` → `bridge.request_indefinite` → true 则（把 rule 写入共享 rules）执行，false 返回 denial ToolResponse
3. guardian 预检（execute_command/exec_command 且 risk ≥ Critical → block，照抄 headless :105-172）
4. `registry.execute_with_context`（ToolContext：root agent context、workdir = 会话绑定、effective_mode、checkpoint + turn_id）

注意：borrow 问题——全部 owned（`Arc<ToolRegistry>`、owned `AgentExecutionContext::root(SessionId::new(session_id))`）。`AgentExecutionContext` 用 `state.root_context(session_id)` 的缓存值（与 /tools/execute 共享，subagent 继承取消语义一致）。

- [ ] **Step 3: 验证 + Commit**

Run: `cargo test daemon::run_loop && cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add src/daemon/run_loop.rs
git commit -m "feat(daemon): add RootToolPort with indefinite bridge escalation"
```

---

### Task 4: run registry + POST /run + POST /cancel

**Files:**
- Modify: `src/daemon/state.rs`（`session_runs` 字段）、`src/daemon/run_loop.rs`（handlers + run 任务）、`src/daemon/routes.rs`、`src/daemon/handlers.rs`（update_session run 锁）

**Interfaces:**
- Produces（Change 2 依赖）：
  ```
  POST /api/v1/sessions/:id/run     {message} → 202 {run_id, session_id} | 404 | 409 | 400
  POST /api/v1/sessions/:id/cancel  → 204 | 404
  PUT  /api/v1/sessions/:id         run 活跃时 → 409 {"error":"run active"}
  ```
  ```rust
  pub struct SessionRun { pub run_id: String, pub cancel: CancellationToken, pub started_at: std::time::Instant }
  ```

- [ ] **Step 1: 写失败测试**

`run_loop.rs` 测试（registry 纯逻辑，不启动真 run）：

```rust
#[test] fn claim_rejects_second_run() { /* claim ok → 第二次 → Err(409) */ }
#[test] fn finish_releases_claim() { /* claim → finish → 再 claim 成功 */ }
```

handlers 层（路由级测试若项目无先例，则测 handle 函数本身 + mock 状态太重→改为测 registry + 在 Task 5 做 curl 冒烟验证，此处如实说明）。

- [ ] **Step 2: 实现**

`run_loop.rs` 追加：

```rust
pub(crate) struct RunRegistry { inner: Arc<RwLock<HashMap<String, SessionRun>>> }
impl RunRegistry {
    pub fn claim(&self, session_id: &str, run: SessionRun) -> Result<(), (StatusCode, String)>;
    pub fn finish(&self, session_id: &str, run_id: &str);
    pub fn cancel(&self, session_id: &str) -> bool;
    pub fn is_active(&self, session_id: &str) -> bool;
}
```

`post_run` handler：
1. 空消息 → 400；`session_manager.get(&id)` 无 → 404
2. `registry.claim(...)` → 409
3. spawn run 任务（§spec Change 1 §2 步骤 2-8：seed history 自 `Session.messages`（serde 互转）+ push user msg；ApiLlmPort；RootToolPort（workdir = `state.session_workdir(&id)`）；DaemonEventSink；`prompts::assemble_instructions`（PromptContext 参照 headless_runtime.rs:214-227 构造）；`tokio::select!` loop vs cancel；结束发 TurnDone/TurnError 事件 + 最终 save + `registry.finish`）

`post_cancel`：`registry.cancel(id)` → true: 204 / false: 404。

`update_session` 锁（handlers.rs:912 函数开头）：

```rust
if state.session_runs.read().await.contains_key(&id) {
    return Err((StatusCode::CONFLICT, "run active".to_string()));
}
```

（注意该 handler 返回类型，匹配现有错误风格。）

- [ ] **Step 3: 验证 + Commit**

Run: `cargo test daemon:: && cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add src/daemon/
git commit -m "feat(daemon): add session run registry with run/cancel endpoints"
```

---

### Task 5: GET /sessions/:id/events SSE + 持久化桥 + 冒烟

**Files:**
- Modify: `src/daemon/run_loop.rs`（SSE handler + save bridge）、`src/daemon/routes.rs`

**Interfaces:**
- Consumes: Task 1 hub、Task 4 run 任务
- Produces: `GET /api/v1/sessions/:id/events`（SSE，`text/event-stream`，逐行 `data: <SessionEvent JSON>\n\n`）

- [ ] **Step 1: 实现 SSE handler**

模式照抄 trace SSE（handlers.rs:308-366）：`hub.subscribe()` → 过滤 `ev.session_id == id` → `Sse::new(stream)`；每 15s keep-alive comment。无冷回放（v1，spec 已定）。

- [ ] **Step 2: 实现持久化桥**

DaemonEventSink 处理 `RuntimeEvent::SaveSession` 时（除广播 Save 事件外）触发保存：

```rust
// 在 run 任务里持有 history handle；SaveSession 时：
let mut msgs: Vec<ChatMessage> = history_handle.lock().await.clone();
crate::api::types::sanitize_tool_call_pairing(&mut msgs);
// ChatMessage → SessionMessage（serde 互转），load session → 替换 messages → save()
```

实现为 run 任务内的一个 `save_now(state, session_id, history_handle)` 函数；turn 结束强制再调一次。单测：构造含 tool_calls/tool 配对的 history → save 后 `Session.messages` 配对完整（`tool_calls[i].id` 与后续 `tool_call_id` 对齐）。

- [ ] **Step 3: 路由注册 + 手动冒烟**

routes.rs 注册三条。手动冒烟（写进报告）：

```bash
# 起 daemon，curl 序列：
curl -X POST /api/v1/sessions -d '{"name":"smoke"}'           # → id
curl -N /api/v1/sessions/<id>/events &                        # 挂 SSE
curl -X POST /api/v1/sessions/<id>/run -d '{"message":"用 file_write 在工作区建 hello.txt"}'  # → 202
# 观察 SSE 事件流：tool_start → tool_result → turn_done；文件真实落盘
curl /api/v1/sessions/<id>                                    # 消息历史含 tool_calls 配对
curl -X POST /api/v1/sessions/<id>/run -d '{"message":"x"}'   # 第二次（串行应成功）
```

- [ ] **Step 4: 验证 + Commit**

Run: `cargo test daemon:: && cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add src/daemon/
git commit -m "feat(daemon): stream session events and persist loop history"
```

---

## 验收清单

- [ ] `cargo test --all` 通过，`cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] curl 冒烟全过（202/409/204/事件流/持久化配对）
- [ ] 权限冒烟：run 触发 Ask → `/tools/pending-permissions` 可见 → resolve 后继续执行（验证无限期挂起 + 现有 web 链路兼容）
