## Context

项目的 agent 派发今天有三套彼此脱节的结构，没有一套是 Org-Graph：

1. **`AgentDefinition`/`AgentsService`（`teams/subagent.rs`）**--看似 agent 注册表，实为死注册表。`execute_agent` 是单轮 `chat()`，真实运行时不走它（仅 CLI `run_agent` + stress_tests 调用）。
2. **`task.rs execute_with_context`（`tools/meta/task.rs`）**--真实派发路径。从模型 JSON 读 `subagent_type` 字符串，用硬编码 `match` 分支决定 system_prompt / allowed_tools / budget。
3. **`AgentCoordinator`（`agent/coordinator.rs`）**--真实执行引擎，管 depth+并发+生命周期。但 `SpawnChildRequest` 只有 `{ label }`，**对节点类型/能力/权限/budget 完全无知**。

节点类型的"能干什么/不能干什么/预算多少"散在三处硬编码：`match _subagent_type` 分支、`filter_allowed_tools()`（`is_leaf`/`explore_readonly`/`MUTATING_FS_TOOLS`）、全局 `SubagentLimits`（`max_depth`/`max_concurrent`/`token_budget_k`/`max_rounds`）。coordinator 派发时无法做任何强制。

本 change 是系统级 graph 工程的起点 A（Org-Graph）：把隐式组织结构提取成显式、数据驱动、可被 coordinator 强制校验的 **NodeContract**。

## Goals / Non-Goals

**Goals:**
- 新建 `NodeContract` 类型，声明五维：能力、权限边界、资源预算、IO schema（声明不校验）、身份/谱系（复用 `AgentExecutionContext`）。
- 新建 `src/org_graph/` 模块 + `NodeRegistry`（5 个内置节点契约：explore / plan / general-purpose / verification / wgenty-code-guide）。
- `SpawnChildRequest` 携带 `node_type`，`AgentCoordinator::reserve_child` 派发时查 registry 强制校验三维（能力 / 权限边界 / 资源预算），违反则 `CoordinatorError` 拒绝。
- `task.rs` 从硬编码 `match` 改为读契约，`filter_allowed_tools` 从硬编码逻辑改为读 `contract.permissions` + `contract.capabilities`。
- `SubagentLimits` 作为 budget 全局默认，NodeContract 的 budget 字段 `Option` 覆盖它，保留现有配置语义。

**Non-Goals:**
- 不做 IO schema 强校验--契约里声明 `input_type`/`output_type`，但运行时不校验，模型输出仍按现状处理。IO 强校验 + 重试/修复子系统留后续 change。
- 不做 Work-Graph 动态子图组装（起点 B）、外部锚点复核（起点 C）、跨 agent 路由引擎（起点 D）。
- 不删除 `AgentDefinition`/`AgentsService`--死注册表保留，CLI runner 继续用，新派发路径只读 NodeContract。
- 不改 `runtime/loop_.rs`（agent loop 内部）--契约只在派发边界（spawn 前）生效。
- 不做 NodeContract 的运行时动态加载/热更新--内置契约编译期固定。

## Decisions

### D1: NodeContract 是数据驱动的强类型 struct，不是 trait object
契约用 `serde` struct 表达，编译期注册进 `NodeRegistry`。**不用 trait**（如 `trait NodeBehavior`），因为契约要声明的是静态约束（能力集合、权限位、预算上限），不是动态行为；用 struct 可序列化、可序列化往返测试、可在 coordinator 处纯函数校验。替代方案（trait object）会把校验逻辑分散到各 impl，且无法序列化。

### D2: 三维强制放在 coordinator 的 reserve_child，不在 task.rs
`task.rs` 负责读契约组装派发请求，coordinator 在 `reserve_child` 做强制校验。**校验下沉到 coordinator** 因为它是唯一派发入口（所有 spawn 必经），且已持有 depth/并发治理；在它之上加契约校验是最窄拦截点。替代方案（在 task.rs 校验）会绕过 RLM delegate 等其它派发路径，留漏洞。

### D3: SpawnChildRequest 携带 node_type 而非整个 NodeContract
`SpawnChildRequest` 扩展为携带 `node_type: NodeType`（枚举），coordinator 内部用 `NodeRegistry::get(&node_type)` 查契约。**不携带整个 contract**，因为契约是注册表事实源，携带 contract 副本会让模型输入有机会篡改契约（违反"trusted context，never from model JSON"原则）。node_type 是轻量枚举，可信。

### D4: budget 用 Option 覆盖全局 SubagentLimits，不替换它
NodeContract 的 `budget: ResourceBudget` 字段全为 `Option`（`max_depth: Option<usize>` 等），`None` 时 coordinator 回退到现有 `SubagentLimits` 全局值。**保留 SubagentLimits** 因为它已绑定用户配置体系（settings YAML），删它会破坏现有配置；NodeContract 的 budget 是 per-node-type 覆盖层。替代方案（contract 完全接管 budget）会丢失用户运行时配置能力。

### D5: IO schema 声明用 Rust 强类型 + serde，不强校验
契约声明 `input_type` / `output_type` 为 `std::any::TypeId` 或类型名字符串（设计阶段定），但运行时不校验模型输出。**声明不校验** 因为 agent 输入输出本质是自由文本，强校验需重试/修复子系统（已确认留后续 change）。本 change 只把"这个节点类型声明什么 IO 类型"显式化，为后续强制铺路。

### D6: filter_allowed_tools 改为读契约，但保留为纯函数
`filter_allowed_tools` 签名从 `(names, subagent_type, depth, max_depth, explore_readonly)` 改为 `(names, &NodeContract)`，内部读 `contract.permissions` + `contract.capabilities`。**保留为纯函数** 因为它有单测覆盖且无副作用，改签名不改形态最稳。`explore_readonly` 配置项作为 budget/permission 的全局默认源传入，contract 的 permission 字段 `Option` 覆盖它。

### D7: 5 个内置节点契约照搬现有硬编码语义，不借机调整
explore/plan = leaf（can_spawn=false, can_mutate_fs 取决于 explore_readonly），general-purpose = 可 spawn，verification/guide = 现有工具集。**不借机调 budget 默认值**（如给 explore 更小 token budget）以避免行为变更，保持"合法派发行为不变"的无回归承诺。调整留后续 change。

## Risks / Trade-offs

- **[SpawnChildRequest 扩展影响所有调用点]** -> `reserve_child` 有多调用点（task.rs / fallback.rs / rlm/pipeline.rs / daemon/handlers.rs）。Mitigation：`node_type` 设默认值（`GeneralPurpose`），未显式传的调用点行为不变；逐个调用点补 node_type 并加测试。
- **[coordinator 强制校验可能破坏现有 fallback 语义]** -> 现在 depth-limit 触发 structural fallback（task.rs self-execution）。契约拒绝不能误触发 fallback。Mitigation：契约违反用独立 `CoordinatorError` 变体（如 `ContractViolation`），与 `DepthLimitReached` 区分；fallback 只认 structural 失败，不认契约违反。
- **[filter_allowed_tools 签名变更影响测试]** -> 现有 `filter_allowed_tools` 单测。Mitigation：改签名时同步改测试，新签名读 contract 更易构造测试用例。
- **[AgentDefinition 与 NodeContract 两层并存]** -> 死注册表和新契约并存，可能混淆。Mitigation：NodeContract 文档明确标注"真实派发路径的事实源"，AgentDefinition 文档标注"遗留 CLI runner 专用"；不删 AgentDefinition 避免破坏 CLI。
- **[IO schema 声明但不校验，可能被误用为已强制]** -> Mitigation：契约字段文档明确"声明态，运行时不校验"，验收场景只测三维强制不测 IO。

## Migration Plan

1. 新建 `src/org_graph/` 模块 + NodeContract/NodeRegistry + 5 内置契约，先不接任何调用点（纯新增）。
2. 扩展 `SpawnChildRequest` 加 `node_type`（带默认值），coordinator `reserve_child` 加三维校验（此时校验逻辑可独立测试）。
3. 改 `task.rs` 读契约 + `filter_allowed_tools` 读契约（逐调用点补 node_type）。
4. 衔接 `SubagentLimits` 作为 budget 默认源。
5. 全量回归测试，确认合法派发行为不变。

回滚：各步独立可回滚；NodeContract 模块删除不影响现有路径（纯新增起步）。

## Open Questions

1. **delegate（RLM）路径的 node_type 归属**：RLM 的 `SubTask` 没有 node_type 概念，它 spawn 的子 agent 走哪个契约？倾向统一按 `general-purpose`，需在 design 阶段确认 RLM 子任务是否需独立契约类型。
2. **coordinator 契约违反的回退**：契约违反硬拒绝报错 vs 降级路径？倾向硬拒绝（契约是法律），但需确认不破坏 RLM replan 等依赖派发成功的路径。
3. **IO schema 的类型表达**：`input_type`/`output_type` 用 `TypeId`、类型名字符串、还是轻量 schema struct？design 阶段定。
