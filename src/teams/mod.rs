//! Teams Module — multi-agent coordination with subagent spawning,
//! async JSONL mailboxes, shutdown/approval protocols, and worktree isolation.
//!
//! Corresponds to harness mechanisms s04, s09-s12 (subagents, agent teams,
//! team protocols, autonomous agents, worktree isolation).

pub mod approval_registry;
pub mod failure_diagnostics;
pub mod guarding_tool_port;
pub mod mailbox;
pub mod permission_bridge;
pub mod rollback;
pub mod subagent;
pub mod subagent_health;
pub mod subagent_loop;
pub mod subagent_mailbox;
pub mod subagent_trace;
pub mod trace_sink;
pub mod worktree;

pub use guarding_tool_port::{
    format_permission_summary, GuardingToolPort, SubagentPermissionContext,
};
pub use mailbox::{Mailbox, TeamConfig, TeamManager, TeamMember, TeamMessage};
pub use permission_bridge::{PermissionBridge, StructuredApproval};
pub use subagent::{
    AgentDefinition, AgentSession, AgentStatus, AgentStatusReport, AgentType, AgentsService,
};
pub use subagent_health::{
    FailureMode, HealthPeriod, HealthStatus, SubagentHealth, SubagentHealthAnalyzer,
};
pub use subagent_loop::run_subagent_loop_with_permissions;
pub use subagent_mailbox::{
    StoredResult, SubagentResponse, SubagentResultMailbox, MAX_INLINE_RESULT_LEN,
};
pub use subagent_trace::SubagentTraceReporter;
pub use worktree::WorktreeIsolation;
