use serde::{Deserialize, Serialize};
use std::fmt;
use tokio_util::sync::CancellationToken;

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates an identifier from its string wire representation.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier's string wire representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Identifies an agent execution session and serializes as a plain string.
    SessionId
);
string_id!(
    /// Identifies an agent within a session and serializes as a plain string.
    AgentId
);
string_id!(
    /// Identifies one tool invocation and serializes as a plain string.
    ToolInvocationId
);

/// Trusted execution identity and cancellation state for an agent.
///
/// Parent identity and depth are derived internally when creating children.
/// Cancellation propagates downward from a parent context to its descendants.
#[derive(Debug, Clone)]
pub struct AgentExecutionContext {
    /// Session shared by the root agent and all descendants.
    pub session_id: SessionId,
    /// Identity of this agent execution.
    pub agent_id: AgentId,
    /// Parent agent identity, or `None` for the root agent.
    pub parent_id: Option<AgentId>,
    /// Trusted hierarchy depth, with the root at depth zero.
    pub depth: usize,
    /// Cancellation token inherited downward through child tokens.
    pub cancellation: CancellationToken,
}

impl AgentExecutionContext {
    /// Creates a root execution context with a generated agent identity.
    pub fn root(session_id: SessionId) -> Self {
        Self {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            session_id,
            parent_id: None,
            depth: 0,
            cancellation: CancellationToken::new(),
        }
    }

    pub(crate) fn child(&self, agent_id: AgentId) -> Self {
        Self {
            session_id: self.session_id.clone(),
            agent_id,
            parent_id: Some(self.agent_id.clone()),
            depth: self.depth + 1,
            cancellation: self.cancellation.child_token(),
        }
    }
}

/// Lifecycle state for a trusted agent execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLifecycleStatus {
    /// Created but not yet running.
    Pending,
    /// Actively executing.
    Running,
    /// Suspended until child agents reach terminal states.
    WaitingForChildren,
    /// Producing the final result after execution.
    Finalizing,
    /// Cancellation is in progress.
    Cancelling,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Finished due to cancellation.
    Cancelled,
}

impl AgentLifecycleStatus {
    /// Returns whether no further lifecycle work can occur for this execution.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Context supplied to a tool invocation on behalf of an agent.
pub struct ToolContext<'a> {
    /// Trusted identity and cancellation state of the invoking agent.
    pub agent: &'a AgentExecutionContext,
    /// Identity assigned to this tool invocation.
    pub invocation_id: ToolInvocationId,
    /// Trusted identifier of the originating root turn, supplied by the
    /// daemon for root-agent invocations so identity-sensitive tools (e.g.
    /// `task`) can group direct children under one root turn. `None` for
    /// non-root agents and direct/test contexts; never accepted from model
    /// JSON.
    pub origin_turn_id: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_context_is_derived_from_parent() {
        let root = AgentExecutionContext::root(SessionId::new("session-a"));
        let child_id = AgentId::new("child-a");
        let child = root.child(child_id.clone());

        assert_eq!(child.session_id, root.session_id);
        assert_eq!(child.agent_id, child_id);
        assert_eq!(child.parent_id.as_ref(), Some(&root.agent_id));
        assert_eq!(child.depth, 1);
        assert!(!child.cancellation.is_cancelled());

        root.cancellation.cancel();

        assert!(child.cancellation.is_cancelled());
    }

    #[test]
    fn only_completed_failed_and_cancelled_are_terminal() {
        assert!(!AgentLifecycleStatus::Pending.is_terminal());
        assert!(!AgentLifecycleStatus::Running.is_terminal());
        assert!(!AgentLifecycleStatus::WaitingForChildren.is_terminal());
        assert!(!AgentLifecycleStatus::Finalizing.is_terminal());
        assert!(!AgentLifecycleStatus::Cancelling.is_terminal());
        assert!(AgentLifecycleStatus::Completed.is_terminal());
        assert!(AgentLifecycleStatus::Failed.is_terminal());
        assert!(AgentLifecycleStatus::Cancelled.is_terminal());
    }
}
