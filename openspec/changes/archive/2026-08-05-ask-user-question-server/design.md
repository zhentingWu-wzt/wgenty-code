---
comet_change: ask-user-question-server
role: technical-design
canonical_spec: openspec
---

# 设计：服务端 agent loop 的 ask_user_question 支持

## 背景

服务端 loop（Change 1）的 `RootToolPort` 覆盖了普通工具执行 + 权限审批（PermissionBridge），但 **InteractionPort 未接入**。`run_agent_loop_inner`（loop_.rs:1010）在 `interaction` 为 None 时返回 "not available"，agent 调 `ask_user_question` 静默失败。

权限审批已经跑通的端到端链路（本设计的参考模板）：
```
工具 Ask → PermissionBridge.request() 挂起 → trace_hub 广播 permission_pending
→ 前端 trace SSE 收到 → 弹 PermissionModal → POST /tools/resolve-permission
→ bridge.resolve() 唤醒 → 工具继续
```

## 决策（Decisions）

### D1 —— 复用 trace_hub 推送，新增 InteractionBridge（非复用 PermissionBridge）

- **不复用 PermissionBridge**：权限和问题语义不同（权限是 approve/deny，问题是选项 + 自由文本），强行混用会让 StructuredApproval 载荷臃肿、前端分发混乱。
- **新增 `InteractionBridge`**（`src/daemon/interaction_bridge.rs`）：结构与 PermissionBridge 几乎相同（`Mutex<HashMap<String, PendingQuestion>>` + oneshot），但载荷是 `QuestionPayload { request_id, question, options, multi_select }`，返回 `String`（用户答案的 JSON，匹配 `InteractionPort::ask_user_question` 的签名）。
- **WaiterGuard 同款**：复用 permission_bridge 刚修的 RAII 清理模式，避免幽灵问题（dropped waiter 残留）。

### D2 —— TraceEvent 加 `question` 字段（而非新 kind）

- `TraceEvent` 已有 `kind`（progress/permission_pending/permission_resolved）。**新增 `TraceEventKind::QuestionPending` / `QuestionResolved`** + `question: Option<QuestionPayload>` 字段。
- 前端订阅现有 `/subagents/trace/stream`，按 `kind` 分发：permission → 权限弹窗，question → 问题弹窗。
- 零新端点、零新 SSE 连接——和服务端 loop 的"复用通道"哲学一致。

### D3 —— RootToolPort 实现 InteractionPort

- `run_loop.rs` 的 `RootToolPort` 加 `impl InteractionPort for RootToolPort`：
  ```rust
  async fn ask_user_question(&self, args: &serde_json::Value) -> String {
      let payload = QuestionPayload::from_args(args, &self.session_id);
      self.interaction_bridge.request(payload).await
  }
  ```
- `run_agent_loop` 构造时（run_loop.rs 的 `run_loop` 函数）把 `Some(&root_tool_port as &dyn InteractionPort)` 传入 `interaction` 参数。

### D4 —— 前端：复用 PermissionModal 的 ModalShell

- `PermissionModal` 已重构为 `ModalShell`（共享 UI shell）。新增 `pendingQuestion` store 字段 + 一个 question 专用 modal（渲染 question + options 列表 + "Other" 自由输入），resolve 走 `POST /api/v1/interactions/:id/resolve`。
- trace SSE hook（`usePermissionTrace`）扩展：`question_pending` → push question；`question_resolved` → dismiss。

### D5 —— 答案回传端点

```
POST /api/v1/interactions/:request_id/resolve
  body: { "answer": "<JSON string>" }    # 选项 value 或自由文本
  200 → { "resolved": true }
  404 → 无此 pending 问题（已超时/被别人答了）
```

## 风险 / 权衡

| 风险 | 缓解 |
|------|------|
| InteractionBridge 与 PermissionBridge 代码高度相似 | 接受重复；强行抽象通用 bridge 会模糊两种语义，得不偿失 |
| 问题挂起期间 daemon 重启 | 同权限：重启 = 任务终止，用户重发（服务端 loop v1 已有此约束） |
| TraceEvent 又加字段 | `#[serde(default)]` 向后兼容；question 字段 skip_if_none |

## 测试

- `interaction_bridge`：request/resolve/timeout/dropped-waiter 清理（照抄 permission_bridge 测试模式）
- `RootToolPort::ask_user_question`：args → payload 映射 + bridge 挂起 → resolve 后返回答案
- 前端：trace SSE question 事件 → modal 渲染 → resolve POST
