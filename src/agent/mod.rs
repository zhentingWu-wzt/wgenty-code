//! Agent Module - shared agent loop and SSE stream processing.
//!
//! The agent loop runs in every communication channel:
//!   - CLI REPL:    cli::repl (simple line-mode loop)
//!   - TUI REPL:    cli::tui_repl (full Ratatui loop)
//!   - Daemon API:  daemon::handlers (HTTP SSE proxy)
//!
//! `StreamProcessor` handles the duplicated SSE parsing logic previously
//! found in both frontends, producing structured `StreamEvent`s.
//!
//! `runtime` holds pure loop helpers (compaction, tool timeouts) shared by
//! every frontend; later phases host the full `AgentRuntime` here.

pub mod capability;
pub mod coordinator;
pub mod core;
pub mod events;
pub mod fallback;
pub mod runtime;
pub mod store;
pub mod task_group;

pub use coordinator::{
    AgentCoordinator, ChildReservation, ChildResult, ChildResultHandle, ChildTerminal,
    ChildTerminalStatus, CoordinatorError, JoinPolicy, ParentOutcome, SpawnChildRequest,
};
pub use core::StreamProcessor;
pub use events::{StreamEvent, StreamResult};
pub use store::{
    AgentRecord, ChildSummary, DirectChildView, InMemoryAgentStore, LocalAgentView, SelfView,
    StoreError,
};
pub use task_group::{TaskGroupDelivery, TaskGroupError, TaskGroupId, TaskGroupStore};

// Identity vocabulary (agent/session ids, trusted `AgentExecutionContext`,
// `ToolContext`) lives in `tools::context` so the tools layer does not depend
// on `agent` (AGENTS.md module-dependency rule). Re-exported here because the
// agent layer remains the primary producer of these values.
pub use crate::tools::context::{
    AgentExecutionContext, AgentId, AgentLifecycleStatus, CheckpointCapture, SessionId,
    ToolContext, ToolInvocationId,
};

// Subagent progress vocabulary lives in the top-level `progress` module
// (shared by agent, teams, tools, transcript, daemon and TUI without a
// dependency on `agent`). Re-exported here for existing import paths.
pub use crate::progress::{
    ErrorType, ProgressCallback, SubagentEvent, SubagentEventType, SubagentMetadata,
    SubagentProgress, SubagentStatus,
};
