## Context

契约层（`org-graph-node-contract`，已归档）让每次 subagent 分发都解析到一个可信 `NodeType`，coordinator 在 `coordinator.rs:507` 已持有 `node_type` 并据契约强制。但这份真实派发数据从未持久化——`subagent_transcripts` 表无 `node_type` 列，coordinator 的 `node_types` 内存 map 在子 agent 结束时即丢弃（`coordinator.rs:875`）。本 change 闭合该观测缺口：把 dispatch 已知的 `node_type` 持久化进 transcript，并在离线 Subagent CLI inspector 中展示。

## Goals / Non-Goals

**Goals:**

- 持久化每次 run 的 `NodeType`（向后兼容 legacy row / 旧库）。
- Subagent CLI `list` / `trace` 展示 `node_type`。
- 零回归（dispatch、契约强制、已有 transcript 字段语义不动）。

**Non-Goals:**

- 静态注册表渲染（姊妹 change `org-graph-contract-viewer` 负责）。
- budget 激活、IO shape 强制、关系层。
- ContractViolation 计数 / 实时 daemon 视图按 node_type 过滤。
- 复杂 per-node-type 分析仪表盘（成功率 / token 画像留后续 change）。

## Decisions

### D1：node_type 存为 `Option<NodeType>`，DB 列存 serde 字符串

- DB 列 `node_type TEXT`，存 `NodeType` 的 serde 字符串（`"Explore"` / `"Plan"` / …）。Rust 侧字段 `Option<NodeType>`，`#[serde(default)]` 兼容 legacy。
- 读取时反序列化；**未知字符串降级为 `None`，不 panic**（前向兼容未来新 `NodeType` 变体或脏数据）。

### D2：幂等 ALTER，照搬 `failure_diagnostics` 迁移模式

复用 `src/transcript/store.rs:134` 的 `column_exists` + `ALTER TABLE ADD COLUMN` 守卫，保证重复打开安全、旧库自动升级。

### D3：透传管道——复用 dispatch 已有的 node_type

`coordinator.rs:507` 已 `node_type: nt.clone()`。在 transcript 构造点（`save_minimal_transcript` `task.rs:337` + `store.save` `task.rs:1009`）把 `node_type` 写入字段。**先 grep 确认 `node_type` 当前到达管道的最深位置**，从那里接到 transcript 构造，避免重复传参。

### D4：list/trace 最小改动展示

- `list`：`SubagentTranscriptHeader`（`mod.rs:76`）加 `node_type`，渲染加一列；legacy（`None`）显示占位 `-`。
- `trace`：渲染 header 时多展示一行 `node_type`。
- `health`：本期默认**不**按 node_type 分组（Non-Goal），留 Open Question。

## Risks / Trade-offs

- **[Risk] 透传管道可能需穿过多层**（dispatch config → spawn → transcript 构造）→ Mitigation：先定位 `node_type` 已到达的最深位置，从该处接续，避免重复传递；若管道过深，考虑在 transcript 构造点就近从 coordinator 上下文取。
- **[Risk] legacy DB 含未知 `node_type` 字符串** → Mitigation：未知值降级 `None`，不 panic（D1）。
- **[Trade-off] 字符串 vs 整数编码** → 选字符串：可读、复用 `NodeType` serde、未来加变体无需迁移编码表。

## Migration Plan

- schema 幂等 `ALTER`，随首次打开自动迁移，无手工步骤。
- 新 run 立即写 `node_type`；legacy row 保持 `None`，`list` 以占位符显示。

## Open Questions

- `health` 是否按 node_type 分组（本期默认不做，等真实数据积累后再评估）。
- `node_type` 列在 `list` 中的列宽 / 位置（design 阶段定）。
- `NodeType` 的依赖方向：`transcript` 模块若要持有 `NodeType`，确认不引入循环依赖（`org_graph` 已是叶子纯数据模块，预期安全）。
