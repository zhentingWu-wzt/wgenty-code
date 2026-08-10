# Comet Design Handoff

- Change: org-graph-node-contract
- Phase: design
- Mode: compact
- Context hash: ecb52febd7d4cd42df1313b82bc594120869e546f24d72070c1b0502d1f44ee4

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/org-graph-node-contract/proposal.md

- Source: openspec/changes/org-graph-node-contract/proposal.md
- Lines: 1-32
- SHA256: e32ba7edd4a3a893c49c7fab75c66a7ca7dce05cbca093cd5e496ec7164c61ff

```md
## Why

项目的 agent 组织结构今天是一组**散落在派发代码里的隐式约定**：每个 agent 节点类型（explore / plan / general-purpose / verification / guide）"能调用哪些工具、能否 spawn 子节点、能否改文件、预算多少 depth/并发/token"，分别硬编码在 `task.rs` 的 `match _subagent_type` 分支、`filter_allowed_tools()` 的 `is_leaf`/`explore_readonly` 逻辑、以及全局 `SubagentLimits` 配置里。与此同时 `AgentDefinition`/`AgentsService` 这个看似"agent 注册表"的结构实际上是个**死注册表**--它的 `execute_agent` 只是单轮 `chat()`，真实运行时根本不走它（只有 CLI `run_agent` 和 stress_tests 调用）。

这造成三个问题：①没有单一事实源能回答"explore 这个节点类型是什么"；②新增一个节点类型要改三处代码且彼此无法保证一致；③`AgentCoordinator`（真实执行引擎）的 `SpawnChildRequest` 只有 `{ label }`，**派发时对节点的能力/权限/budget 完全无知**，无法做任何强制。这是引入系统级 graph 工程（Org-Graph）的起点：把隐式组织结构提取成显式、数据驱动、可被 coordinator 强制校验的 **NodeContract**。

## What Changes

- **新增 NodeContract 类型**：每个节点类型一张契约卡，声明五维--能力（`capabilities`）、权限边界（`permissions`）、资源预算（`budget`）、IO schema（`input_type`/`output_type`，**声明不校验**）、身份/谱系（复用现有 `AgentExecutionContext`）。
- **新增 `src/org_graph/` 模块**：`NodeContract` 定义 + `NodeRegistry`（5 个内置节点契约）+ 三维强制校验逻辑。
- **扩展 `SpawnChildRequest`**：从 `{ label }` 携带 `node_type`，使 coordinator 在派发时能查契约。
- **`AgentCoordinator::reserve_child` 强制校验三维**：派发时查 registry，校验能力（requested tools ⊆ capabilities）、权限边界（can_spawn / can_mutate_fs / can_exec）、资源预算（depth leaf 禁 spawn + per-node-type 并发/token 覆盖），违反则 `CoordinatorError` 拒绝。
- **`task.rs execute_with_context` 改为读契约**：从硬编码 `match _subagent_type` 分支改为 `registry.get(node_type)` 读契约，system_prompt / allowed_tools / budget 全来自 NodeContract，消除硬编码 match。
- **`filter_allowed_tools` 改为读契约**：从 `is_leaf`/`explore_readonly` 硬编码逻辑改为读 `contract.permissions` + `contract.capabilities`。
- **`SubagentLimits` 作为 budget 全局默认**：NodeContract 的 `budget` 字段为 `Option`，`None` 时回退到现有全局 `SubagentLimits`，保留现有配置语义。
- **`AgentDefinition`/`AgentsService` 并存不删**：死注册表保留，CLI `run_agent` + stress_tests 继续用旧的；新派发路径只读 NodeContract。迁移/删除留后续 change。

## Capabilities

### New Capabilities
- `org-graph-node-contract`: 组织结构图节点契约（NodeContract）建模与 coordinator 强制校验--声明每个节点类型的能力、权限边界、资源预算，coordinator 在派发时强制三维校验。

### Modified Capabilities
- `subagent-tool-permissions`: "Role-enforced tool visibility for explore and plan" 需求从硬编码 `is_leaf`/`explore_readonly` 逻辑改为由 NodeContract 的 `permissions` + `capabilities` 驱动。

## Impact

- **新增代码**：`src/org_graph/`（NodeContract / NodeRegistry / 校验逻辑）、模块注册。
- **修改代码**：`src/agent/coordinator.rs`（`SpawnChildRequest` + `reserve_child` 校验）、`src/tools/meta/task.rs`（`execute_with_context` 读契约 + `filter_allowed_tools` 读契约）、`src/config/agent.rs`（`SubagentLimits` 作为 budget 默认源的衔接）。
- **不修改**：`src/agent/runtime/loop_.rs`（agent loop 内部不动，契约只在派发边界生效）、`src/teams/subagent.rs`（AgentDefinition/AgentsService 并存不删）。
- **行为变更**：派发时违反契约的请求从"静默按硬编码逻辑处理"变为"coordinator 显式拒绝并返回 `CoordinatorError`"。合法派发行为不变（无回归）。
- **依赖**：无新外部依赖；serde 已是硬依赖。

```

## openspec/changes/org-graph-node-contract/design.md

- Source: openspec/changes/org-graph-node-contract/design.md
- Lines: 1-74
- SHA256: ab9e9c5ab90050f62cfccbefc5d67a710d15bab81eba834bb9840595d5871fdd

```md
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

```

## openspec/changes/org-graph-node-contract/tasks.md

- Source: openspec/changes/org-graph-node-contract/tasks.md
- Lines: 1-48
- SHA256: b3b30ac5221cab5a4100e738d0c9fdbfc26d4aeb0854ff68828c4d606f09b3f7

```md
## 1. NodeContract 类型基础

- [ ] 1.1 新建 `src/org_graph/mod.rs` 模块，定义 `NodeType` 枚举（Explore / Plan / GeneralPurpose / Verification / WgentyCodeGuide）、`Capability`、`PermissionBoundary`、`ResourceBudget`、`IoSchema` 类型；定义 `NodeContract` struct 含五维字段，全部派生 `Serialize/Deserialize/Debug/Clone`
- [ ] 1.2 在 `src/main.rs`（或 lib root）注册 `org_graph` 模块
- [ ] 1.3 添加 `NodeContract` 序列化往返测试（五维字段齐全，序列化->反序列化断言相等）

## 2. NodeRegistry 与内置契约

- [ ] 2.1 在 `src/org_graph/registry.rs` 实现 `NodeRegistry`（持有 5 个内置 `NodeContract`，`get(&NodeType) -> Option<&NodeContract>`）
- [ ] 2.2 填充 5 个内置节点契约，capabilities/permissions/budget 照搬现有硬编码语义（explore/plan=leaf can_spawn=false；general-purpose 可 spawn；explore_readonly 作 can_mutate_fs 默认源；IO schema 声明态留占位）
- [ ] 2.3 添加测试：5 个内置契约均可在 registry 查到；查询不存在类型返回 None；契约内容与现有硬编码语义对齐

## 3. SpawnChildRequest 扩展

- [ ] 3.1 给 `SpawnChildRequest`（`agent/coordinator.rs`）加 `node_type: NodeType` 字段，带默认值 `GeneralPurpose`；`SpawnChildRequest::new` 签名向后兼容
- [ ] 3.2 给 `AgentCoordinator` 加 `NodeRegistry` 引用（构造时注入或内部默认持有）
- [ ] 3.3 测试：显式传 node_type 与默认值两种构造路径

## 4. coordinator 三维强制校验

- [ ] 4.1 新增 `CoordinatorError::ContractViolation` 变体（携带维度+原因），与 `DepthLimitReached` 等 structural 错误区分
- [ ] 4.2 在 `reserve_child` 加三维校验：能力（requested tools ⊆ capabilities）、权限边界（can_spawn/can_mutate_fs/can_exec）、资源预算（leaf 禁 spawn + per-node-type depth/concurrent/token 覆盖）；违反返回 `ContractViolation`
- [ ] 4.3 衔接 `SubagentLimits` 作为 budget 全局默认：contract.budget 字段为 None 时回退全局值，Some 时覆盖
- [ ] 4.4 测试：能力越纲拒绝、权限边界拒绝、budget 拒绝（leaf 禁 spawn）、合法派发放行、budget None 回退全局、budget Some 覆盖全局

## 5. task.rs 读契约

- [ ] 5.1 改 `execute_with_context`：从硬编码 `match _subagent_type` 分支改为 `NodeRegistry::get(&node_type)` 读 `NodeContract`；system_prompt / allowed_tools / budget 全来自契约
- [ ] 5.2 改 `filter_allowed_tools` 签名从 `(names, subagent_type, depth, max_depth, explore_readonly)` 改为读 `&NodeContract`；内部读 `permissions` + `capabilities`；`explore_readonly` 作 `can_mutate_fs` 全局默认源
- [ ] 5.3 模型 JSON 的 `subagent_type` 字符串经派发层映射为可信 `NodeType` 枚举（不直接注入 SpawnChildRequest）
- [ ] 5.4 测试：explore/plan/general-purpose 三种节点派发的 system_prompt + allowed_tools + budget 与变更前硬编码路径完全一致（无回归）

## 6. 其余调用点补 node_type

- [ ] 6.1 排查并更新 `reserve_child` / `reserve_child_in_group` 所有调用点（fallback.rs / rlm/pipeline.rs / daemon/handlers.rs / run_script.rs），补 node_type 或依赖默认值
- [ ] 6.2 确认 delegate（RLM）路径的 node_type 归属（Open Question 1：倾向统一 general-purpose），落实并在 design 阶段记录决策
- [ ] 6.3 确认契约违反回退策略（Open Question 2：倾向硬拒绝不触发 fallback），确认 fallback.rs 只认 structural 失败不认 ContractViolation

## 7. AgentDefinition 并存验证

- [ ] 7.1 确认 `AgentDefinition`/`AgentsService` 未被新派发路径引用（新路径只读 NodeContract）
- [ ] 7.2 测试：CLI `run_agent` 仍走 AgentsService 旧路径，行为不变；stress_tests 仍工作

## 8. 验证与收尾

- [ ] 8.1 `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] 8.2 `cargo test -p wgenty-code` 全量通过（含 org_graph 模块测试、coordinator 校验测试、task.rs 无回归测试、现有 subagent 测试无回归）
- [ ] 8.3 手动：真实 `task`/`delegate` 派发 explore 与 general-purpose 节点，确认 system_prompt/工具集/budget 来自契约且行为与变更前一致

```

## openspec/changes/org-graph-node-contract/specs/org-graph-node-contract/spec.md

- Source: openspec/changes/org-graph-node-contract/specs/org-graph-node-contract/spec.md
- Lines: 1-113
- SHA256: ac0e741b0813bbc6bfa61296f3881bbd9515ab18c321eec256fc82bdd2cec276

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: NodeContract 建模组织结构图节点契约
系统 SHALL 提供一个 `NodeContract` 类型，作为每个 agent 节点类型（explore / plan / general-purpose / verification / wgenty-code-guide）的显式契约声明。`NodeContract` SHALL 声明五个维度：能力（`capabilities`）、权限边界（`permissions`）、资源预算（`budget`）、IO schema（`input_type` / `output_type`，声明态）、身份/谱系（复用现有 `AgentExecutionContext`）。`NodeContract` SHALL 派生 `Serialize`/`Deserialize`，可序列化往返。`NodeContract` 是组织结构图的事实源，SHALL 替代 `task.rs` 中的硬编码 `match` 分支作为派发依据。

#### Scenario: NodeContract 序列化往返无损
- **WHEN** 一个包含完整五维声明的 `NodeContract` 被序列化为 JSON 再反序列化
- **THEN** 反序列化后的 `NodeContract` SHALL 与原对象按字段相等

#### Scenario: 内置节点契约覆盖五种类型
- **WHEN** `NodeRegistry` 初始化完成
- **THEN** SHALL 包含 explore、plan、general-purpose、verification、wgenty-code-guide 五个节点类型的 `NodeContract`
- **AND** 每个契约的 `capabilities`/`permissions`/`budget` SHALL 与变更前硬编码语义一致（无行为变更）

### Requirement: NodeRegistry 提供节点契约查询
系统 SHALL 提供 `NodeRegistry`，持有所有内置 `NodeContract` 并提供按 `NodeType` 查询的能力。`NodeRegistry::get(&NodeType) -> Option<&NodeContract>` SHALL 返回对应节点类型的契约。查询不存在的节点类型 SHALL 返回 `None`。

#### Scenario: 查询存在的节点类型
- **WHEN** 用 `NodeType::Explore` 查询 `NodeRegistry`
- **THEN** SHALL 返回 `Some(&NodeContract)`，其内容为 explore 节点契约

#### Scenario: 查询不存在的节点类型
- **WHEN** 用一个未注册的 `NodeType` 查询 `NodeRegistry`
- **THEN** SHALL 返回 `None`

### Requirement: SpawnChildRequest 携带 node_type
`SpawnChildRequest` SHALL 新增 `node_type: NodeType` 字段，使 `AgentCoordinator` 在派发时能查询节点契约。`node_type` SHALL 有默认值（`GeneralPurpose`），未显式传入的调用点行为不变。`node_type` 来源于可信派发层，SHALL NOT 来自模型 JSON 直接注入（模型输出的 `subagent_type` 字符串须经派发层映射为可信 `NodeType` 枚举）。

#### Scenario: 显式传入 node_type
- **WHEN** 派发层构造 `SpawnChildRequest` 时传入 `NodeType::Explore`
- **THEN** coordinator SHALL 能读到 `node_type = Explore` 并查询对应契约

#### Scenario: 未传入 node_type 使用默认值
- **WHEN** 派发层构造 `SpawnChildRequest` 时未显式传入 `node_type`
- **THEN** `node_type` SHALL 为 `GeneralPurpose`，派发行为与变更前一致

### Requirement: coordinator 在派发时强制校验三维契约
`AgentCoordinator::reserve_child` SHALL 在派发时查询 `NodeRegistry` 获取 `node_type` 对应的 `NodeContract`，并强制校验三维契约。**三维校验分两层**：coordinator 在 `reserve_child` 校验权限边界的 `can_spawn`（leaf 节点禁止 spawn）和资源预算的 `depth`（`contract.budget.max_depth` 覆盖全局）；能力（requested tools ⊆ `contract.capabilities`）和权限边界的 `can_mutate_fs` 在 `task.rs` 的 `filter_allowed_tools` 校验（工具集在派发层组装时计算）。两层校验 SHALL 读取同一份 `NodeContract`。任一维度违反 SHALL 返回 `CoordinatorError` 拒绝派发，SHALL NOT 静默放行。契约违反 SHALL 使用独立的 `CoordinatorError::ContractViolation` 变体（携带 `node_type`/`dimension`/`reason`），与 `DepthLimitReached` 等 structural 错误区分。`ContractViolation` SHALL NOT 触发 structural fallback（`fallback_eligible_from_coordinator_error` 的穷尽 match `_ => None` 自动覆盖此变体），SHALL 经 `map_coordinator_error` 映射为 `ToolError`（`code: "contract_violation"`）回给父 agent，由模型决定处理方式。

#### Scenario: leaf 节点禁止 spawn 被 coordinator 拒绝
- **WHEN** 一个 leaf 节点（`NodeType::Explore`，`permissions.can_spawn = false`）的派发请求试图 spawn 子节点
- **THEN** coordinator SHALL 在 `reserve_child` 返回 `CoordinatorError::ContractViolation`（dimension=Permission），拒绝派发

#### Scenario: 能力越纲被 task.rs 拒绝
- **WHEN** 一个 `NodeType::Explore` 的派发请求包含了 `task` 工具（explore 契约的 `capabilities.allowed_tools` 不含 spawn 工具）
- **THEN** `task.rs filter_allowed_tools` SHALL 依据 `contract.capabilities` 白名单剥离该工具
- **AND** 若剥离后仍违反契约 SHALL 经 coordinator 返回 `ContractViolation`，拒绝派发

#### Scenario: 权限边界 can_mutate_fs 被 task.rs 拒绝
- **WHEN** `explore_readonly` 生效时，一个 `NodeType::Explore` 的派发请求包含 `file_write`（explore 契约 `permissions.can_mutate_fs = false`）
- **THEN** `task.rs filter_allowed_tools` SHALL 依据 `contract.permissions.can_mutate_fs` 剥离该工具

#### Scenario: ContractViolation 不触发 fallback
- **WHEN** coordinator 返回 `CoordinatorError::ContractViolation`
- **THEN** `fallback_eligible_from_coordinator_error` SHALL 返回 `None`（不触发 structural fallback）
- **AND** SHALL 经 `map_coordinator_error` 映射为 `ToolError`（`code: "contract_violation"`）回给父 agent

#### Scenario: 合法派发不被拒绝
- **WHEN** 一个 `NodeType::Explore` 的派发请求只含 explore 契约 `capabilities` 内的工具、符合 `permissions`、在 `budget` 内
- **THEN** coordinator SHALL 放行派发，行为与变更前一致（无回归）

### Requirement: NodeContract budget 覆盖全局 SubagentLimits 默认
`NodeContract.budget` 的各字段 SHALL 为 `Option` 类型。`None` 时 coordinator SHALL 回退到现有全局 `SubagentLimits` 配置（`max_depth`/`max_concurrent`/`token_budget_k`/`max_rounds`）。`Some` 时 SHALL 用契约值覆盖全局默认。此设计 SHALL 保留现有用户配置（settings YAML）的 `SubagentLimits` 语义不变。

#### Scenario: budget 字段为 None 时回退全局默认
- **WHEN** 一个 `NodeContract.budget.max_depth` 为 `None`
- **THEN** coordinator SHALL 使用全局 `SubagentLimits.max_depth` 作为该节点的 depth 上限

#### Scenario: budget 字段为 Some 时覆盖全局默认
- **WHEN** 一个 `NodeContract.budget.token_budget_k` 为 `Some(32)`
- **THEN** coordinator SHALL 使用 32 作为该节点的 token 预算，忽略全局 `SubagentLimits.token_budget_k`

### Requirement: IO schema 声明态不强制校验
`NodeContract` SHALL 声明 `input_type` / `output_type` 字段描述节点类型的输入输出类型。本 change SHALL NOT 在运行时校验模型输入输出是否符合这些类型声明。`input_type`/`output_type` 仅为声明态，为后续 IO 强制 change 铺路。本 change 的强制校验 SHALL 只覆盖能力、权限边界、资源预算三维。

#### Scenario: IO 类型声明存在但不被校验
- **WHEN** 一个 `NodeContract` 声明了 `input_type` 和 `output_type`
- **THEN** 派发与返回时 SHALL NOT 校验模型输入输出是否符合这些类型声明
- **AND** 模型输出 SHALL 仍按现有 String 路径处理


```

Full source: openspec/changes/org-graph-node-contract/specs/org-graph-node-contract/spec.md

## openspec/changes/org-graph-node-contract/specs/subagent-tool-permissions/spec.md

- Source: openspec/changes/org-graph-node-contract/specs/subagent-tool-permissions/spec.md
- Lines: 1-15
- SHA256: 952e0949804a14ef1b37701145813cf23adaa7031cc4fbf70bd822c2db6a2033

```md
## MODIFIED Requirements

### Requirement: Role-enforced tool visibility for explore and plan

`explore` 和 `plan` 节点类型的工具可见性 SHALL 由其 `NodeContract` 的 `permissions`（`can_mutate_fs`）和 `capabilities` 驱动，而非硬编码 `is_leaf`/`explore_readonly` 逻辑。当 `explore_readonly` 全局配置启用（默认 true）且节点契约的 `permissions.can_mutate_fs = false` 时，`explore` 和 `plan` 子 agent SHALL NOT 拥有变更文件系统工具（`file_write`、`file_edit`、`apply_patch`）。`general-purpose` 子 agent MAY 保留完整工具集，受 depth 限制和统一权限管线约束。`filter_allowed_tools` SHALL 读取 `NodeContract.permissions` + `NodeContract.capabilities` 决定工具可见性，`explore_readonly` 配置作为 `permissions.can_mutate_fs` 的全局默认源，契约字段 `Option` 覆盖它。

#### Scenario: Explore cannot call file_write

- **WHEN** an `explore` subagent attempts to call `file_write` with `explore_readonly=true`（其 `NodeContract.permissions.can_mutate_fs = false`）
- **THEN** the call SHALL fail as not allowed for that agent type before execution

#### Scenario: Explore can call file_read

- **WHEN** an `explore` subagent calls `file_read` on a path inside the workspace
- **THEN** the tool SHALL be visible and proceed through the unified permission pipeline

```
