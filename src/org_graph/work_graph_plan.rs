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

/// Risk tier supplied by a trusted caller; closed set, code-interpreted.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    #[default]
    Medium,
    High,
}

/// External anchor phases a plan routes through; closed set.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkGraphPhase {
    CompileAnchor,
    TestAnchor,
    VerifyGate,
}

fn default_phases() -> Vec<WorkGraphPhase> {
    vec![
        WorkGraphPhase::CompileAnchor,
        WorkGraphPhase::TestAnchor,
        WorkGraphPhase::VerifyGate,
    ]
}

/// Trusted facts used by the code-owned graph selector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkGraphRequest {
    pub task_kind: WorkGraphTaskKind,
    /// Requires an external human approval before a successful graph can end.
    pub requires_human_review: bool,
    /// `High` forces the terminal review gate even without an explicit request.
    #[serde(default)]
    pub risk: Risk,
    /// `false` composes a compile+verify graph without the test anchor phase.
    #[serde(default = "default_has_test_infra")]
    pub has_test_infra: bool,
    /// Diagnostic channel capacity; `0` strips the diagnose stage entirely.
    #[serde(default = "default_max_specialists")]
    pub max_specialists: u8,
}

impl Default for WorkGraphRequest {
    fn default() -> Self {
        Self {
            task_kind: WorkGraphTaskKind::Implementation,
            requires_human_review: false,
            risk: Risk::Medium,
            has_test_infra: true,
            max_specialists: 1,
        }
    }
}

fn default_has_test_infra() -> bool {
    true
}

fn default_max_specialists() -> u8 {
    1
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
    /// Anchor phases this plan routes through; legacy checkpoints restore the
    /// full compile → test → verify sequence.
    #[serde(default = "default_phases")]
    pub phases: Vec<WorkGraphPhase>,
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
            phases: default_phases(),
            bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkGraphPlanError {
    #[error("Work-Graph role {role:?} for node '{node_id}' is not registered in Org-Graph")]
    UnregisteredRole { node_id: String, role: NodeType },
    #[error("Work-Graph composition declares {count} nodes; the hard cap is {max}")]
    TooManyNodes { count: usize, max: usize },
    #[error("Work-Graph edge references undeclared stage '{node_id}'")]
    UndeclaredStage { node_id: String },
    #[error("Work-Graph edge {from}→{to} is not in the code-owned edge whitelist")]
    EdgeNotAllowed { from: String, to: String },
    #[error("Work-Graph specialist capacity {value} exceeds the closed range 0..=3")]
    InvalidSpecialistCapacity { value: u8 },
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

/// The complete code-owned edge whitelist. Every composed plan edge must be
/// one of these pairs; this bounds the reachable shapes (including cycles)
/// to the declared retry loop and review gate.
static ALLOWED_PLAN_EDGES: &[(&str, &str)] = &[
    ("implement", "verify"),
    ("verify", "diagnose"),
    ("diagnose", "implement"),
    ("verify", "human-review"),
];

/// Hard node cap for any composed plan.
pub const MAX_WORK_GRAPH_NODES: usize = 8;

/// Structural validation for a composed plan. Pure; performed after every
/// composition (and reusable by future callers) so a malformed plan can never
/// be persisted.
pub fn validate_composition(plan: &WorkGraphPlan) -> Result<(), WorkGraphPlanError> {
    if plan.nodes.len() > MAX_WORK_GRAPH_NODES {
        return Err(WorkGraphPlanError::TooManyNodes {
            count: plan.nodes.len(),
            max: MAX_WORK_GRAPH_NODES,
        });
    }
    for edge in &plan.edges {
        for node_id in [&edge.from, &edge.to] {
            if !plan.nodes.iter().any(|node| &node.id == node_id) {
                return Err(WorkGraphPlanError::UndeclaredStage {
                    node_id: node_id.clone(),
                });
            }
        }
        if !ALLOWED_PLAN_EDGES
            .iter()
            .any(|(from, to)| *from == edge.from && *to == edge.to)
        {
            return Err(WorkGraphPlanError::EdgeNotAllowed {
                from: edge.from.clone(),
                to: edge.to.clone(),
            });
        }
    }
    // Validation-only binding: roles must resolve against the builtin
    // registry. The returned plan keeps its (possibly empty) bindings.
    plan.bind_registry(&NodeRegistry::builtin(&Default::default()))?;
    Ok(())
}

/// Compose a bounded Work-Graph from closed-set structured facts.
///
/// The default fact combination reproduces the canonical registry templates
/// byte-for-byte; any deviation composes a signature template id (for example
/// `impl+no-test+review+risk-high`) so persisted plans stay self-describing.
/// Composition is followed by structural validation — a malformed result is
/// rejected, never downgraded to the nearest legal shape.
pub fn compose_work_graph(request: &WorkGraphRequest) -> Result<WorkGraphPlan, WorkGraphPlanError> {
    if request.max_specialists > 3 {
        return Err(WorkGraphPlanError::InvalidSpecialistCapacity {
            value: request.max_specialists,
        });
    }
    let review_gate = request.requires_human_review || request.risk == Risk::High;
    let registry = GraphTemplateRegistry::builtin();
    let template = registry
        .find(request.task_kind, review_gate)
        .expect("the builtin registry covers every closed-set combination");
    let mut plan = template.instantiate();

    // Stage splices: low risk or zero diagnostic capacity removes the
    // predeclared diagnose stage and its edges.
    if request.risk == Risk::Low || request.max_specialists == 0 {
        plan.nodes.retain(|node| node.id != "diagnose");
        plan.edges
            .retain(|edge| edge.from != "diagnose" && edge.to != "diagnose");
    }

    // Phase splices: no test infrastructure drops the test anchor phase.
    if !request.has_test_infra {
        plan.phases
            .retain(|phase| *phase != WorkGraphPhase::TestAnchor);
    }

    let deviates =
        !request.has_test_infra || request.risk != Risk::Medium || request.max_specialists != 1;
    plan.template_id = if deviates {
        let base_word = match request.task_kind {
            WorkGraphTaskKind::Implementation => "impl",
            WorkGraphTaskKind::Diagnosis => "diag",
        };
        let mut signature = String::from(base_word);
        if !request.has_test_infra {
            signature.push_str("+no-test");
        }
        if review_gate {
            signature.push_str("+review");
        }
        match request.risk {
            Risk::Low => signature.push_str("+risk-low"),
            Risk::High => signature.push_str("+risk-high"),
            Risk::Medium => {}
        }
        if request.max_specialists != 1 {
            signature.push_str(&format!("+spec-{}", request.max_specialists));
        }
        signature
    } else {
        plan.template_id
    };

    validate_composition(&plan)?;
    Ok(plan)
}

/// Selects a registered graph template from structured task facts.
pub fn select_work_graph(request: &WorkGraphRequest) -> WorkGraphPlan {
    compose_work_graph(request).expect("closed-set facts always compose a valid plan")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_request_includes_anchored_root_cause_retry_cycle() {
        let plan = select_work_graph(&WorkGraphRequest {
            task_kind: WorkGraphTaskKind::Diagnosis,
            requires_human_review: false,
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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

    fn request(
        task_kind: WorkGraphTaskKind,
        risk: Risk,
        has_test_infra: bool,
        max_specialists: u8,
        requires_human_review: bool,
    ) -> WorkGraphRequest {
        WorkGraphRequest {
            task_kind,
            risk,
            has_test_infra,
            max_specialists,
            requires_human_review,
        }
    }

    #[test]
    fn composition_matrix_is_total_and_well_formed() {
        // 2 task kinds × 3 risks × 2 test-infra × 2 review gates = 24 closed
        // combinations. Every one must compose to a valid plan satisfying
        // the structural invariants.
        for task_kind in [
            WorkGraphTaskKind::Implementation,
            WorkGraphTaskKind::Diagnosis,
        ] {
            for risk in [Risk::Low, Risk::Medium, Risk::High] {
                for has_test_infra in [true, false] {
                    for requires_human_review in [true, false] {
                        let request =
                            request(task_kind, risk, has_test_infra, 1, requires_human_review);
                        let plan = compose_work_graph(&request)
                            .unwrap_or_else(|error| panic!("{request:?}: {error:?}"));
                        let has_role =
                            |role: NodeType| plan.nodes.iter().any(|node| node.role == role);
                        assert!(
                            has_role(NodeType::GeneralPurpose) && has_role(NodeType::Verification),
                            "implement/verify always present: {}",
                            plan.template_id
                        );
                        let wants_review = requires_human_review || risk == Risk::High;
                        assert_eq!(
                            has_role(NodeType::HumanReview),
                            wants_review,
                            "review gate iff requested or high risk: {}",
                            plan.template_id
                        );
                        let wants_diagnose = risk != Risk::Low;
                        assert_eq!(
                            has_role(NodeType::RootCause),
                            wants_diagnose,
                            "diagnose iff risk != Low: {}",
                            plan.template_id
                        );
                        let wants_test = plan.phases.contains(&WorkGraphPhase::TestAnchor);
                        assert_eq!(
                            wants_test, has_test_infra,
                            "test anchor phase iff test infra: {}",
                            plan.template_id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn default_facts_compose_the_canonical_templates() {
        // The P0 closed set keeps its canonical template ids so persisted
        // plans and existing callers see zero migration.
        for (task_kind, review, expected) in [
            (
                WorkGraphTaskKind::Implementation,
                false,
                "implementation-v1",
            ),
            (
                WorkGraphTaskKind::Implementation,
                true,
                "implementation-v1-human-review",
            ),
            (WorkGraphTaskKind::Diagnosis, false, "diagnosis-v1"),
            (
                WorkGraphTaskKind::Diagnosis,
                true,
                "diagnosis-v1-human-review",
            ),
        ] {
            let plan = compose_work_graph(&request(task_kind, Risk::Medium, true, 1, review))
                .expect("default facts compose");
            assert_eq!(plan.template_id, expected);
            assert_eq!(
                plan.phases,
                vec![
                    WorkGraphPhase::CompileAnchor,
                    WorkGraphPhase::TestAnchor,
                    WorkGraphPhase::VerifyGate
                ]
            );
        }
    }

    #[test]
    fn deviations_compose_signature_template_ids() {
        let plan = compose_work_graph(&request(
            WorkGraphTaskKind::Implementation,
            Risk::High,
            false,
            1,
            false,
        ))
        .expect("compose deviations");
        assert_eq!(plan.template_id, "impl+no-test+review+risk-high");

        let plan = compose_work_graph(&request(
            WorkGraphTaskKind::Diagnosis,
            Risk::Low,
            true,
            0,
            false,
        ))
        .expect("compose low-risk diagnosis");
        assert_eq!(plan.template_id, "diag+risk-low+spec-0");
    }

    #[test]
    fn high_risk_forces_the_review_gate_without_explicit_request() {
        let plan = compose_work_graph(&request(
            WorkGraphTaskKind::Implementation,
            Risk::High,
            true,
            1,
            false,
        ))
        .expect("compose high risk");
        assert!(plan.permits_role_edge(NodeType::Verification, NodeType::HumanReview));
        assert!(plan
            .nodes
            .iter()
            .any(|node| node.role == NodeType::HumanReview));
    }

    #[test]
    fn zero_specialists_strips_the_diagnose_channel() {
        let plan = compose_work_graph(&request(
            WorkGraphTaskKind::Implementation,
            Risk::Medium,
            true,
            0,
            false,
        ))
        .expect("compose zero specialists");
        assert!(!plan
            .nodes
            .iter()
            .any(|node| node.role == NodeType::RootCause));
        assert!(!plan.permits_role_edge(NodeType::Verification, NodeType::RootCause));
    }

    #[test]
    fn validation_rejects_oversized_and_undeclared_plans() {
        let mut oversized = compose_work_graph(&WorkGraphRequest::default()).expect("compose");
        oversized.nodes = (0..9)
            .map(|index| WorkGraphPlanNode {
                id: format!("n{index}"),
                role: NodeType::GeneralPurpose,
            })
            .collect();
        assert_eq!(
            validate_composition(&oversized),
            Err(WorkGraphPlanError::TooManyNodes { count: 9, max: 8 })
        );

        let mut undeclared_edge =
            compose_work_graph(&WorkGraphRequest::default()).expect("compose");
        undeclared_edge.edges.push(WorkGraphPlanEdge {
            from: "verify".into(),
            to: "ghost".into(),
        });
        assert!(
            matches!(validate_composition(&undeclared_edge), Err(WorkGraphPlanError::UndeclaredStage { node_id, .. }) if node_id == "ghost")
        );

        let mut forbidden_edge = compose_work_graph(&WorkGraphRequest::default()).expect("compose");
        forbidden_edge.edges.push(WorkGraphPlanEdge {
            from: "verify".into(),
            to: "implement".into(),
        });
        assert!(
            matches!(validate_composition(&forbidden_edge), Err(WorkGraphPlanError::EdgeNotAllowed { from, to, .. }) if from == "verify" && to == "implement")
        );
    }

    #[test]
    fn legacy_json_restores_new_fields_to_defaults() {
        let request: WorkGraphRequest =
            serde_json::from_str(r#"{"task_kind":"implementation","requires_human_review":false}"#)
                .expect("legacy request json");
        assert_eq!(request.risk, Risk::Medium);
        assert!(request.has_test_infra);
        assert_eq!(request.max_specialists, 1);

        let plan: WorkGraphPlan =
            serde_json::from_str(r#"{"template_id":"implementation-v1","nodes":[],"edges":[]}"#)
                .expect("legacy plan json");
        assert_eq!(
            plan.phases,
            vec![
                WorkGraphPhase::CompileAnchor,
                WorkGraphPhase::TestAnchor,
                WorkGraphPhase::VerifyGate
            ]
        );
    }
}
