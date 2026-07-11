//! Agent Module - shared agent loop and SSE stream processing.
//!
//! The agent loop runs in every communication channel:
//!   - CLI REPL:    cli::repl (simple line-mode loop)
//!   - TUI REPL:    cli::tui_repl (full Ratatui loop)
//!   - Daemon API:  daemon::handlers (HTTP SSE proxy)
//!
//! `StreamProcessor` handles the duplicated SSE parsing logic previously
//! found in both frontends, producing structured `StreamEvent`s.

pub mod capability;
pub mod coordinator;
pub mod core;
pub mod events;
pub mod identity;
pub mod progress;
pub mod store;

pub use coordinator::{
    AgentCoordinator, ChildReservation, ChildResult, ChildResultHandle, ChildTerminal,
    ChildTerminalStatus, CoordinatorError, JoinPolicy, ParentOutcome, SpawnChildRequest,
};
pub use core::StreamProcessor;
pub use events::{StreamEvent, StreamResult};
pub use identity::{
    AgentExecutionContext, AgentId, AgentLifecycleStatus, SessionId, ToolContext, ToolInvocationId,
};
pub use progress::{ProgressCallback, SubagentMetadata, SubagentProgress, SubagentStatus};
pub use store::{
    AgentRecord, ChildSummary, DirectChildView, InMemoryAgentStore, LocalAgentView, SelfView,
    StoreError,
};
