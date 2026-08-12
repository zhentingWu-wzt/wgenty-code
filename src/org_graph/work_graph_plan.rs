//! Code-owned dynamic construction of bounded Work-Graph instances.
//!
//! A plan is selected from registered templates using structured task facts.
//! It never accepts model-provided node types or arbitrary edges.

use serde::{Deserialize, Serialize};

use super::NodeType;

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
    ];
    let mut edges = vec![WorkGraphPlanEdge {
        from: "implement".into(),
        to: "verify".into(),
    }];
    let template_id = match request.task_kind {
        WorkGraphTaskKind::Implementation => "implementation-v1",
        WorkGraphTaskKind::Diagnosis => {
            nodes.insert(
                0,
                WorkGraphPlanNode {
                    id: "diagnose".into(),
                    role: NodeType::RootCause,
                },
            );
            edges.insert(
                0,
                WorkGraphPlanEdge {
                    from: "diagnose".into(),
                    to: "implement".into(),
                },
            );
            "diagnosis-v1"
        }
    };
    if request.requires_human_review {
        nodes.push(WorkGraphPlanNode {
            id: "human-review".into(),
            // Verification is the existing least-privilege, non-mutating
            // gate role. The human itself does not receive agent authority.
            role: NodeType::Verification,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_request_selects_root_cause_before_implementation() {
        let plan = select_work_graph(&WorkGraphRequest {
            task_kind: WorkGraphTaskKind::Diagnosis,
            requires_human_review: false,
        });

        assert_eq!(plan.template_id, "diagnosis-v1");
        assert_eq!(plan.nodes[0].role, NodeType::RootCause);
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
    }
}
