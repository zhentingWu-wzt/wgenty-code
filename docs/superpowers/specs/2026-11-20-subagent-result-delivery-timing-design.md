---
comet_change: subagent-result-delivery-timing
role: technical-design
canonical_spec: openspec
---

# Technical Design: 子 agent 结果投递时机改进（L0 finalize-join + L1 回合内提示）

日期：2026-11-20 · 上游事实源：`openspec/changes/subagent-result-delivery-timing/`（proposal/design/tasks）

## 1. 背景与根因

子 agent 结果的消费方是 LLM 对话历史；注入只能发生在回合边界。现有三条回传路径：

1. TUI 闲置轮询（500ms，`!has_running_turn()` 门控）——回合运行中不 poll。
2. daemon 调度器（`TaskGroupReady` ping → `RunFinished` 重试）——回合活跃时让位。
3. finalize join——`SubagentSynthesis` 注释承诺 root 会 join，但 TUI（`tui/agent/core.rs`）与 headless（`cli/headless_runtime.rs`）的 root loop 均为 `synthesis: None`，从未接线。

实测后果：20 分钟审查回合中两个子 agent 于 02:20 完成，结果 02:36 回合结束才送达，主 agent 中途只能自行重做核查。

## 2. 方案

### 2.1 L0 — root finalize join（await 端点 + TUI SynthesisPort）

**daemon 端点** `POST /api/v1/agents/task-groups/await`

```
请求: { session_id, generation, timeout_ms? }   // timeout_ms 默认 30000，上限 120000
响应: 200 TaskGroupDeliveryResponse | 204（超时/无就绪组）
```

实现要点（`src/daemon/handlers/agents.rs`）：
- 快路径：`claim_ready_root_group` 一次，命中即返。
- 慢路径：`timeout_ms` 窗口内每 200ms 重试 claim（readiness ping 是 hint，claim 重查是事实源——与既有"readiness is re-verified by claiming"原则一致）。
- 不持 `session_runs` claim：探测性等待不产生合成回合；消费的原子性与互斥由 claim 自身保证。
- 路由注册进 `src/daemon/routes.rs`，与 `/claim` 相邻。

**TUI 侧** `RootSynthesisPort`（新类型，放 `src/tui/agent/adapters.rs`）：

```rust
impl SynthesisPort for RootSynthesisPort {
    async fn on_candidate_final(&self, _candidate: &str) -> Result<Option<String>, RuntimeError> {
        match self.client.await_task_group(&self.session_id, self.generation, TIMEOUT).await {
            Ok(Some(delivery)) if !delivery.results.is_empty() =>
                Ok(Some(format_child_result_batch(&delivery.results))),  // <child-results> 注入，回合继续
            _ => Ok(None),  // 未命中/网络错误 → 正常 finalize（降级=现状）
        }
    }
}
```

- `format_child_result_batch` 从 `teams/subagent_loop.rs` 导出复用（格式契约与 TUI `process_continuation`、daemon `structured_task_group_continuation` 三方一致）。
- `client.await_task_group`（`src/tui/client.rs`）：POST await 端点，204→`Ok(None)`，用 no-timeout client（30s 等待 > 300s 工具 client 是安全的，但 stream client 无超时更合适——选用 `http_tools()`，300s 上限覆盖 120s 最大等待）。
- 接线：`tui/agent/core.rs` 构造 `RootSynthesisPort` 传入 `LoopHooks.synthesis`（替换 `None`）。`generation` 取自 `AgentLoop` 已持有的 `agent_generation`。
- 恰好一次：delivery 被 claim 后即从 store 移除；loop 的 `Ok(Some)` 注入会继续回合，下一次 finalize 的 await 只会命中新的组。

**注释修正**（`teams/subagent_loop.rs:412`）：改为"root 的 finalize join 由 TUI root loop 通过 await 端点接线；本 port 用于子 agent 嵌套子代的合成屏障"。

### 2.2 L1 — 回合内就绪提示

**`TaskGroupStore::peek_ready`**（`src/agent/task_group.rs`）：

```rust
pub async fn peek_ready(&self, session_id: &SessionId, owner_id: &AgentId, generation: u64) -> Option<usize>
// 返回满足 (session, owner, generation, is_ready) 的最小组 id 对应的成员数；不修改状态。
```

**捎带**（`src/daemon/handlers/tools.rs` `execute_tool`）：构造 `ExecuteToolResponse` 后，若 `peek_ready(...) == Some(n)` 且 `n > 0`，在 `content` 末尾追加：

```
\n<reminder>{n} background subagent result(s) are ready and will be delivered when this turn ends. You may finish this turn early to receive them.</reminder>
```

- 复用 task-board nudge（`agent/runtime/loop_.rs:957`）已验证的"JSON+尾随 `<reminder>`"模式。
- 需要 root owner 上下文：`execute_tool` 已通过 `state.root_context(session_id)` 拿到 root（现有代码），传其 agent_id 与 current generation。
- 只提示不投递：投递仍守 claim 恰好一次，提示不改变消息序。

### 2.3 端到端时序

```
子 agent 完成 → record_result → group ready → ping
  ├─ TUI await（finalize）──claim──→ 本回合末尾注入（新）
  ├─ TUI 闲置轮询───────────claim──→ 独立 continuation 回合（现状，兜底）
  └─ daemon 调度器──────────claim──→ daemon 合成回合（现状，兜底）
```

## 3. 错误处理

- await 端点网络失败/5xx：TUI 返回 `Ok(None)` 正常 finalize，不阻塞回合结束；下轮 TUI 轮询兜底。
- await 超时（子任务卡死）：204 → 正常 finalize；子 agent 自身 timeout 兜底（TaskGroupStore deadline）。
- peek_ready 出错：跳过捎带（提示是 best-effort）。
- daemon 重启：组状态丢失与现状一致（in-memory store 既有行为）。

## 4. 测试策略

- `task_group.rs` 单测：peek_ready 就绪计数 / 未就绪 None / generation 不匹配 None / 不消费（peek 后 claim 仍可得）。
- `handlers/agents.rs` 测试：await 命中（组已就绪立即 200）、await 超时 204、与 claim 恰好一次（await 得手后 claim 204）。
- `handlers/tools.rs` 测试：有就绪组时 content 含 reminder、无组时不含；permission-required 路径不捎带。
- TUI adapters 单测：RootSynthesisPort 命中→Some(batch 含 `<child-results>`)、未命中/错误→None。
- 全量 `cargo test --lib` + `cargo clippy --lib -- -D warnings`。

## 5. 边界与非目标

- headless runtime 不接线（交互形态不同，TODO 注明）。
- 不做回合中段真实注入（L2/L3）；不改变 TUI 轮询与 daemon 调度器行为。
- `<child-results>` 批格式沿用现有 JSON contract（child_id/status/summary/error_code/partial_result），不变更。
