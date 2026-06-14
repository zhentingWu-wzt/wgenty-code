//! Teams Module — multi-agent coordination with subagent spawning,
//! async JSONL mailboxes, shutdown/approval protocols, and worktree isolation.
//!
//! Corresponds to harness mechanisms s04, s09-s12 (subagents, agent teams,
//! team protocols, autonomous agents, worktree isolation).

pub mod mailbox;
pub mod subagent;
pub mod subagent_loop;
pub mod subagent_mailbox;

pub use mailbox::{Mailbox, TeamConfig, TeamManager, TeamMember, TeamMessage};
pub use subagent::{
    AgentDefinition, AgentSession, AgentStatus, AgentStatusReport, AgentType, AgentsService,
};
pub use subagent_loop::run_subagent_loop;
pub use subagent_mailbox::{
    SubagentResponse, SubagentResultMailbox, StoredResult, MAX_INLINE_RESULT_LEN,
};
