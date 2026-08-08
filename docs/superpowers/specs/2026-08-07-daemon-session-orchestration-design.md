---
comet_change: daemon-session-orchestration
role: technical-design
canonical_spec: openspec
---

# Design Doc: daemon-session-orchestration

> 深度技术设计。上游事实源：`openspec/changes/daemon-session-orchestration/`（proposal/design/specs/tasks）。本文不重复需求，只细化实现。

## 1. 总体架构

全局单 daemon（沿用已落地的多项目注册表），在其上补齐五个可靠性缺口：

```
┌──────────────────────── daemon（全局单实例）────────────────────────┐
│  SessionEventHub（已有）           GlobalEventHub（新增 §3）         │
│  per-session seq + fan-out         独立 seq + fan-out                 │
│  ├─ 会话事件                        ├─ TodosChanged（全量快照）       │
│  ├─ SyncLost（新增 §2）             ├─ BackgroundResult               │
│  └─ 环形重放缓冲（新增 §2）         ├─ ModeChanged / ModelChanged     │
│                                     └─ TaskGroupResult                │
│  GET /sessions/:id/events?after=    GET /events（新）                 │
│  发现文件 ~/.wgenty-code/daemon.json（§6）                            │
└──────────────────────────────────────────────────────────────────────┘
```

## 2. 事件流重放与失步（src/daemon/run_loop.rs）

### 2.1 SessionEventBuffer

```rust
struct SessionEventBuffer {
    events: VecDeque<SessionEvent>, // 容量 capacity（默认 1024，对齐 TRACE_HUB）
}
```

- 存储位置：`DaemonState` 中 `session_buffers: RwLock<HashMap<String, SessionEventBuffer>>`，与 `session_seq_counter` 同级
- 写入点：事件 publish 到 `SessionEventHub` 的同一处同步压入缓冲（保证缓冲与广播一致）；容量满时 `pop_front` 淘汰
- `oldest_seq()` / `latest_seq()` 供衔接判定

### 2.2 after=seq 订阅流程

`GET /sessions/:id/events?after=<seq>`：

1. 无 `after` → 维持现状（live-only）
2. 有 `after`：
   - `after >= latest_seq` → 直接挂实时
   - `after + 1 >= oldest_seq` → 先按序发送缓冲中 `seq > after` 的事件，再挂实时；衔接处实时事件按 seq 去重（订阅 broadcast 先建立、重放后发送，实时流中 seq ≤ 缓冲已发的丢弃）
   - `after + 1 < oldest_seq` → 发送 `SyncLost` 事件后关闭流（或保持流由客户端决定重订，见 2.3）

### 2.3 SyncLost

```rust
SessionEventKind::SyncLost // data: { reason: "evicted" | "lagged", latest_seq }
```

- 触发 A（订阅时）：after 的 seq 已淘汰
- 触发 B（运行中）：该连接的 `broadcast::Receiver::recv()` 返回 `Lagged`——当前仅 `tracing::warn`（run_loop.rs:810-817），改为向该连接发送 SyncLost 事件
- 客户端恢复约定（写入 API 文档注释）：收到 SyncLost → `GET /sessions/:id` 全量恢复 → 以响应中最新的 turn 状态对齐后重新订阅（不携带 after 或携带最新 seq）

## 3. 全局事件总线（新增 src/daemon/global_events.rs）

### 3.1 独立 Hub

```rust
pub struct GlobalEvent { pub seq: u64, pub kind: GlobalEventKind, pub data: serde_json::Value }
pub enum GlobalEventKind { TodosChanged, BackgroundResult, ModeChanged, ModelChanged, TaskGroupResult }
pub type GlobalEventHub = broadcast::Sender<GlobalEvent>;
```

- 不复用 SessionEventHub 信封：会话/全局语义分离，独立 seq 空间（`AtomicU64`），避免会话事件高频挤占全局事件
- 挂到 `DaemonState.global_event_hub` + `global_seq_counter`

### 3.2 生产者挂接点

| 事件 | 挂接位置 |
|------|---------|
| TodosChanged | `todo_state` 更新的所有 handler（GET /todos 轮询兼容保留）；data 为全量快照（todos 体积小，YAGNI 不做增量 diff） |
| BackgroundResult | background_manager 完成回调；**先写入保留队列再广播** |
| ModeChanged | permission-mode 切换 handler |
| ModelChanged | models/switch handler |
| TaskGroupResult | task-group claim 对应的结果产生处 |

### 3.3 背景结果保留队列

- `background_results: RwLock<VecDeque<BackgroundResult>>`，容量上限（默认 256），满则淘汰最旧
- `GET /background/results` 改为**读快照**（不再 drain），旧客户端轮询行为不变、结果不再被抢占
- 容量淘汰接受（极低频场景；多端在线时经事件获取）

### 3.4 GET /events

SSE 端点，15s keep-alive（与 sessions events 一致）；v1 live-only 不重放——客户端对齐状态用现有 GET 端点兜底。

## 4. 审批语义收敛（src/daemon/handlers.rs）

- `POST /interactions/:id/resolve`：已决议 → 409 + 当前决议内容（替换现 404）
- `POST /tools/resolve-permission`（subagent）：已决议 → 409（替换 `{success:false}`）
- `"default"` 硬编码清零（handlers.rs:485,654,664,699,740,765,1397,1431,1458,1507 约 10 处）：
  - server-side 路径（run/events/interactions 等）：请求缺 session_id → 400
  - 旧端点路径（chat_stream 配套 tools/execute、approve 等）：保留 `unwrap_or("default")` 映射 + `// deprecated(compat)` 注释，不在本 change 破 TUI client-side 模式

## 5. 会话存储版本化（src/context/memory_session.rs + handlers.rs）

```rust
pub struct Session {
    // ...existing fields
    #[serde(default)]
    pub version: u64, // 历史文件反序列化为 0
}
```

- `PUT /sessions/:id` 请求体增加可选 `expected_version: Option<u64>`：
  - `Some(v)` 且 `v != current.version` → 409 + `{ current_version }`
  - `None` → 维持现状兼容（last-write-wins 风险留给未升级客户端，文档标注）
- 写入成功一律 `version += 1`；run 写盘路径（save_gen 机制处）同样推进版本
- run 活跃 409 保持既有语义

## 6. 发现文件（src/utils/ + src/daemon/mod.rs）

### 6.1 格式与生命周期

`~/.wgenty-code/daemon.json`：

```json
{ "port": 8371, "token": "...", "pid": 12345, "started_at": "...", "heartbeat_at": "..." }
```

- 写入：临时文件 + rename 原子替换；daemon 启动时写入
- 心跳：tokio 任务每 30s 更新 `heartbeat_at`（重写整个文件，量小无妨）
- 过期阈值：120s；正常退出删除文件；token 同时保留写入全局 `daemon.token`（兼容现有读取方）

### 6.2 discover_daemon()

```rust
pub fn discover_daemon() -> Option<DiscoveredDaemon> // { port, token }
```

判定链：文件存在且可解析（失败→None）→ token 与 `daemon.token` 匹配（不匹配→None）→ `heartbeat_at` 未过期（过期→None）。pid 存活检查为辅助信号（跨平台差异大，心跳为主）。

### 6.3 TUI 接入（dogfood 之一部分）

TUI `start_daemon` 流程前置：先 `discover_daemon()`，命中则跳过拉起直接连接；未命中走现有拉起逻辑。

## 7. dogfood 迁移：TUI todos 订阅化

- TUI todos 面板数据源从 `GET /todos` 轮询切换为 `GET /events` 订阅 `TodosChanged`（快照直接替换本地状态）
- 保留轮询作为订阅失败回退（连接断开时回退 500ms 轮询 + 周期性重试订阅）
- 验证：与轮询行为等价（todos 变更在两端实时同步）

## 8. 错误处理矩阵

| 场景 | 行为 |
|------|------|
| after seq 淘汰 | SyncLost 事件 → 客户端全量恢复 |
| 运行中 Lagged | SyncLost 事件（仅该连接） |
| 缺 session_id（server-side 路径） | 400 |
| 重复审批应答 | 409 + 当前决议 |
| PUT 版本冲突 | 409 + current_version |
| 发现文件损坏/过期/token 不匹配 | 视为失效，回退拉起，不阻塞启动 |
| 全局流断连 | 客户端回退轮询 + 重试订阅 |

## 9. 测试策略

- **单元**：环形缓冲淘汰与 oldest/latest 边界；after 衔接去重（重放末尾与实时开头重叠）；版本校验矩阵（None/Some 匹配/Some 不匹配/历史 version=0）；发现文件原子写、过期判定、损坏容错
- **集成**（daemon 测试）：双订阅者 after 续传一致性；注入慢消费者触发 Lagged→SyncLost；重复审批 409（interaction + subagent 两条路径）；并发 PUT 冲突 409；全局事件多端 fan-out 与背景结果非抢占
- **dogfood**：TUI todos 订阅化与轮询等价性
- **回归**：`cargo test` daemon/session/run_loop 套件；TUI client-side 与 server-side 双模式手动验证

## 10. 边界条件

- 会话缓冲只覆盖进程生命周期：daemon 重启后 after 续传不可用（缓冲空）→ after 直接 SyncLost，客户端全量恢复（正确性不依赖缓冲）
- ContentDelta 高频刷爆 1024 窗口：接受，SyncLost 兜底；容量留配置项（`daemon.event_buffer_capacity`）
- 多项目：事件流按 session 隔离天然兼容；全局事件是 daemon 级（跨项目），TodosChanged 等 data 需带项目维度字段以便客户端过滤
- 旧 TUI（不携带 expected_version / 不走发现文件）行为完全不变

## 11. Spec Patch（已回写 delta spec）

1. 发现文件 per-working-dir → 全局 `~/.wgenty-code/daemon.json`
2. 「不改变任何客户端默认行为」→ 放宽为「仅 TUI todos 一处 dogfood 迁移」
3. 失步信号明确为 `SessionEventKind::SyncLost` SSE 事件
