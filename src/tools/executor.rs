use crate::agent::ToolContext;
use crate::api::ChatMessage;
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::runtime::guardian::{Guardian, GuardianDecision};
use crate::runtime::hooks::{HookEvent, HookManager};
use crate::tools::ToolRegistry;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    policy: ToolPermissionPolicy,
    session_rules: Arc<RwLock<HashSet<String>>>,
    hook_manager: Arc<HookManager>,
    guardian: Guardian,
    /// Active workflow state handle (e.g. Comet phase: "open", "design", "build", etc.).
    /// `None` if no workflow state is active.
    pub state_handle: Option<Arc<RwLock<String>>>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, policy: ToolPermissionPolicy) -> Self {
        Self {
            registry,
            policy,
            session_rules: Arc::new(RwLock::new(HashSet::new())),
            hook_manager: Arc::new(HookManager::default()),
            guardian: Guardian::default(),
            state_handle: None,
        }
    }

    pub fn with_hooks(mut self, hook_manager: Arc<HookManager>) -> Self {
        self.hook_manager = hook_manager;
        self
    }

    /// Set the workflow state handle (e.g., from the Comet subsystem or TUI app).
    pub fn set_state_handle(&mut self, handle: Option<Arc<RwLock<String>>>) {
        self.state_handle = handle;
    }

    pub fn tool_definitions(&self) -> Vec<crate::api::ToolDefinition> {
        self.registry
            .list()
            .into_iter()
            .map(|t| crate::api::ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }

    /// Validate a tool call before execution. Returns PolicyDecision.
    pub async fn validate_tool_call(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<PolicyDecision, crate::tools::ToolError> {
        let tool = self.registry.get(tool_name);
        let session_rules = self.session_rules.read().await;
        match tool {
            Some(t) => self
                .policy
                .validate_tool_call(t, tool_name, args, &session_rules),
            None => Ok(PolicyDecision::Allow),
        }
    }

    /// Record an approved session rule so future calls skip the prompt.
    pub async fn approve_rule(&self, rule: String) {
        self.session_rules.write().await.insert(rule);
    }

    /// Remove an approved session rule (for "allow once" flow).
    pub async fn unapprove_rule(&self, rule: &str) {
        self.session_rules.write().await.remove(rule);
    }

    /// Execute a tool call directly (policy already passed).
    /// Run a guardian security check before executing a high-risk tool.
    /// Returns Some(decision) if the tool was blocked by guardian.
    pub fn guardian_check(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<GuardianDecision> {
        if tool_name != "execute_command" && tool_name != "exec_command" {
            return None;
        }
        if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
            let decision = self.guardian.check(tool_name, cmd);
            if !decision.allowed {
                return Some(decision);
            }
            if decision.requires_approval {
                tracing::warn!(
                    risk = ?decision.risk_level,
                    tool = tool_name,
                    "Guardian flagged command for approval"
                );
            }
        }
        None
    }

    pub async fn execute_tool_call(
        &self,
        context: &ToolContext<'_>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> ChatMessage {
        let result = self
            .registry
            .execute_with_context(context, tool_name, args)
            .await;
        let content = match result {
            Ok(result) => serde_json::json!({
                "success": true,
                "output_type": result.output_type,
                "content": result.content,
                "metadata": result.metadata
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "success": false,
                "error": {
                    "message": e.message,
                    "code": e.code
                }
            })
            .to_string(),
        };

        ChatMessage::tool(tool_call_id, content)
    }

    /// Execute a tool call with Pre/Post hooks wrapping execution.
    ///
    /// PreToolUse hooks with `when_state` filtering replace the old CometGuard
    /// phase-guard logic. If the workflow state matches a hook's `when_state`
    /// and that hook returns `{ "continue_execution": false }`, the tool call
    /// is blocked.
    ///
    /// PostToolUse hooks run after execution and cannot block.
    ///
    /// The trusted [`ToolContext`] supplies the agent identity and session;
    /// `session_id` is derived from `context.agent.session_id` and never read
    /// from model-supplied JSON.
    pub async fn execute_with_hooks(
        &self,
        context: &ToolContext<'_>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> ChatMessage {
        let session_id = context.agent.session_id.as_str();
        tracing::info!(
            tool = tool_name,
            args_len = serde_json::to_string(&args).map(|s| s.len()).unwrap_or(0),
            "ToolExecutor: executing with hooks"
        );

        // Read the current workflow state from the shared handle
        let state_val = self
            .state_handle
            .as_ref()
            .and_then(|h| h.try_read().ok())
            .map(|r| r.clone());
        let state_str = state_val.as_deref();

        // PreToolUse hooks — hooks with when_state matching the current state
        // may block the tool by returning `{ "continue_execution": false }`.
        let pre_ctx = HookManager::pre_tool_context(tool_name, &args, Some(session_id))
            .with_state(state_val.clone());
        let pre_outcomes = self
            .hook_manager
            .fire(&HookEvent::PreToolUse, &pre_ctx, state_str, None)
            .await;

        // If any hook blocked execution, fire a Notification hook and return
        for outcome in &pre_outcomes {
            if !outcome.continue_execution {
                let reason = outcome.reason.as_deref().unwrap_or("unknown reason");
                let msg = format!("Tool '{}' blocked by hook: {}", tool_name, reason);
                // Fire Notification hook asynchronously (do not await)
                let hook_manager = Arc::clone(&self.hook_manager);
                let notif_ctx = HookManager::notification_context(Some(&msg), Some(session_id))
                    .with_state(state_val.clone());
                let tool_name_owned = tool_name.to_string();
                let state_owned = state_val.clone();
                tokio::spawn(async move {
                    hook_manager
                        .fire(
                            &HookEvent::Notification,
                            &notif_ctx,
                            state_owned.as_deref(),
                            Some(&tool_name_owned),
                        )
                        .await;
                });
                return ChatMessage::tool(tool_call_id, msg);
            }
        }

        // Execute the tool with the trusted context.
        let result = self
            .registry
            .execute_with_context(context, tool_name, args.clone())
            .await;
        let content = match &result {
            Ok(r) => serde_json::json!({
                "success": true,
                "output_type": r.output_type,
                "content": r.content,
                "metadata": r.metadata
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "success": false,
                "error": {"message": e.message, "code": e.code}
            })
            .to_string(),
        };

        // PostToolUse hook
        let post_ctx = HookManager::post_tool_context(tool_name, &args, &content, Some(session_id))
            .with_state(state_val.clone());
        let _post_outcomes = self
            .hook_manager
            .fire(&HookEvent::PostToolUse, &post_ctx, state_str, None)
            .await;

        ChatMessage::tool(tool_call_id, &content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::hooks::{HookAction, HookDefinition, HookEvent, HookManager};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_executor(hook_manager: Arc<HookManager>) -> ToolExecutor {
        let registry = Arc::new(ToolRegistry::new());
        let policy = ToolPermissionPolicy::new(std::path::PathBuf::from("."));
        ToolExecutor::new(registry, policy).with_hooks(hook_manager)
    }

    /// RED: set_state_handle should exist and wire state_handle into the executor.
    #[test]
    fn test_state_handle_setter() {
        let mut executor = make_executor(Arc::new(HookManager::default()));
        let handle = Arc::new(RwLock::new("open".to_string()));
        executor.set_state_handle(Some(handle.clone()));
        // The state_handle should be readable (not yet used in execute_with_hooks)
        let state = executor
            .state_handle
            .as_ref()
            .unwrap()
            .try_read()
            .unwrap()
            .clone();
        assert_eq!(state, "open");
    }

    /// RED: A PreToolUse hook with when_state matching the current state should block the tool.
    #[tokio::test]
    async fn test_execute_with_hooks_blocked_by_when_state_matching() {
        let mut hm = HookManager::default();
        hm.register_workflow_hooks(vec![HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: Some("exec_command".to_string()),
            when_state: Some("open".to_string()),
            actions: vec![HookAction::Command {
                command: "echo '{\"continue_execution\":false,\"reason\":\"blocked by open-phase hook\"}'"
                    .to_string(),
                timeout_secs: 5,
            }],
        }]);
        let hook_manager = Arc::new(hm);
        let mut executor = make_executor(hook_manager);
        executor.set_state_handle(Some(Arc::new(RwLock::new("open".to_string()))));

        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("test"));
        let context = crate::agent::ToolContext {
            agent: &root,
            invocation_id: crate::agent::ToolInvocationId::new("test-1"),
        };
        let result = executor
            .execute_with_hooks(
                &context,
                "test-1",
                "exec_command",
                serde_json::json!({"command": "echo hello"}),
            )
            .await;

        let msg = result.content.unwrap_or_default();
        assert!(
            msg.contains("blocked"),
            "Expected exec_command to be blocked by hook in open phase, got: {}",
            msg
        );
    }

    /// RED: A PreToolUse hook with when_state NOT matching the current state should NOT block.
    #[tokio::test]
    async fn test_execute_with_hooks_allowed_when_state_mismatch() {
        let mut hm = HookManager::default();
        hm.register_workflow_hooks(vec![HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: Some("exec_command".to_string()),
            when_state: Some("open".to_string()), // only fires in "open"
            actions: vec![HookAction::Command {
                command: "echo '{\"continue_execution\":false,\"reason\":\"blocked by open-phase hook\"}'"
                    .to_string(),
                timeout_secs: 5,
            }],
        }]);
        let hook_manager = Arc::new(hm);
        let mut executor = make_executor(hook_manager);
        // State is "build" — hook's when_state is "open" → should NOT fire
        executor.set_state_handle(Some(Arc::new(RwLock::new("build".to_string()))));

        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("test"));
        let context = crate::agent::ToolContext {
            agent: &root,
            invocation_id: crate::agent::ToolInvocationId::new("test-2"),
        };
        let result = executor
            .execute_with_hooks(
                &context,
                "test-2",
                "exec_command",
                serde_json::json!({"command": "echo hello"}),
            )
            .await;

        let msg = result.content.unwrap_or_default();
        assert!(
            !msg.contains("blocked"),
            "Expected exec_command to NOT be blocked in build phase (when_state mismatch), got: {}",
            msg
        );
    }
}
