//! ExecutionSession inner layer: SessionCoordinator + turn chain + verify-gate.
//!
//! This module is the HOW layer of the long-horizon autonomy loop (C-scheme).
//! It is decoupled from any flow-orchestration skill: the caller supplies a
//! [`SessionSource`] and hooks; this module never probes whether a specific
//! skill is installed. File-blob snapshots are reused from
//! [`crate::tools::checkpoint_store`] — this layer adds session chaining, git
//! refs protection, and the verify-gate.
//!
//! Decoupling invariant: this module must not contain references to specific
//! skill names beyond the [`SessionSource::Comet`] enum variant. The
//! `Comet` variant exists because the caller declares the session origin; the
//! core runtime does not branch on it.

pub mod coordinator;
pub mod git;
pub mod hooks;
pub mod session;

pub use coordinator::SessionCoordinator;
pub use hooks::{
    NoHooks, RollbackContext, SessionHooks, VerifyFailAction, VerifyFailContext, VerifyFailure,
};
pub use session::{GitRefs, SessionSource, SessionState, SessionStatus, TurnRecord};
