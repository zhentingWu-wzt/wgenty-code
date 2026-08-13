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
