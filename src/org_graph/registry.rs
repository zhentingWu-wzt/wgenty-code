//! 内置节点契约注册表。构建后不可变（纯数据）。
//!
//! 五个内置契约照搬 `task.rs` 硬编码 match + `filter_allowed_tools` 的现有语义
//! （设计 D7：无回归）。`explore_readonly` 全局配置在构建时驱动 `can_mutate_fs`。

use std::collections::HashMap;

use crate::config::agent::SubagentLimits;
use crate::org_graph::contract::{
    Capability, IoShape, NodeContract, NodeType, PermissionBoundary, ResourceBudget,
};

pub struct NodeRegistry {
    contracts: HashMap<NodeType, NodeContract>,
}

/// 渲染用的稳定枚举顺序（枚举声明序）。HashMap 遍历无序，渲染/测试要求确定性。
const CANONICAL_ORDER: [NodeType; 5] = [
    NodeType::Explore,
    NodeType::Plan,
    NodeType::GeneralPurpose,
    NodeType::Verification,
    NodeType::WgentyCodeGuide,
];

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

    /// 查询节点契约。未知节点类型返回 `None`。
    pub fn get(&self, node_type: &NodeType) -> Option<&NodeContract> {
        self.contracts.get(node_type)
    }

    /// 按稳定顺序（CANONICAL_ORDER）返回全部契约，用于确定性渲染。
    /// 未来若有缺项（自定义契约未注册），自动跳过。
    pub fn iter(&self) -> Vec<&NodeContract> {
        CANONICAL_ORDER
            .iter()
            .filter_map(|nt| self.contracts.get(nt))
            .collect()
    }

    fn explore_contract(s: &SubagentLimits) -> NodeContract {
        NodeContract {
            node_type: NodeType::Explore,
            name: "explore".to_string(),
            description: "Read-only code exploration subagent.".to_string(),
            when_to_use: "Searching and analyzing codebases".to_string(),
            system_prompt: Self::explore_prompt().to_string(),
            model: "default".to_string(),
            capabilities: Capability {
                allowed_tools: vec![],
            },
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
            capabilities: Capability {
                allowed_tools: vec![],
            },
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
            capabilities: Capability {
                allowed_tools: vec![],
            },
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
            // task.rs 硬编码 match 没有 verify arm（CLI 路径走 AgentDefinition）。
            system_prompt: String::new(),
            model: "default".to_string(),
            capabilities: Capability {
                allowed_tools: vec![],
            },
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
            // task.rs 硬编码 match 没有 guide arm（CLI 路径走 AgentDefinition）。
            system_prompt: String::new(),
            model: "default".to_string(),
            capabilities: Capability {
                allowed_tools: vec![],
            },
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
        "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a planning subagent. Your role is to break down \
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

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(readonly: bool) -> NodeRegistry {
        let s = SubagentLimits {
            explore_readonly: readonly,
            ..Default::default()
        };
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
    fn builtin_system_prompts_retain_distinct_opening_phrases() {
        // Byte-identity guard: the system prompts were migrated verbatim from
        // the old task.rs match block. Each builtin the task-tool dispatch path
        // can emit (explore/plan/GP) must retain its distinguishing opening
        // phrase so the migration never silently drifts. verify/guide are
        // CLI-path-only (empty prompt by design; see comments in `builtin`).
        let r = registry(true);
        assert!(
            r.get(&NodeType::Explore)
                .unwrap()
                .system_prompt
                .contains("code exploration subagent"),
            "explore prompt lost its opening phrase"
        );
        assert!(
            r.get(&NodeType::Plan)
                .unwrap()
                .system_prompt
                .contains("planning subagent"),
            "plan prompt lost its opening phrase"
        );
        assert!(
            r.get(&NodeType::GeneralPurpose)
                .unwrap()
                .system_prompt
                .contains("general-purpose subagent spawned by a coordinator"),
            "general-purpose prompt lost its opening phrase"
        );
    }

    #[test]
    fn explore_is_leaf_and_readonly_when_explore_readonly() {
        let r = registry(true);
        let c = r.get(&NodeType::Explore).unwrap();
        assert!(!c.permissions.can_spawn);
        assert!(
            !c.permissions.can_mutate_fs,
            "explore_readonly=true => can_mutate_fs=false"
        );
    }

    #[test]
    fn explore_can_mutate_when_not_readonly() {
        let r = registry(false);
        let c = r.get(&NodeType::Explore).unwrap();
        assert!(
            c.permissions.can_mutate_fs,
            "explore_readonly=false => can_mutate_fs=true"
        );
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
            assert_eq!(
                c.budget,
                ResourceBudget::default(),
                "{:?} budget not all-None",
                nt
            );
        }
    }

    #[test]
    fn explore_plan_gp_system_prompts_nonempty() {
        let r = registry(true);
        assert!(!r.get(&NodeType::Explore).unwrap().system_prompt.is_empty());
        assert!(!r.get(&NodeType::Plan).unwrap().system_prompt.is_empty());
        assert!(!r
            .get(&NodeType::GeneralPurpose)
            .unwrap()
            .system_prompt
            .is_empty());
    }

    #[test]
    fn verify_guide_system_prompts_empty() {
        // task.rs 硬编码 match 没有 verify/guide arm；这两个契约仅作声明。
        let r = registry(true);
        assert!(r
            .get(&NodeType::Verification)
            .unwrap()
            .system_prompt
            .is_empty());
        assert!(r
            .get(&NodeType::WgentyCodeGuide)
            .unwrap()
            .system_prompt
            .is_empty());
    }

    #[test]
    fn builtin_capabilities_all_wildcard() {
        // 内置契约 allowed_tools 均为空（通配符），照搬现有「无正向白名单」语义。
        let r = registry(true);
        for nt in [
            NodeType::Explore,
            NodeType::Plan,
            NodeType::GeneralPurpose,
            NodeType::Verification,
            NodeType::WgentyCodeGuide,
        ] {
            let c = r.get(&nt).unwrap();
            assert!(
                c.capabilities.allowed_tools.is_empty(),
                "{:?} should have wildcard (empty) allowed_tools",
                nt
            );
        }
    }

    #[test]
    fn iter_returns_all_five_in_canonical_order() {
        let r = registry(true);
        let ordered: Vec<NodeType> = r.iter().into_iter().map(|c| c.node_type.clone()).collect();
        assert_eq!(
            ordered,
            vec![
                NodeType::Explore,
                NodeType::Plan,
                NodeType::GeneralPurpose,
                NodeType::Verification,
                NodeType::WgentyCodeGuide,
            ]
        );
    }

    #[test]
    fn iter_consistent_with_get() {
        let r = registry(true);
        let collected: Vec<&NodeContract> = r.iter();
        assert_eq!(collected.len(), 5, "iter returns all five builtins");
        for c in r.iter() {
            assert_eq!(
                c, r.get(&c.node_type).unwrap(),
                "iter entry must match get() for {:?}",
                c.node_type
            );
        }
    }
}
