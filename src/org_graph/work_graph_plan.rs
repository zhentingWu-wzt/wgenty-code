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

/// One named node role declared by a static graph template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateStage {
    pub id: &'static str,
    pub role: NodeType,
}

/// A code-owned static template from which bounded plan instances are built.
///
/// Templates are the only source of graph structure; the composer and the
/// selector consume them, the model never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphTemplate {
    pub id: &'static str,
    pub stages: &'static [TemplateStage],
    pub edges: &'static [(&'static str, &'static str)],
}

/// The immutable set of registered static graph templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphTemplateRegistry {
    pub templates: &'static [GraphTemplate],
}

impl GraphTemplateRegistry {
    /// The builtin, code-owned template set covering every closed-set request
    /// combination (`task_kind` × `requires_human_review`).
    pub fn builtin() -> Self {
        Self {
            templates: BUILTIN_GRAPH_TEMPLATES,
        }
    }

    /// Find the template registered for a closed-set request combination.
    pub fn find(
        &self,
        task_kind: WorkGraphTaskKind,
        requires_human_review: bool,
    ) -> Option<&'static GraphTemplate> {
        let base_id = match task_kind {
            WorkGraphTaskKind::Implementation => "implementation-v1",
            // The first failing external anchor activates the already-present
            // diagnostic edge. A diagnosis request does not let an LLM claim a
            // root cause before that anchor exists.
            WorkGraphTaskKind::Diagnosis => "diagnosis-v1",
        };
        let expected_id = if requires_human_review {
            format!("{base_id}-human-review")
        } else {
            base_id.to_string()
        };
        self.templates
            .iter()
            .find(|template| template.id == expected_id)
    }
}

impl GraphTemplate {
    /// Instantiate a bounded plan from this template. Bindings stay empty
    /// until `WorkGraphPlan::bind_registry` resolves role contracts.
    fn instantiate(&self) -> WorkGraphPlan {
        WorkGraphPlan {
            template_id: self.id.to_string(),
            nodes: self
                .stages
                .iter()
                .map(|stage| WorkGraphPlanNode {
                    id: stage.id.to_string(),
                    role: stage.role.clone(),
                })
                .collect(),
            edges: self
                .edges
                .iter()
                .map(|(from, to)| WorkGraphPlanEdge {
                    from: from.to_string(),
                    to: to.to_string(),
                })
                .collect(),
            bindings: Vec::new(),
        }
    }
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

static CORE_STAGES: &[TemplateStage] = &[
    TemplateStage {
        id: "implement",
        role: NodeType::GeneralPurpose,
    },
    TemplateStage {
        id: "verify",
        role: NodeType::Verification,
    },
    TemplateStage {
        id: "diagnose",
        role: NodeType::RootCause,
    },
];

static CORE_EDGES: &[(&str, &str)] = &[
    ("implement", "verify"),
    ("verify", "diagnose"),
    ("diagnose", "implement"),
];

static REVIEW_STAGES: &[TemplateStage] = &[
    TemplateStage {
        id: "implement",
        role: NodeType::GeneralPurpose,
    },
    TemplateStage {
        id: "verify",
        role: NodeType::Verification,
    },
    TemplateStage {
        id: "diagnose",
        role: NodeType::RootCause,
    },
    TemplateStage {
        id: "human-review",
        role: NodeType::HumanReview,
    },
];

static REVIEW_EDGES: &[(&str, &str)] = &[
    ("implement", "verify"),
    ("verify", "diagnose"),
    ("diagnose", "implement"),
    ("verify", "human-review"),
];

static BUILTIN_GRAPH_TEMPLATES: &[GraphTemplate] = &[
    GraphTemplate {
        id: "implementation-v1",
        stages: CORE_STAGES,
        edges: CORE_EDGES,
    },
    GraphTemplate {
        id: "implementation-v1-human-review",
        stages: REVIEW_STAGES,
        edges: REVIEW_EDGES,
    },
    GraphTemplate {
        id: "diagnosis-v1",
        stages: CORE_STAGES,
        edges: CORE_EDGES,
    },
    GraphTemplate {
        id: "diagnosis-v1-human-review",
        stages: REVIEW_STAGES,
        edges: REVIEW_EDGES,
    },
];

/// Selects a registered graph template from structured task facts.
pub fn select_work_graph(request: &WorkGraphRequest) -> WorkGraphPlan {
    let registry = GraphTemplateRegistry::builtin();
    registry
        .find(request.task_kind, request.requires_human_review)
        .expect("the builtin registry covers every closed-set request combination")
        .instantiate()
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

    #[test]
    fn builtin_registry_covers_every_closed_set_variant() {
        let registry = GraphTemplateRegistry::builtin();
        let ids: Vec<&str> = registry.templates.iter().map(|t| t.id).collect();
        assert!(ids.contains(&"implementation-v1"));
        assert!(ids.contains(&"implementation-v1-human-review"));
        assert!(ids.contains(&"diagnosis-v1"));
        assert!(ids.contains(&"diagnosis-v1-human-review"));
        assert_eq!(ids.len(), 4, "no undeclared variants may exist");
    }

    #[test]
    fn human_review_variants_declare_the_terminal_review_gate() {
        for template_id in [
            "implementation-v1-human-review",
            "diagnosis-v1-human-review",
        ] {
            let registry = GraphTemplateRegistry::builtin();
            let template = registry
                .templates
                .iter()
                .find(|template| template.id == template_id)
                .expect("human-review variant registered");
            assert!(
                template
                    .stages
                    .iter()
                    .any(|stage| stage.id == "human-review" && stage.role == NodeType::HumanReview),
                "{template_id} must declare the human-review stage"
            );
            assert!(
                template
                    .edges
                    .iter()
                    .any(|(from, to)| *from == "verify" && *to == "human-review"),
                "{template_id} must declare the verify→human-review terminal edge"
            );
            let plan = select_work_graph(&WorkGraphRequest {
                task_kind: if template_id.starts_with("diagnosis") {
                    WorkGraphTaskKind::Diagnosis
                } else {
                    WorkGraphTaskKind::Implementation
                },
                requires_human_review: true,
            });
            assert_eq!(plan.template_id, template_id);
            assert!(plan.permits_role_edge(NodeType::Verification, NodeType::HumanReview));
        }
    }

    #[test]
    fn template_edges_reference_declared_stages() {
        let registry = GraphTemplateRegistry::builtin();
        for template in registry.templates {
            for (from, to) in template.edges {
                assert!(
                    template.stages.iter().any(|stage| stage.id == *from),
                    "{} references undeclared stage '{from}'",
                    template.id
                );
                assert!(
                    template.stages.iter().any(|stage| stage.id == *to),
                    "{} references undeclared stage '{to}'",
                    template.id
                );
            }
        }
    }
}
