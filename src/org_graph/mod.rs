//! Org-Graph 节点契约模块：纯数据 + 纯函数校验，无 async / I/O / 状态。

pub mod contract;
pub mod registry;
pub mod render;
pub mod work_state;

pub use contract::{
    Capability, ContractDimension, IoShape, NodeContract, NodeType, PermissionBoundary,
    ResourceBudget,
};
pub use registry::NodeRegistry;
pub use work_state::{
    AuditCommandRun, Budget, CompileResult, FieldPerms, GeneratedDiff, GraphAuditAnchor,
    GraphAuditEvent, GraphAuditKind, GraphAuditProfile, GraphAuditRoute, HumanReview, StepAction,
    StepRecord, TestResult, VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
