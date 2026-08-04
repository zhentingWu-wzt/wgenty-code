# 任务：服务端 agent loop 的 ask_user_question 支持

## 1. daemon 端 InteractionBridge

- [x] 1.1 新建 `src/daemon/interaction_bridge.rs`：`InteractionBridge`（`Mutex<HashMap<String, PendingQuestion>>` + oneshot + `WaiterGuard` RAII 清理，照抄 permission_bridge 模式）
- [x] 1.2 `QuestionPayload` 结构（request_id / session_id / question / options / multi_select），`from_args(args, session_id)` 从工具 args 解析
- [x] 1.3 `request(payload) -> String`（挂起等答案）、`resolve(id, answer)`、单元测试（request/resolve/timeout/dropped）

## 2. trace 推送通道

- [x] 2.1 `TraceEventKind` 加 `QuestionPending` / `QuestionResolved`；`TraceEvent` 加 `question: Option<QuestionPayload>`（`#[serde(default)]` 向后兼容）
- [x] 2.2 `TraceEvent::question()` 构造函数；`InteractionBridge` 在 request/resolve 时经 `trace_hub()` 广播

## 3. RootToolPort 接入 InteractionPort

- [x] 3.1 `impl InteractionPort for RootToolPort`：`ask_user_question(args)` → `InteractionBridge::request`
- [x] 3.2 `run_loop` 构造 `run_agent_loop` 时传入 `Some(&root_tool_port as &dyn InteractionPort)`
- [x] 3.3 测试：ask_user_question 工具调用 → bridge 挂起 → resolve 后 loop 收到答案继续

## 4. 答案回传端点

- [x] 4.1 `POST /api/v1/interactions/:id/resolve` handler（body `{ answer }` → `InteractionBridge::resolve`）
- [x] 4.2 路由注册（protected）；handler 测试

## 5. 前端

- [ ] 5.1 `client.resolveInteraction(id, answer)` 方法
- [ ] 5.2 store 加 `pendingQuestion` 字段 + push/clear actions
- [ ] 5.3 `usePermissionTrace` 分发 `question_pending` / `question_resolved`
- [ ] 5.4 QuestionModal 组件（question + options 列表 + Other 自由输入），resolve 调 client

## 6. 验证与收尾

- [ ] 6.1 `cargo fmt` + `cargo clippy --all-targets --features daemon -- -D warnings` + `cargo test`
- [ ] 6.2 `cd web && npm run build && npm test`
- [ ] 6.3 更新 WGENTY.md（如需记录交互端点）
