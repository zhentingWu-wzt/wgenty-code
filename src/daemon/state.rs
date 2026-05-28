//! DaemonState — shared state for the HTTP API server.

use crate::permissions::ToolPermissionPolicy;
use crate::state::AppState;
use crate::tools::{ToolExecutor, ToolRegistry};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Per-session permission rules.
struct SessionRules {
    approved: HashSet<String>,
}

impl SessionRules {
    fn new() -> Self {
        Self {
            approved: HashSet::new(),
        }
    }
}

/// Shared state for all daemon HTTP handlers.
pub struct DaemonState {
    pub app_state: AppState,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_executor: ToolExecutor,
    sessions: Arc<RwLock<std::collections::HashMap<String, SessionRules>>>,
}

impl DaemonState {
    pub fn new(app_state: AppState) -> Self {
        let tool_registry = Arc::new(ToolRegistry::new());
        let policy = ToolPermissionPolicy::from_settings(&app_state.settings);

        Self {
            app_state,
            tool_executor: ToolExecutor::new(tool_registry.clone(), policy),
            tool_registry,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Check if a session rule is approved for a given session.
    pub async fn is_rule_approved(&self, session_id: &str, rule: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .map(|s| s.approved.contains(rule))
            .unwrap_or(false)
    }

    /// Approve a session rule.
    pub async fn approve_rule(&self, session_id: &str, rule: String) {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionRules::new)
            .approved
            .insert(rule);
    }
}
