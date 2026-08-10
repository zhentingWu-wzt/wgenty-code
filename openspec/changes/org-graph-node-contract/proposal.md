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
