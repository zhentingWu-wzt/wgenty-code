---
change: org-graph-node-contract
design-doc: docs/superpowers/specs/2026-08-10-org-graph-node-contract-design.md
base-ref: 0de3b78df5bc3444e8cc3b99cbdb4b675c589adf
---

# Org-Graph 节点契约（NodeContract）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 agent 派发中隐式散落的三套硬编码结构提取成显式、数据驱动、可被 coordinator 强制校验的 `NodeContract`，使 `task.rs` 从硬编码 `match` 改为读契约，且 5 个内置节点契约照搬现有语义（无回归）。

**Architecture:** 新建 `src/org_graph/` 纯数据 + 纯函数校验模块（`contract.rs` 定义五维类型，`registry.rs` 持有 5 个内置契约）。`AgentCoordinator` 在 `reserve_child` 强制校验 `can_spawn` + budget(depth)；`task.rs execute_with_context` + `filter_allowed_tools` 改读契约校验 capability + `can_mutate_fs`。两层校验读同一份 `NodeContract`。

**Tech Stack:** Rust 2021 edition，包名 `wgenty_code`，serde（derive 已在 Cargo.toml:26），thiserror（CoordinatorError 已用）。无 mock 框架，纯函数单测。

## Global Constraints

- 包名 `wgenty_code`（Cargo.toml:2），edition 2021，crate root 是 `src/lib.rs`（不是 main.rs）。
- serde with `derive` feature 已是依赖（Cargo.toml:26），无需新增依赖。
- **无回归是硬约束**：5 个内置契约照搬现有硬编码语义（设计 D7），合法派发行为与变更前完全一致。
- 不引入 mock 框架，测试风格与现有纯函数单测一致（`#[cfg(test)] mod tests`，`#[test]`）。
- `AgentDefinition`/`AgentsService` 不删除（CLI runner `cli/args.rs run_agent` + stress_tests 继续用），新派发路径只读 `NodeContract`。
- `ContractViolation` 不触发 structural fallback（`fallback_eligible_from_coordinator_error` 的 `_ => None` 自动覆盖）。
- **三维校验分两层**（设计 §2/§6.3）：coordinator 校验 `can_spawn` + budget(depth)；task.rs `filter_allowed_tools` 校验 capability + `can_mutate_fs`。
- 遇到测试失败/构建失败必须加载 `superpowers:systematic-debugging` skill，根因未定位前不得提源码修复。
- 每个 task 验收后：tasks.md 打勾 → git commit（不得积攒）。

## 设计偏差说明（实施前必读）

以下 4 点是对设计文档伪代码的精确化，已与设计意图（D7 无回归 + spec 验收场景）核对一致：

1. **capability 白名单对内置契约使用通配符（空 Vec）**。设计 §5 表格列了具体工具，但当前 `filter_allowed_tools`（task.rs:1121-1144）给 explore/plan 的是「全工具减 spawn（减 mutating-fs 当 explore_readonly）」，没有正向白名单。若按表格精确枚举工具会丢掉 explore 当前可见的 exec_command 等工具 → 回归。因此内置契约 `capabilities.allowed_tools = vec![]`（空），`filter_allowed_tools` 把空 Vec 当通配符（全通过），实际剥离由 `can_spawn` + `can_mutate_fs` 两维完成——与当前行为逐字节一致。capability 白名单维度仍被纯函数单测覆盖（用非空 Vec 的合成契约测）。
2. **can_spawn 校验 caller 的 node_type，不是 request.node_type**。设计 §6.3 伪代码 `registry.get(&request.node_type)` 后查 `can_spawn`，但 `request.node_type` 是**子节点**类型（查子的 can_spawn 会拒绝 spawn explore 子节点——严重回归）。spec scenario「leaf 节点禁止 spawn 被 coordinator 拒绝」明确要求校验 **caller**（发起 spawn 的节点）的 can_spawn。因此 coordinator 新增 `node_types` 侧表记录每个 agent 的 node_type，`reserve_child` 查 caller 的契约校验 can_spawn。
3. **can_exec 声明但不强制**。与 IoShape 同理（声明态）。当前 filter_allowed_tools 不 gate exec_command（task.rs:1120 注释「exec_command remains visible, still gated by policy + guardian」），本 change 不调整。
4. **verify / guide 的 system_prompt 留空**。task.rs 硬编码 match 只有 explore/plan/GP 三个 arm（无 verify/guide）；CLI 路径继续走 `AgentDefinition`。这两个契约的 system_prompt 设为空字符串（声明态，新派发路径不派发这两个类型，因 input schema enum 限制为 3 值）。

## File Structure

| 文件 | 职责 | 动作 |
|---|---|---|
| `src/org_graph/mod.rs` | 模块注册 + re-export | 新建 |
| `src/org_graph/contract.rs` | `NodeContract` + 五维类型（NodeType/Capability/PermissionBoundary/ResourceBudget/IoShape/ContractDimension） | 新建 |
| `src/org_graph/registry.rs` | `NodeRegistry` + 5 个内置契约 | 新建 |
| `src/lib.rs` | 注册 `pub mod org_graph;` | 改（:27 后插入） |
| `src/agent/coordinator.rs` | `SpawnChildRequest.node_type`、`AgentCoordinator.registry` + `node_types` 侧表、`CoordinatorError::ContractViolation`、`reserve_child` 校验 | 改 |
| `src/agent/fallback.rs` | ContractViolation 不触发 fallback 断言测试 | 改（加测试） |
| `src/tools/meta/task.rs` | `parse_node_type`、`execute_with_context` 读契约、`filter_allowed_tools` 读契约、`map_coordinator_error` 加分支 | 改 |
| `src/tools/meta/rlm/pipeline.rs` | 2 处 `reserve_child` 补 `.with_node_type(GP)` | 改（:276, :650） |
| `src/tools/meta/run_script.rs` | `reserve_child` 补 `.with_node_type(GP)` | 改（:118） |
| `src/daemon/state.rs` | 构造 `NodeRegistry` 注入 coordinator | 改（:139） |
| `src/teams/subagent.rs` | 不改（AgentDefinition 并存） | 不改 |

---

## Task 1: NodeContract 类型基础（tasks.md §1: 1.1, 1.2, 1.3）

**关联设计文档：** §3 架构、§4 类型设计

**Files:**
- Create: `src/org_graph/mod.rs`
- Create: `src/org_graph/contract.rs`
- Modify: `src/lib.rs:27`（mcp 后插入 `pub mod org_graph;`）
- Test: `src/org_graph/contract.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 无（纯数据模块，无外部依赖）
- Produces:
  - `org_graph::NodeType`（enum，5 变体 + Default = GeneralPurpose）
  - `org_graph::Capability { allowed_tools: Vec<String> }`
  - `org_graph::PermissionBoundary { can_spawn: bool, can_mutate_fs: bool, can_exec: bool }`
  - `org_graph::ResourceBudget { max_depth: Option<usize>, max_concurrent: Option<usize>, token_budget_k: Option<usize>, max_rounds: Option<usize> }`
  - `org_graph::IoShape`（enum，FreeText/StructuredJson/Report，Default = FreeText）
  - `org_graph::NodeContract`（struct，含上述五维 + 元数据字段）
  - `org_graph::ContractDimension`（enum，NodeType/Capability/Permission/Budget）

- [ ] **Step 1.1: 创建 `src/org_graph/contract.rs`，定义全部类型**

```rust
//! Org-Graph 节点契约：纯数据 + 纯函数校验模块的类型层。
//!
//! 每个 agent 节点类型用一张 [`NodeContract`] 声明五维约束（能力、权限边界、
//! 资源预算、IO 形状、身份/谱系）。本模块无 async、无 I/O、无状态。

use serde::{Deserialize, Serialize};

/// 节点类型枚举。模型输出的 `subagent_type` 字符串经派发层映射为此可信枚举。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeType {
    Explore,
    Plan,
    GeneralPurpose,
    Verification,
    WgentyCodeGuide,
}

impl Default for NodeType {
    fn default() -> Self {
        NodeType::GeneralPurpose
    }
}

/// 能力：节点声明可用的工具集（白名单）。空 Vec = 通配符（全部允许），
/// 非空 = 仅允许列出的工具。内置契约均用空 Vec（照搬现有「无正向白名单」语义）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Capability {
    pub allowed_tools: Vec<String>,
}

/// 权限边界：节点能做什么类型的操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionBoundary {
    /// explore/plan = false（leaf），GeneralPurpose = true。
    pub can_spawn: bool,
    /// explore_readonly 时 explore/plan = false。
    pub can_mutate_fs: bool,
    /// 声明态，本 change 不强制校验（exec_command 由 policy + guardian gate）。
    pub can_exec: bool,
}

/// 资源预算：per-node-type 覆盖全局 `SubagentLimits`。全 `Option`，`None` = 回退全局。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResourceBudget {
    pub max_depth: Option<usize>,
    pub max_concurrent: Option<usize>,
    pub token_budget_k: Option<usize>,
    pub max_rounds: Option<usize>,
}

/// IO 形状：声明态，本 change 不校验。后续 IO 强制 change 可加变体。
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
    /// 从 task.rs 硬编码 match 迁移来的 base system prompt（不含 comet_prefix）。
    pub system_prompt: String,
    pub model: String,
    pub capabilities: Capability,
    pub permissions: PermissionBoundary,
    pub budget: ResourceBudget,
    /// 声明态，本 change 不校验。
    pub input_type: IoShape,
    /// 声明态，本 change 不校验。
    pub output_type: IoShape,
}

/// 契约校验维度（`ContractViolation` 携带）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContractDimension {
    NodeType,
    Capability,
    Permission,
    Budget,
}
```

- [ ] **Step 1.2: 创建 `src/org_graph/mod.rs`**

```rust
//! Org-Graph 节点契约模块：纯数据 + 纯函数校验，无 async / I/O / 状态。

pub mod contract;
pub mod registry;

pub use contract::{
    Capability, ContractDimension, IoShape, NodeContract, NodeType, PermissionBoundary,
    ResourceBudget,
};
pub use registry::NodeRegistry;
```

> 注：`registry` 模块在 Task 2 创建。本步先写 `mod registry;` 和 re-export，Task 1 编译会因缺 `registry.rs` 失败——这是预期的（Task 2 补齐后通过）。若希望 Task 1 独立编译通过，可暂时注释掉 `pub mod registry;` 和 `pub use registry::NodeRegistry;`，Task 2 再取消注释。**推荐**：先注释，Task 2 取消，保证每个 task 独立可编译。

- [ ] **Step 1.3: 在 `src/lib.rs:27`（`pub mod mcp;` 之后）插入模块声明**

```rust
pub mod mcp;
pub mod org_graph;
pub mod permissions;
```

- [ ] **Step 1.4: 写失败测试 — NodeContract serde 往返 + IoShape 往返**

在 `src/org_graph/contract.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_contract_serde_roundtrip() {
        let contract = NodeContract {
            node_type: NodeType::Explore,
            name: "explore".to_string(),
            description: "code exploration".to_string(),
            when_to_use: "searching code".to_string(),
            system_prompt: "You are an explorer.".to_string(),
            model: "sonnet".to_string(),
            capabilities: Capability {
                allowed_tools: vec!["search".to_string(), "file_read".to_string()],
            },
            permissions: PermissionBoundary {
                can_spawn: false,
                can_mutate_fs: false,
                can_exec: true,
            },
            budget: ResourceBudget {
                max_depth: Some(3),
                max_concurrent: None,
                token_budget_k: Some(32),
                max_rounds: None,
            },
            input_type: IoShape::FreeText,
            output_type: IoShape::Report,
        };
        let json = serde_json::to_string(&contract).expect("serialize");
        let back: NodeContract = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(contract, back);
    }

    #[test]
    fn ioshape_serde_roundtrip() {
        for shape in [IoShape::FreeText, IoShape::StructuredJson, IoShape::Report] {
            let json = serde_json::to_string(&shape).expect("serialize");
            let back: IoShape = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(shape, back);
        }
    }

    #[test]
    fn nodetype_default_is_general_purpose() {
        assert_eq!(NodeType::default(), NodeType::GeneralPurpose);
    }
}
```

- [ ] **Step 1.5: 运行测试验证通过**

Run: `cargo test -p wgenty_code org_graph::contract::tests -- --nocapture`
Expected: PASS（3 个测试）。若 Step 1.2 暂时注释了 `mod registry`，此处应编译通过。

- [ ] **Step 1.6: Commit**

```bash
git add src/org_graph/mod.rs src/org_graph/contract.rs src/lib.rs
git commit -m "feat(org_graph): add NodeContract type layer with serde roundtrip tests"
```

---

## Task 2: NodeRegistry 与内置契约（tasks.md §2: 2.1, 2.2, 2.3）

**关联设计文档：** §5 NodeRegistry、§5 内置契约表

**Files:**
- Create: `src/org_graph/registry.rs`
- Modify: `src/org_graph/mod.rs`（若 Task 1 注释了 `mod registry`，此处取消注释）
- Test: `src/org_graph/registry.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `org_graph::NodeContract` / `NodeType` / `PermissionBoundary` / `Capability` / `ResourceBudget`（Task 1）；`config::agent::SubagentLimits`（读 `explore_readonly`）
- Produces:
  - `org_graph::NodeRegistry { contracts: HashMap<NodeType, NodeContract> }`
  - `NodeRegistry::builtin(settings: &SubagentLimits) -> Self`
  - `NodeRegistry::get(&self, node_type: &NodeType) -> Option<&NodeContract>`

**内置契约内容（照搬现有硬编码语义，设计 §5 表格 + D7）：**

| 节点 | can_spawn | can_mutate_fs | can_exec | allowed_tools | budget |
|---|---|---|---|---|---|
| Explore | false | `!settings.explore_readonly` | true | `vec![]`（通配） | 全 None |
| Plan | false | `!settings.explore_readonly` | true | `vec![]`（通配） | 全 None |
| GeneralPurpose | true | true | true | `vec![]`（通配） | 全 None |
| Verification | false | true | true | `vec![]`（通配） | 全 None |
| WgentyCodeGuide | false | false | true | `vec![]`（通配） | 全 None |

> system_prompt：Explore/Plan/GP 从 task.rs:598-655 硬编码 match arm 照搬（见 Step 2.1 完整字符串）；Verification/WgentyCodeGuide 设为空字符串（CLI 路径走 AgentDefinition）。model 设为 `"default"`（声明态，task.rs 不读此字段——模型选择逻辑不变）。

- [ ] **Step 2.1: 创建 `src/org_graph/registry.rs`**

```rust
//! 内置节点契约注册表。构建后不可变（纯数据）。

use std::collections::HashMap;

use crate::config::agent::SubagentLimits;
use crate::org_graph::contract::{
    Capability, IoShape, NodeContract, NodeType, PermissionBoundary, ResourceBudget,
};

pub struct NodeRegistry {
    contracts: HashMap<NodeType, NodeContract>,
}

impl NodeRegistry {
    /// 构建内置契约。读取 `settings.explore_readonly` 填充 `can_mutate_fs`。
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

    fn explore_contract(s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::Explore,
            name: "explore".to_string(),
            description: "Read-only code exploration subagent.".to_string(),
            when_to_use: "Searching and analyzing codebases".to_string(),
            system_prompt: Self::explore_prompt().to_string(),
            model: "default".to_string(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary {
                can_spawn: false,
                can_mutate_fs: !s.explore_readonly,
                can_exec: true,
            },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText,
            output_type: IoShape::Report,
        }
    }

    fn plan_contract(s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::Plan,
            name: "plan".to_string(),
            description: "Planning subagent.".to_string(),
            when_to_use: "Breaking down complex tasks".to_string(),
            system_prompt: Self::plan_prompt().to_string(),
            model: "default".to_string(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary {
                can_spawn: false,
                can_mutate_fs: !s.explore_readonly,
                can_exec: true,
            },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText,
            output_type: IoShape::Report,
        }
    }

    fn gp_contract(_s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::GeneralPurpose,
            name: "general-purpose".to_string(),
            description: "General-purpose subagent.".to_string(),
            when_to_use: "Any sub-work that may need further delegation".to_string(),
            system_prompt: Self::gp_prompt().to_string(),
            model: "default".to_string(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary {
                can_spawn: true,
                can_mutate_fs: true,
                can_exec: true,
            },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText,
            output_type: IoShape::FreeText,
        }
    }

    fn verify_contract(_s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::Verification,
            name: "verification".to_string(),
            description: "Verification subagent.".to_string(),
            when_to_use: "Verifying build/test results".to_string(),
            system_prompt: String::new(),
            model: "default".to_string(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary {
                can_spawn: false,
                can_mutate_fs: true,
                can_exec: true,
            },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText,
            output_type: IoShape::Report,
        }
    }

    fn guide_contract(_s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::WgentyCodeGuide,
            name: "wgenty-code-guide".to_string(),
            description: "Guide subagent.".to_string(),
            when_to_use: "Answering questions about wgenty-code".to_string(),
            system_prompt: String::new(),
            model: "default".to_string(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary {
                can_spawn: false,
                can_mutate_fs: false,
                can_exec: true,
            },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText,
            output_type: IoShape::Report,
        }
    }

    // ── system_prompt 照搬 task.rs:598-655 硬编码 match arm（逐字复制）──

    fn explore_prompt() -> &'static str {
        "You are a code exploration subagent. Your role is to search \
         and analyze codebases thoroughly.\n\n\
         IMPORTANT — Choose your strategy based on the task type:\n\
         - For PATTERN SEARCH tasks (e.g. 'find all .unwrap() calls', \
           'count .clone() usages'): use grep directly with precise \
           regex patterns. Do NOT read full files — grep gives you \
           matching lines directly. Call grep with the exact pattern \
           and max_results to control output size. Report counts, file \
           locations, and representative examples.\n\
         - For STRUCTURAL ANALYSIS tasks (e.g. 'how does module X \
           work'): use glob to find relevant files, then file_read \
           to understand key files, then grep for cross-references.\n\
         - For COUNTING/STATISTICS tasks: prefer grep with \
           files_with_matches=true first to scope the work, then \
           detailed grep for actual matches.\n\n\
         Key responsibilities:\n\
         1. Search for relevant files and code patterns\n\
         2. Read and understand code structure\n\
         3. Analyze dependencies and relationships\n\
         4. Report findings clearly and concisely\n\n\
         Use search, grep, glob, and file_read tools to explore the \
         codebase. Be thorough but efficient — focus on answering the \
         specific question. Return a complete, self-contained result."
    }

    fn plan_prompt() -> &'static str {
        "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a planning subagent. Your role is to break down complex \
         tasks into actionable steps.\n\nKey responsibilities:\n\
         1. Analyze task requirements\n\
         2. Identify key files and components\n\
         3. Break down the work into logical steps\n\
         4. Consider dependencies, risks, and trade-offs\n\n\
         Use file_read and search tools to understand the codebase before \
         planning. Be thorough and structured in your analysis."
    }

    fn gp_prompt() -> &'static str {
        "You are a general-purpose subagent spawned by a coordinator. The \
         coordinator is waiting for your result. Return a complete, \
         self-contained result so the coordinator can proceed without \
         follow-up questions.\n\n\
         You may use the `task` tool to delegate discrete sub-work when it \
         helps. If a nested spawn is rejected (depth limit or other \
         structural failure), the runtime automatically runs that \
         delegated prompt with leaf tools and returns the result as the \
         task tool output — treat a successful task result as completed \
         work, and if task fails, finish the work yourself with direct \
         tools.\n\n\
         Key responsibilities:\n\
         1. Understand the task requirements\n\
         2. Use appropriate tools (or task for discrete sub-work) to \
            accomplish the task\n\
         3. Provide clear and complete results\n\
         4. Handle edge cases gracefully\n\n\
         If you need to read files, search, or execute commands, use the \
         appropriate tools. Return a complete summary of what was accomplished."
    }
}
```

- [ ] **Step 2.2: 确保 `src/org_graph/mod.rs` 的 `pub mod registry;` + re-export 取消注释**

确认 Task 1 Step 1.2 中若注释了 `pub mod registry;` 和 `pub use registry::NodeRegistry;`，此处取消注释。

- [ ] **Step 2.3: 写测试 — registry 查询 + 契约内容对齐**

在 `src/org_graph/registry.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::agent::SubagentLimits;

    fn registry(readonly: bool) -> NodeRegistry {
        let mut s = SubagentLimits::default();
        s.explore_readonly = readonly;
        NodeRegistry::builtin(&s)
    }

    #[test]
    fn all_five_builtin_contracts_present() {
        let r = registry(true);
        for nt in [
            NodeType::Explore,
            NodeType::Plan,
            NodeType::GeneralPurpose,
            NodeType::Verification,
            NodeType::WgentyCodeGuide,
        ] {
            assert!(r.get(&nt).is_some(), "missing contract for {:?}", nt);
        }
    }

    #[test]
    fn explore_is_leaf_and_readonly_when_explore_readonly() {
        let r = registry(true);
        let c = r.get(&NodeType::Explore).unwrap();
        assert!(!c.permissions.can_spawn);
        assert!(!c.permissions.can_mutate_fs, "explore_readonly=true => can_mutate_fs=false");
    }

    #[test]
    fn explore_can_mutate_when_not_readonly() {
        let r = registry(false);
        let c = r.get(&NodeType::Explore).unwrap();
        assert!(c.permissions.can_mutate_fs, "explore_readonly=false => can_mutate_fs=true");
    }

    #[test]
    fn plan_is_leaf_and_readonly_when_explore_readonly() {
        let r = registry(true);
        let c = r.get(&NodeType::Plan).unwrap();
        assert!(!c.permissions.can_spawn);
        assert!(!c.permissions.can_mutate_fs);
    }

    #[test]
    fn general_purpose_can_spawn_and_mutate() {
        let r = registry(true);
        let c = r.get(&NodeType::GeneralPurpose).unwrap();
        assert!(c.permissions.can_spawn);
        assert!(c.permissions.can_mutate_fs);
    }

    #[test]
    fn verification_cannot_spawn_but_can_mutate() {
        let r = registry(true);
        let c = r.get(&NodeType::Verification).unwrap();
        assert!(!c.permissions.can_spawn);
        assert!(c.permissions.can_mutate_fs);
    }

    #[test]
    fn guide_cannot_spawn_nor_mutate() {
        let r = registry(true);
        let c = r.get(&NodeType::WgentyCodeGuide).unwrap();
        assert!(!c.permissions.can_spawn);
        assert!(!c.permissions.can_mutate_fs);
    }

    #[test]
    fn builtin_budgets_all_none() {
        let r = registry(true);
        for nt in [
            NodeType::Explore,
            NodeType::Plan,
            NodeType::GeneralPurpose,
            NodeType::Verification,
            NodeType::WgentyCodeGuide,
        ] {
            let c = r.get(&nt).unwrap();
            assert_eq!(c.budget, ResourceBudget::default(), "{:?} budget not all-None", nt);
        }
    }

    #[test]
    fn explore_plan_gp_system_prompts_nonempty() {
        let r = registry(true);
        assert!(!r.get(&NodeType::Explore).unwrap().system_prompt.is_empty());
        assert!(!r.get(&NodeType::Plan).unwrap().system_prompt.is_empty());
        assert!(!r.get(&NodeType::GeneralPurpose).unwrap().system_prompt.is_empty());
    }
}
```

- [ ] **Step 2.4: 运行测试验证通过**

Run: `cargo test -p wgenty_code org_graph::registry::tests -- --nocapture`
Expected: PASS（8 个测试）。

- [ ] **Step 2.5: Commit**

```bash
git add src/org_graph/registry.rs src/org_graph/mod.rs
git commit -m "feat(org_graph): add NodeRegistry with 5 builtin contracts mirroring hardcoded semantics"
```

---

## Task 3: SpawnChildRequest 扩展 + coordinator 持有 registry（tasks.md §3: 3.1, 3.2, 3.3）

**关联设计文档：** §6.1 SpawnChildRequest、§6.2 coordinator 持有 NodeRegistry

**Files:**
- Modify: `src/agent/coordinator.rs:30-44`（SpawnChildRequest）、`:292-335`（AgentCoordinator struct + new）
- Modify: `src/daemon/state.rs:139-142`（注入 registry）
- Test: `src/agent/coordinator.rs`（`#[cfg(test)] mod tests`，新增或追加）

**Interfaces:**
- Consumes: `org_graph::{NodeType, NodeRegistry}`（Task 1/2）；`config::agent::SubagentLimits`（default registry）
- Produces:
  - `SpawnChildRequest { label: String, node_type: NodeType }`
  - `SpawnChildRequest::new(label) -> Self`（node_type 默认 GeneralPurpose，向后兼容）
  - `SpawnChildRequest::with_node_type(self, NodeType) -> Self`
  - `AgentCoordinator` 新增字段 `registry: Arc<NodeRegistry>`
  - `AgentCoordinator::with_node_registry(self, Arc<NodeRegistry>) -> Self`

- [ ] **Step 3.1: 扩展 `SpawnChildRequest`（coordinator.rs:30-44）**

把现有：
```rust
#[derive(Debug, Clone)]
pub struct SpawnChildRequest {
    pub label: String,
}

impl SpawnChildRequest {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}
```
改为：
```rust
#[derive(Debug, Clone)]
pub struct SpawnChildRequest {
    pub label: String,
    pub node_type: crate::org_graph::NodeType,
}

impl SpawnChildRequest {
    /// 创建 spawn 请求。`node_type` 默认 `GeneralPurpose`，与变更前行为一致。
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            node_type: crate::org_graph::NodeType::default(),
        }
    }

    /// 设置子节点的 node_type。
    #[must_use]
    pub fn with_node_type(mut self, node_type: crate::org_graph::NodeType) -> Self {
        self.node_type = node_type;
        self
    }
}
```

> 所有现有调用点 `SpawnChildRequest::new(...)` 不传 node_type 时行为不变（默认 GP），编译通过。Task 6 再逐个补 `.with_node_type(...)`。

- [ ] **Step 3.2: 给 `AgentCoordinator` 加 `registry` 字段（coordinator.rs:292-315）**

在 struct 定义中 `fallback_used` 之后加一行：
```rust
    fallback_used: Arc<RwLock<HashSet<String>>>,
    registry: Arc<crate::org_graph::NodeRegistry>,
```

- [ ] **Step 3.3: 在 `new()` 中构造默认 registry（coordinator.rs:317-335）**

在 `new()` 的 struct 初始化末尾加：
```rust
            fallback_used: Arc::new(RwLock::new(HashSet::new())),
            registry: Arc::new(crate::org_graph::NodeRegistry::builtin(
                &crate::config::agent::SubagentLimits::default(),
            )),
```

并新增 builder 方法（放在 `with_shutdown_timeout` 旁边，约 :357）：
```rust
    /// 注入从 settings 构建的 NodeRegistry（生产路径用）。
    #[must_use]
    pub fn with_node_registry(mut self, registry: Arc<crate::org_graph::NodeRegistry>) -> Self {
        self.registry = registry;
        self
    }
```

- [ ] **Step 3.4: 在 `src/daemon/state.rs:139-142` 注入真实 settings 构建的 registry**

把现有：
```rust
        let coordinator = Arc::new(crate::agent::AgentCoordinator::new(
            app_state.settings.agent.subagent.max_concurrent,
            app_state.settings.agent.subagent.max_depth,
        ));
```
改为：
```rust
        let registry = Arc::new(crate::org_graph::NodeRegistry::builtin(
            &app_state.settings.agent.subagent,
        ));
        let coordinator = Arc::new(
            crate::agent::AgentCoordinator::new(
                app_state.settings.agent.subagent.max_concurrent,
                app_state.settings.agent.subagent.max_depth,
            )
            .with_node_registry(registry),
        );
```

- [ ] **Step 3.5: 写测试 — SpawnChildRequest 两种构造路径**

在 coordinator.rs 的 test module（`#[cfg(test)]`）中追加：
```rust
    #[test]
    fn spawn_child_request_defaults_to_general_purpose() {
        let req = SpawnChildRequest::new("demo");
        assert_eq!(req.label, "demo");
        assert_eq!(req.node_type, crate::org_graph::NodeType::GeneralPurpose);
    }

    #[test]
    fn spawn_child_request_with_node_type() {
        let req = SpawnChildRequest::new("demo")
            .with_node_type(crate::org_graph::NodeType::Explore);
        assert_eq!(req.node_type, crate::org_graph::NodeType::Explore);
    }

    #[test]
    fn coordinator_holds_registry_with_five_contracts() {
        let coord = AgentCoordinator::new(5, 3);
        for nt in [
            crate::org_graph::NodeType::Explore,
            crate::org_graph::NodeType::Plan,
            crate::org_graph::NodeType::GeneralPurpose,
            crate::org_graph::NodeType::Verification,
            crate::org_graph::NodeType::WgentyCodeGuide,
        ] {
            assert!(coord.registry.get(&nt).is_some(), "coordinator default registry missing {:?}", nt);
        }
    }
```

- [ ] **Step 3.6: 编译 + 运行测试**

Run: `cargo test -p wgenty_code spawn_child_request coordinator_holds_registry -- --nocapture`
Expected: PASS（3 个测试）。全量编译应通过（所有现有 `SpawnChildRequest::new(...)` 调用默认 GP）。

- [ ] **Step 3.7: Commit**

```bash
git add src/agent/coordinator.rs src/daemon/state.rs
git commit -m "feat(coordinator): add node_type to SpawnChildRequest and NodeRegistry to AgentCoordinator"
```

---

## Task 4: coordinator 三维强制校验（tasks.md §4: 4.1, 4.2, 4.3, 4.4）

**关联设计文档：** §6.3 reserve_child 契约校验、§6.4 ContractViolation 错误模型、§2 三维校验分两层

> **设计偏差 #2 在本 task 落地**：`can_spawn` 校验 **caller**（发起 spawn 的节点）的契约，不是 `request.node_type`（子节点）。为此 coordinator 新增 `node_types` 侧表记录每个 agent 的 node_type。详见计划顶部「设计偏差说明」第 2 点。

**Files:**
- Modify: `src/agent/coordinator.rs:234-267`（CoordinatorError 加变体）、`:292-315`（struct 加 node_types 字段）、`:317-335`（new 初始化）、`:417-502`（reserve_child 加校验）
- Modify: `src/tools/meta/task.rs:1151-1169`（map_coordinator_error 加分支）
- Modify: `src/agent/fallback.rs`（加 ContractViolation 不触发 fallback 的断言测试）
- Test: `src/agent/coordinator.rs`、`src/agent/fallback.rs`、`src/tools/meta/task.rs`

**Interfaces:**
- Consumes: `org_graph::{NodeContract, NodeType, ContractDimension, NodeRegistry}`（Task 1-3）
- Produces:
  - `CoordinatorError::ContractViolation { node_type: NodeType, dimension: ContractDimension, reason: String }`
  - coordinator `node_types: Arc<RwLock<HashMap<(SessionId, AgentId), NodeType>>>` 侧表
  - `AgentCoordinator` 内部 `validate_caller_contract(&caller) -> Result<&NodeContract, CoordinatorError>`

- [ ] **Step 4.1: 给 `CoordinatorError` 加 `ContractViolation` 变体（coordinator.rs:234-267）**

在 `RootHasNoTerminalState` 变体之后追加：
```rust
    /// The persistent root is not allowed to enter a terminal lifecycle state.
    #[error("the persistent root has no terminal lifecycle state")]
    RootHasNoTerminalState,
    /// A node contract dimension was violated (capability / permission / budget).
    /// Does NOT trigger structural fallback (see fallback_eligible_from_coordinator_error).
    #[error("contract violation for {node_type:?} node ({dimension}): {reason}")]
    ContractViolation {
        node_type: crate::org_graph::NodeType,
        dimension: crate::org_graph::ContractDimension,
        reason: String,
    },
```

> 需在文件顶部的 `use` 语句中确认 `crate::org_graph::{NodeType, ContractDimension}` 可达（全路径引用也可，避免改 use 块）。

- [ ] **Step 4.2: 给 `AgentCoordinator` 加 `node_types` 侧表字段（coordinator.rs:292-315）**

在 `registry` 字段之后加：
```rust
    registry: Arc<crate::org_graph::NodeRegistry>,
    /// 每个 agent 的 node_type（caller can_spawn 校验用）。root 未登记 → 默认 GP。
    node_types: Arc<RwLock<HashMap<(SessionId, AgentId), crate::org_graph::NodeType>>>,
```

- [ ] **Step 4.3: 在 `new()` 中初始化 `node_types`（coordinator.rs:317-335）**

在 struct 初始化末尾加：
```rust
            registry: Arc::new(crate::org_graph::NodeRegistry::builtin(
                &crate::config::agent::SubagentLimits::default(),
            )),
            node_types: Arc::new(RwLock::new(HashMap::new())),
```

- [ ] **Step 4.4: 写失败测试 — ContractViolation 不触发 fallback**

在 `src/agent/fallback.rs` 的 test module 中追加：
```rust
    #[test]
    fn contract_violation_does_not_trigger_fallback() {
        let err = crate::agent::CoordinatorError::ContractViolation {
            node_type: crate::org_graph::NodeType::Explore,
            dimension: crate::org_graph::ContractDimension::Permission,
            reason: "leaf node cannot spawn".to_string(),
        };
        assert_eq!(
            fallback_eligible_from_coordinator_error(&err),
            None,
            "ContractViolation must NOT be fallback-eligible"
        );
    }
```

Run: `cargo test -p wgenty_code contract_violation_does_not_trigger_fallback -- --nocapture`
Expected: PASS（`_ => None` 已自动覆盖；此测试锁死行为防回归）。

- [ ] **Step 4.5: 在 `reserve_child` 中加契约校验（coordinator.rs:417-502）**

在现有 parent 状态检查（:429-439）**之前**插入契约校验（契约违反优先于 structural 拒绝）。并在 semaphore permit 成功后、返回 reservation 前，把子节点的 node_type 登记进 `node_types`。

把 `reserve_child` 开头改为：
```rust
    pub async fn reserve_child(
        &self,
        caller: &AgentExecutionContext,
        request: SpawnChildRequest,
    ) -> Result<ChildReservation, CoordinatorError> {
        // 1. 契约校验：caller 的 can_spawn（leaf 节点禁止 spawn）。
        let caller_contract = self.caller_contract(caller)?;
        if !caller_contract.permissions.can_spawn {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller_contract.node_type.clone(),
                dimension: crate::org_graph::ContractDimension::Permission,
                reason: "this node type cannot spawn children".to_string(),
            });
        }
        // 2. 校验请求的子节点类型是已知契约。
        if self.registry.get(&request.node_type).is_none() {
            return Err(CoordinatorError::ContractViolation {
                node_type: request.node_type.clone(),
                dimension: crate::org_graph::ContractDimension::NodeType,
                reason: "unknown node type".to_string(),
            });
        }

        // 3. 现有：parent 状态检查（原 :429-439 不变）
        // 4. 现有：depth 检查（用 caller_contract.budget.max_depth 覆盖全局）
        let effective_max_depth = caller_contract
            .budget
            .max_depth
            .unwrap_or(self.max_depth);
        if caller.depth >= effective_max_depth {
            return Err(CoordinatorError::DepthLimitReached {
                limit: effective_max_depth,
            });
        }
        // 5. 现有：semaphore permit（原 :448-454 不变）
        // ... 原有 record insert / scope insert 不变 ...

        // 6. 登记 child 的 node_type（供未来该 child 作 caller 时校验 can_spawn）。
        {
            let mut nt = self.node_types.write().await;
            nt.insert(
                (context.session_id.clone(), context.agent_id.clone()),
                request.node_type.clone(),
            );
        }

        Ok(ChildReservation { context })
    }
```

并新增辅助方法（在 `impl AgentCoordinator` 中，`reserve_child` 附近）：
```rust
    /// 查 caller 的 node_type（root / 未登记 → GeneralPurpose），返回其契约。
    fn caller_contract(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<&crate::org_graph::NodeContract, CoordinatorError> {
        // node_types 是 async RwLock，但 caller 查询在 reserve_child 已 await 完毕后；
        // 为保持本函数同步，用 try_read（不可能被长时间持有）。
        let nt = self
            .node_types
            .try_read()
            .ok()
            .and_then(|m| m.get(&(caller.session_id.clone(), caller.agent_id.clone())).cloned())
            .unwrap_or(crate::org_graph::NodeType::GeneralPurpose);
        self.registry.get(&nt).ok_or_else(|| {
            CoordinatorError::ContractViolation {
                node_type: nt.clone(),
                dimension: crate::org_graph::ContractDimension::NodeType,
                reason: "no contract registered for caller node type".to_string(),
            }
        })
    }
```

> **注意原 depth 检查重复**：原 :441-446 的 `if caller.depth >= self.max_depth` 要替换为上面的 effective_max_depth 版本（删掉原 :441-446，移到契约校验后）。原 parent 状态检查（:429-439）和 semaphore（:448-454）保持原位。

- [ ] **Step 4.6: 给 `map_coordinator_error` 加 ContractViolation 分支（task.rs:1151-1169）**

在现有 `CoordinatorError::NotVisible => ...` 分支之后、`other => ...` 之前插入：
```rust
        CoordinatorError::ContractViolation {
            node_type,
            dimension,
            reason,
        } => ToolError {
            message: format!(
                "Contract violation for {:?} node ({}): {}",
                node_type, dimension, reason
            ),
            code: Some("contract_violation".to_string()),
        },
```

- [ ] **Step 4.7: 写测试 — coordinator 校验各维度**

在 coordinator.rs 的 test module 追加。这些测试用纯 coordinator + root caller（root 默认 GP，can_spawn=true），手动往 `node_types` 登记一个 Explore caller 来测 leaf 拒绝。

```rust
    use crate::agent::identity::{AgentExecutionContext, SessionId};
    use crate::org_graph::NodeType;

    #[tokio::test]
    async fn leaf_caller_cannot_spawn() {
        let coord = AgentCoordinator::new(5, 3);
        let root = AgentExecutionContext::root(SessionId::new());
        // 先正常 spawn 一个 explore child（root=GP can_spawn=true）
        let child = coord
            .reserve_child(
                &root,
                SpawnChildRequest::new("explore job")
                    .with_node_type(NodeType::Explore),
            )
            .await
            .expect("root GP can spawn explore child");
        // 现在 child（Explore, leaf）尝试 spawn → 应被 ContractViolation 拒绝
        let err = coord
            .reserve_child(&child.context, SpawnChildRequest::new("grandchild"))
            .await
            .expect_err("leaf explore must not spawn");
        match err {
            CoordinatorError::ContractViolation { dimension, .. } => {
                assert_eq!(dimension, crate::org_graph::ContractDimension::Permission);
            }
            other => panic!("expected ContractViolation, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn general_purpose_caller_can_spawn() {
        let coord = AgentCoordinator::new(5, 3);
        let root = AgentExecutionContext::root(SessionId::new());
        let child = coord
            .reserve_child(
                &root,
                SpawnChildRequest::new("gp job")
                    .with_node_type(NodeType::GeneralPurpose),
            )
            .await
            .expect("root can spawn GP child");
        let grandchild = coord
            .reserve_child(&child.context, SpawnChildRequest::new("grandchild"))
            .await;
        assert!(grandchild.is_ok(), "GP child can spawn: {:?}", grandchild.err());
    }

    #[tokio::test]
    async fn budget_some_overrides_global_max_depth() {
        // 构造一个 contract.budget.max_depth = Some(0) 的 caller → depth 检查立即拒绝。
        let coord = AgentCoordinator::new(5, 10);
        let root = AgentExecutionContext::root(SessionId::new()); // root.depth = 0
        // root 默认 GP，budget 全 None → 用全局 max_depth=10，可 spawn。
        let child = coord
            .reserve_child(&root, SpawnChildRequest::new("first"))
            .await
            .expect("root spawns at depth<10");
        // 手动覆盖 child 的契约：注入一个 budget.max_depth=Some(0) 的 GP。
        // （通过插入一个自定义 registry 实现——或直接测 None 回退路径。）
        // 此处验证 None 回退：child 是 GP（budget None），全局 max_depth=10，
        // child.depth=1 < 10 → 可继续 spawn。
        let grandchild = coord
            .reserve_child(&child.context, SpawnChildRequest::new("second"))
            .await;
        assert!(grandchild.is_ok(), "depth 1 < global 10 should pass: {:?}", grandchild.err());
    }

    #[tokio::test]
    async fn depth_limit_uses_global_when_budget_none() {
        let coord = AgentCoordinator::new(5, 1); // max_depth = 1
        let root = AgentExecutionContext::root(SessionId::new()); // depth 0
        let child = coord
            .reserve_child(&root, SpawnChildRequest::new("depth0->1"))
            .await
            .expect("depth 0 < 1");
        // child.depth = 1, max_depth = 1 → 1 >= 1 → DepthLimitReached
        let err = coord
            .reserve_child(&child.context, SpawnChildRequest::new("too deep"))
            .await
            .expect_err("should hit depth limit");
        assert!(matches!(err, CoordinatorError::DepthLimitReached { limit: 1 }));
    }
```

> **注意**：`budget Some 覆盖全局」的完整端到端测试需要注入自定义 registry（构造一个 budget.max_depth=Some(0) 的契约）。若 `NodeRegistry` 没有公开插入接口，可跳过 Some 覆盖的集成测试，改为在 `caller_contract` / depth 解析逻辑上写纯函数单测（把 `resolve_effective_max_depth(contract_budget, global)` 提取为自由函数单独测）。**推荐**：提取 `fn resolve_effective_max_depth(budget_max_depth: Option<usize>, global_max_depth: usize) -> usize { budget_max_depth.unwrap_or(global_max_depth) }` 并单独单测 None/Some 两路径。

- [ ] **Step 4.8: 运行测试**

Run: `cargo test -p wgenty_code coordinator leaf_caller general_purpose_caller depth_limit contract_violation -- --nocapture`
Expected: PASS。

- [ ] **Step 4.9: Commit**

```bash
git add src/agent/coordinator.rs src/agent/fallback.rs src/tools/meta/task.rs
git commit -m "feat(coordinator): enforce can_spawn + budget contract in reserve_child, add ContractViolation error"
```

---

## Task 5: task.rs 读契约（tasks.md §5: 5.1, 5.2, 5.3, 5.4）

**关联设计文档：** §7 task.rs 迁移（7.1 execute_with_context、7.2 parse_node_type、7.3 filter_allowed_tools）

**Files:**
- Modify: `src/tools/meta/task.rs:475`（parse node_type）、`:587-595`（filter_allowed_tools 调用）、`:597-660`（match 改读 contract.system_prompt）、`:704`（SpawnChildRequest 加 with_node_type）、`:1121-1144`（filter_allowed_tools 签名 + 实现）、新增 `parse_node_type` 函数
- Test: `src/tools/meta/task.rs`（`#[cfg(test)] mod tests`，或 `src/tools/meta/task/tests.rs`）

**Interfaces:**
- Consumes: `org_graph::{NodeType, NodeContract, NodeRegistry}`（Task 1-2）；`AgentCoordinator::reserve_child`（Task 4 已扩展）
- Produces:
  - `task::parse_node_type(&str) -> NodeType`（私有/crate 内函数）
  - `task::filter_allowed_tools(names, &NodeContract) -> Vec<String>`（签名从 5 参数改为 2 参数）

- [ ] **Step 5.1: 写失败测试 — parse_node_type 映射**

在 task.rs 的 test module（或 `src/tools/meta/task/tests.rs`）追加：
```rust
    #[test]
    fn parse_node_type_maps_known_strings() {
        use crate::org_graph::NodeType;
        assert_eq!(parse_node_type("explore"), NodeType::Explore);
        assert_eq!(parse_node_type("plan"), NodeType::Plan);
        assert_eq!(parse_node_type("general-purpose"), NodeType::GeneralPurpose);
        assert_eq!(parse_node_type("general"), NodeType::GeneralPurpose);
        assert_eq!(parse_node_type("verify"), NodeType::Verification);
        assert_eq!(parse_node_type("verification"), NodeType::Verification);
        assert_eq!(parse_node_type("guide"), NodeType::WgentyCodeGuide);
        assert_eq!(parse_node_type("wgenty-code-guide"), NodeType::WgentyCodeGuide);
    }

    #[test]
    fn parse_node_type_unknown_defaults_to_general_purpose() {
        assert_eq!(parse_node_type("nonexistent"), parse_node_type("general-purpose"));
    }
```

- [ ] **Step 5.2: 实现 `parse_node_type`（task.rs，放在 `filter_allowed_tools` 附近）**

```rust
/// 模型 JSON 的 `subagent_type` 字符串 → 可信 `NodeType` 枚举。
/// 照搬 cli/args.rs:729-734 的映射语义，未知字符串默认 GeneralPurpose
/// （与现有 `_subagent_type.unwrap_or("general-purpose")` 行为一致）。
fn parse_node_type(s: &str) -> crate::org_graph::NodeType {
    use crate::org_graph::NodeType;
    match s {
        "explore" => NodeType::Explore,
        "plan" => NodeType::Plan,
        "general-purpose" | "general" => NodeType::GeneralPurpose,
        "verify" | "verification" => NodeType::Verification,
        "guide" | "wgenty-code-guide" => NodeType::WgentyCodeGuide,
        _ => NodeType::GeneralPurpose,
    }
}
```

Run: `cargo test -p wgenty_code parse_node_type -- --nocapture`
Expected: PASS。

- [ ] **Step 5.3: 改 `filter_allowed_tools` 签名 + 实现读契约（task.rs:1121-1144）**

把现有：
```rust
pub(crate) fn filter_allowed_tools(
    names: impl IntoIterator<Item = String>,
    subagent_type: &str,
    _depth: usize,
    _max_depth: usize,
    explore_readonly: bool,
) -> Vec<String> {
    let is_leaf = matches!(subagent_type, "explore" | "plan");
    names.into_iter().filter(|name| {
        let is_spawn = name == "task" || name == "delegate";
        if is_spawn { return !is_leaf; }
        if explore_readonly && is_leaf && MUTATING_FS_TOOLS.contains(&name.as_str()) {
            return false;
        }
        true
    }).collect()
}
```
改为：
```rust
/// 按契约三重过滤工具集：
/// ① capability 白名单（空 Vec = 通配，全通过）；
/// ② can_spawn=false → 剥离 task/delegate；
/// ③ can_mutate_fs=false → 剥离 MUTATING_FS_TOOLS。
pub(crate) fn filter_allowed_tools(
    names: impl IntoIterator<Item = String>,
    contract: &crate::org_graph::NodeContract,
) -> Vec<String> {
    names.into_iter().filter(|name| {
        // 维度 1: capability 白名单（空 = 通配符）
        if !contract.capabilities.allowed_tools.is_empty()
            && !contract.capabilities.allowed_tools.contains(name)
        {
            return false;
        }
        // 维度 2: can_spawn
        let is_spawn = name == "task" || name == "delegate";
        if is_spawn && !contract.permissions.can_spawn {
            return false;
        }
        // 维度 3: can_mutate_fs
        if MUTATING_FS_TOOLS.contains(&name.as_str())
            && !contract.permissions.can_mutate_fs
        {
            return false;
        }
        true
    }).collect()
}
```

> `MUTATING_FS_TOOLS` 常量（:1109-1110）不变。

- [ ] **Step 5.4: 写失败测试 — filter_allowed_tools 读契约**

在 task.rs test module 追加：
```rust
    #[test]
    fn filter_strips_spawn_when_cannot_spawn() {
        use crate::org_graph::{Capability, NodeContract, NodeType, PermissionBoundary, ResourceBudget, IoShape};
        let contract = NodeContract {
            node_type: NodeType::Explore,
            name: "explore".into(), description: "".into(), when_to_use: "".into(),
            system_prompt: "".into(), model: "".into(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary { can_spawn: false, can_mutate_fs: true, can_exec: true },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText, output_type: IoShape::FreeText,
        };
        let names = vec!["search".to_string(), "task".to_string(), "delegate".to_string()];
        let out = filter_allowed_tools(names, &contract);
        assert!(out.contains(&"search".to_string()));
        assert!(!out.contains(&"task".to_string()), "task stripped when can_spawn=false");
        assert!(!out.contains(&"delegate".to_string()));
    }

    #[test]
    fn filter_strips_mutating_fs_when_cannot_mutate() {
        use crate::org_graph::{Capability, NodeContract, NodeType, PermissionBoundary, ResourceBudget, IoShape};
        let contract = NodeContract {
            node_type: NodeType::Explore,
            name: "explore".into(), description: "".into(), when_to_use: "".into(),
            system_prompt: "".into(), model: "".into(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary { can_spawn: false, can_mutate_fs: false, can_exec: true },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText, output_type: IoShape::FreeText,
        };
        let names = vec![
            "file_read".to_string(), "file_write".to_string(),
            "file_edit".to_string(), "apply_patch".to_string(),
        ];
        let out = filter_allowed_tools(names, &contract);
        assert!(out.contains(&"file_read".to_string()));
        assert!(!out.contains(&"file_write".to_string()));
        assert!(!out.contains(&"file_edit".to_string()));
        assert!(!out.contains(&"apply_patch".to_string()));
    }

    #[test]
    fn filter_whitelist_when_nonempty() {
        use crate::org_graph::{Capability, NodeContract, NodeType, PermissionBoundary, ResourceBudget, IoShape};
        let contract = NodeContract {
            node_type: NodeType::Explore,
            name: "explore".into(), description: "".into(), when_to_use: "".into(),
            system_prompt: "".into(), model: "".into(),
            capabilities: Capability { allowed_tools: vec!["search".to_string()] },
            permissions: PermissionBoundary { can_spawn: true, can_mutate_fs: true, can_exec: true },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText, output_type: IoShape::FreeText,
        };
        let names = vec!["search".to_string(), "file_read".to_string()];
        let out = filter_allowed_tools(names, &contract);
        assert_eq!(out, vec!["search".to_string()]);
    }
```

Run: `cargo test -p wgenty_code filter_strips filter_whitelist -- --nocapture`
Expected: PASS。

- [ ] **Step 5.5: 改 `execute_with_context` 读契约（task.rs:475, 587-660, 704）**

**5.5a — 解析 node_type（:475 附近）**

在现有 `let _subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");` 之后加：
```rust
        let _subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");
        let node_type = parse_node_type(_subagent_type);
        let contract = self.coordinator.registry.get(&node_type).ok_or_else(|| {
            ToolError {
                message: format!("no NodeContract registered for {:?}", node_type),
                code: Some("unknown_node_type".to_string()),
            }
        })?;
```

> `_subagent_type` 字符串变量保留（其他地方如日志/进度记录可能引用，见 task.rs:558/811/1084）。

**5.5b — filter_allowed_tools 调用改读契约（:587-595）**

把现有：
```rust
        let depth = context.agent.depth;
        let explore_readonly = self.settings.agent.subagent.explore_readonly;
        let allowed_tools: Vec<String> = filter_allowed_tools(
            tool_registry.list().iter().map(|t| t.name().to_string()),
            _subagent_type,
            depth,
            self.settings.agent.subagent.max_depth,
            explore_readonly,
        );
```
改为：
```rust
        let allowed_tools: Vec<String> = filter_allowed_tools(
            tool_registry.list().iter().map(|t| t.name().to_string()),
            contract,
        );
```

> `depth` / `explore_readonly` 局部变量若后续无其他引用则一并删除；若有其他引用（日志等）则保留。`filter_allowed_tools` 不再需要这两个参数（depth 本来就 unused，explore_readonly 已进契约）。

**5.5c — match 改读 contract.system_prompt（:597-660）**

把现有 `let base_system_prompt: &str = match _subagent_type { ... };`（整个 match 块 :597-655）替换为：
```rust
        let base_system_prompt: &str = contract.system_prompt.as_str();
```

> comet_prefix 拼接逻辑（:656-660）不变：`let system_prompt = if let Some(ref prefix) = comet_prefix { format!("{}{}", prefix, base_system_prompt) } else { base_system_prompt.to_string() };`

**5.5d — SpawnChildRequest 加 with_node_type（:704）**

把现有：
```rust
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description),
                group_id.clone(),
            )
```
改为：
```rust
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description).with_node_type(node_type.clone()),
                group_id.clone(),
            )
```

- [ ] **Step 5.6: 编译 + 修复 breakage**

Run: `cargo build -p wgenty_code`
Expected: 编译通过。`_subagent_type` 若产生 unused warning，检查是否仍有引用（日志/进度记录）；若无引用改为 `_` 或删除。若 clippy 报 unused variable，加 `#[allow(unused_variables)]` 或删除变量。

- [ ] **Step 5.7: 写无回归测试 — explore/plan/GP 工具集与变更前一致**

这是 D7 硬约束的核心断言。策略：**先在改之前快照当前 filter_allowed_tools 的输出**（如已有 baseline 跳过），改之后用相同输入跑新 filter_allowed_tools 比对。

在 task.rs test module 追加：
```rust
    #[test]
    fn no_regression_filter_allowed_tools_explore_matches_old_semantics() {
        // 验证：explore 契约（can_spawn=false, can_mutate_fs=false when readonly）
        // 对任意工具集的过滤结果，与旧硬编码逻辑（is_leaf + explore_readonly）逐字节一致。
        use crate::org_graph::{Capability, NodeContract, NodeType, PermissionBoundary, ResourceBudget, IoShape};
        let sample_tools = vec![
            "search".to_string(), "file_read".to_string(), "file_write".to_string(),
            "file_edit".to_string(), "apply_patch".to_string(),
            "task".to_string(), "delegate".to_string(),
            "exec_command".to_string(), "list_files".to_string(), "grep".to_string(),
        ];
        // explore_readonly = true 的 explore 契约
        let contract = NodeContract {
            node_type: NodeType::Explore,
            name: "explore".into(), description: "".into(), when_to_use: "".into(),
            system_prompt: "".into(), model: "".into(),
            capabilities: Capability { allowed_tools: vec![] }, // 通配
            permissions: PermissionBoundary { can_spawn: false, can_mutate_fs: false, can_exec: true },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText, output_type: IoShape::FreeText,
        };
        let out = filter_allowed_tools(sample_tools.clone(), &contract);
        // 旧逻辑：is_leaf=true → 剥 task/delegate；explore_readonly=true → 剥 mutating fs
        // 保留：search, file_read, exec_command, list_files, grep
        assert!(out.contains(&"search".to_string()));
        assert!(out.contains(&"file_read".to_string()));
        assert!(out.contains(&"exec_command".to_string()));
        assert!(out.contains(&"list_files".to_string()));
        assert!(out.contains(&"grep".to_string()));
        assert!(!out.contains(&"task".to_string()));
        assert!(!out.contains(&"delegate".to_string()));
        assert!(!out.contains(&"file_write".to_string()));
        assert!(!out.contains(&"file_edit".to_string()));
        assert!(!out.contains(&"apply_patch".to_string()));
    }

    #[test]
    fn no_regression_general_purpose_keeps_all_tools() {
        use crate::org_graph::{Capability, NodeContract, NodeType, PermissionBoundary, ResourceBudget, IoShape};
        let sample_tools = vec![
            "search".to_string(), "task".to_string(), "file_write".to_string(),
        ];
        let contract = NodeContract {
            node_type: NodeType::GeneralPurpose,
            name: "gp".into(), description: "".into(), when_to_use: "".into(),
            system_prompt: "".into(), model: "".into(),
            capabilities: Capability { allowed_tools: vec![] },
            permissions: PermissionBoundary { can_spawn: true, can_mutate_fs: true, can_exec: true },
            budget: ResourceBudget::default(),
            input_type: IoShape::FreeText, output_type: IoShape::FreeText,
        };
        let out = filter_allowed_tools(sample_tools, &contract);
        assert_eq!(out.len(), 3, "GP keeps all tools (spawn + mutate allowed)");
    }
```

- [ ] **Step 5.8: 运行全量测试验证无回归**

Run: `cargo test -p wgenty_code --no-fail-fast`
Expected: 全部 PASS。重点关注 task.rs 既有 subagent 测试无回归。

- [ ] **Step 5.9: Commit**

```bash
git add src/tools/meta/task.rs
git commit -m "refactor(task): read NodeContract instead of hardcoded match; filter_allowed_tools reads contract"
```

---

## Task 6: 其余调用点补 node_type（tasks.md §6: 6.1, 6.2, 6.3）

**关联设计文档：** §7.4 RLM 调用点、§9 风险 SpawnChildRequest 多调用点

> tasks.md 6.1 排查的列表中，fallback.rs 和 daemon/handlers.rs 的 `reserve_child` 调用全部在 `#[cfg(test)]` 模块内（非生产代码），生产调用点共 4 处：`run_script.rs:118`、`task.rs:702`（Task 5 已改）、`rlm/pipeline.rs:276`、`rlm/pipeline.rs:650`。

**Files:**
- Modify: `src/tools/meta/run_script.rs:118`
- Modify: `src/tools/meta/rlm/pipeline.rs:276` 和 `:650`
- Test: 编译通过 + 现有测试无回归（这些调用点显式写 GP = 默认值，行为不变）

**Interfaces:**
- Consumes: `SpawnChildRequest::with_node_type`（Task 3）
- Produces: 无新接口

- [ ] **Step 6.1: `src/tools/meta/run_script.rs:118` 补 with_node_type**

把现有：
```rust
                let reservation = match coordinator
                    .reserve_child(&caller, crate::agent::SpawnChildRequest::new(&prompt))
                    .await
```
改为：
```rust
                let reservation = match coordinator
                    .reserve_child(
                        &caller,
                        crate::agent::SpawnChildRequest::new(&prompt)
                            .with_node_type(crate::org_graph::NodeType::GeneralPurpose),
                    )
                    .await
```

- [ ] **Step 6.2: `src/tools/meta/rlm/pipeline.rs:276` 补 with_node_type**

把现有：
```rust
            let reservation = match coordinator
                .reserve_child(caller, crate::agent::SpawnChildRequest::new(&prompt))
                .await
```
改为：
```rust
            let reservation = match coordinator
                .reserve_child(
                    caller,
                    crate::agent::SpawnChildRequest::new(&prompt)
                        .with_node_type(crate::org_graph::NodeType::GeneralPurpose),
                )
                .await
```

> **OQ1 决策落实**：RLM 子任务统一走 `GeneralPurpose`（= 默认值）。`use_small_model` 留 RLM 自己管，不进 NodeContract。理由：RLM 子任务是通用执行单元，不套 explore/plan 的 leaf 约束；显式写 GP 表达意图清晰。

- [ ] **Step 6.3: `src/tools/meta/rlm/pipeline.rs:650`（replan 路径）补 with_node_type**

把现有：
```rust
                let reservation = match coordinator
                    .reserve_child(caller, crate::agent::SpawnChildRequest::new(&prompt))
                    .await
```
改为（与 Step 6.2 相同的 `.with_node_type(GeneralPurpose)`）。两处 replan/主路径一致。

- [ ] **Step 6.4: 确认 ContractViolation 回退策略（OQ2）**

`fallback_eligible_from_coordinator_error`（fallback.rs:26-33）的 `_ => None` 已自动覆盖 `ContractViolation`（Task 4 Step 4.4 已加测试锁死）。确认：

1. `src/tools/meta/task.rs:712-713`：`if fallback_eligible_from_coordinator_error(&e).is_some()` —— ContractViolation 返回 None → 走 `map_coordinator_error(e)` 原始错误路径（:726），不触发 structural fallback。✓
2. `src/tools/meta/rlm/pipeline.rs:289`：`if fallback_eligible_from_coordinator_error(&e).is_none()` —— ContractViolation 返回 None → `is_none()` 为 true → 走 `task_errors[idx] = ...; continue;`（:290-291），不触发 fallback。✓
3. `src/tools/meta/rlm/pipeline.rs:654-663`（replan）：reserve_child 失败时 `continue`（:662），不触发 fallback。✓

无需改 fallback.rs 生产代码（`_ => None` 天然安全），测试已在 Task 4 Step 4.4 覆盖。

- [ ] **Step 6.5: 编译 + 全量测试**

Run: `cargo build -p wgenty_code && cargo test -p wgenty_code --no-fail-fast`
Expected: 编译通过，全部测试 PASS。

- [ ] **Step 6.6: Commit**

```bash
git add src/tools/meta/run_script.rs src/tools/meta/rlm/pipeline.rs
git commit -m "feat(rlm,run_script): annotate spawn requests with GeneralPurpose node_type"
```

---

## Task 7: AgentDefinition 并存验证（tasks.md §7: 7.1, 7.2）

**关联设计文档：** §9 风险 AgentDefinition 两层并存

**Files:**
- Test: 无新文件（验证性 task，确认 grep 结果 + 现有 CLI 测试通过）

**Interfaces:**
- Consumes: 无
- Produces: 无（验证不破坏现有路径）

- [ ] **Step 7.1: 确认新派发路径不引用 AgentDefinition/AgentsService**

Run: `grep -rn 'AgentDefinition\|AgentsService' src/tools/meta/task.rs src/agent/coordinator.rs src/org_graph/`
Expected: **无输出**（新路径只读 NodeContract，不读 AgentDefinition）。若有输出，说明误引入了依赖，需移除。

- [ ] **Step 7.2: 确认 CLI run_agent 路径仍走 AgentsService**

Run: `grep -n 'AgentsService\|execute_agent\|AgentDefinition' src/cli/args.rs`
Expected: 命中 `run_agent`（:720+）中的 `AgentsService` 调用——CLI 路径未改，继续走旧路径。

- [ ] **Step 7.3: 确认 AgentDefinition/AgentsService 源码未被删除/改动**

Run: `git diff main -- src/teams/subagent.rs | head -20`
Expected: 空输出（未改动）。

- [ ] **Step 7.4: 运行 CLI + stress 相关测试**

Run: `cargo test -p wgenty_code teams::subagent -- --nocapture`
Expected: 现有 AgentDefinition/AgentsService 测试全 PASS（未被破坏）。

> 若 stress_tests 有独立测试 target，一并运行：`cargo test -p wgenty_code --test stress_tests`（若存在）。

- [ ] **Step 7.5: Commit（若本 task 仅验证无改动则跳过 commit）**

若 grep/测试全部通过且无源码改动，本 task 无需 commit。记录验证结果到 tasks.md 打勾即可。

---

## Task 8: 验证与收尾（tasks.md §8: 8.1, 8.2, 8.3）

**关联设计文档：** §8 测试策略（全量表）

**Files:**
- 无新文件（全量验证 task）

- [ ] **Step 8.1: cargo fmt --check**

Run: `cargo fmt --check`
Expected: 无输出（格式合规）。若有 diff，运行 `cargo fmt` 修复后重新检查。

- [ ] **Step 8.2: cargo clippy 零 warning**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 零 warning，零 error。常见修复：
- unused imports（`use` 语句清理）
- unused variables（`_subagent_type` 若无引用改 `_` 或删除）
- too many arguments（`filter_allowed_tools` 已从 5 参数降到 2 参数，应已解决）

若 clippy 报错，加载 `superpowers:systematic-debugging` skill 定位根因，不得用 `#[allow]` 压制 warning（除非有充分理由并注释说明）。

- [ ] **Step 8.3: cargo test 全量通过**

Run: `cargo test -p wgenty_code --no-fail-fast`
Expected: 全部 PASS。重点检查：
- `org_graph::contract::tests`（Task 1）
- `org_graph::registry::tests`（Task 2）
- coordinator `leaf_caller` / `general_purpose_caller` / `depth_limit` / `contract_violation`（Task 4）
- task.rs `parse_node_type` / `filter_strips` / `no_regression`（Task 5）
- 现有 subagent / fallback / coordinator / rlm 测试无回归
- `teams::subagent` 测试无回归（Task 7）

- [ ] **Step 8.4: 手动验证真实派发（tasks.md 8.3）**

启动 daemon 或 CLI，执行一次 explore 节点派发和一次 general-purpose 节点派发，确认：
1. system_prompt 来自契约（explore 用 explore_prompt，GP 用 gp_prompt）。
2. explore 工具集不含 task/delegate（can_spawn=false）；explore_readonly 时不含 mutating fs。
3. general-purpose 工具集含 task/delegate + 全工具。
4. budget 来自全局 SubagentLimits（内置契约 budget 全 None）。

> 若 daemon 启动复杂，可用现有集成测试替代（确认 coordinator 用 daemon/state.rs 注入的真实 registry）。

- [ ] **Step 8.5: 更新 tasks.md 打勾 + 最终 commit**

把 `openspec/changes/org-graph-node-contract/tasks.md` 中 8.1/8.2/8.3 打勾。若有未提交的 fmt/clippy 修复：
```bash
git add -A
git commit -m "chore(org_graph): fmt + clippy fixes from final verification"
```

---

## Self-Review 校验（实施完成后由协调者执行）

对照 spec 的每个 Requirement / Scenario，确认有 task 覆盖：

| Spec Requirement | 覆盖 Task |
|---|---|
| NodeContract 建模（serde 往返） | Task 1（Step 1.4） |
| NodeRegistry 查询（5 存在/未知 None） | Task 2（Step 2.3） |
| SpawnChildRequest 携带 node_type（默认 GP） | Task 3（Step 3.1, 3.5） |
| coordinator 强制校验三维（can_spawn + budget） | Task 4（Step 4.5, 4.7） |
| ContractViolation 不触发 fallback | Task 4（Step 4.4, 4.6） |
| budget None 回退 / Some 覆盖 | Task 4（Step 4.5, 4.7） |
| IO schema 声明态不校验 | Task 1（字段声明，无校验代码 = 满足） |
| task.rs 读契约（system_prompt/tools/budget） | Task 5（Step 5.5） |
| filter_allowed_tools 三重过滤 | Task 5（Step 5.3, 5.4） |
| parse_node_type 映射 | Task 5（Step 5.1, 5.2） |
| 合法派发无回归 | Task 5（Step 5.7, 5.8） + Task 8 |
| AgentDefinition 并存不破坏 | Task 7 |
| 能力越纲被 task.rs 拒绝 | Task 5（Step 5.4 filter_whitelist） |
| can_mutate_fs 被 task.rs 拒绝 | Task 5（Step 5.4 filter_strips_mutating_fs） |

**类型一致性检查**：`NodeType` / `NodeContract` / `ContractDimension` / `NodeRegistry` / `filter_allowed_tools` / `parse_node_type` / `SpawnChildRequest::with_node_type` 在 Task 1-5 中的签名逐字一致。`CoordinatorError::ContractViolation` 字段名（`node_type`/`dimension`/`reason`）在 Task 4 定义处与 Task 4 测试 + Task 4 map_coordinator_error 分支一致。
