# org-graph-node-contract Specification

## Purpose
TBD - created by archiving change org-graph-node-contract. Update Purpose after archive.
## Requirements
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

### Requirement: task.rs 从硬编码 match 改为读契约
`task.rs execute_with_context` SHALL 从硬编码 `match _subagent_type` 分支改为通过 `NodeRegistry::get(&node_type)` 读取 `NodeContract`。system_prompt、allowed_tools、budget SHALL 全部来自 `NodeContract`。`filter_allowed_tools` SHALL 从硬编码 `is_leaf`/`explore_readonly` 逻辑改为读 `contract.capabilities` + `contract.permissions`，做三重过滤：①能力白名单（`contract.capabilities.allowed_tools`）、②`can_spawn`（剥离 task/delegate）、③`can_mutate_fs`（剥离 `MUTATING_FS_TOOLS`）。`explore_readonly` 全局配置 SHALL 在 `NodeRegistry::builtin(settings)` 构建契约时驱动 `permissions.can_mutate_fs`，不再作为 `filter_allowed_tools` 的独立参数。模型 JSON 的 `subagent_type` 字符串 SHALL 经 `parse_node_type` 派发层映射为可信 `NodeType` 枚举（照搬 `cli/args.rs` 现有映射语义），SHALL NOT 直接注入 `SpawnChildRequest`。变更后合法派发行为 SHALL 与变更前完全一致（无回归）。

#### Scenario: explore 派发读契约产出正确 prompt 和工具集
- **WHEN** 派发一个 `NodeType::Explore` 节点
- **THEN** system_prompt SHALL 来自 `NodeContract`，与变更前硬编码 explore 分支一致
- **AND** allowed_tools SHALL 来自 `contract.capabilities` + `contract.permissions` 三重过滤，与变更前 `filter_allowed_tools` 一致

#### Scenario: filter_allowed_tools 三重过滤
- **WHEN** `filter_allowed_tools` 处理一个 `NodeType::Explore` 契约的工具列表
- **THEN** SHALL 依据 `contract.capabilities.allowed_tools` 白名单保留工具
- **AND** SHALL 依据 `contract.permissions.can_spawn=false` 剥离 task/delegate
- **AND** SHALL 依据 `contract.permissions.can_mutate_fs=false` 剥离 file_write/file_edit/apply_patch

#### Scenario: parse_node_type 映射可信枚举
- **WHEN** 模型 JSON 的 `subagent_type` 为 "explore" / "plan" / "general-purpose" / "verify" / "guide"
- **THEN** SHALL 经 `parse_node_type` 映射为对应 `NodeType` 枚举
- **AND** 未知字符串 SHALL 默认映射为 `NodeType::GeneralPurpose`（与现有行为一致）

#### Scenario: 合法派发无回归
- **WHEN** 对 explore / plan / general-purpose 三种节点类型各派发一次合法请求
- **THEN** 每种的 system_prompt、allowed_tools、budget SHALL 与变更前硬编码路径完全一致

### Requirement: AgentDefinition 与 NodeContract 并存不破坏
本 change SHALL NOT 删除 `AgentDefinition`/`AgentsService`。`AgentDefinition` 作为遗留 CLI runner（`cli/args.rs run_agent`）和 stress_tests 的注册表保留。新派发路径（`task.rs` + `AgentCoordinator`）SHALL 只读 `NodeContract`，SHALL NOT 读 `AgentDefinition`。两层并存 SHALL NOT 破坏现有 CLI agent runner 和 stress_tests。

#### Scenario: CLI agent runner 仍工作
- **WHEN** 通过 CLI `run_agent` 调用一个 agent
- **THEN** SHALL 走 `AgentsService`/`AgentDefinition` 旧路径，行为与变更前一致

#### Scenario: 真实派发走 NodeContract
- **WHEN** 通过 `task`/`delegate` 工具派发子 agent
- **THEN** SHALL 走 `NodeContract`/`NodeRegistry` 新路径，SHALL NOT 读 `AgentDefinition`

