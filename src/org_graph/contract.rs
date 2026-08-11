//! Org-Graph 节点契约：纯数据 + 纯函数校验模块的类型层。
//!
//! 每个 agent 节点类型用一张 [`NodeContract`] 声明五维约束（能力、权限边界、
//! 资源预算、IO 形状、身份/谱系）。本模块无 async、无 I/O、无状态。

use serde::{Deserialize, Serialize};

/// 节点类型枚举。模型输出的 `subagent_type` 字符串经派发层映射为此可信枚举。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeType {
    Explore,
    Plan,
    #[default]
    GeneralPurpose,
    Verification,
    RootCause,
    WgentyCodeGuide,
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
    /// 字段级状态访问越权（WorkState 字段读写 API 强制）。
    State,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodetype_default_is_general_purpose() {
        assert_eq!(NodeType::default(), NodeType::GeneralPurpose);
    }

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
    fn ioshape_default_is_free_text() {
        assert_eq!(IoShape::default(), IoShape::FreeText);
    }

    #[test]
    fn contract_dimension_serde_roundtrip() {
        for dim in [
            ContractDimension::NodeType,
            ContractDimension::Capability,
            ContractDimension::Permission,
            ContractDimension::Budget,
            ContractDimension::State,
        ] {
            let json = serde_json::to_string(&dim).expect("serialize");
            let back: ContractDimension = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(dim, back);
        }
    }
}
