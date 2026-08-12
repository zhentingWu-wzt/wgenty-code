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
    select_work_graph, WorkGraphPlan, WorkGraphPlanEdge, WorkGraphPlanNode, WorkGraphRequest,
    WorkGraphTaskKind,
};
pub use work_state::{
    AuditCommandRun, Budget, CompileResult, FieldPerms, GeneratedDiff, GraphAuditAnchor,
    GraphAuditCommands, GraphAuditEvent, GraphAuditKind, GraphAuditProfile, GraphAuditRoute,
    HumanReview, SpecialistEvidence, SpecialistReport, SpecialistReportKind, StepAction,
    StepRecord, TestResult, VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
