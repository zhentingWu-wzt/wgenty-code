//! Permissions Module — tool permission governance and sandboxing.
//!
//! Corresponds to the harness permission mechanism: sandboxing, approval
//! workflows, and trust boundaries between the agent and external systems.

pub mod policy;

pub use policy::{PermissionRequest, PolicyDecision, ToolPermissionPolicy};
