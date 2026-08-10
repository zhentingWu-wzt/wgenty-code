---
comet_change: org-graph-node-contract
role: technical-design
canonical_spec: openspec
---

# Design: Org-Graph 节点契约（NodeContract）

> 本文档是对 `openspec/changes/org-graph-node-contract/design.md` 高层决策（D1-D7）的深度技术细化。OpenSpec delta spec（`specs/org-graph-node-contract/spec.md` + `specs/subagent-tool-permissions/spec.md`）是上游事实源，本文不重复需求，只细化实现方案、技术风险、测试策略与边界条件。

## 1. 背景与现状

项目的 agent 派发今天有三套彼此脱节的结构，没有一套是 Org-Graph：

1. **`AgentDefinition`/`AgentsService`（`teams/subagent.rs:42-92`）**--看似 agent 注册表，实为死注册表。`execute_agent`（subagent.rs:286）是单轮 `api_client.chat()`，真实运行时不走它（仅 CLI `run_agent` + stress_tests 调用）。
2. **`task.rs execute_with_context`（`tools/meta/task.rs:468+`）**--真实派发路径。从模型 JSON 读 `subagent_type` 字符串，用硬编码 `match _subagent_type` 分支决定 system_prompt / allowed_tools / budget。
3. **`AgentCoordinator`（`agent/coordinator.rs:294`）**--真实执行引擎，管 depth+并发+生命周期。但 `SpawnChildRequest`（coordinator.rs:43-48）只有 `{ label: String }`，**对节点类型/能力/权限/budget 完全无知**。

节点类型的"能干什么/不能干什么/预算多少"散在三处硬编码：`match _subagent_type` 分支（task.rs:575+）、`filter_allowed_tools()`（task.rs:1121，`is_leaf`/`explore_readonly`/`MUTATING_FS_TOOLS`）、全局 `SubagentLimits`（config/agent.rs:177，`max_depth`/`max_concurrent`/`token_budget_k`/`max_rounds`）。coordinator 派发时无法做任何强制。

本 change 把隐式组织结构提取成显式、数据驱动、可被 coordinator 强制校验的 **NodeContract**。

## 2. 目标与非目标

见 OpenSpec proposal/design。此处补充深度约束：

- **三维校验分两层**：coordinator 校验 `can_spawn` + `budget(depth)`（spawn 那刻可判断）；`capability` + `can_mutate_fs` 在 task.rs `filter_allowed_tools` 校验（工具集在派发层算）。两者读同一份 NodeContract，校验时机不同。
- **ContractViolation 不触发 fallback**：结构上天然成立（`fallback_eligible_from_coordinator_error` 穷尽 match `_ => None`，fallback.rs:26-32）。
- **唯一行为变更**：违反契约的派发从"静默按硬编码逻辑处理"变为"coordinator 显式拒绝 + 回 ToolError 给父 agent"。合法派发行为不变（无回归）。
- 不引入 mock 框架，测试风格与现有纯函数单测一致。

## 3. 架构：纯数据 + 纯函数校验模块

`org_graph` 是纯数据 + 纯函数校验模块，无 async、无 I/O、无状态。coordinator 和 task.rs 依赖它，不是反过来。

```
src/org_graph/
├── mod.rs          # 模块注册 + re-export
├── contract.rs     # NodeContract + 五维类型
└── registry.rs     # NodeRegistry + 5 个内置契约

数据流：
  模型 JSON {subagent_type, prompt, ...}
    -> task.rs: parse_node_type(str) -> NodeType（可信枚举）
    -> task.rs: registry.get(&NodeType) -> &NodeContract
    -> task.rs: filter_allowed_tools(tools, contract)
                ↑ 校验 capability（白名单）+ can_mutate_fs
    -> task.rs: SpawnChildRequest::new(prompt).with_node_type(NodeType)
    -> coordinator.reserve_child:
         validate_contract: can_spawn? budget(depth)?
                            ↑ 校验 can_spawn + budget
         (ContractViolation -> 不触发 fallback, 回 ToolError)
```

## 4. 类型设计（`src/org_graph/contract.rs`）

```rust
/// 节点类型枚举。模型输出的 subagent_type 字符串经派发层映射为此可信枚举。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeType {
    Explore,
    Plan,
    GeneralPurpose,   // 默认值；RLM 子任务也走这个
    Verification,
    WgentyCodeGuide,
}

impl Default for NodeType {
    fn default() -> Self { NodeType::GeneralPurpose }
}

/// 能力：节点声明可用的工具集（白名单）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capability {
    pub allowed_tools: Vec<String>,
}

/// 权限边界：节点能做什么类型的操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionBoundary {
    pub can_spawn: bool,        // explore/plan=false, GP=true
    pub can_mutate_fs: bool,    // explore_readonly 时 explore/plan=false
    pub can_exec: bool,
}

/// 资源预算：per-node-type 覆盖全局 SubagentLimits。全 Option，None=回退全局。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResourceBudget {
    pub max_depth: Option<usize>,
    pub max_concurrent: Option<usize>,
    pub token_budget_k: Option<usize>,
    pub max_rounds: Option<usize>,
}

/// IO 形状：声明态，不校验。后续 IO 强制 change 可加变体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum IoShape {
    #[default]
    FreeText,
    StructuredJson,
    Report,
}

/// 节点契约：组织结构图的事实源。一张卡声明一个节点类型的五维约束。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeContract {
    pub node_type: NodeType,
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub system_prompt: String,        // 从 task.rs 硬编码 match 迁移来
    pub model: String,
    pub capabilities: Capability,
    pub permissions: PermissionBoundary,
    pub budget: ResourceBudget,
    pub input_type: IoShape,          // 声明态
    pub output_type: IoShape,         // 声明态
}

/// 契约校验维度（ContractViolation 携带）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContractDimension {
    NodeType,
    Capability,
    Permission,
    Budget,
}
```

**与现有类型关系**：`NodeContract` 是 `AgentDefinition` 的契约化升级，但不替换它（AgentDefinition 并存不删，CLI runner 继续用）。`SpawnChildRequest` 扩展携带 `NodeType`。`SubagentLimits` 作为 budget 全局默认源，contract.budget 的 Option 字段覆盖它。

## 5. NodeRegistry（`src/org_graph/registry.rs`）

```rust
pub struct NodeRegistry {
    contracts: HashMap<NodeType, NodeContract>,
}

impl NodeRegistry {
    /// 构建内置契约。读取 settings 填充 can_mutate_fs 和 budget 默认值。
    pub fn builtin(settings: &SubagentLimits) -> Self {
        let mut contracts = HashMap::new();
        contracts.insert(NodeType::Explore, Self::explore_contract(settings));
        contracts.insert(NodeType::Plan, Self::plan_contract(settings));
        contracts.insert(NodeType::GeneralPurpose, Self::gp_contract(settings));
        contracts.insert(NodeType::Verification, Self::verify_contract(settings));
        contracts.insert(NodeType::WgentyCodeGuide, Self::guide_contract(settings));
        Self { contracts }
    }

    pub fn get(&self, node_type: &NodeType) -> Option<&NodeContract> {
        self.contracts.get(node_type)
    }
}
```

**5 个内置契约照搬现有硬编码语义（D7，不借机调整）**：

| 节点 | can_spawn | can_mutate_fs | capabilities | budget |
|---|---|---|---|---|
| Explore | false | `!explore_readonly` | search/file_read/list_files/grep/glob | None（回退全局） |
| Plan | false | `!explore_readonly` | search/file_read/list_files | None |
| GeneralPurpose | true | true | 全工具集 | None |
| Verification | false | true | file_read/search/execute_command | None |
| WgentyCodeGuide | false | false | file_read/search | None |

`explore_readonly` 全局配置在契约构建时驱动 `can_mutate_fs`（`!explore_readonly`）。budget 字段全 None（回退全局 SubagentLimits），保持现有配置语义不变。

## 6. SpawnChildRequest 扩展 + coordinator 校验

### 6.1 SpawnChildRequest

```rust
#[derive(Debug, Clone)]
pub struct SpawnChildRequest {
    pub label: String,
    pub node_type: NodeType,   // 新增，默认 GeneralPurpose
}

impl SpawnChildRequest {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), node_type: NodeType::default() }
    }
    pub fn with_node_type(mut self, node_type: NodeType) -> Self {
        self.node_type = node_type;
        self
    }
}
```

`new()` 保持原签名行为（默认 GP），现有调用点不传 node_type 时行为不变。

### 6.2 coordinator 持有 NodeRegistry

```rust
pub struct AgentCoordinator {
    // ... 现有字段 ...
    registry: Arc<NodeRegistry>,   // 新增
}

impl AgentCoordinator {
    pub fn new(max_concurrent: usize, max_depth: usize) -> Self {
        Self {
            // ... 现有 ...
            registry: Arc::new(NodeRegistry::builtin(&default_subagent_limits())),
        }
    }
    // 或构造时传入 settings：
    pub fn with_registry(max_concurrent: usize, max_depth: usize, settings: &SubagentLimits) -> Self { ... }
}
```

### 6.3 reserve_child 契约校验

在现有 depth 检查**之前**插入契约校验（契约违反优先于 depth 拒绝）：

```rust
pub async fn reserve_child(&self, caller, request) -> Result<ChildReservation, CoordinatorError> {
    // 1. 契约校验（新增）
    let contract = self.registry.get(&request.node_type).ok_or_else(|| {
        CoordinatorError::ContractViolation {
            node_type: request.node_type.clone(),
            dimension: ContractDimension::NodeType,
            reason: "unknown node type".to_string(),
        }
    })?;
    self.validate_contract(contract, caller)?;

    // 2. 现有：parent 状态检查
    // 3. 现有：depth 检查（contract.budget.max_depth 覆盖全局 max_depth）
    // 4. 现有：semaphore permit
}

fn validate_contract(&self, contract: &NodeContract, caller: &AgentExecutionContext) -> Result<(), CoordinatorError> {
    // 维度: can_spawn（leaf 节点调 reserve_child 即违规）
    if !contract.permissions.can_spawn {
        return Err(CoordinatorError::ContractViolation {
            node_type: contract.node_type.clone(),
            dimension: ContractDimension::Permission,
            reason: "this node type cannot spawn children".to_string(),
        });
    }
    // budget: depth 实际值在现有 depth 检查里做（contract.budget.max_depth 覆盖）
    Ok(())
}
```

> **⚠️ 实现偏差（build 阶段修正）**：上面伪代码读取 `request.node_type`（**子**节点类型）并检查其
> `can_spawn`，这在语义上是错的——它会让「spawn 一个 explore/plan 子节点」永远失败（因为 leaf 的
> `can_spawn=false`）。`can_spawn` 的正确语义是「**调用者**能否成为 parent」，不是「子节点能否被 spawn」。
>
> 实现中（`coordinator.rs` `caller_contract` + `reserve_child`）改为：
> 1. 先用 `caller_contract(caller)` 查**调用者**的 node_type（未注册的 root → GeneralPurpose，`can_spawn=true`）；
> 2. 检查 **caller_contract** 的 `can_spawn`；
> 3. 再单独校验 `request.node_type` 是已知契约（`ContractDimension::NodeType`，防御性 depth-in-depth）。
>
> 调用者→node_type 的映射存在 coordinator 的 `node_types` 旁表（`reserve_child` 时 `record_child_node_type`
> 写入，`finish_child` 时清理）。测试 `leaf_caller_cannot_spawn`（leaf 调用者拒绝）与
> `all_builtin_child_types_accepted_from_root`（root 可 spawn 任意 builtin 子类型）共同锁定两端语义。


**三维校验分两层（关键设计）**：

| 维度 | 校验位置 | 原因 |
|---|---|---|
| Permission: can_spawn | coordinator `reserve_child` | spawn 那刻就能判断 |
| Budget: depth | coordinator `reserve_child`（contract.budget.max_depth 覆盖全局） | spawn 那刻有 caller.depth |
| Capability + can_mutate_fs | task.rs `filter_allowed_tools`（读契约算工具集） | 工具集是派发层算的 |

### 6.4 ContractViolation 错误模型

```rust
pub enum CoordinatorError {
    // ... 现有变体 ...
    ContractViolation {
        node_type: NodeType,
        dimension: ContractDimension,
        reason: String,
    },
}
```

`fallback_eligible_from_coordinator_error`（fallback.rs:26-32）的 `_ => None` 自动覆盖 `ContractViolation`（不触发 fallback，已验证安全）。RLM replan 路径 `reserve_child` 失败时 `continue`（pipeline.rs:660），不破坏 replan 循环。

`task.rs map_coordinator_error` 加分支：

```rust
CoordinatorError::ContractViolation { node_type, dimension, reason } => ToolError {
    message: format!("Contract violation for {:?} node ({}): {}", node_type, dimension, reason),
    code: Some("contract_violation".to_string()),
},
```

## 7. task.rs 迁移

### 7.1 execute_with_context 改读契约

```rust
async fn execute_with_context(&self, context, input) -> Result<ToolOutput, ToolError> {
    let subagent_type_str = input["subagent_type"].as_str().unwrap_or("general-purpose");
    let node_type = parse_node_type(subagent_type_str);          // 字符串 -> NodeType
    let contract = self.registry.get(&node_type).ok_or_else(..)?; // 查契约
    let base_system_prompt = &contract.system_prompt;             // 来自契约（不再 match）
    let allowed_tools = filter_allowed_tools(                     // 读契约
        tool_registry.list().iter().map(|t| t.name().to_string()),
        contract,
    );
    let token_budget = resolve_token_budget(input, contract, &self.settings);
    let request = SpawnChildRequest::new(&prompt).with_node_type(node_type.clone());
    // ... 后续 coordinator.reserve_child_in_group 不变 ...
}
```

### 7.2 parse_node_type 映射（照搬 cli/args.rs:730-735）

```rust
fn parse_node_type(s: &str) -> NodeType {
    match s {
        "explore" => NodeType::Explore,
        "plan" => NodeType::Plan,
        "general-purpose" | "general" => NodeType::GeneralPurpose,
        "verify" | "verification" => NodeType::Verification,
        "guide" | "wgenty-code-guide" => NodeType::WgentyCodeGuide,
        _ => NodeType::GeneralPurpose,   // 未知默认 GP（与现有行为一致）
    }
}
```

### 7.3 filter_allowed_tools 改读契约

```rust
pub(crate) fn filter_allowed_tools(
    names: impl IntoIterator<Item = String>,
    contract: &NodeContract,
) -> Vec<String> {
    names.into_iter().filter(|name| {
        let is_spawn = name == "task" || name == "delegate";
        let is_mutate_fs = MUTATING_FS_TOOLS.contains(&name.as_str());
        // 维度1: capability 白名单
        if !contract.capabilities.allowed_tools.contains(name) { return false; }
        // 维度2: permission.can_spawn
        if is_spawn && !contract.permissions.can_spawn { return false; }
        // 维度3: permission.can_mutate_fs
        if is_mutate_fs && !contract.permissions.can_mutate_fs { return false; }
        true
    }).collect()
}
```

### 7.4 RLM 调用点

RLM 两处 `reserve_child`（pipeline.rs:276 / :650）显式写 `.with_node_type(NodeType::GeneralPurpose)`（等于默认值，但意图清晰）。`use_small_model` 留 RLM 自己管，不进 NodeContract（OQ1 决策）。

## 8. 测试策略

| 层级 | 测试 | 文件 |
|---|---|---|
| 纯函数 | NodeContract serde 往返（五维齐全） | contract.rs |
| 纯函数 | NodeRegistry 查询（5 存在/未知 None） | registry.rs |
| 纯函数 | IoShape serde 往返 | contract.rs |
| 纯函数 | parse_node_type 映射（5 类型+未知默认 GP） | task.rs |
| 纯函数 | filter_allowed_tools 读契约（白名单+can_spawn+can_mutate_fs） | task.rs |
| 纯函数 | validate_contract（can_spawn 拒绝） | coordinator.rs |
| 纯函数 | ContractViolation 不触发 fallback（fallback_eligible 返回 None） | fallback.rs |
| 纯函数 | budget None 回退全局/Some 覆盖 | coordinator.rs |
| 纯函数 | map_coordinator_error 映射 ContractViolation | task.rs |
| 无回归 | explore/plan/GP 派发 prompt+tools+budget 与变更前一致 | task.rs |
| 并存 | CLI run_agent + stress_tests 走 AgentDefinition | cli/args.rs |

不写 `run_rlm_pipeline` 端到端单测（需 LLM+coordinator，现有测试也规避）。

## 9. 风险与权衡

- **[三维校验分两层]** coordinator 校验 can_spawn+budget，task.rs 校验 capability+can_mutate_fs。Mitigation：两者读同一份 NodeContract，Spec Patch 已精确化验收场景；filter_allowed_tools 纯函数单测覆盖。
- **[ContractViolation 不触发 fallback]** 结构上天然成立（`_ => None`）。Mitigation：fallback.rs 加单测断言 ContractViolation 返回 None。
- **[SpawnChildRequest 多调用点]** task.rs / fallback.rs / rlm/pipeline.rs / daemon/handlers.rs / run_script.rs。Mitigation：node_type 默认 GP，未传的行为不变；逐个补 .with_node_type。
- **[NodeRegistry::builtin 需读 settings]** 编译期固定改为运行时读 settings。Mitigation：coordinator 构造时传 settings；registry 纯数据，构建后不可变。
- **[AgentDefinition 两层并存]** Mitigation：文档标注各自用途，不删 AgentDefinition 避免破坏 CLI。
- **[IoShape 声明不校验]** Mitigation：字段文档明确"声明态"，验收场景只测三维强制不测 IO。

## 10. Spec Patch

回写 `specs/org-graph-node-contract/spec.md`：
1. "coordinator 在派发时强制校验三维契约"需求：精确化场景，区分 coordinator 校验维度（can_spawn + budget）和 task.rs 校验维度（capability + can_mutate_fs）；补充"ContractViolation 不触发 fallback"场景。
2. "task.rs 从硬编码 match 改为读契约"需求：补充"filter_allowed_tools 读 contract.capabilities + contract.permissions 三重过滤"场景。

Spec Patch 仅补充验收场景 + 精确化歧义描述，不大幅重写需求结构。
