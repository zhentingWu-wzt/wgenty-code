---
change: daemon-session-orchestration
design-doc: docs/superpowers/specs/2026-08-07-daemon-session-orchestration-design.md
base-ref: d5f046a58c8aa9a9989e9ec346d35ccad637721a
---

# daemon-session-orchestration 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标：** 在已落地的 server-side loop（`run_session_turn`/`RunRegistry`/`SessionEventHub`）之上补齐五个可靠性缺口：会话事件流重放与失步信号、全局事件总线（含 TUI todos dogfood 迁移）、审批语义收敛、会话存储版本化、daemon 可发现部署。

**架构：** 全局单 daemon。`SessionEventHub` 增加 per-session 环形重放缓冲与 `after=<seq>` 续传、`SyncLost` 失步事件；新增 `GlobalEventHub`（独立 seq 空间）承载 todos/背景结果/模式/模型/task-group 事件并暴露 `GET /events`；审批重复应答统一 409；`Session` 增加单调 `version` 字段做乐观并发控制；daemon 写入 `~/.wgenty-code/daemon.json` 发现文件（30s 心跳、120s 过期），UI 启动先发现后拉起。

**技术栈：** Rust / tokio / axum（SSE）/ serde；错误处理 `thiserror`（库）+ `anyhow`（应用层）。

**上游文档：**
- Design Doc：`docs/superpowers/specs/2026-08-07-daemon-session-orchestration-design.md`（模块设计、错误处理矩阵 §8、测试策略 §9、边界条件 §10）
- 任务边界：`openspec/changes/daemon-session-orchestration/tasks.md`（6 组）
- delta specs：`openspec/changes/daemon-session-orchestration/specs/daemon-session-orchestration/spec.md`、`specs/daemon-event-stream/spec.md`

## Global Constraints

- `cargo fmt` 统一格式；`cargo clippy --all-targets -- -D warnings` 零 warning（CI 强制）。
- 库代码用 `thiserror` 派生错误枚举；应用层用 `anyhow::Result` + `.context("...")`；禁止无上下文裸 `?` 与无理由 `unwrap()`（锁中毒沿用现有 `expect("lock poisoned: ...")` 惯例）。
- Commit 遵循 Conventional Commits（英文），格式 `<type>(<scope>): <描述>`，scope 用 `daemon` / `tui` / `config`。
- 用户可见字符串走 `i18n/`（Fluent）；daemon API 的 JSON 错误体沿用现有英文字面量惯例。
- 跨平台：linux/macos/windows 均可编译；发现文件原子写不依赖平台特定 API。
- 向后兼容红线（spec「向后兼容」Requirement）：无 `after` 的 events 订阅维持 live-only；`PUT /sessions/:id` 无 `expected_version` 维持现状；旧 chat_stream 配套端点保留 `"default"` 兼容映射；TUI client-side / server-side 双模式行为不变。
- 每任务结束运行 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` 后再提交。

## 现状关键锚点（base-ref 上的行号）

| 位置 | 内容 |
|------|------|
| `src/daemon/run_loop.rs:26-50` | `SessionEvent` / `SessionEventKind`（9 个变体，snake_case serde） |
| `src/daemon/run_loop.rs:54` | `pub type SessionEventHub = broadcast::Sender<SessionEvent>`（全局单 hub，按 session_id 过滤） |
| `src/daemon/run_loop.rs:128-139` | `DaemonEventSink::publish`（run 事件唯一发布点之一） |
| `src/daemon/run_loop.rs:379-388` | `RootToolPort::publish_event`（PermissionRequired/AskUser 发布点） |
| `src/daemon/run_loop.rs:785-824` | `get_session_events`（live-only，Lagged 仅 warn 于 810-817） |
| `src/daemon/state.rs:135,491,692` | `session_seq_counters` 与 `session_seq_counter()` |
| `src/daemon/state.rs:64` | `todo_state: Arc<RwLock<TodoState>>`（**当前无任何写入方**，见 Task 7 说明） |
| `src/daemon/handlers.rs:485,654,664,699,740,765,1397,1431,1458,1507` | 10 处 `"default"` 硬编码 |
| `src/daemon/handlers.rs:715-731` | `resolve_interaction`（已决议返回 404，见 729） |
| `src/daemon/handlers.rs:692-710` | `resolve_subagent_permission`（已决议返回 `{success:false}`） |
| `src/daemon/handlers.rs:849-854` | `get_background_results` 调 `drain_results()`（抢占语义） |
| `src/daemon/handlers.rs:992-1054` | `update_session`（run 活跃 409 于 998-1003；无版本校验） |
| `src/context/memory_session.rs:12-34` | `Session` 结构（无 version 字段）；`save` 于 :441 |
| `src/utils/mod.rs:146-188` | `daemon.token` 读写删助手 |
| `src/daemon/mod.rs:37-139` | `daemon::run`（token 写入 :89-94，退出清理 :136） |
| `src/tui/util.rs:27-` | TUI `start_daemon`（进程内嵌 daemon，绑定 8371/随机端口） |
| `src/tui/client.rs:939` | TUI `get_todos` 轮询客户端 |
| `src/daemon/routes.rs` | 路由注册（todos :99、background/results :107、session events :155-156、interactions resolve :84-85、resolve-permission :79-80、permission-mode :92、model/switch :66） |
| `src/config/mod.rs:29-44` | `Settings`（无 daemon 配置段） |

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/daemon/run_loop.rs` | `SessionEventBuffer`、`SyncLost` 变体、publish 双写、after 重放、Lagged 失步 | 修改 |
| `src/daemon/global_events.rs` | `GlobalEvent`/`GlobalEventKind`/`GlobalEventHub`、`GET /events` handler | 新建 |
| `src/daemon/state.rs` | `session_buffers`、`global_event_hub`、`global_seq_counter`、`background_results` 保留队列、`broadcast_global` 助手 | 修改 |
| `src/daemon/handlers.rs` | 审批 409、`"default"` 清理、背景结果快照读、`expected_version` 校验、生产者挂接 | 修改 |
| `src/daemon/interaction_bridge.rs` | 已决议记录与 `ResolveOutcome` 三分结果 | 修改 |
| `src/daemon/routes.rs` | 注册 `GET /events` | 修改 |
| `src/context/memory_session.rs` | `Session.version` 字段 | 修改 |
| `src/daemon/models.rs` | `UpdateSessionRequest.expected_version` 等请求/响应模型 | 修改 |
| `src/config/mod.rs` | `DaemonConfig { event_buffer_capacity }` | 修改 |
| `src/utils/mod.rs`（或 `src/utils/discovery.rs`） | 发现文件读写、心跳、`discover_daemon()` | 新建模块 + 修改 |
| `src/daemon/mod.rs` | 启动写发现文件、心跳任务、退出清理 | 修改 |
| `src/tui/util.rs` / `src/tui/client.rs` | `start_daemon` 前置发现；todos 订阅化 + 轮询回退 | 修改 |

---

## 第 1 组：事件流重放与失步信号（tasks.md 1.1–1.3）

### Task 1: `SessionEventBuffer` 环形缓冲 + 容量配置

- [x] Task 1 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/run_loop.rs`（新增 buffer 结构，约 :54 `SessionEventHub` 定义之后）
- Modify: `src/daemon/state.rs:135`（`session_seq_counters` 旁新增 `session_buffers`）
- Modify: `src/config/mod.rs:29-44`（`Settings` 新增 `daemon` 配置段）
- Test: `src/daemon/run_loop.rs` 内 `#[cfg(test)] mod tests`（既有，:1059 起）

**Interfaces:**
- Produces:
  - `pub(crate) struct SessionEventBuffer { events: VecDeque<SessionEvent>, capacity: usize }`，方法 `push(&mut self, ev: SessionEvent)`（满则 `pop_front`）、`oldest_seq(&self) -> Option<u64>`、`latest_seq(&self) -> Option<u64>`、`events_after(&self, after: u64) -> impl Iterator<Item = &SessionEvent>`
  - `DaemonState::session_buffer(&self, session_id: &str) -> std::sync::Arc<std::sync::RwLock<SessionEventBuffer>>`（与 `session_seq_counter()` 同模式，懒创建）
  - `DaemonState::event_buffer_capacity(&self) -> usize`（读 `settings.daemon.event_buffer_capacity`，默认 1024）
  - `config::DaemonConfig { pub event_buffer_capacity: usize }`（`Default = 1024`），`Settings.daemon: DaemonConfig`（`#[serde(default)]`）

- [ ] **Step 1: 写失败测试（buffer 淘汰与 oldest/latest 边界）**

在 `src/daemon/run_loop.rs` 的 `mod tests` 中加：

```rust
fn ev(seq: u64) -> SessionEvent {
    SessionEvent {
        seq,
        session_id: "s".into(),
        run_id: "r".into(),
        kind: SessionEventKind::Save,
        data: serde_json::json!({}),
    }
}

#[test]
fn buffer_evicts_oldest_when_full() {
    let mut buf = SessionEventBuffer::new(3);
    for seq in 1..=5 {
        buf.push(ev(seq));
    }
    assert_eq!(buf.oldest_seq(), Some(3));
    assert_eq!(buf.latest_seq(), Some(5));
    let after: Vec<u64> = buf.events_after(3).map(|e| e.seq).collect();
    assert_eq!(after, vec![4, 5]);
}

#[test]
fn buffer_empty_bounds() {
    let buf = SessionEventBuffer::new(3);
    assert_eq!(buf.oldest_seq(), None);
    assert_eq!(buf.latest_seq(), None);
    assert_eq!(buf.events_after(0).count(), 0);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::run_loop -- buffer 2>&1 | tail -5`
Expected: 编译失败（`SessionEventBuffer` 未定义）

- [ ] **Step 3: 实现 `SessionEventBuffer` 与配置段**

`src/daemon/run_loop.rs`（`SessionEventHub` 定义之后）：

```rust
/// Per-session fixed-capacity replay buffer. Lives only for the daemon
/// process lifetime — after a restart it is empty and `after=` resumes
/// answer `SyncLost` (correctness never depends on the buffer; clients
/// fall back to `GET /sessions/:id`). Capacity comes from
/// `daemon.event_buffer_capacity` (default 1024, aligned with TRACE_HUB).
pub(crate) struct SessionEventBuffer {
    events: VecDeque<SessionEvent>,
    capacity: usize,
}

impl SessionEventBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { events: VecDeque::with_capacity(capacity.min(4096)), capacity }
    }

    /// Push an event; evicts the oldest when full. Must be called at the same
    /// point the event is published to the hub so buffer and broadcast agree.
    pub(crate) fn push(&mut self, ev: SessionEvent) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(ev);
    }

    pub(crate) fn oldest_seq(&self) -> Option<u64> {
        self.events.front().map(|e| e.seq)
    }

    pub(crate) fn latest_seq(&self) -> Option<u64> {
        self.events.back().map(|e| e.seq)
    }

    pub(crate) fn events_after(&self, after: u64) -> impl Iterator<Item = &SessionEvent> {
        self.events.iter().filter(move |e| e.seq > after)
    }
}
```

`src/config/mod.rs` `Settings` 中新增：

```rust
    #[serde(default)]
    pub daemon: DaemonConfig,
```

并新增（紧邻 `Settings` 定义）：

```rust
/// Daemon-process tuning. All fields optional; absent = defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Per-session SSE replay buffer capacity (Task: event replay).
    pub event_buffer_capacity: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self { event_buffer_capacity: 1024 }
    }
}
```

`src/daemon/state.rs`：`DaemonState` 字段区（`session_seq_counters` 旁，:135）新增 `session_buffers: Arc<std::sync::RwLock<HashMap<String, Arc<std::sync::RwLock<run_loop::SessionEventBuffer>>>>>`，在构造处（:491 旁）初始化为空 map，并实现：

```rust
    /// Lazily-created per-session replay buffer, mirroring `session_seq_counter`.
    pub fn session_buffer(
        &self,
        session_id: &str,
    ) -> Arc<std::sync::RwLock<run_loop::SessionEventBuffer>> {
        let capacity = self.event_buffer_capacity();
        let mut map = self
            .session_buffers
            .write()
            .expect("session_buffers lock poisoned");
        map.entry(session_id.to_string())
            .or_insert_with(|| {
                Arc::new(std::sync::RwLock::new(run_loop::SessionEventBuffer::new(capacity)))
            })
            .clone()
    }

    pub fn event_buffer_capacity(&self) -> usize {
        self.app_state.settings.daemon.event_buffer_capacity
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib daemon::run_loop -- buffer`
Expected: 2 passed

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/run_loop.rs src/daemon/state.rs src/config/mod.rs
git commit -m "feat(daemon): add per-session event replay buffer with configurable capacity"
```

---

### Task 2: publish 双写（广播同时压入缓冲）

- [x] Task 2 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/run_loop.rs:128-139`（`DaemonEventSink::publish`）
- Modify: `src/daemon/run_loop.rs:379-388`（`RootToolPort::publish_event`）及 `RootToolPort` 构造（:313-331、:336-364）
- Modify: `src/daemon/run_loop.rs:904-909`（`run_session_turn` 中 sink 构造）
- Test: `src/daemon/run_loop.rs` `mod tests`

**Interfaces:**
- Consumes: Task 1 的 `DaemonState::session_buffer()`、`SessionEventBuffer::push`
- Produces: `DaemonEventSink::new` 与 `RootToolPort::new` 签名各增加一个 `buffer: Arc<std::sync::RwLock<SessionEventBuffer>>` 参数（Task 3 依赖缓冲中有数据）

- [ ] **Step 1: 写失败测试（publish 后缓冲可见同 seq 事件）**

```rust
#[tokio::test]
async fn publish_writes_to_hub_and_buffer() {
    let (hub, mut rx) = tokio::sync::broadcast::channel(16);
    let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(8)));
    let sink = DaemonEventSink::new(
        "s1".into(),
        "r1".into(),
        hub,
        Arc::new(AtomicU64::new(1)),
        buffer.clone(),
    );
    sink.publish(SessionEventKind::Save, serde_json::json!({}));
    let got = rx.recv().await.expect("hub event");
    assert_eq!(got.seq, 1);
    let buf = buffer.read().expect("buffer lock poisoned");
    assert_eq!(buf.latest_seq(), Some(1));
}
```

注意：既有测试（:1147 附近）构造 `DaemonEventSink::new(...)` 的调用点需同步加参数，编译器会指出全部位置。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::run_loop -- publish_writes`
Expected: 编译失败（`new` 参数数量不匹配）

- [ ] **Step 3: 实现双写**

`DaemonEventSink` 增加字段 `buffer: Arc<std::sync::RwLock<SessionEventBuffer>>`，`publish` 末尾：

```rust
        // No subscribers is normal (e.g. run without an attached SSE client);
        // broadcast::Sender::send only errors in that case, so ignore it.
        let _ = self.hub.send(event.clone());
        self.buffer
            .write()
            .expect("session buffer lock poisoned")
            .push(event);
```

`RootToolPort` 同样增加 `buffer` 字段，`publish_event` 末尾做相同双写；`RootToolPort::new` 内用 `state.session_buffer(session_id)` 获取；`new_for_test` 用 `Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16)))`。`run_session_turn` 构造 sink 时传 `state.session_buffer(session_id)`。

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon::run_loop`
Expected: 全部通过（含既有 maps_runtime_events 等用例）

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/run_loop.rs src/daemon/state.rs
git commit -m "feat(daemon): dual-write session events to replay buffer on publish"
```

---

### Task 3: `GET /sessions/:id/events?after=<seq>` 重放 + 订阅时 SyncLost

- [x] Task 3 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/run_loop.rs:40-50`（`SessionEventKind` 增加 `SyncLost`）
- Modify: `src/daemon/run_loop.rs:785-824`（`get_session_events` 重写）
- Modify: `src/daemon/routes.rs:155-156`（handler 签名加 `Query`，路由不变）
- Test: `src/daemon/run_loop.rs` `mod tests`

**Interfaces:**
- Consumes: Task 1 `SessionEventBuffer`（`oldest_seq`/`latest_seq`/`events_after`）、Task 2 双写
- Produces:
  - `SessionEventKind::SyncLost`（serde `"sync_lost"`），data 形如 `{ "reason": "evicted" | "lagged", "latest_seq": <u64> }`
  - `get_session_events(State, Path, Query<SessionEventsQuery>)`，`SessionEventsQuery { after: Option<u64> }`（放 `src/daemon/models.rs`）
  - 客户端恢复约定（写入 handler doc 注释）：收到 `sync_lost` → `GET /sessions/:id` 全量恢复 → 不携带 after 或携带最新 seq 重新订阅

- [ ] **Step 1: 写失败测试（after 重放 + 淘汰失步的判定逻辑）**

把衔接判定抽成纯函数便于单测（放在 `get_session_events` 上方）：

```rust
/// Replay/attach decision for `after=` subscriptions (design §2.2).
pub(crate) enum CatchUp {
    /// No `after` — live-only (existing default).
    LiveOnly,
    /// `after >= latest` — nothing missed, attach live directly.
    UpToDate,
    /// Buffer covers `after + 1` — replay these, then attach live (dedup by seq).
    Replay(Vec<SessionEvent>),
    /// `after + 1 < oldest` (or buffer empty) — send SyncLost with latest seq.
    SyncLost { latest_seq: u64 },
}

pub(crate) fn plan_catch_up(after: Option<u64>, buf: &SessionEventBuffer) -> CatchUp {
    let Some(after) = after else { return CatchUp::LiveOnly };
    match (buf.oldest_seq(), buf.latest_seq()) {
        (_, Some(latest)) if after >= latest => CatchUp::UpToDate,
        (Some(oldest), Some(latest)) if after + 1 >= oldest => {
            CatchUp::Replay(buf.events_after(after).cloned().collect())
        }
        // Buffer empty (daemon restarted) or requested seq evicted.
        _ => CatchUp::SyncLost { latest_seq: buf.latest_seq().unwrap_or(0) },
    }
}
```

测试：

```rust
#[test]
fn catch_up_matrix() {
    let mut buf = SessionEventBuffer::new(4);
    for seq in 3..=6 {
        buf.push(ev(seq));
    }
    assert!(matches!(plan_catch_up(None, &buf), CatchUp::LiveOnly));
    assert!(matches!(plan_catch_up(Some(6), &buf), CatchUp::UpToDate));
    match plan_catch_up(Some(4), &buf) {
        CatchUp::Replay(evs) => assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![5, 6]),
        other => panic!("expected Replay, got {other:?}"),
    }
    assert!(matches!(
        plan_catch_up(Some(1), &buf),
        CatchUp::SyncLost { latest_seq: 6 }
    ));
    let empty = SessionEventBuffer::new(4);
    assert!(matches!(
        plan_catch_up(Some(9), &empty),
        CatchUp::SyncLost { latest_seq: 0 }
    ));
}
```

（`CatchUp` 需 `#[derive(Debug)]` 供 panic 打印。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::run_loop -- catch_up`
Expected: 编译失败（`plan_catch_up` 未定义）

- [ ] **Step 3: 实现**

1. `SessionEventKind` 末尾加 `SyncLost,`（serde 自动得 `"sync_lost"`）。
2. 实现 `plan_catch_up` / `CatchUp`（代码见 Step 1）。
3. `src/daemon/models.rs` 加：

```rust
#[derive(Debug, Deserialize)]
pub struct SessionEventsQuery {
    pub after: Option<u64>,
}
```

4. 重写 `get_session_events`（保持 404、keep-alive、先 subscribe 再响应的既有语义）：

```rust
/// GET /api/v1/sessions/:id/events — SSE stream of the session's events.
///
/// Without `after`: live-only (default, unchanged). With `after=<seq>`:
/// replay buffered events with `seq > after` in order, then attach live;
/// live events with `seq <= ` the last replayed seq are dropped (dedup at
/// the seam). If `after` fell out of the buffer window the client receives a
/// `sync_lost` event (`{ reason, latest_seq }`) — recovery contract: do a
/// full `GET /sessions/:id`, realign on the latest turn state, then
/// re-subscribe (without `after`, or with the newest seq). 404 for unknown
/// session. Keep-alive comment every 15s.
pub(crate) async fn get_session_events(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Query(query): Query<crate::daemon::models::SessionEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    if state.resolve_session(&id).await.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let buffer = state.session_buffer(&id);
    let catch_up = {
        let buf = buffer.read().expect("session buffer lock poisoned");
        plan_catch_up(query.after, &buf)
    };

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    // Subscribe BEFORE replaying so seam events can't be lost; dedup below.
    let mut live = state.session_event_hub.subscribe();

    let mut seam_seq = match &catch_up {
        CatchUp::Replay(evs) => {
            for ev in evs {
                let data = serde_json::to_string(ev).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).is_err() {
                    return Ok(Sse::new(UnboundedReceiverStream::new(rx))
                        .keep_alive(KeepAlive::default()));
                }
            }
            evs.last().map(|e| e.seq).unwrap_or(0)
        }
        CatchUp::SyncLost { latest_seq } => {
            let ev = SessionEvent {
                seq: 0,
                session_id: id.clone(),
                run_id: String::new(),
                kind: SessionEventKind::SyncLost,
                data: serde_json::json!({ "reason": "evicted", "latest_seq": latest_seq }),
            };
            let data = serde_json::to_string(&ev).unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(data)));
            // Keep the stream open; the client decides to resubscribe (§2.3).
            *latest_seq
        }
        CatchUp::LiveOnly | CatchUp::UpToDate => query.after.unwrap_or(0),
    };

    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(ev) => {
                    if ev.session_id != id || ev.seq <= seam_seq {
                        continue; // other session, or already replayed
                    }
                    seam_seq = ev.seq;
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Task 4 turns this into a SyncLost event; keep warn for now.
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "session events SSE subscriber lagged; oldest events dropped for this subscriber"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}
```

- [ ] **Step 4: 运行测试确认通过 + 编译全 crate**

Run: `cargo test --lib daemon::run_loop && cargo check --all-targets`
Expected: 测试通过；`get_session_events` 调用方（routes）编译通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/run_loop.rs src/daemon/models.rs src/daemon/routes.rs
git commit -m "feat(daemon): replay-after-seq resume and SyncLost on evicted seq for session events"
```

---

### Task 4: 运行中 Lagged → SyncLost（仅该连接）

- [x] Task 4 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/run_loop.rs`（Task 3 重写的 `get_session_events` 中 Lagged 分支）
- Test: `src/daemon/run_loop.rs` `mod tests`

**Interfaces:**
- Consumes: Task 3 的 `SyncLost` 变体与 `seam_seq` 去重
- Produces: 无新签名；行为变更：Lagged 时该连接收到 `sync_lost`（`reason: "lagged"`），其他订阅者不受影响（spec 场景「慢订阅者失步」）

- [ ] **Step 1: 写失败测试（慢消费者触发 Lagged 后收到 sync_lost）**

```rust
#[tokio::test]
async fn lagged_subscriber_receives_sync_lost() {
    // 小容量 hub + 不消费的 receiver，制造 Lagged。
    let (hub, _keep) = tokio::sync::broadcast::channel(2);
    let mut slow = hub.subscribe();
    for seq in 1..=10u64 {
        let _ = hub.send(ev(seq));
    }
    // 模拟 get_session_events 的 Lagged 分支逻辑：
    let n = match slow.recv().await {
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => n,
        other => panic!("expected Lagged, got {other:?}"),
    };
    assert!(n > 0);
    let sync = sync_lost_event("s", "lagged", ev(10).seq);
    assert_eq!(sync.kind, SessionEventKind::SyncLost);
    assert_eq!(sync.data["reason"], "lagged");
    assert_eq!(sync.data["latest_seq"], 10);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::run_loop -- lagged_subscriber`
Expected: 编译失败（`sync_lost_event` 未定义）

- [ ] **Step 3: 实现**

在 `run_loop.rs` 加构造助手（Task 3 的 SyncLost 分支也改用它，消除重复）：

```rust
/// Build the out-of-band `sync_lost` event for one connection (design §2.3).
/// `seq: 0` and empty `run_id` mark it as control-plane, not run output.
fn sync_lost_event(session_id: &str, reason: &str, latest_seq: u64) -> SessionEvent {
    SessionEvent {
        seq: 0,
        session_id: session_id.to_string(),
        run_id: String::new(),
        kind: SessionEventKind::SyncLost,
        data: serde_json::json!({ "reason": reason, "latest_seq": latest_seq }),
    }
}
```

`get_session_events` 的 Lagged 分支改为：

```rust
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "session events SSE subscriber lagged; sending sync_lost to this subscriber"
                    );
                    let latest = buffer
                        .read()
                        .expect("session buffer lock poisoned")
                        .latest_seq()
                        .unwrap_or(seam_seq);
                    let ev = sync_lost_event(&id, "lagged", latest);
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    seam_seq = latest; // skip everything up to latest; client resyncs
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return;
                    }
                    continue;
                }
```

（`buffer` 与 `id` 需在 `tokio::spawn` 前 clone/move 进闭包。）

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon::run_loop`
Expected: 全部通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/run_loop.rs
git commit -m "feat(daemon): send sync_lost SSE event to lagged session-event subscribers"
```

---

## 第 2 组：全局事件总线（tasks.md 2.1–2.4）

### Task 5: 全局事件类型与序号空间（`src/daemon/global_events.rs`）

- [x] Task 5 完成标记：本节全部 Step 完成并提交

**Files:**
- Create: `src/daemon/global_events.rs`
- Modify: `src/daemon/mod.rs:10-22`（注册 `pub(crate) mod global_events;`）
- Modify: `src/daemon/state.rs`（`DaemonState` 增加 `global_event_hub`、`global_seq_counter`，构造处初始化）
- Test: `src/daemon/global_events.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces（Task 6/7/8/12 依赖）：

```rust
pub struct GlobalEvent { pub seq: u64, pub kind: GlobalEventKind, pub data: serde_json::Value }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlobalEventKind { TodosChanged, BackgroundResult, ModeChanged, ModelChanged, TaskGroupResult }

pub type GlobalEventHub = tokio::sync::broadcast::Sender<GlobalEvent>;
```

  - `DaemonState.global_event_hub: GlobalEventHub`（容量 1024）
  - `DaemonState.global_seq_counter: Arc<AtomicU64>`（初值 1）
  - `DaemonState::broadcast_global(&self, kind: GlobalEventKind, data: serde_json::Value)`——分配 seq、发送、忽略无订阅者错误（与 `DaemonEventSink::publish` 同惯例）

设计要点（design §3.1）：不复用 `SessionEventHub` 信封；独立 seq 空间避免会话高频事件挤占全局事件。

- [ ] **Step 1: 写失败测试（seq 单调 + 多订阅者收到相同序列）**

```rust
#[tokio::test]
async fn broadcast_global_assigns_monotonic_seq_to_all_subscribers() {
    let state = test_daemon_state().await; // 复用 handlers.rs 测试中已有的构造助手
    let mut a = state.global_event_hub.subscribe();
    let mut b = state.global_event_hub.subscribe();
    state.broadcast_global(GlobalEventKind::ModeChanged, serde_json::json!({"mode": "yolo"}));
    state.broadcast_global(GlobalEventKind::ModelChanged, serde_json::json!({"profile": "p1"}));
    for expected_seq in [1u64, 2] {
        let ea = a.recv().await.expect("subscriber a");
        let eb = b.recv().await.expect("subscriber b");
        assert_eq!(ea.seq, expected_seq);
        assert_eq!(eb.seq, expected_seq);
    }
}
```

`test_daemon_state()`：handlers.rs 既有测试（:1773 附近）已直接调 `update_session(State(state)...)`，说明测试里有 DaemonState 构造路径；照搬该构造方式提取为助手放在测试模块。若该构造过重，退化为只测 `broadcast_global` 的自由函数版本（hub + counter 作为参数）——优先复用既有构造。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::global_events 2>&1 | tail -5`
Expected: 编译失败（模块/字段不存在）

- [ ] **Step 3: 实现**

`src/daemon/global_events.rs`：

```rust
//! Global (daemon-wide, cross-project) event bus: todos changes, background
//! results, permission-mode / model switches, task-group results. Separate
//! envelope and seq space from the per-session `SessionEventHub` so
//! high-frequency session deltas can't starve global events (design §3.1).
//! v1 is live-only — clients realign via the existing GET endpoints.

use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicU64;

/// One event on the global bus. `seq` is monotonic across the daemon process
/// for client dedup/ordering; it is NOT resumable after a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalEvent {
    pub seq: u64,
    pub kind: GlobalEventKind,
    /// Kind-specific payload. Cross-project events carry project/session
    /// dimension fields so clients can filter (design §10).
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlobalEventKind {
    /// Full todos snapshot (small; YAGNI: no incremental diff).
    TodosChanged,
    BackgroundResult,
    ModeChanged,
    ModelChanged,
    TaskGroupResult,
}

pub type GlobalEventHub = tokio::sync::broadcast::Sender<GlobalEvent>;

/// Hub channel capacity; aligned with the session event hub.
pub const GLOBAL_EVENT_HUB_CAPACITY: usize = 1024;

pub fn new_global_event_hub() -> GlobalEventHub {
    tokio::sync::broadcast::channel(GLOBAL_EVENT_HUB_CAPACITY).0
}
```

`src/daemon/state.rs`：字段区加 `pub global_event_hub: crate::daemon::global_events::GlobalEventHub` 与 `global_seq_counter: Arc<AtomicU64>`（在 `session_event_hub` :126 旁），构造处初始化 `global_events::new_global_event_hub()` 与 `Arc::new(AtomicU64::new(1))`，并实现：

```rust
    /// Publish one global event. No subscribers is normal; ignore the error.
    pub fn broadcast_global(
        &self,
        kind: crate::daemon::global_events::GlobalEventKind,
        data: serde_json::Value,
    ) {
        let event = crate::daemon::global_events::GlobalEvent {
            seq: self
                .global_seq_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            kind,
            data,
        };
        let _ = self.global_event_hub.send(event);
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib daemon::global_events`
Expected: 1 passed

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/global_events.rs src/daemon/mod.rs src/daemon/state.rs
git commit -m "feat(daemon): add global event bus with independent seq space"
```

---

### Task 6: `GET /events` 全局 SSE 端点

- [x] Task 6 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/global_events.rs`（新增 handler）
- Modify: `src/daemon/routes.rs`（注册路由，:99 todos 路由旁）
- Test: `src/daemon/global_events.rs` `mod tests`

**Interfaces:**
- Consumes: Task 5 的 `GlobalEventHub`
- Produces: `pub(crate) async fn get_global_events(State<Arc<DaemonState>>) -> Sse<impl Stream<...>>`；路由 `GET /api/v1/events`（protected router，与既有端点同鉴权）。v1 live-only（design §3.4）

- [ ] **Step 1: 写失败测试（订阅者收到订阅后的事件、收不到订阅前的）**

```rust
#[tokio::test]
async fn global_events_stream_is_live_only() {
    let hub = new_global_event_hub();
    // 订阅前的事件不可见。
    let _ = hub.send(GlobalEvent { seq: 1, kind: GlobalEventKind::ModeChanged, data: serde_json::json!({}) });
    let mut rx = hub.subscribe();
    let _ = hub.send(GlobalEvent { seq: 2, kind: GlobalEventKind::ModelChanged, data: serde_json::json!({}) });
    let got = rx.recv().await.expect("live event");
    assert_eq!(got.seq, 2);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::global_events -- live_only`
Expected: 编译失败或测试不存在

- [ ] **Step 3: 实现 handler + 路由**

`src/daemon/global_events.rs` 追加（复用 run_loop 的 mpsc + spawn 模式；Lagged 时仅 warn——全局流客户端用 GET 端点兜底对齐，§3.4）：

```rust
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{Stream, StreamExt};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// GET /api/v1/events — global SSE stream (live-only, v1). Clients that
/// disconnect re-subscribe and realign via the plain GET endpoints
/// (GET /todos, GET /background/results, ...). Keep-alive every 15s.
pub(crate) async fn get_global_events(
    State(state): State<Arc<crate::daemon::state::DaemonState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    // Subscribe before responding so no event is missed between connect and
    // stream start (mirrors get_session_events).
    let mut live = state.global_event_hub.subscribe();

    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "global events SSE subscriber lagged; it should realign via GET endpoints"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}
```

`src/daemon/routes.rs`（:99 旁）：

```rust
        .route("/api/v1/events", get(global_events::get_global_events))
```

（确认 `futures`/`tokio-stream` 已在 Cargo.toml——run_loop.rs 已用同套 imports，应有。）

- [ ] **Step 4: 运行测试确认通过 + 编译**

Run: `cargo test --lib daemon::global_events && cargo check --all-targets`
Expected: 全部通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/global_events.rs src/daemon/routes.rs
git commit -m "feat(daemon): add GET /events global SSE endpoint"
```

---

### Task 7: 全局事件生产者挂接（TodosChanged / ModeChanged / ModelChanged / TaskGroupResult）

- [x] Task 7 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/state.rs`（`apply_todos_update` 助手）
- Modify: `src/daemon/handlers.rs:736-758`（`set_permission_mode`）、`:124-171`（`switch_model`）、`:1548-`（`claim_task_group` 结果产生处）
- Test: `src/daemon/handlers.rs` 既有测试模块（:1700 起）

**Interfaces:**
- Consumes: Task 5 `DaemonState::broadcast_global`
- Produces:
  - `DaemonState::apply_todos_update(&self, items: Vec<crate::tasks::TodoItem>)`——写 `todo_state` 并广播 `TodosChanged`（data 为全量快照，含 `project` 维度字段，design §3.2/§10）
  - `set_permission_mode` 响应前广播 `ModeChanged`（data `{ session_id, mode, effective_mode }`）
  - `switch_model` 响应前广播 `ModelChanged`（data `{ profile, model_name, provider }`）
  - task-group 结果产生处广播 `TaskGroupResult`

**关于 TodosChanged 挂接点的现状说明（实现前必读）：** base-ref 上 `DaemonState.todo_state`（state.rs:64）**没有任何写入方**——`TodoWriteTool` 已移除（见 `src/tasks/todo_write.rs:4-5` 注释），`update_plan` 是 TUI 本地渲染（`src/tools/meta/update_plan.rs:64` 返回 "requires interactive execution in the TUI"）。实现第一步先运行 `grep -rn "todo_state" src/ --include="*.rs"` 确认；若确实仍无写入方，`apply_todos_update` 作为唯一收敛入口落地（供后续写入方与测试调用），TodosChanged 的端到端验证并入 Task 8/17 的集成场景。不要为了挂接而发明新的 todo 写入路径。

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn apply_todos_update_broadcasts_full_snapshot() {
    let state = test_daemon_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state
        .apply_todos_update(vec![crate::tasks::TodoItem {
            content: "write plan".into(),
            status: "in_progress".into(),
            active_form: None,
            subagent: None,
        }])
        .await;
    let ev = rx.recv().await.expect("todos event");
    assert_eq!(ev.kind, crate::daemon::global_events::GlobalEventKind::TodosChanged);
    assert_eq!(ev.data["items"][0]["content"], "write plan");
    // 快照与 GET /todos 读取同源。
    let todos = state.todo_state.read().await;
    assert_eq!(todos.items.len(), 1);
}
```

（`TodoItem` 字段名以 `src/tasks/todo_write.rs:21-31` 实际定义为准微调。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon -- todos_update_broadcasts 2>&1 | tail -5`
Expected: 编译失败（`apply_todos_update` 未定义）

- [ ] **Step 3: 实现**

`src/daemon/state.rs`：

```rust
    /// Single write-path for the shared todo list: update state, then
    /// broadcast a full-snapshot TodosChanged (snapshots are small; YAGNI:
    /// no incremental diff). `project` lets multi-project clients filter.
    pub async fn apply_todos_update(&self, items: Vec<crate::tasks::TodoItem>) {
        {
            let mut todos = self.todo_state.write().await;
            todos.items = items;
        }
        let snapshot = {
            let todos = self.todo_state.read().await;
            serde_json::json!({
                "project": self.app_state.settings.storage.working_dir,
                "items": todos.items,
                "has_open_items": todos.has_open_items(),
            })
        };
        self.broadcast_global(crate::daemon::global_events::GlobalEventKind::TodosChanged, snapshot);
    }
```

`set_permission_mode`（handlers.rs:753 `Json(...)` 返回前）：

```rust
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({
            "session_id": session_id,
            "mode": body.mode,
            "effective_mode": effective,
        }),
    );
```

`switch_model`（handlers.rs:164 `Ok(Json(...))` 前）：

```rust
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({
            "profile": body.profile,
            "model_name": model_name,
            "provider": provider,
        }),
    );
```

`claim_task_group`（handlers.rs:1548 起）：在结果产生/落库处广播 `TaskGroupResult`，data 携带 `{ task_group_id, project, status }` 等结果摘要（字段以实现时该 handler 实际返回体为准）。

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon`
Expected: 全部通过（含既有 update_session 用例）

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/state.rs src/daemon/handlers.rs
git commit -m "feat(daemon): broadcast todos/mode/model/task-group changes on the global event bus"
```

---

### Task 8: 背景结果保留队列 + 广播 + 快照读取（废除 drain 抢占）

- [x] Task 8 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/state.rs`（`background_results: Arc<tokio::sync::RwLock<VecDeque<BackgroundResult>>>`，容量 256）
- Modify: `src/daemon/handlers.rs:849-854`（`get_background_results` 改读快照）
- Modify: 背景结果产生处——`src/tools/execution/background.rs:246` `drain_results` 的调用方；daemon 侧在结果完成回调挂接
- Test: `src/daemon/handlers.rs` 测试模块

**Interfaces:**
- Consumes: Task 5 `broadcast_global`
- Produces:
  - `DaemonState::record_background_result(&self, result: crate::tools::execution::background::BackgroundResult)`——**先入保留队列（满 256 淘汰最旧）再广播** `BackgroundResult`（design §3.2/§3.3 顺序要求）
  - `DaemonState::background_results_snapshot(&self) -> Vec<BackgroundResult>`
  - `GET /api/v1/background/results` 语义变更：返回快照不再 drain（轮询兼容保留、结果不被抢占）

- [ ] **Step 1: 写失败测试（保留 + 非抢占 + 淘汰）**

```rust
#[tokio::test]
async fn background_results_are_retained_not_drained() {
    let state = test_daemon_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state.record_background_result(sample_bg_result("r1")).await;
    state.record_background_result(sample_bg_result("r2")).await;
    // 广播在入队之后到达。
    let ev = rx.recv().await.expect("broadcast");
    assert_eq!(ev.kind, crate::daemon::global_events::GlobalEventKind::BackgroundResult);
    // 两次快照读取内容一致（不再先到先得）。
    let first = state.background_results_snapshot().await;
    let second = state.background_results_snapshot().await;
    assert_eq!(first.len(), 2);
    assert_eq!(first.len(), second.len());
}
```

`sample_bg_result(id)` 按 `BackgroundResult`（`src/tools/execution/background.rs`）实际字段构造。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon -- background_results_are_retained 2>&1 | tail -5`
Expected: 编译失败

- [ ] **Step 3: 实现**

`src/daemon/state.rs`：

```rust
/// Retained background results; newest at back, oldest evicted past capacity.
/// Retention accepts eviction at extreme volume (very low frequency; online
/// clients receive results via the event bus) — design §3.3.
pub const BACKGROUND_RESULTS_CAPACITY: usize = 256;

// DaemonState 字段：
// pub background_results: Arc<tokio::sync::RwLock<VecDeque<crate::tools::execution::background::BackgroundResult>>>,

    /// Retain-then-broadcast: the result MUST be queryable before any client
    /// sees the event, so offline-then-online clients can still fetch it.
    pub async fn record_background_result(
        &self,
        result: crate::tools::execution::background::BackgroundResult,
    ) {
        {
            let mut retained = self.background_results.write().await;
            if retained.len() == BACKGROUND_RESULTS_CAPACITY {
                retained.pop_front();
            }
            retained.push_back(result.clone());
        }
        self.broadcast_global(
            crate::daemon::global_events::GlobalEventKind::BackgroundResult,
            serde_json::json!({ "result": result }),
        );
    }

    pub async fn background_results_snapshot(
        &self,
    ) -> Vec<crate::tools::execution::background::BackgroundResult> {
        self.background_results.read().await.iter().cloned().collect()
    }
```

`get_background_results`（handlers.rs:849-854）改为：

```rust
pub async fn get_background_results(
    State(state): State<Arc<DaemonState>>,
) -> Json<serde_json::Value> {
    // Snapshot read (no drain): results are retained so every client can
    // query them; the old first-come-first-served drain is abolished.
    let results = state.background_results_snapshot().await;
    Json(serde_json::json!({ "results": results }))
}
```

结果产生处挂接：找到 daemon 中背景任务完成并把结果放进 `background_manager` 的位置（`grep -rn "background_manager" src/daemon/`），在完成回调处改为调用 `state.record_background_result(result)`（保留队列成为 daemon 侧唯一事实源；`background_manager.drain_results` 在 daemon HTTP 层不再被调用——工具内机制保留给非 daemon 路径，不删除）。

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon`
Expected: 全部通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/state.rs src/daemon/handlers.rs
git commit -m "feat(daemon): retain and broadcast background results instead of drain-stealing"
```

---

### Task 9: dogfood——TUI todos 面板切换为 `GET /events` 订阅（保留轮询回退）

- [x] Task 9 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/tui/client.rs`（新增 `subscribe_events` SSE 客户端方法，仿既有 SSE 消费方式）
- Modify: `src/tui/app/`（todos 刷新回路：订阅驱动 + 失败回退 500ms 轮询 + 周期性重试订阅）
- Test: TUI 侧若无现成 harness，则以手动验证为主（Task 17 验收）

**Interfaces:**
- Consumes: Task 6 `GET /api/v1/events`、Task 7 `TodosChanged` 快照
- Produces:
  - `TuiClient::subscribe_events(&self) -> anyhow::Result<impl Stream<Item = GlobalEventWire>>`；`GlobalEventWire { seq: u64, kind: String, data: serde_json::Value }`（TUI 侧线格式，不依赖 daemon 类型）
  - todos 面板数据源切换逻辑：收到 `kind == "todos_changed"` → `data.items` 快照直接替换本地状态；订阅断开 → 回退 `get_todos()`（client.rs:939）500ms 轮询 + 周期性重试订阅

- [ ] **Step 1: 阅读既有 SSE 消费代码确定复用点**

Run: `grep -n "eventsource\|text/event-stream\|chat_stream\|EventSource" src/tui/client.rs | head -20`
确定 TUI 现有 SSE 解析方式（chat stream 消费），`subscribe_events` 沿用同一解析路径。

- [ ] **Step 2: 实现 `subscribe_events` + todos 订阅回路**

```rust
// src/tui/client.rs
/// Wire shape of one daemon global event (GET /api/v1/events).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GlobalEventWire {
    pub seq: u64,
    pub kind: String,
    pub data: serde_json::Value,
}

/// Subscribe to the daemon global event stream (SSE). Caller handles
/// reconnect/fallback; this is a single attempt.
pub async fn subscribe_events(
    &self,
) -> anyhow::Result<impl futures::Stream<Item = anyhow::Result<GlobalEventWire>>> {
    let url = format!("{}/api/v1/events", self.base_url);
    let resp = self
        .client
        .get(url)
        .bearer_auth(&self.token) // 与既有请求同鉴权方式，按 client.rs 实际字段微调
        .send()
        .await
        .context("connect global event stream")?;
    // 按既有 chat stream 的 SSE 行解析方式把 data: 行反序列化为 GlobalEventWire。
    // 实现：Step 1 定位到的解析函数若是私有，抽出为 pub(crate) 复用；例如
    //   let events = parse_sse_lines(resp.bytes_stream()).map(|line| {
    //       serde_json::from_str::<GlobalEventWire>(&line).context("parse global event")
    //   });
    //   Ok(events)
    // 函数名/签名以 Step 1 找到的既有实现为准，不新写第二套 SSE 解析。
}
```

todos 回路（伪结构，落到 app 内现有 todos 刷新的位置）：

```rust
// 订阅模式：TodosChanged 快照直接替换本地 todos 状态。
// 断连/失败：回退 500ms get_todos() 轮询，并每 ~5s 重试 subscribe_events()。
```

- [ ] **Step 3: 编译 + 静态检查**

Run: `cargo check --all-targets && cargo clippy --all-targets -- -D warnings`
Expected: 零 warning

- [ ] **Step 4: 手动等价性验证（dogfood 验收，记录到 commit message）**

- 启动 daemon + TUI，触发 todo 变更（update_plan / TodoWrite），确认面板实时更新且无轮询请求日志。
- 杀掉 SSE 连接（如重启 daemon），确认面板回退轮询且 daemon 恢复后重新订阅。

- [ ] **Step 5: 提交**

```bash
git add src/tui/client.rs src/tui/app/
git commit -m "feat(tui): subscribe todos panel to GET /events with polling fallback (dogfood)"
```

---

## 第 3 组：审批语义收敛（tasks.md 3.1–3.2）

### Task 10: `POST /interactions/:id/resolve` 已决议 → 409 + 当前决议

- [x] Task 10 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/interaction_bridge.rs:91-159`（`InteractionBridge` 增加已决议记录 + 三分结果）
- Modify: `src/daemon/handlers.rs:715-731`（`resolve_interaction` 状态码映射）
- Test: `src/daemon/interaction_bridge.rs` `mod tests`（:164 起，既有）

**Interfaces:**
- Produces:

```rust
pub enum ResolveOutcome {
    /// Waiter found and answered.
    Resolved,
    /// Same request resolved before; carries the first answer.
    AlreadyResolved(String),
    /// Never existed (or waiter cleaned up without resolve).
    Unknown,
}
```

  - `InteractionBridge::resolve(&self, request_id: &str, answer: String) -> ResolveOutcome`（签名从 `bool` 变更；唯一调用方是 handlers.rs:720）
  - handler 映射：`Resolved → 200 {"resolved":true}`；`AlreadyResolved(answer) → 409 {"resolved":false,"answer":<answer>}`；`Unknown → 404`（保持「从未存在」的既有语义）

- [ ] **Step 1: 写失败测试（重复 resolve 区分 AlreadyResolved/Unknown）**

```rust
#[tokio::test]
async fn double_resolve_reports_already_resolved_with_first_answer() {
    let bridge = InteractionBridge::new();
    let payload = sample("q1"); // 既有测试助手 :168
    let waiter = tokio::spawn({
        let bridge = bridge.clone_arc(); // 若无 clone_arc，用 Arc::new(bridge) 共享
        async move { bridge.request(payload).await }
    });
    tokio::task::yield_now().await;
    assert!(matches!(
        bridge.resolve("q1", r#"{"selected":["A"]}"#.into()).await,
        ResolveOutcome::Resolved
    ));
    assert!(matches!(
        bridge.resolve("q1", r#"{"selected":["B"]}"#.into()).await,
        ResolveOutcome::AlreadyResolved(ans) if ans.contains('A')
    ));
    assert!(matches!(
        bridge.resolve("nope", "{}".into()).await,
        ResolveOutcome::Unknown
    ));
    let _ = waiter.await;
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon::interaction_bridge -- double_resolve 2>&1 | tail -5`
Expected: 编译失败（`ResolveOutcome` 未定义）

- [ ] **Step 3: 实现**

`src/daemon/interaction_bridge.rs`：

```rust
/// Outcome of resolving a pending question (design §4: duplicate answers get
/// a distinguishable signal instead of a bare "not found").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    Resolved,
    /// Carries the first answer so the 409 response can show the resolution.
    AlreadyResolved(String),
    Unknown,
}

/// Cap on remembered resolutions; eviction at cap is fine — a duplicate
/// answer long after the fact degrades to 404 (client stops retrying anyway).
const RESOLVED_CAP: usize = 256;

// InteractionBridge 增加字段：
// resolved: Mutex<VecDeque<(String, String)>>,  // (request_id, first answer), FIFO
```

`resolve` 重写：

```rust
    /// Resolve a pending question. Distinguishes first resolve from a
    /// duplicate (409 semantics at the HTTP layer) and from unknown ids.
    pub async fn resolve(&self, request_id: &str, answer: String) -> ResolveOutcome {
        let entry = self.inner.lock().await.remove(request_id);
        match entry {
            Some(entry) => {
                let delivered = entry.tx.send(answer.clone()).is_ok();
                if delivered {
                    let mut resolved = self.resolved.lock().await;
                    if resolved.len() == RESOLVED_CAP {
                        resolved.pop_front();
                    }
                    resolved.push_back((request_id.to_string(), answer));
                    ResolveOutcome::Resolved
                } else {
                    // Waiter dropped (run cancelled) — treat as unknown.
                    ResolveOutcome::Unknown
                }
            }
            None => {
                let resolved = self.resolved.lock().await;
                match resolved.iter().find(|(id, _)| id == request_id) {
                    Some((_, first)) => ResolveOutcome::AlreadyResolved(first.clone()),
                    None => ResolveOutcome::Unknown,
                }
            }
        }
    }
```

`resolve_interaction`（handlers.rs:715-731）：

```rust
    match state.interaction_bridge.resolve(&request_id, body.answer).await {
        crate::daemon::interaction_bridge::ResolveOutcome::Resolved => {
            Ok(Json(serde_json::json!({ "resolved": true })))
        }
        crate::daemon::interaction_bridge::ResolveOutcome::AlreadyResolved(answer) => {
            // Duplicate answer: conflict, and hand back the standing resolution.
            Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "resolved": false, "answer": answer })),
            ))
        }
        // Never existed (or waiter gone). 404 so the client stops retrying.
        crate::daemon::interaction_bridge::ResolveOutcome::Unknown => {
            Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "unknown interaction" }))))
        }
    }
```

（返回类型随之改为 `Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)>`。）

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon::interaction_bridge && cargo check --all-targets`
Expected: 通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/interaction_bridge.rs src/daemon/handlers.rs
git commit -m "fix(daemon): return 409 with standing resolution for duplicate interaction answers"
```

---

### Task 11: `POST /tools/resolve-permission`（subagent）已决议 → 409

- [x] Task 11 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/handlers.rs:692-710`（`resolve_subagent_permission`）
- Modify: `src/permissions/`（`PermissionBridge::resolve` 所在文件——`grep -rn "pub async fn resolve" src/permissions/` 定位）
- Test: handlers 或 permissions 既有测试模块

**Interfaces:**
- Consumes: Task 10 的 `ResolveOutcome` 模式（不复用类型——PermissionBridge 在 permissions 层，定义自己的三分结果避免跨层依赖）
- Produces:
  - `PermissionBridge::resolve(&self, request_id: &str, approved: bool) -> PermissionResolveOutcome { Resolved, AlreadyResolved(bool), Unknown }`（bool 为首次决议的 approved 值；`resolve` 内部沿用 Task 10 的 256 上限 FIFO 记录）
  - handler 映射：`Resolved → 200 {"success":true,"resolved":true}`；`AlreadyResolved(approved) → 409 {"success":false,"resolved":true,"approved":<bool>}`；`Unknown → 404 {"success":false,"resolved":false}`

注意 design §4 只要求「已决议 → 409（替换 `{success:false}`）」；`Unknown` 也返回非 200 是语义澄清（此前 `{success:false}` 无法区分），与 spec「重复应答返回 409」不冲突。`always` 规则审批分支（:696-701）只在 `Resolved` 路径执行——当前代码在 resolve 之前先批规则，重复应答会重复批规则（幂等但 noisy）；顺手把规则审批移入 `Resolved` 分支内，保持「不产生二次效应」（spec 场景原文）。

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn duplicate_subagent_permission_resolve_conflicts() {
    // 构造 PermissionBridge，注册一个 Ask waiter（沿用既有测试构造方式），
    // 第一次 resolve(true) → Resolved；第二次 → AlreadyResolved(true)；
    // 未知 id → Unknown。
}
```

（实现时先 `grep -rn "permission_bridge" src/daemon/handlers.rs src/permissions/ | head` 找到 waiter 注册 API 与既有测试，照搬构造。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib -- duplicate_subagent_permission 2>&1 | tail -5`
Expected: 编译失败

- [ ] **Step 3: 实现**

`PermissionBridge::resolve` 按 Task 10 同款模式改造（已决议记录 `Mutex<VecDeque<(String, bool)>>`，上限 256）。handler 重写：

```rust
pub async fn resolve_subagent_permission(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<crate::daemon::models::ResolveSubagentPermissionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    use crate::permissions::PermissionResolveOutcome as Outcome;
    match state
        .permission_bridge
        .resolve(&body.request_id, body.approved)
        .await
    {
        Outcome::Resolved => {
            // Approve the standing rule only on the FIRST resolution — a
            // duplicate answer must produce no second effect (spec §审批).
            if body.approved && body.always {
                if let Some(rule) = body.session_rule.clone() {
                    state.tool_executor.approve_rule(rule.clone()).await;
                    // deprecated(compat): legacy global rule scope, see Task 12.
                    state.approve_rule("default", rule).await;
                }
            }
            Ok(Json(serde_json::json!({ "success": true, "resolved": true })))
        }
        Outcome::AlreadyResolved(approved) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false, "resolved": true, "approved": approved,
            })),
        )),
        Outcome::Unknown => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "success": false, "resolved": false })),
        )),
    }
}
```

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon && cargo test --lib permissions`
Expected: 全部通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/permissions/ src/daemon/handlers.rs
git commit -m "fix(daemon): return 409 for duplicate subagent permission resolves"
```

---

### Task 12: `"default"` 硬编码清零（server-side → 400；旧端点加 deprecated 注释）

- [x] Task 12 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/handlers.rs`（:485、:654、:664、:699、:740、:765、:1397、:1431、:1458、:1507）
- Test: `src/daemon/handlers.rs` 测试模块

**Interfaces:**
- Produces: 行为契约——
  - **server-side 路径**（agents/* 系列 :1397/:1431/:1458/:1507，permission-mode :740/:765）：请求缺 `session_id` → `400 Bad Request`，错误体 `{"error":"session_id required"}`
  - **旧端点路径**（chat_stream 配套 `execute_tool` :485、`approve_tool` :654、`unapprove_tool` :664、`resolve_subagent_permission` :699 的规则审批）：保留 `unwrap_or("default")` / 字面量 `"default"` 映射，逐一加 `// deprecated(compat): legacy chat_stream client path; server-side callers must pass session_id` 注释

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn permission_mode_without_session_id_is_rejected() {
    let state = test_daemon_state().await;
    let result = set_permission_mode(
        State(state),
        Json(crate::daemon::models::SetPermissionModeRequest {
            session_id: None,
            mode: crate::permissions::RootPermissionMode::Normal,
            effective_mode: None,
        }),
    )
    .await;
    assert!(result.is_err()); // 400
}
```

（`SetPermissionModeRequest` 字段以 models.rs 实际定义为准；`get_permission_mode` 与 agents/* 四个端点同法各加一例。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon -- session_id_is_rejected 2>&1 | tail -5`
Expected: 测试失败（当前回退 "default" 返回 200）

- [ ] **Step 3: 实现**

server-side 路径统一改为（以 `set_permission_mode` 为例）：

```rust
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let Some(session_id) = body.session_id.as_deref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id required"})),
        ));
    };
```

返回类型相应加 `(StatusCode, Json<serde_json::Value>)` 错误通道。`get_permission_mode` 与 `get_agent_self`/`navigate_agent_view`/`get_child_transcript`/`cancel_child` 四个 agents 端点同法处理（它们已有 `Result<_, StatusCode>`，直接 `.ok_or(StatusCode::BAD_REQUEST)?`）。

旧端点路径（:485/:654/:664/:699）逐行加注释，行为不变，例如：

```rust
    // deprecated(compat): legacy chat_stream client path; server-side callers
    // must pass session_id. Removal is a separate change.
    let session_id = body.session_id.as_deref().unwrap_or("default");
```

- [ ] **Step 4: 运行测试确认通过 + 回归 + 全局复查**

Run: `cargo test --lib daemon && grep -n '"default"' src/daemon/handlers.rs`
Expected: 测试通过；剩余 `"default"` 仅旧端点路径 4 处且均带 `deprecated(compat)` 注释

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/handlers.rs
git commit -m "refactor(daemon): require session_id on server-side paths, annotate legacy default fallback"
```

---

## 第 4 组：会话存储版本化（tasks.md 4.1–4.2）

### Task 13: `Session.version` 字段 + 历史兼容

- [x] Task 13 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/context/memory_session.rs:12-34`（字段）、:49-64（`with_id` 初始化）
- Test: `src/context/memory_session.rs` 既有测试模块

**Interfaces:**
- Produces: `Session { #[serde(default)] pub version: u64 }`——历史文件无该字段，反序列化为 0；`with_id`/`new` 初始化为 0（Task 14 依赖）

- [ ] **Step 1: 写失败测试（历史文件兼容）**

```rust
#[test]
fn legacy_session_without_version_deserializes_as_zero() {
    // 复制一份现有持久化 session JSON（去掉 version 字段）做反序列化。
    let legacy = serde_json::json!({
        "id": "s1", "name": "s1", "project_path": null,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "messages": [], "metadata": {}, "status": "Active"
    });
    let session: Session = serde_json::from_value(legacy).expect("legacy json parses");
    assert_eq!(session.version, 0);
}
```

（必填字段以 `Session`  serde 实际要求微调——`ui_messages`/`status`/`metadata` 均有 `#[serde(default)]`。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib context::memory_session -- legacy_session 2>&1 | tail -5`
Expected: 编译失败（`version` 字段不存在）

- [ ] **Step 3: 实现**

`Session` 结构体（`lazy_message_count` 前）加：

```rust
    /// Optimistic-concurrency version, bumped on every persisted write
    /// (PUT overwrite and run saves). `#[serde(default)]` keeps pre-versioning
    /// files loadable as version 0.
    #[serde(default)]
    pub version: u64,
```

`with_id` 初始化加 `version: 0,`。

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib context::memory_session`
Expected: 全部通过（含既有 lazy index 用例）

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/context/memory_session.rs
git commit -m "feat(daemon): add version field to Session with legacy zero compatibility"
```

---

### Task 14: `PUT /sessions/:id` `expected_version` 409 + run 写盘推进版本

- [x] Task 14 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/daemon/models.rs`（`UpdateSessionRequest` 加 `expected_version: Option<u64>`；`SessionResponse` 加 `version: u64`）
- Modify: `src/daemon/handlers.rs:992-1054`（`update_session` 校验与推进）；`:936-990`（`create_session`/`get_session` 响应带 version）
- Modify: `src/daemon/run_loop.rs:862-891`（`save_session_history` 推进版本）
- Test: `src/daemon/handlers.rs` 测试模块（:1773 附近既有 update_session 用例需同步补 `expected_version: None`）

**Interfaces:**
- Consumes: Task 13 `Session.version`
- Produces:
  - `UpdateSessionRequest.expected_version: Option<u64>`
  - `SessionResponse.version: u64`
  - 校验矩阵（design §5）：`Some(v)` 且 `v != current.version` → `409 {"error":"version conflict","current_version":<n>}`；`None` → 维持现状（last-write-wins 风险留给未升级客户端，doc 注释标注）；写入成功一律 `version += 1`；run 活跃 409（:998-1003）语义不变；run 写盘（`save_session_history`）同样推进版本

- [ ] **Step 1: 写失败测试（版本校验矩阵）**

```rust
#[tokio::test]
async fn update_session_version_matrix() {
    let state = test_daemon_state().await;
    let id = "ver-matrix".to_string();

    // 首次 upsert（无 expected_version）→ version 0 -> 1。
    let body = |expected_version: Option<u64>| crate::daemon::models::UpdateSessionRequest {
        name: None, messages: None, ui_messages: None, expected_version,
    };
    let r1 = update_session(State(state.clone()), Path(id.clone()), Json(body(None)))
        .await
        .expect("upsert ok");
    assert_eq!(r1.version, 1);

    // Some(0) 匹配旧版本? 不匹配——当前已是 1 → 409 + current_version。
    let err = update_session(State(state.clone()), Path(id.clone()), Json(body(Some(0))))
        .await
        .expect_err("stale expected_version conflicts");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert_eq!(err.1.0["current_version"], 1);

    // Some(1) 匹配 → 成功，version 推进到 2。
    let r3 = update_session(State(state.clone()), Path(id.clone()), Json(body(Some(1))))
        .await
        .expect("matching expected_version ok");
    assert_eq!(r3.version, 2);

    // None → 兼容路径，照常成功。
    update_session(State(state.clone()), Path(id.clone()), Json(body(None)))
        .await
        .expect("no expected_version stays compatible");
}
```

（`UpdateSessionRequest` 实际字段以 models.rs 为准；`SessionResponse` 返回 `Json(...)`，测试里取 `.0.version`。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib daemon -- version_matrix 2>&1 | tail -5`
Expected: 编译失败（字段不存在）

- [ ] **Step 3: 实现**

`models.rs`：

```rust
// UpdateSessionRequest 增加：
    /// Optimistic concurrency guard: when `Some`, the write is rejected with
    /// 409 + current_version unless it matches the stored version. `None`
    /// keeps legacy last-write-wins behavior (risk borne by unupgraded
    /// clients — documented in design §5).
    #[serde(default)]
    pub expected_version: Option<u64>,

// SessionResponse 增加：
    pub version: u64,
```

`update_session` 在 run-lock 检查之后、字段应用之前插入：

```rust
    if let Some(expected) = body.expected_version {
        if expected != session.version {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "version conflict",
                    "current_version": session.version,
                })),
            ));
        }
    }
```

`mgr.save(&session)` 之前加 `session.version += 1;`；响应构造加 `version: session.version`。`create_session`/`get_session` 响应同步带 `version`（新建为 0——首次 save 即在创建路径，按既有 `mgr.save` 调用点决定是否推进；保持「save 才推进」语义：create 路径 save 前 `session.version += 1` 不做，version 从 0 起，由后续写推进——与测试矩阵一致，upsert 路径首次 PUT 由 0 → 1）。

`save_session_history`（run_loop.rs:884-888 区间）：

```rust
    session.messages = messages;
    session.updated_at = chrono::Utc::now();
    session.version += 1; // run saves participate in the same version sequence
    session.lazy_message_count = None;
```

既有测试（:1773、:1908 等）构造 `UpdateSessionRequest` 处补 `expected_version: None`。

- [ ] **Step 4: 运行测试确认通过 + 回归**

Run: `cargo test --lib daemon && cargo test --lib context`
Expected: 全部通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/daemon/models.rs src/daemon/handlers.rs src/daemon/run_loop.rs
git commit -m "feat(daemon): optimistic version check on session overwrite, bump version on saves"
```

---

## 第 5 组：daemon 可发现部署（tasks.md 5.1–5.2）

### Task 15: 全局发现文件 `~/.wgenty-code/daemon.json`（原子写 + 心跳 + 退出清理）

- [x] Task 15 完成标记：本节全部 Step 完成并提交

**Files:**
- Create: `src/utils/discovery.rs`
- Modify: `src/utils/mod.rs:5-11`（`pub mod discovery;`）
- Modify: `src/daemon/mod.rs:37-139`（启动写入、心跳任务、退出清理）
- Test: `src/utils/discovery.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces（Task 16 依赖）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

pub fn discovery_file_path() -> PathBuf;                 // ~/.wgenty-code/daemon.json
pub fn write_discovery_file(file: &DiscoveryFile) -> anyhow::Result<()>;  // tmp + rename 原子替换
pub fn read_discovery_file() -> Option<DiscoveryFile>;   // 损坏 → None
pub fn remove_discovery_file() -> anyhow::Result<()>;    // 不存在 → Ok
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
pub const HEARTBEAT_STALE_SECS: u64 = 120;
```

- [ ] **Step 1: 写失败测试（原子写、损坏容错、往返）**

```rust
#[test]
fn discovery_file_roundtrip_and_corruption_tolerance() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("daemon.json");
    let file = DiscoveryFile {
        port: 8371,
        token: "tok".into(),
        pid: 123,
        started_at: Utc::now(),
        heartbeat_at: Utc::now(),
    };
    write_discovery_file_to(&path, &file).expect("write");
    let back = read_discovery_file_from(&path).expect("reads back");
    assert_eq!(back.port, 8371);
    assert_eq!(back.token, "tok");

    std::fs::write(&path, b"{ not json").expect("corrupt");
    assert!(read_discovery_file_from(&path).is_none()); // 损坏 → None，不 panic
}
```

（测试用带路径参数的内部函数 `write_discovery_file_to`/`read_discovery_file_from`；公开无参函数薄封装真实路径。`tempfile` 若不在 dev-dependencies 则用 `std::env::temp_dir()` + 唯一子目录，先查 Cargo.toml。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib utils::discovery 2>&1 | tail -5`
Expected: 编译失败（模块不存在）

- [ ] **Step 3: 实现**

`src/utils/discovery.rs`：

```rust
//! Daemon discovery file (`~/.wgenty-code/daemon.json`): lets UI processes
//! reuse an already-running global daemon instead of spawning a duplicate.
//! Writes are atomic (temp file + rename). The token ALSO stays in
//! `daemon.token` for existing readers (design §6.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
pub const HEARTBEAT_STALE_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

pub fn discovery_file_path() -> PathBuf {
    crate::utils::config_dir().join("daemon.json")
}

pub fn write_discovery_file(file: &DiscoveryFile) -> anyhow::Result<()> {
    write_discovery_file_to(&discovery_file_path(), file)
}

pub fn read_discovery_file() -> Option<DiscoveryFile> {
    read_discovery_file_from(&discovery_file_path())
}

pub fn remove_discovery_file() -> anyhow::Result<()> {
    let path = discovery_file_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

fn write_discovery_file_to(path: &Path, file: &DiscoveryFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string(file)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?; // atomic on all supported platforms
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_discovery_file_from(path: &Path) -> Option<DiscoveryFile> {
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok() // corrupt → None (treated as absent)
}
```

`src/daemon/mod.rs` `run()`：token 写入（:89-94）之后追加：

```rust
    // Discovery file: write now, heartbeat every 30s, delete on clean exit.
    let discovery = crate::utils::discovery::DiscoveryFile {
        port,
        token: api_token.clone(),
        pid: std::process::id(),
        started_at: chrono::Utc::now(),
        heartbeat_at: chrono::Utc::now(),
    };
    if let Err(e) = crate::utils::discovery::write_discovery_file(&discovery) {
        // Non-fatal: discovery is additive; the token file path still works.
        tracing::warn!(error = %e, "failed to write daemon discovery file");
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
            crate::utils::discovery::HEARTBEAT_INTERVAL_SECS,
        ));
        loop {
            ticker.tick().await;
            let mut f = discovery.clone();
            f.heartbeat_at = chrono::Utc::now();
            if let Err(e) = crate::utils::discovery::write_discovery_file(&f) {
                tracing::warn!(error = %e, "daemon discovery heartbeat write failed");
            }
        }
    });
```

退出清理（:136 旁）：`let _ = crate::utils::discovery::remove_discovery_file();`。

- [ ] **Step 4: 运行测试确认通过 + 编译**

Run: `cargo test --lib utils::discovery && cargo check --all-targets`
Expected: 通过

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/utils/discovery.rs src/utils/mod.rs src/daemon/mod.rs
git commit -m "feat(daemon): write global discovery file with heartbeat and exit cleanup"
```

---

### Task 16: `discover_daemon()` + TUI 启动接入

- [x] Task 16 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `src/utils/discovery.rs`（`discover_daemon()`）
- Modify: `src/tui/util.rs:27-`（`start_daemon` 前置发现；`start_daemon` 成功拉起后也写发现文件，复用 Task 15 的写入代码抽函数）
- Test: `src/utils/discovery.rs` 测试模块

**Interfaces:**
- Consumes: Task 15 全部产物、`crate::utils::read_daemon_token()`
- Produces:

```rust
pub struct DiscoveredDaemon { pub port: u16, pub token: String }

/// Discovery decision chain (design §6.2): file exists and parses (else None)
/// → token matches `daemon.token` (else None) → heartbeat fresh (else None).
/// pid liveness is advisory only (cross-platform variance); heartbeat is
/// authoritative. Any failure = stale → caller falls back to spawning.
pub fn discover_daemon() -> Option<DiscoveredDaemon>;
```

- [ ] **Step 1: 写失败测试（判定矩阵：token 不匹配 / 心跳过期 / 命中）**

判定逻辑抽纯函数：

```rust
pub(crate) fn evaluate(
    file: Option<&DiscoveryFile>,
    expected_token: Option<&str>,
    now: DateTime<Utc>,
) -> Option<DiscoveredDaemon> {
    let file = file?;
    let expected = expected_token?;
    if file.token != expected {
        return None; // token mismatch → another daemon instance, do not connect
    }
    let age = now.signed_duration_since(file.heartbeat_at);
    if age.num_seconds() > HEARTBEAT_STALE_SECS as i64 {
        return None; // stale heartbeat → daemon likely dead
    }
    Some(DiscoveredDaemon { port: file.port, token: file.token.clone() })
}
```

测试：

```rust
#[test]
fn evaluate_matrix() {
    let now = Utc::now();
    let fresh = DiscoveryFile { port: 8371, token: "t".into(), pid: 1, started_at: now, heartbeat_at: now };
    assert_eq!(evaluate(Some(&fresh), Some("t"), now).map(|d| d.port), Some(8371));
    assert!(evaluate(Some(&fresh), Some("other"), now).is_none());  // token 不匹配
    assert!(evaluate(Some(&fresh), None, now).is_none());            // 无本地 token
    assert!(evaluate(None, Some("t"), now).is_none());               // 无文件
    let mut stale = fresh.clone();
    stale.heartbeat_at = now - chrono::Duration::seconds(HEARTBEAT_STALE_SECS as i64 + 1);
    assert!(evaluate(Some(&stale), Some("t"), now).is_none());       // 心跳过期
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib utils::discovery -- evaluate 2>&1 | tail -5`
Expected: 编译失败

- [ ] **Step 3: 实现 `discover_daemon()` + TUI 接入**

```rust
pub fn discover_daemon() -> Option<DiscoveredDaemon> {
    let file = read_discovery_file();
    let token = crate::utils::read_daemon_token();
    evaluate(file.as_ref(), token.as_deref(), Utc::now())
}
```

`src/tui/util.rs` `start_daemon` 开头（绑定端口之前）：

```rust
    // Reuse an already-running global daemon when the discovery file checks
    // out; fall through to the embedded spawn path otherwise (design §6.3).
    if let Some(found) = crate::utils::discovery::discover_daemon() {
        // start_daemon 当前返回 (base_url, shutdown_tx, join_handle) 三元组，
        // 复用路径没有本地 join_handle。把返回类型改为枚举或带 Option 句柄的
        // 结构体，调用方（src/tui/app/mod.rs:1132 re-export 的消费处）同步适配：
        // Reused { base_url } 时 shutdown 为空操作。
        return Ok(StartDaemonOutcome::Reused {
            base_url: format!("http://127.0.0.1:{}", found.port),
        });
    }
```

同时把 Task 15 在 `daemon/mod.rs` 里的「写发现文件 + 心跳」逻辑抽成 `pub fn spawn_discovery_writer(port: u16, token: String)`（放 `src/utils/discovery.rs` 或 `src/daemon/mod.rs`），`start_daemon` 拉起成功后同样调用——否则 TUI 内嵌 daemon 不可被发现，两个 UI 复用场景不成立（spec 场景「多 UI 复用 daemon」）。TUI 内嵌路径退出时清理发现文件（随既有 shutdown 流程）。

- [ ] **Step 4: 编译 + 测试 + 手动验证**

Run: `cargo test --lib utils::discovery && cargo check --all-targets`
手动验证（记录结果到 commit message）：
1. `wgenty-code daemon --port 8371` 常驻，再启动 TUI → TUI 日志显示复用（未拉起新实例），鉴权成功。
2. 杀掉 daemon 但保留过期 `daemon.json` → TUI 判定失效，走原拉起路径，不误连。

- [ ] **Step 5: fmt + clippy + 提交**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/utils/discovery.rs src/tui/util.rs src/tui/app/mod.rs src/daemon/mod.rs
git commit -m "feat(tui): reuse running daemon via discovery file before spawning"
```

---

## 第 6 组：验证（tasks.md 6.1–6.5）

### Task 17: 集成验收与回归

- [ ] Task 17 完成标记：本节全部 Step 完成并提交

**Files:**
- Modify: `tests/integration/`（daemon 集成测试，按现有 harness 增补；无对应 harness 的场景用手动验收记录替代）
- Docs: `docs/API.md`（`after=`、`sync_lost`、`GET /events`、409 语义、`expected_version`、发现文件——design §2.3 要求把客户端恢复约定写入 API 文档注释）

- [ ] **Step 1: 断线续传验收（tasks 6.1）**

集成测试或手动脚本：
1. 启动 daemon，创建 session，开两个 SSE 订阅者 A、B。
2. 触发 run 产生若干事件，A 断线后以最后 seq `after=` 重连 → 按序收齐错过事件后接实时，无重复无遗漏（对比 B 的接收序列）。
3. 以 `after=0` 且缓冲已淘汰（或 daemon 重启后）重连 → 收到 `sync_lost`，执行 `GET /sessions/:id` 全量恢复后重新订阅成功。
4. 注入慢消费者（receiver 不消费）→ 仅该连接收到 `sync_lost`（reason=lagged），其他订阅者不受影响。

- [ ] **Step 2: 全局事件验收（tasks 6.2）**

1. 两个客户端同订 `GET /events`：切换 permission-mode、切换模型、产生背景结果 → 两端收到相同 seq 序列的对应事件。
2. 背景结果：客户端 C 离线时产生结果，C 上线后 `GET /background/results` 仍可查到；在线两端均收到广播（无抢占）。

- [ ] **Step 3: 审批与版本冲突验收（tasks 6.3）**

1. 同一 interaction 双客户端 resolve → 先者 200，后者 409 + 当前决议。
2. subagent `resolve-permission` 同理 409。
3. 两写入方基于同一 `expected_version` 并发 PUT → 一者成功，另一者 409 + `current_version`，重读后重试成功。
4. 两个 session 各自触发审批 → 规则归属各自 session（`GET /permission-mode?session_id=` 分别查询互不可见）。

- [ ] **Step 4: 发现文件验收（tasks 6.4）**

即 Task 16 Step 4 的两条手动场景，复核记录。

- [ ] **Step 5: 回归套件 + 性能约束检查（tasks 6.5）**

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --release
time ./target/release/wgenty_code --version   # 启动增量 ≤ 5%
ls -lh ./target/release/wgenty_code           # 二进制增量 ≤ 500KB（对照 base-ref 构建）
```

TUI client-side 与 server-side 双模式手动回归：对话、工具执行、审批正常。

- [ ] **Step 6: API 文档更新 + 最终提交**

`docs/API.md` 增补：`GET /sessions/:id/events?after=` 与 sync_lost 恢复约定、`GET /events`、409 语义表、`PUT /sessions/:id` 的 `expected_version`、`~/.wgenty-code/daemon.json` 格式与失效判定。

```bash
git add tests/integration/ docs/API.md
git commit -m "test(daemon): acceptance coverage for replay, global bus, 409s, and discovery"
```

---

## 自查（Self-Review）

**Spec 覆盖核对：**
- 会话事件流重放/续传 → Task 1–3（含「新订阅者仅实时」场景：Task 3 `CatchUp::LiveOnly` 分支）✅
- 失步信号（淘汰 + Lagged，客户端全量恢复约定）→ Task 3、4、17 Step 6（文档）✅
- 全局事件流（5 类事件、单调 seq、多订阅者同序列）→ Task 5、6、7 ✅
- 背景结果广播 + 保留可查 + 废除 drain → Task 8 ✅
- 轮询端点兼容 + dogfood 迁移等价 → Task 8（GET 保留）、9 ✅
- 审批 409 + 多会话隔离 → Task 10、11、12 ✅
- 会话版本化（含 run 写盘推进、version=0 兼容、run 活跃 409 保持）→ Task 13、14 ✅
- 可发现部署（原子写、心跳 30s/过期 120s、退出清理、token 匹配、失效不误连）→ Task 15、16 ✅
- 向后兼容（无 after live-only / 无 expected_version 兼容 / 旧端点 default 映射保留）→ Task 3、12、14 各分支 ✅

**与 tasks.md 6 组对应：** 1.1→T1、1.2→T3、1.3→T3/T4；2.1→T5、2.2→T6、2.3→T8、2.4→T9；3.1→T10/T11、3.2→T12；4.1→T13、4.2→T14；5.1→T15、5.2→T16；6.1–6.5→T17。挂接类 2.1 细化为 T5（类型）+T7（生产者）。

**已知风险与实现提示：**
- `DaemonEventSink::new` / `RootToolPort::new` 签名变更波及测试构造（编译器逐一指出），属预期改动面。
- Task 7 TodosChanged：base-ref 上 `todo_state` 无写入方（见 Task 7 说明），`apply_todos_update` 先作为收敛入口落地，端到端验证在 T17。
- Task 16 需要调整 `start_daemon` 返回类型以表达「复用」分支，调用方适配范围以编译器报错为准。
- 会话缓冲不跨 daemon 重启（design §10）：重启后 `after=` 直接 `sync_lost`，正确性不依赖缓冲——T17 Step 1.3 覆盖。
