## Why

契约层（`org-graph-node-contract`，已归档）让每次 subagent 分发都解析到一个可信 `NodeType`，coordinator 在 `coordinator.rs:507` 已持有该 `node_type` 并据契约强制。但这份**真实派发数据**——每次 run 实际用了哪个 NodeType——从未被持久化：`subagent_transcripts` 表没有 `node_type` 列，coordinator 的 `node_types` 内存 map 在子 agent 结束时即丢弃（`coordinator.rs:875`）。结果是无法离线审计"过去哪些 run 是 explore/plan/GP/verify/guide"，也无法为后续 budget 激活 / IO 强制提供 per-node-type 的真实派发画像。本 change 闭合这个观测缺口，是 Org-Graph 可观测性路线的第二步（运行时遥测），把契约层从"声明态"推向"可观测态"。

## What Changes

- `SubagentTranscript`（`src/transcript/mod.rs:16`）新增 `node_type: Option<NodeType>` 字段，`#[serde(default)]` 兼容 legacy row。
- `SubagentTranscriptHeader`（`src/transcript/mod.rs:76`，`list` 视图投影）新增 `node_type`，供列表展示。
- `subagent_transcripts` 表新增 `node_type` 列，幂等 `ALTER`（照搬 `failure_diagnostics` 迁移模式 `store.rs:134`），旧库升级不崩、legacy row 为 `None`。
- 在 `save_minimal_transcript`（`task.rs:337`）与 `store.save`（`task.rs:1009`）透传 `node_type`——dispatch 路径 `coordinator.rs:507` 已有 `node_type`，仅缺向 transcript 的透传。
- 扩展 Subagent CLI inspector（`src/cli/subagent.rs` + `SubagentCommands`）：`list` 显示 `node_type` 列；`trace <id>` 显示该 run 的 `node_type`。`health` 是否按 node_type 分组留待 design 阶段定。
- 不改变任何已有字段语义、不改 dispatch 行为；契约强制逻辑零回归。

## Capabilities

### New Capabilities

- `org-graph-dispatch-telemetry`: 持久化每次 subagent 分发的 `NodeType` 到 transcript store，并在离线 Subagent CLI inspector（`list` / `trace`）中按 `NodeType` 展示，为后续 budget/IO 优化提供真实派发画像。

### Modified Capabilities

<!-- 无。transcript schema 加列是本新能力的实现细节；不改变 subagent-transcript-storage 已有需求语义，故以新能力承载全部需求，避免修改既有 spec。 -->

## Impact

- **数据模型**：`SubagentTranscript` / `SubagentTranscriptHeader` 加字段；`transcript/store.rs` schema 迁移（幂等 `ALTER`，向后兼容）。
- **透传路径**：`task.rs` 的 `save_minimal_transcript` / `store.save` 调用点接入 `node_type`；`coordinator.rs` 已有 `node_type`，仅延伸管道。
- **CLI**：`src/cli/subagent.rs`（`list` / `trace` 渲染）+ `src/cli/mod.rs`（如需新参数）。
- **回归风险**：低——字段为 `Option`/可空列，legacy row 与旧库均安全；dispatch 与契约强制路径不动。
- **依赖**：`NodeType` 已派生 `Serialize/Deserialize`（`contract.rs:9`），列存储用其字符串形式即可，无新 crate。
