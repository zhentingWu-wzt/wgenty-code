//! Comet Module — OpenSpec + Superpowers workflow compatibility layer.
//!
//! Provides phase-aware state tracking, guard rules for phase-appropriate
//! tooling, workflow change discovery, and subagent dispatch protocol
//! documentation.

pub mod guard;
pub mod protocol;
pub mod state;
pub mod workflow;

pub use guard::CometGuard;
pub use state::{CometPhase, CometState};
pub use workflow::ChangeInfo;
