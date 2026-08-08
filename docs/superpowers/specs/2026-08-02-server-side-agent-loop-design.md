# 服务端 Agent Loop（web 变观察者）设计

日期：2026-08-02
状态：已确认（方案 A：两个连续 change）

## 背景与目标

目前 agent loop 跑在浏览器里（daemon `/chat/stream` 纯透传 + web 驱动 `/tools/execute`），关掉浏览器任务就死。本设计把 loop 迁入 daemon：`POST /sessions/:id/run` 后 daemon 自己跑完整 turn（LLM 调用 + 工具执行 + 消息持久化），web 从驱动者降级为观察者——关闭浏览器/换设备，任务照跑，回来可接着看。

### 已确认的决策

| 决策点 | 结论 |
|:--|:--|
| 应用范围 | 全部会话迁移到服务端 loop；web 不再驱动任何 turn |
| 权限超时 | 无限期挂起——root loop 的权限请求不设超时，等用户从任何设备回来处理 |
| 流式通道 | v1 用 SSE（`GET /sessions/:id/events`）；事件信封 `SessionEvent` 按"将来平移 WebSocket 零成本"设计（传输层可换，事件模型不变） |
| 交付方式 | 两个连续 change：Change 1 纯 daemon（curl 可验证）；Change 2 web 观察者化 |

### v1 明确不做

- compaction hook（长会话上下文压缩，后续接 `ApiCompactor`）
- TUI 迁移到 `/run`（TUI 保持现有 in-process loop；绑定关系在 daemon 可后续共享）
- WebSocket 传输层（只做信封设计）
- daemon 重启中断恢复的自动续跑（重启 = 进行中的 run 终止，已保存的消息不丢，用户手动重发；见错误处理）

## 关键探索结论（已核实）

- `run_agent_loop(args)`（`src/agent/runtime/loop_.rs:155`）的全部依赖 daemon 都能进程内构造：`ApiLlmPort`（settings_handle + 池化 http client）、`MutexHistoryStore`、`RegistryToolPort`（headless 已有，`src/cli/headless_runtime.rs:89`）、`prompts::assemble_instructions`。
- loop 自己管理 history 并在回合/turn 边界发 `RuntimeEvent::SaveSession`；`RuntimeEvent` 已涵盖 ContentDelta/ReasoningDelta/ToolStart/ToolResult/StreamDone/StreamError/SaveSession（`src/agent/runtime/events.rs:8-45`）。
- 权限：`GuardingToolPort::resolve_ask` → `PermissionBridge::request_with_timeout`（subagent 路径）。root loop 复用 `state.permission_bridge` + 共享 `session_rules`；web 现有 trace SSE + `/tools/resolve-permission` 流程零改动兼容。
- loop 无取消感知；subagent 用 `tokio::select!` 包取消令牌（`subagent_loop.rs:1339-1372`）——照抄。
- 并发风险：web 现有 autosave（全量 PUT）与服务端 run 写会话会互相覆盖——**必须有 run 锁**。
- `MemorySessionManager::add_message` 不支持 tool_calls 配对——持久化必须整史快照 + `sanitize_tool_call_pairing` + `session_manager.save()`（TUI 同款）。

## Change 1：Daemon 设计

### 1. Run registry（`DaemonState` 新增）

```rust
pub session_runs: Arc<RwLock<HashMap<String, SessionRun>>>

pub struct SessionRun {
    pub run_id: String,
    pub cancel: CancellationToken,   // 每 turn 新建，不污染缓存的 root context
    pub started_at: Instant,
}
```

- 每会话同时最多一个 run：已存在 → `POST /run` 返回 409。
- **run 锁**：`update_session`（PUT /sessions/:id）在 run 活跃期间返回 409 `{"error":"run active"}`——挡住旧 web 的 autosave 竞争（Change 2 后 web 不再 PUT，但锁仍防第三方客户端）。
- run 结束（完成/出错/取消）→ 从 map 移除。

### 2. 端点

```
POST /api/v1/sessions/:id/run
  body: { "message": "..." }
  202 → { "run_id": "...", "session_id": "..." }     # 异步接受，立即返回
  404 → session 不存在    409 → 已有活跃 run
  400 → 空消息

POST /api/v1/sessions/:id/cancel
  204 → 已取消    404 → 无活跃 run

GET  /api/v1/sessions/:id/events        # SSE，见 §3
```

`POST /run` 的处理（spawn 后台任务，立即 202）：

1. 认领 run registry（占位，失败即 409）
2. 加载 session → `MutexHistoryStore` 以 `Session.messages` 为种子 → push 用户消息
3. `ApiLlmPort`（settings_handle 克隆 + 池化 client）
4. RootToolPort（见 §4）
5. DaemonEventSink（见 §3）+ HistoryStore→Session 持久化桥（见 §5）
6. `prompts::assemble_instructions` 构造 system_messages
7. `tokio::select!` 包 `run_agent_loop` vs `cancel.cancelled()`（subagent 同款）；loop config 用 `StreamStyle::default()`（`use_tool_definitions: true`）
8. 结束：整史快照 sanitize 后 `session_manager.save()`，registry 移除

### 3. 事件通道：`GET /sessions/:id/events`（SSE）

**信封（WebSocket-ready）**：

```json
{
  "seq": 42,                     // 每会话单调递增，重连去重/续传用
  "session_id": "d1",
  "run_id": "r1",
  "kind": "content_delta",       // content_delta | reasoning_delta | tool_start
                                 // | tool_result | turn_done | turn_error | save
  "data": { "text": "..." }      // kind 对应的负载（映射自 RuntimeEvent）
}
```

- 实现：daemon 侧 `SessionEventHub`（每会话一个 broadcast channel，容量 1024、drop-oldest，模式照抄 trace hub `trace_sink.rs:35-66`）。`DaemonEventSink` 把 `RuntimeEvent` 映射为 `SessionEvent` 广播。
- **冷启动**：SSE 不带回放（v1）；迟到的客户端先 `GET /sessions/:id` 拿持久化消息做冷状态，再订 SSE 接增量。`seq` 允许客户端丢弃重复。
- **将来升级 WebSocket**：同一 `SessionEvent` JSON 模型原样走 WS 帧，增加 subscribe/unsubscribe 控制消息即可——信封设计不变。
- 权限事件**不**走这个通道——沿用现有 trace SSE + `/tools/resolve-permission`（已验证兼容）。

### 4. RootToolPort

新 port（`src/daemon/run_loop.rs` 或 `src/agent/runtime/` 下新文件），组合：

- `state.tool_registry` 执行（含 guardian 短路、checkpoint 捕获）
- `ToolPermissionPolicy` 校验 → Allow 直走；Ask → `state.permission_bridge` 挂起
- **无超时**：给 `PermissionBridge` 加 `request()`（不带 deadline 的挂起变体；现有 `request_with_timeout` 保留给 subagent）。fail-open 不存在——只能 approve/deny。
- 共享 `session_rules`（approve 后规则对会话生效，与现有行为一致）
- `StructuredApproval.from = session_id`——web 的 trace 过滤和弹窗归因正确

### 5. 持久化桥

`DaemonEventSink` 收到 `SaveSession` 时：`sanitize_tool_call_pairing(history)` → 整史写入 `Session.messages` → `session_manager.save()`（写盘 + 更新内存索引，与 TUI 的 spawn_save_session 同语义）。turn 结束强制最终保存一次。

## Change 2：Web 设计

### 1. sessionRunner 重写（驱动者 → 观察者）

```
发送消息 → POST /sessions/:id/run {message}
         → 本地乐观渲染 user 消息 + 状态置 running
         → 订阅 GET /sessions/:id/events（SSE）
           · content_delta/reasoning_delta → appendAssistant
           · tool_start/tool_result → attachToolExec
           · turn_done → 状态 idle，断开 SSE
           · turn_error → 状态 error + 错误条
Stop → POST /sessions/:id/cancel
```

### 2. 删除的东西

- `web/src/agent/loop.ts` 的浏览器内 `runAgentLoop` 调用（整个 client-side 驱动路径）
- autosave（`saveSession` PUT）——daemon 自己持久化
- `toWireMessages`（不再需要客户端拼装 wire 历史）

### 3. 保留的东西

- 权限弹窗链路（trace SSE + resolve-permission，零改动）
- 会话冷加载：`GET /sessions/:id` → `sessionMessagesToDisplay`
- `sessionManager` 状态机（running/awaiting_approval/idle/error）
- beforeunload 提示（run 在服务端了，可以弱化——v1 保留文案改为提示"任务在服务端继续运行"？简单起见保留现状，Change 2 内决定）

### 4. 重连

SSE 断开（网络抖动/daemon 重启）→ 指数退避重连；重连后先 `GET /sessions/:id` 对齐冷状态再继续订增量（`seq` 去重）。daemon 重启场景：run 已死，事件流返回 404/无事件 → web 把该会话状态置 idle 并提示。

## 错误处理

- **run 锁冲突**：`POST /run` 409 → web 提示"该会话已有任务在运行"
- **权限挂起期间**：会话状态保持 `awaiting_approval`（trace SSE 驱动）；用户任何设备回来都能在弹窗处理
- **daemon 重启**：进行中 run 终止，registry 丢失；`POST /run` 幂等接受新 run；事件 SSE 连接失败 → web 重连 + 冷状态对齐
- **LLM 上游错误**：loop 的 stream 重试（`stream_max_retries`）后仍失败 → `turn_error` 事件 + 状态 error + 持久化已有进度

## 测试

**daemon（Change 1）**

- run registry：认领/冲突 409/结束释放；`update_session` 在 run 活跃时 409
- `POST /run`：session 不存在 404、空消息 400、成功 202 且用户消息入史
- `POST /cancel`：令牌触发 loop 退出（select! 分支），registry 移除
- 事件映射：`RuntimeEvent` → `SessionEvent` 的 kind/data 对应正确（单测 DaemonEventSink）
- 持久化桥：`SaveSession` 触发整史 save，tool_calls 配对完整
- RootToolPort：Ask → bridge 挂起 → resolve 后执行（复用 PermissionBridge 现有测试模式）

**web（Change 2）**

- sessionRunner：POST /run 调用 + 乐观渲染；SSE 各 kind 驱动 store 正确；turn_done/turn_error 状态迁移
- Stop → cancel POST
- 删掉的代码无残留引用（typecheck + lint）

**验收**：`cargo test --all` + clippy + web 全绿；手动冒烟：POST /run 发起任务 → 关掉浏览器 → 重开 → 会话显示任务完成且历史完整。

## 新增依赖

无（axum broadcast/SSE 均现有）。
