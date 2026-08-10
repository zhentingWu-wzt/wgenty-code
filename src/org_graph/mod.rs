//! Org-Graph 节点契约模块：纯数据 + 纯函数校验，无 async / I/O / 状态。

pub mod contract;

pub use contract::{
    Capability, ContractDimension, IoShape, NodeContract, NodeType, PermissionBoundary,
    ResourceBudget,
};
