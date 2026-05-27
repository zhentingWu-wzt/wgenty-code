//! Teams Module — multi-agent coordination with subagent spawning,
//! async JSONL mailboxes, shutdown/approval protocols, and worktree isolation.
//!
//! Corresponds to harness mechanisms s04, s09-s12 (subagents, agent teams,
//! team protocols, autonomous agents, worktree isolation).

pub mod subagent;

pub use subagent::{
    AgentDefinition, AgentSession, AgentStatus, AgentStatusReport, AgentType, AgentsService,
};
