use serde::{Deserialize, Serialize};
use std::fmt;
use tokio_util::sync::CancellationToken;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

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

string_id!(SessionId);
string_id!(AgentId);
string_id!(ToolInvocationId);

#[derive(Debug, Clone)]
pub struct AgentExecutionContext {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub depth: usize,
    pub cancellation: CancellationToken,
}

impl AgentExecutionContext {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLifecycleStatus {
    Pending,
    Running,
    WaitingForChildren,
    Finalizing,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl AgentLifecycleStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

pub struct ToolContext<'a> {
    pub agent: &'a AgentExecutionContext,
    pub invocation_id: ToolInvocationId,
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
