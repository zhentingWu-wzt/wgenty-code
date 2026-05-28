use crate::api::ChatMessage;
use crate::hooks::{HookEvent, HookManager};
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::tools::ToolRegistry;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    policy: ToolPermissionPolicy,
    session_rules: Arc<RwLock<HashSet<String>>>,
    hook_manager: Arc<HookManager>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, policy: ToolPermissionPolicy) -> Self {
        Self {
            registry,
            policy,
            session_rules: Arc::new(RwLock::new(HashSet::new())),
            hook_manager: Arc::new(HookManager::default()),
        }
    }

    pub fn with_hooks(mut self, hook_manager: Arc<HookManager>) -> Self {
        self.hook_manager = hook_manager;
        self
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

    /// Execute a tool call directly (policy already passed).
    pub async fn execute_tool_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> ChatMessage {
        let result = self.registry.execute(tool_name, args).await;
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
    /// PreToolUse hooks run before execution; if any hook returns
    /// `{ "continue_execution": false }` the tool call is blocked.
    /// PostToolUse hooks run after execution and cannot block.
    pub async fn execute_with_hooks(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
        session_id: Option<&str>,
    ) -> ChatMessage {
        // PreToolUse hook
        let pre_ctx = HookManager::pre_tool_context(tool_name, &args, session_id);
        let pre_outcomes = self
            .hook_manager
            .fire(&HookEvent::PreToolUse, &pre_ctx)
            .await;

        // Check if any hook blocked execution
        for outcome in &pre_outcomes {
            if outcome.blocked {
                return ChatMessage::tool(
                    tool_call_id,
                    &format!("Tool '{}' blocked by hook: {}", tool_name, outcome.output),
                );
            }
        }

        // Execute the tool
        let result = self.registry.execute(tool_name, args.clone()).await;
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
        let post_ctx = HookManager::post_tool_context(tool_name, &args, &content, session_id);
        let _post_outcomes = self
            .hook_manager
            .fire(&HookEvent::PostToolUse, &post_ctx)
            .await;

        ChatMessage::tool(tool_call_id, &content)
    }
}
