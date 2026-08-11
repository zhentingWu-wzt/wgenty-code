# Comet Design Handoff

- Change: org-graph-contract-viewer
- Phase: design
- Mode: compact
- Context hash: 65696cb1940b08c3649d8bd1ddb37f9defe44e61b868c798c7796cc781b4e689

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/org-graph-contract-viewer/proposal.md

- Source: openspec/changes/org-graph-contract-viewer/proposal.md
- Lines: 1-32
- SHA256: 44206a21e156973db8bbcad4111a4df348adb0b7ada415ea3f8bef377e3e7898

```md
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

```

## openspec/changes/org-graph-contract-viewer/design.md

- Source: openspec/changes/org-graph-contract-viewer/design.md
- Lines: 1-59
- SHA256: f08c9d44c80108e28fd5b345d906d92b296f5fea69663d6620ee9f46ac7b5c48

```md
## Context

契约层（`org-graph-node-contract`，已归档）把 agent 派发的隐式硬编码提取成显式、数据驱动的五维 `NodeContract`，集中存于 `src/org_graph/registry.rs` 的 `NodeRegistry`。当前 `NodeRegistry` 只暴露 `get(&NodeType) -> Option<&NodeContract>`，无遍历入口；契约只能读源码查看。本 change 为其加一层纯只读渲染，兑现"显式化"承诺，是 Org-Graph 可观测性路线的第一步（静态视图）。

## Goals / Non-Goals

**Goals:**

- 把内置 `NodeRegistry`（5 个契约 × 5 维）渲染成 `table` / `dot` / `mermaid` / `json` 四种可读视图。
- 纯函数渲染、零运行时副作用、零回归。
- 为 Org-Graph 后续可观测性（运行时分发遥测，见姊妹 change）与多层演进（关系层/编排层）打基础。

**Non-Goals:**

- 运行时分发数据 / `transcript` 持久化（姊妹 change `org-graph-dispatch-telemetry` 负责）。
- budget 激活、IO shape 强制、关系层。
- ContractViolation 计数 / 实时 daemon 视图。
- 自定义（非内置）契约的渲染——本期只渲染 `NodeRegistry::builtin()`。

## Decisions

### D1：渲染模块为纯函数，置于 `src/org_graph/render.rs`

输入 `&NodeRegistry` + `Format` 枚举，输出 `String`。无 I/O、无 async、无状态。便于单测、与现有 `org_graph` 模块"纯数据 + 纯函数校验"的风格一致。

### D2：为 `NodeRegistry` 增加只读遍历入口

当前只有 `get(&NodeType)`。新增 `iter()`（或 `all()`）按稳定顺序返回全部契约的迭代器，供渲染层遍历。**纯新增只读 API，不改已有签名，零回归。** 顺序以 `NodeType` 的稳定枚举顺序为准（Explore, Plan, GeneralPurpose, Verification, WgentyCodeGuide）。

### D3：四种格式各自独立纯函数

`render_table` / `render_dot` / `render_mermaid` / `render_json`。`json` 直接复用 `NodeContract` 已派生的 `Serialize`（`serde_json::to_string_pretty`）；`table` / `dot` / `mermaid` 手写格式化，**不引入新外部 crate**。

### D4：命令挂载点 = 新顶层 `org-graph` 命令组

命令形如 `wgenty-code org-graph contracts [--format table|dot|mermaid|json]`。

- **选择理由**：Org-Graph 是规划中的多层子系统（契约层已落地；关系层/编排层/可观测性后续演进）。顶层 `org-graph` 组有扩展空间（未来可挂 `org-graph dispatch`、`org-graph health` 等）；塞进现有 `subagent` 子命令组会让该组承担两个正交关注点（subagent 运行审计 vs 组织图结构）。
- **备选**：`wgenty-code subagent contracts`——复用现有组、CLI 表面更小，但概念耦合且扩展受限。
- **结论**：选顶层 `org-graph` 组。若 design 阶段评审认为 CLI 表面增长不可接受，可回退到 subagent 子命令。

### D5：`Format` 由 clap 派生为 `value_enum`

`#[derive(clap::ValueEnum)]` 的 `Format { Table, Dot, Mermaid, Json }`，默认 `Table`。与现有 `SubagentCommands` 的 `TraceFormat` / `HealthPeriodArg` 用法一致。

## Risks / Trade-offs

- **[Risk] `explore_readonly` 配置读取时机** → 渲染需基于与 `NodeRegistry::builtin()` 相同的 `SubagentLimits` 构造。Mitigation：命令路径复用现有配置加载逻辑构造 `SubagentLimits`，不在渲染层重新读取/猜测配置。
- **[Risk] DOT/Mermaid 语法合法性** → 手写格式化易出隐蔽语法错误，外部渲染器静默失败。Mitigation：对 `dot` 输出加一个"可被 `dot` 解析"的冒烟测试（若 CI 无 graphviz 则降级为结构断言）；mermaid 加图类型声明与节点闭合的结构断言。
- **[Trade-off] 顶层命令组增加 CLI 表面** → 换取 Org-Graph 子系统的长期扩展空间（见 D4）。

## Migration Plan

纯新增命令，无数据/配置迁移。旧版本用户不受影响；新命令首次可用即生效。

## Open Questions

- DOT 冒烟测试是否依赖 CI 环境 `graphviz`：若不可得，降级为结构断言（design 阶段定）。
- 表格列宽 / 长字段（如 system_prompt）截断策略：默认不显示 system_prompt 全文（过长），仅显示摘要或省略（design 阶段定）。

```

## openspec/changes/org-graph-contract-viewer/tasks.md

- Source: openspec/changes/org-graph-contract-viewer/tasks.md
- Lines: 1-33
- SHA256: c02f76ff84b40b27bb8e87d9371054cc26616a0f33ce3f6e797d3db9c445f004

```md
# Tasks

## 1. NodeRegistry 遍历入口

- [ ] 1.1 为 `NodeRegistry` 新增 `iter()`（或 `all()`）只读遍历入口，按 `NodeType` 枚举稳定顺序（Explore, Plan, GeneralPurpose, Verification, WgentyCodeGuide）返回全部契约
- [ ] 1.2 为遍历入口加单测：五个内置契约全部出现、顺序稳定、与逐个 `get()` 结果一致

## 2. 渲染模块（`src/org_graph/render.rs`）

- [ ] 2.1 定义 `Format` 枚举（`Table` / `Dot` / `Mermaid` / `Json`），派生 `clap::ValueEnum`，默认 `Table`
- [ ] 2.2 实现 `render_json`：复用 `NodeContract` 的 `Serialize`，输出 `NodeContract` JSON 数组
- [ ] 2.3 实现 `render_table`：终端表格，覆盖五维字段（`system_prompt` 过长，默认截断/省略）
- [ ] 2.4 实现 `render_dot`：合法 Graphviz DOT，每个契约渲染为节点
- [ ] 2.5 实现 `render_mermaid`：合法 mermaid 图定义，每个契约渲染为节点
- [ ] 2.6 实现统一入口 `render(registry, format) -> String`，按 `Format` 分派到上述函数

## 3. 渲染输出单测

- [ ] 3.1 `json` 输出可被 `NodeContract` serde 反序列化，且与 `NodeRegistry::builtin()` 逐字段相等
- [ ] 3.2 `explore_readonly=true` / `false` 时，Explore 与 Plan 的 `can_mutate_fs` 在所有四种格式中如实反映
- [ ] 3.3 `dot` 输出以 `digraph` 声明开头、节点闭合（结构断言；CI 有 graphviz 则加 `dot` 解析冒烟测试）
- [ ] 3.4 `mermaid` 输出以合法图类型声明开头，且五个契约均成为图中节点

## 4. CLI 命令接线

- [ ] 4.1 在 `src/cli/mod.rs` 新增顶层 `OrgGraph` 命令组与 `Contracts` 子命令，带 `--format` value_enum（默认 `table`）
- [ ] 4.2 命令处理逻辑：加载 `SubagentLimits` 配置 → 构造 `NodeRegistry::builtin()` → `render()` → 打印到 stdout
- [ ] 4.3 在 `main.rs` 命令分派处接线 `Commands::OrgGraph { action: OrgGraphCommands::Contracts { format } }`

## 5. 集成验证

- [ ] 5.1 `cargo build` 通过；`cargo test` 全绿（新增测试通过 + 已有测试零回归）
- [ ] 5.2 手动验证四种格式输出（`table` / `dot` / `mermaid` / `json`）符合预期，`--format` 缺省为 `table`

```

## openspec/changes/org-graph-contract-viewer/specs/org-graph-contract-viewer/spec.md

- Source: openspec/changes/org-graph-contract-viewer/specs/org-graph-contract-viewer/spec.md
- Lines: 1-60
- SHA256: 42af5096d05efc538225c5ed0af6f78abe503b17b6db6730d2bb0f63b13fd13e

```md
## ADDED Requirements

### Requirement: 默认表格视图渲染全部内置契约

系统 SHALL 提供一个 CLI 命令，默认以终端表格形式列出所有内置 `NodeContract` 的五维约束，至少包含：node type / can_spawn / can_mutate_fs / can_exec / input→output IO shape / budget / allowed_tools。

#### Scenario: 默认调用列出五个内置契约

- **WHEN** 用户运行契约渲染命令且不带 `--format`
- **THEN** 输出包含 Explore / Plan / GeneralPurpose / Verification / WgentyCodeGuide 五个节点类型
- **AND** 每个节点显示 can_spawn / can_mutate_fs / can_exec 三个权限位

#### Scenario: 表格涵盖五维约束

- **WHEN** 用户运行契约渲染命令（默认格式）
- **THEN** 输出为每个节点呈现 capability（allowed_tools）、permission、budget、IO shape、identity 五个维度的可读字段

### Requirement: Graphviz DOT 格式导出

系统 SHALL 支持 `--format dot`，输出合法的 Graphviz DOT 文本，每个内置契约渲染为一个节点，可被 `dot -Tsvg` 等工具渲染。

#### Scenario: dot 输出可被 Graphviz 解析

- **WHEN** 用户运行命令带 `--format dot`
- **THEN** 输出以 `digraph` 声明开头的合法 DOT 文本
- **AND** 输出可被 `dot` 命令成功解析（无语法错误）

### Requirement: Mermaid 格式导出

系统 SHALL 支持 `--format mermaid`，输出可嵌入 Markdown 并被 GitHub / mermaid 渲染的图定义。

#### Scenario: mermaid 输出合法图定义

- **WHEN** 用户运行命令带 `--format mermaid`
- **THEN** 输出以合法 mermaid 图类型声明开头（`graph` 或 `flowchart`）
- **AND** 每个内置契约渲染为图中的一个节点

### Requirement: JSON 格式导出

系统 SHALL 支持 `--format json`，输出可被 `NodeContract` 的 `Deserialize` 反序列化的 JSON 数组。

#### Scenario: json 输出可 serde 反序列化且逐字段相等

- **WHEN** 用户运行命令带 `--format json`
- **THEN** 输出为 JSON 数组，每个元素是一个完整 `NodeContract`
- **AND** 反序列化结果与 `NodeRegistry::builtin()` 中的契约逐字段相等

### Requirement: 权限维度反映 explore_readonly 配置

系统 SHALL 在渲染时如实反映 `explore_readonly` 配置驱动的 `can_mutate_fs` 维度：`explore_readonly=true` 时 Explore 与 Plan 的 `can_mutate_fs=false`；反之 `true`。

#### Scenario: explore_readonly=true 时 explore/plan 为只读

- **WHEN** `explore_readonly` 配置为 true 时渲染契约
- **THEN** Explore 与 Plan 节点的 can_mutate_fs 显示为 false

#### Scenario: explore_readonly=false 时 explore/plan 可写

- **WHEN** `explore_readonly` 配置为 false 时渲染契约
- **THEN** Explore 与 Plan 节点的 can_mutate_fs 显示为 true

```
