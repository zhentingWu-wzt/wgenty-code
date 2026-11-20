# Design: 子 agent 结果投递时机改进（L0 finalize-join + L1 回合内提示）

## Context

三条现有回传路径的排查结论（2026-11-20 会话实测 + 代码走读）：

| 路径 | 机制 | 阻塞条件 |
|------|------|----------|
| TUI 轮询 | Tick + `!has_running_turn()` → `claim_task_group`（500ms 节流） | 回合运行中不 poll |
| daemon 调度器 | `TaskGroupReady` ping → `prepare_task_group_continuation` | `session_runs.claim` 失败（回合活跃）→ 等 `RunFinished` 重试 |
| finalize join | `SubagentSynthesis::synthesize_root_children` | **未接线**：TUI `core.rs:95` 与 headless `headless_runtime.rs:497` 均 `synthesis: None` |

根因：结果的消费方是 LLM 对话历史，注入只可能发生在回合边界（现状）或 finalize 前（未接线）。

## Goals / Non-Goals

**Goals**
- L0：root finalize 前有界等待并注入就绪子结果，回合内可见。
- L1：回合内每次工具响应捎带就绪计数提示（只提示不投递）。
- 投递恰好一次语义在"await 端点 / TUI 轮询 / daemon 调度器"三方竞争下保持。

**Non-Goals**
- 不做回合中段抢占式注入（L2/L3）——工具结果捎带已在 L1 覆盖提示场景，真实注入仍守回合边界。
- 不改 daemon 调度器与 TUI 轮询的既有行为（它们作为 await 不可用时的兜底）。
- 不给 headless runtime 接线（其交互形态不同，留待后续；注释相应修正为"由 TUI root loop 接线"）。

## Decisions

### D1: await 端点走 daemon，TUI 以 HTTP SynthesisPort 消费

TUI 进程内没有 `AgentCoordinator` 句柄（子 agent 生命周期全在 daemon）。新端点：

```
POST /api/v1/agents/task-groups/await
{ "session_id": "...", "generation": u64, "timeout_ms": 30000 }
→ 200 TaskGroupDeliveryResponse | 204 (timeout / nothing ready)
```

实现（daemon 侧）：
1. 快路径：`claim_ready_root_group` 一次，有则直接返回。
2. 慢路径：`subscribe_ready_groups()` + 循环 { 短睡 200ms → 重试 claim }，整体受 `timeout_ms` 上界（默认 30s，上限 120s）。
   - 不持有 `session_runs` claim：await 是**探测性等待**，不产生合成回合，无需与用户回合互斥；真正的消费仍是 claim（原子、恰好一次）。
   - 用轮询重试而非纯订阅唤醒：订阅 ping 是 hint（可能丢失/滞后），claim 重查是 source of truth，与既有"readiness is re-verified by claiming"原则一致。

TUI 侧新 `RootSynthesisPort`（实现 `SynthesisPort`）：
- `on_candidate_final` → `client.await_task_group(session_id, generation, 30s)`
- `Ok(Some(delivery))` → 返回 `Some(format_child_result_batch(delivery.results))`（复用 `<child-results>` 格式，loop 会作为 user 消息注入并继续）
- `Ok(None)` / 网络错误 → 返回 `None`（正常 finalize；降级路径=现状，等待期错误不影响回合结束）
- 连续注入去重：TUI 侧每个回合最多注入一次（回合结束即重新装配 loop），无需跨回合 `synthesized` set——delivery 被 claim 后即从 store 消失，天然恰好一次。

### D2: L1 提示挂 daemon `execute_tool`，用 `peek_ready` 不消费

- `TaskGroupStore::peek_ready(session_id, owner_id, generation) -> Option<usize>`：只读返回就绪组的成员数（不 remove_delivery）。
- `execute_tool`（handlers/tools.rs）在构造响应后：`peek_ready > 0` 时在 `content` 末尾追加
  `\n<reminder>N background subagent result(s) are ready and will be delivered when this turn ends. You may finish this turn early to receive them.</reminder>`
  - 复用 task-board nudge（loop_.rs:957）已验证的"JSON + 尾随 `<reminder>`"模式，模型侧解析兼容。
  - 只对当前 session 的 root-direct 组计数；generation 不匹配不计（旧 generation 的组不属于本回合）。
  - 投递本体绝不走此通道——提示只改变 agent 的收尾决策，不改变消息序。

### D3: 端到端时序与竞争

```
子 agent 完成 → record_result → group ready → ping
  ├─ TUI await 端点（回合 finalize）──claim──→ 注入本回合末尾（新）
  ├─ TUI 闲置轮询（回合已结束）──claim──→ 独立 continuation 回合（现状）
  └─ daemon 调度器（RunFinished）──claim──→ daemon 合成回合（现状）
```
- 三方共享同一原子 claim；任一方得手即从 store 移除，其余空手，恰好一次。
- await 与 TUI 轮询的间隙竞态：await 返回投递后回合继续，若期间用户排队了新输入，`pending_inputs.front_mut().continuation` 挂载逻辑（continuation.rs:43-48）不受影响（那是 TUI 自己 claim 的路径）。

### D4: 风险与缓解

| 风险 | 缓解 |
|------|------|
| finalize 被拖慢最多 30s | 快路径 claim 命中立即返回；超时只在确有子任务未完成时发生，且等待本身就是目的（拿结果） |
| await 与调度器双 claim | 原子 claim 天然互斥；输家 204/None 走各自兜底 |
| reminder 注入破坏工具 JSON 解析 | 与 task-board nudge 同一模式，已有生产验证；只在 content 末尾追加纯文本 |
| headless 未接线造成新不一致 | 注释明确 TUI 接线现状；headless 列为 Non-Goal 并留 TODO |

## Migration / Compatibility

纯增量：新端点、新字段无破坏；`peek_ready` 为新增只读方法。旧客户端（web/TUI 旧版）行为不变。

## Open Questions

- await 慢路径 200ms 重试间隔是否需要配置项？（默认写死，观察后再说）
