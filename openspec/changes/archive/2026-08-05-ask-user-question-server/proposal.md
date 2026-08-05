# 提案：服务端 agent loop 的 ask_user_question 支持

## 为什么（Why）

服务端 agent loop（`2026-08-02-server-side-agent-loop-design.md` 的 Change 1）已上线，但 `ask_user_question` 这类**需要工具执行期间和用户双向交互**的工具没有通道。

根因（代码事实）：
- `AskUserQuestionTool::execute` 直接返回错误 `interactive_required`——工具本身不执行交互（`src/tools/meta/ask_user_question.rs`）。
- 真正的拦截在 `run_agent_loop_inner`（`loop_.rs:498,668,1011`）：loop 看到工具名是 `ask_user_question` 时，不走 ToolPort，而是走 `InteractionPort::ask_user_question(args) -> String`。
- **服务端 loop 构造时 `interaction` 参数是 None**（`src/daemon/run_loop.rs` 未传 InteractionPort），`RootToolPort` 也没实现该 trait。结果 loop_.rs:1010 走 else 分支，返回 "ask_user_question is not available on this path"。

影响：服务端 loop 下 agent 调 `ask_user_question` 时**静默失败**（工具报错，agent 收到 "not available"），用户永远看不到问题。这是一个已上线架构的功能缺口。

## 变更内容（What Changes）

给服务端 loop 接入 `InteractionPort`，复用权限审批（PermissionBridge）的成熟推送 + 阻塞模式：

1. **daemon 端 `InteractionBridge`**：仿 `PermissionBridge`，工具执行中把问题（question + options）挂起等答案，带 `request_id`。
2. **推送通道**：问题经现有 `trace_hub()` 广播一条 `permission_pending` 风格的事件（`TraceEvent` 加 `question` kind 或复用 permission 载荷扩展）到前端。
3. **前端**：订阅 trace SSE 收到 question 事件 → 弹问题 modal（复用 PermissionModal 的 shell）→ 用户选 → `POST /api/v1/interactions/:id/resolve` 回传答案。
4. **daemon 解锁**：`InteractionBridge` 收到答案，唤醒阻塞中的工具，返回给 loop。

## 非目标（Non-goals）

- 不改动 TUI 的 in-process loop（TUI 已有自己的 InteractionPort 实现，走 `interaction_tui`）。
- 不改动 `ask_user_question` 工具本身的 schema。
- `confirm` / `ask` 等其他 InteractionService 方法不在本期（只做 ask_user_question，它是唯一被 loop 特殊处理的交互工具）。
- 不引入 WebSocket（沿用 SSE + HTTP POST，与服务端 loop v1 一致）。

## 影响（Impact）

- **Code**：`src/daemon/`（新增 InteractionBridge + handler + 路由）、`src/teams/trace_sink.rs`（TraceEvent 加 question 载荷）、`src/daemon/run_loop.rs`（构造时注入 InteractionPort）。
- **API**：新增 `POST /api/v1/interactions/:id/resolve`。
- **前端**：`web/src/hooks/usePermissionTrace.ts`（分发 question 事件）、新增 question modal 或复用 PermissionModal。
- **依赖**：无新增。
