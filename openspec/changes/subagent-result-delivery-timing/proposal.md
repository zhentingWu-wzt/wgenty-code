# Proposal: 子 agent 结果按回合边界投递的时机改进

## Why

排查确认：root agent 派生 `task` 子 agent 后，子结果**只在回合边界**可见（TUI 闲置轮询 500ms / daemon 调度器在 `RunFinished` 后重试），且 TUI/headless 的 root loop 均为 `synthesis: None`——`subagent_loop.rs:412` 注释承诺的"root finalize 前 join 子 agent"从未接线。长回合中（实测 20 分钟审查回合），主 agent 对已完成的子结果完全无感，只能重复劳动（本次事故中主 agent 等不到审查结果自行重做了对抗式核查）。

两条改进：

1. **L0 — root finalize join 接线**：root loop 的 finalize 路径接上 `SynthesisPort`，回合结束前有界等待就绪的子结果并内联注入，消除"完成→下轮才可见"。
2. **L1 — 回合内就绪提示**：daemon 在工具执行响应中捎带"N 个子任务结果已就绪待投递"的 `<reminder>`（只提示不投递），让长回合中的 agent 能感知并主动收尾。

## What Changes

- 新增 daemon 端点 `POST /api/v1/agents/task-groups/await`：有界（默认 30s）等待会话的就绪 root-direct 组并原子 claim，返回与现有 claim 相同的 payload；超时/无组返回 204。
- `TaskGroupStore` 新增只读 `peek_ready`（不消费；claim 的 exactly-once 语义不变）。
- TUI root loop（`tui/agent/core.rs`）接线 `SynthesisPort`：finalize 候选出现时调 await 端点，有投递则注入 `<child-results>` 继续，无投递则正常结束。
- daemon `execute_tool` 响应末尾捎带就绪计数 `<reminder>`（复用 task-board nudge 的既有注入模式与措辞风格）。
- 修正 `subagent_loop.rs` 中"root 会 join"的注释与实现不符（接线后注释成立）。

## Impact

- 变更文件：`src/daemon/routes.rs`、`src/daemon/handlers/agents.rs`、`src/agent/task_group.rs`、`src/tui/agent/core.rs`、`src/tui/agent/adapters.rs`（或新增模块）、`src/tui/client.rs`、`src/daemon/handlers/tools.rs`、`src/teams/subagent_loop.rs`（注释）
- 兼容性：新增端点与响应尾部追加均为增量；现有 500ms 轮询、daemon 调度器路径保持不变（await 端点与轮询共享同一原子 claim，先到先得、恰好一次）
- 风险：finalize 多等最多 30s（可通过无就绪组立即返回规避——先 peek 再 await）；双通道投递竞争由既有 claim 原子性兜底
