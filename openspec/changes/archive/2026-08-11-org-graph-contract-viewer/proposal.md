## Why

契约层（`org-graph-node-contract`，已归档）把 agent 派发的隐式硬编码（`task.rs` match 分支 / `filter_allowed_tools` / 全局 `SubagentLimits`）提取成显式、数据驱动的五维契约（capability / permission / budget / IO shape / identity），集中存于 `NodeRegistry`。但这些契约目前**只能读源码查看**——既无法在 CLI 里快速审计"当前每个节点类型能做什么"，也无法导出成图给文档或团队分享。"显式化"承诺要求契约不仅存在于代码，还要能被低成本观察。本 change 是 Org-Graph 可观测性路线的第一步（静态视图），低风险、纯只读。

## What Changes

- 新增 CLI 渲染能力：把内置 `NodeRegistry`（5 个 `NodeContract` × 5 维约束）渲染成人类可读视图。
- 支持四种输出格式：
  - `table`（终端表格，默认）
  - `dot`（Graphviz DOT，org-chart 风格，可被 `dot -Tsvg` 渲染）
  - `mermaid`（可嵌入 Markdown / GitHub 渲染）
  - `json`（机器可读，复用 `NodeContract` 已派生的 `Serialize`）
- 新增纯函数渲染模块：只读 `NodeRegistry` 纯数据，零运行时副作用、零 I/O。
- 命令挂载点（新顶层 `org-graph` 命令组 vs 现有 `subagent` 子命令组）与默认格式留待 design 阶段定。

## Capabilities

### New Capabilities

- `org-graph-contract-viewer`: 把 `NodeRegistry` 中的内置节点契约（五维约束）渲染成 `table` / `dot` / `mermaid` / `json` 四种可读视图，供 CLI 审计与文档导出。

### Modified Capabilities

<!-- 无。契约本身（org-graph-node-contract）不变；本 change 纯粹是只读渲染层，不改变任何 spec 级行为。 -->

## Impact

- **新增代码**：`src/org_graph/` 下新增渲染子模块；`src/cli/` 接入新命令（挂载点待定）。
- **数据来源**：只读 `NodeRegistry::builtin()`，不修改契约数据。
- **不触碰**：运行时分发路径、`transcript` store、`coordinator`、契约强制逻辑。
- **回归风险**：极低——纯新增只读路径，不改已有行为。
- **依赖**：`NodeContract` 已派生 `Serialize`（`json` 格式直接可用）；`table` / `dot` / `mermaid` 需新增格式化代码（无新外部 crate 依赖）。
