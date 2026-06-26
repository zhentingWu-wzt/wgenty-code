use crate::api::ChatMessage;
use crate::comet::{CometGuard, CometState};
use crate::runtime::guardian::{Guardian, GuardianDecision};
use crate::runtime::hooks::{HookEvent, HookManager};
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::tools::ToolRegistry;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    policy: ToolPermissionPolicy,
    session_rules: Arc<RwLock<HashSet<String>>>,
    hook_manager: Arc<HookManager>,
    guardian: Guardian,
    /// Active comet workflow state, read once from the working directory.
    /// `None` if no active comet change is in progress.
    comet_state: Option<CometState>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, policy: ToolPermissionPolicy) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let comet_state = CometState::read(&cwd);
        Self {
            registry,
            policy,
            session_rules: Arc::new(RwLock::new(HashSet::new())),
            hook_manager: Arc::new(HookManager::default()),
            guardian: Guardian::default(),
            comet_state,
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
    /// Comet phase guard runs first; if the current phase disallows this tool,
    /// a Notification hook is fired and the call is blocked.
    ///
    /// PreToolUse hooks run after the comet guard; if any hook returns
    /// `{ "continue_execution": false }` the tool call is blocked.
    /// PostToolUse hooks run after execution and cannot block.
    pub async fn execute_with_hooks(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
        session_id: Option<&str>,
    ) -> ChatMessage {
        tracing::info!(
            tool = tool_name,
            args_len = serde_json::to_string(&args).map(|s| s.len()).unwrap_or(0),
            "ToolExecutor: executing with hooks"
        );

        // ── Comet phase guard (before PreToolUse hooks) ──────────────────
        if let Some(ref state) = self.comet_state {
            let guard_args = json_args_to_strings(&args);
            let decision = CometGuard::check(&state.phase, tool_name, &guard_args);
            if decision.blocked {
                let msg = decision.error_message.unwrap_or_else(|| {
                    format!(
                        "Tool '{}' blocked by comet guard in {:?} phase",
                        tool_name, state.phase
                    )
                });
                // Fire Notification hook asynchronously (do not await)
                let hook_manager = Arc::clone(&self.hook_manager);
                let notif_ctx = HookManager::notification_context(Some(&msg), session_id);
                let tool_name_owned = tool_name.to_string();
                tokio::spawn(async move {
                    hook_manager
                        .fire(&HookEvent::Notification, &notif_ctx, None, Some(&tool_name_owned))
                        .await;
                });
                return ChatMessage::tool(tool_call_id, msg);
            }
        }

        // PreToolUse hook
        let pre_ctx = HookManager::pre_tool_context(tool_name, &args, session_id);
        let pre_outcomes = self
            .hook_manager
            .fire(&HookEvent::PreToolUse, &pre_ctx, None, None)
            .await;

        // Check if any hook blocked execution
        for outcome in &pre_outcomes {
            if !outcome.continue_execution {
                let reason = outcome.reason.as_deref().unwrap_or("unknown reason");
                return ChatMessage::tool(
                    tool_call_id,
                    format!("Tool '{}' blocked by hook: {}", tool_name, reason),
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
            .fire(&HookEvent::PostToolUse, &post_ctx, None, None)
            .await;

        ChatMessage::tool(tool_call_id, &content)
    }
}

/// Convert tool arguments from JSON to `Vec<String>` for comet guard checking.
///
/// For `exec_command` / `execute_command` tools, the JSON object's `"command"`
/// field is split on whitespace (e.g. `"git status"` → `["git", "status"]`).
/// For other tools, object values are stringified.
fn json_args_to_strings(args: &serde_json::Value) -> Vec<String> {
    match args {
        serde_json::Value::Object(obj) => {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                cmd.split_whitespace().map(|s| s.to_string()).collect()
            } else {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    })
                    .collect()
            }
        }
        serde_json::Value::String(s) => vec![s.clone()],
        _ => vec![args.to_string()],
    }
}
