//! Code-owned dynamic construction of bounded Work-Graph instances.
//!
//! A plan is selected from registered templates using structured task facts.
//! It never accepts model-provided node types or arbitrary edges.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{NodeRegistry, NodeType};

/// Structured category supplied by a trusted caller at node creation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkGraphTaskKind {
    /// A normal repository implementation with deterministic verification.
    Implementation,
    /// Diagnosis work that must produce a RootCause handoff before changes.
    Diagnosis,
}

/// Trusted facts used by the code-owned graph selector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphRequest {
    pub task_kind: WorkGraphTaskKind,
    /// Requires an external human approval before a successful graph can end.
    pub requires_human_review: bool,
}

impl Default for WorkGraphRequest {
    fn default() -> Self {
        Self {
            task_kind: WorkGraphTaskKind::Implementation,
            requires_human_review: false,
        }
    }
}

/// A graph node chosen from the Org-Graph's registered role pool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphPlanNode {
    pub id: String,
    pub role: NodeType,
}

/// A directed, code-owned edge between two plan nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphPlanEdge {
    pub from: String,
    pub to: String,
}

/// One bounded, task-specific instance selected from static templates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphPlan {
    pub template_id: String,
    pub nodes: Vec<WorkGraphPlanNode>,
    pub edges: Vec<WorkGraphPlanEdge>,
    #[serde(default)]
    pub bindings: Vec<WorkGraphRoleBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphRoleBinding {
    pub node_id: String,
    pub role: NodeType,
    pub contract_name: String,
    pub can_spawn: bool,
    pub can_mutate_fs: bool,
    pub can_exec: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkGraphPlanError {
    #[error("Work-Graph role {role:?} for node '{node_id}' is not registered in Org-Graph")]
    UnregisteredRole { node_id: String, role: NodeType },
}

impl WorkGraphPlan {
    /// Resolve every selected role against the immutable Org-Graph registry.
    pub fn bind_registry(&self, registry: &NodeRegistry) -> Result<Self, WorkGraphPlanError> {
        let mut bound = self.clone();
        bound.bindings = self
            .nodes
            .iter()
            .map(|node| {
                let contract = registry.get(&node.role).ok_or_else(|| {
                    WorkGraphPlanError::UnregisteredRole {
                        node_id: node.id.clone(),
                        role: node.role.clone(),
                    }
                })?;
                Ok(WorkGraphRoleBinding {
                    node_id: node.id.clone(),
                    role: node.role.clone(),
                    contract_name: contract.name.clone(),
                    can_spawn: contract.permissions.can_spawn,
                    can_mutate_fs: contract.permissions.can_mutate_fs,
                    can_exec: contract.permissions.can_exec,
                })
            })
            .collect::<Result<Vec<_>, WorkGraphPlanError>>()?;
        Ok(bound)
    }

    /// Returns whether this bounded plan contains an edge between roles.
    ///
    /// Runtime routing uses this rather than trusting a model-suggested next
    /// step. Multiple nodes with the same role are supported deliberately.
    pub fn permits_role_edge(&self, from: NodeType, to: NodeType) -> bool {
        self.edges.iter().any(|edge| {
            self.nodes
                .iter()
                .find(|node| node.id == edge.from)
                .is_some_and(|node| node.role == from)
                && self
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.to)
                    .is_some_and(|node| node.role == to)
        })
    }
}

/// Selects a registered graph template from structured task facts.
pub fn select_work_graph(request: &WorkGraphRequest) -> WorkGraphPlan {
    let mut nodes = vec![
        WorkGraphPlanNode {
            id: "implement".into(),
            role: NodeType::GeneralPurpose,
        },
        WorkGraphPlanNode {
            id: "verify".into(),
            role: NodeType::Verification,
        },
        WorkGraphPlanNode {
            id: "diagnose".into(),
            role: NodeType::RootCause,
        },
    ];
    let mut edges = vec![
        WorkGraphPlanEdge {
            from: "implement".into(),
            to: "verify".into(),
        },
        WorkGraphPlanEdge {
            from: "verify".into(),
            to: "diagnose".into(),
        },
        WorkGraphPlanEdge {
            from: "diagnose".into(),
            to: "implement".into(),
        },
    ];
    let template_id = match request.task_kind {
        WorkGraphTaskKind::Implementation => "implementation-v1",
        WorkGraphTaskKind::Diagnosis => {
            // The first failing external anchor activates the already-present
            // diagnostic edge. A diagnosis request does not let an LLM claim
            // a root cause before that anchor exists.
            "diagnosis-v1"
        }
    };
    if request.requires_human_review {
        nodes.push(WorkGraphPlanNode {
            id: "human-review".into(),
            role: NodeType::HumanReview,
        });
        edges.push(WorkGraphPlanEdge {
            from: "verify".into(),
            to: "human-review".into(),
        });
    }
    WorkGraphPlan {
        template_id: if request.requires_human_review {
            format!("{template_id}-human-review")
        } else {
            template_id.into()
        },
        nodes,
        edges,
        bindings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_request_includes_anchored_root_cause_retry_cycle() {
        let plan = select_work_graph(&WorkGraphRequest {
            task_kind: WorkGraphTaskKind::Diagnosis,
            requires_human_review: false,
        });

        assert_eq!(plan.template_id, "diagnosis-v1");
        assert!(plan
            .nodes
            .iter()
            .any(|node| node.role == NodeType::RootCause));
        assert!(plan
            .edges
            .iter()
            .any(|edge| edge.from == "verify" && edge.to == "diagnose"));
        assert!(plan
            .edges
            .iter()
            .any(|edge| edge.from == "diagnose" && edge.to == "implement"));
    }

    #[test]
    fn human_review_is_an_explicit_terminal_gate_in_selected_plan() {
        let plan = select_work_graph(&WorkGraphRequest {
            task_kind: WorkGraphTaskKind::Implementation,
            requires_human_review: true,
        });

        assert_eq!(plan.template_id, "implementation-v1-human-review");
        assert!(plan.nodes.iter().any(|node| node.id == "human-review"));
        assert!(plan
            .edges
            .iter()
            .any(|edge| edge.from == "verify" && edge.to == "human-review"));
        assert!(plan.permits_role_edge(NodeType::Verification, NodeType::HumanReview));
        assert!(!plan.permits_role_edge(NodeType::HumanReview, NodeType::GeneralPurpose));
    }

    #[test]
    fn bind_registry_captures_registered_contracts() {
        let plan = select_work_graph(&WorkGraphRequest::default());
        let registry = NodeRegistry::builtin(&Default::default());
        let bound = plan.bind_registry(&registry).expect("bind plan");
        assert_eq!(bound.bindings.len(), bound.nodes.len());
        assert!(bound
            .bindings
            .iter()
            .any(|binding| binding.role == NodeType::RootCause && !binding.can_mutate_fs));
    }
}
