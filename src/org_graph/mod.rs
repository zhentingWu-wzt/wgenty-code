//! Org-Graph 节点契约模块：纯数据 + 纯函数校验，无 async / I/O / 状态。

pub mod audit;
pub mod contract;
pub mod registry;
pub mod render;
pub mod work_graph_plan;
pub mod work_state;

pub use audit::WorkGraphAuditSummary;
pub use contract::{
    Capability, ContractDimension, IoShape, NodeContract, NodeType, PermissionBoundary,
    ResourceBudget,
};
pub use registry::NodeRegistry;
pub use work_graph_plan::{
    adapt_work_graph, compose_work_graph, consecutive_test_anchor_failures, select_work_graph,
    validate_composition, Adaptation, AdaptationOutcome, AdaptationReason, GraphTemplate,
    GraphTemplateRegistry, Risk, TemplateStage, WorkGraphPhase, WorkGraphPlan, WorkGraphPlanEdge,
    WorkGraphPlanNode, WorkGraphRequest, WorkGraphTaskKind, MAX_PLAN_ADAPTATIONS,
    MAX_WORK_GRAPH_NODES,
};
pub use work_state::{
    AuditCommandRun, Budget, CompileResult, FieldPerms, GeneratedDiff, GraphAuditAdaptation,
    GraphAuditAnchor, GraphAuditCommands, GraphAuditEvent, GraphAuditKind, GraphAuditProfile,
    GraphAuditRoute, GraphChildBinding, HumanReview, SpecialistEvidence, SpecialistReport,
    SpecialistReportKind, StepAction, StepRecord, TestResult, VerifyFailureKind, VerifyOutcome,
    WorkField, WorkState,
};
