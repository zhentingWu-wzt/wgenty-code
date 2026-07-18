//! GuardingToolPort — unified permission pipeline for subagents.
//!
//! Pipeline: allowed_tools → policy validate → Ask resolve → guardian → execute.

use crate::agent::runtime::ports::{ToolPort, ToolRequest, ToolResponse};
use crate::agent::{AgentExecutionContext, ToolContext, ToolInvocationId};
use crate::api::ToolDefinition;
use crate::config::{RootPermissionMode, SubagentAskStrategy, TimeoutDecision};
use crate::permissions::policy::{PermissionRequest, PolicyDecision, ToolPermissionPolicy};
use crate::runtime::guardian::Guardian;
use crate::teams::mailbox::TeamMessage;
use crate::teams::permission_bridge::{PermissionBridge, StructuredApproval};
use crate::tools::executor::validate_tool_call_shared;
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Permission knobs passed into a subagent loop.
#[derive(Clone)]
pub struct SubagentPermissionContext {
    pub policy: ToolPermissionPolicy,
    pub session_rules: Arc<RwLock<HashSet<String>>>,
    pub bridge: Option<Arc<PermissionBridge>>,
    pub ask_strategy: SubagentAskStrategy,
    pub approval_timeout_secs: u64,
    pub timeout_decision: TimeoutDecision,
    pub guardian: Guardian,
    pub agent_id: String,
    /// Root agent's runtime permission mode, mirrored from the TUI so subagents
    /// can short-circuit policy `Ask` (Yolo/AcceptEdits) without blocking on
    /// the approval bridge. Defaults to `Normal` (escalate/deny per ask_strategy).
    pub root_mode: RootPermissionMode,
    /// Sandbox effective mode for this subagent (includes Plan). Used when
    /// building ToolContext for shell tools.
    pub effective_mode: crate::sandbox::EffectiveMode,
    /// Shared denial reasons for finish summary.
    pub denial_log: Arc<Mutex<Vec<String>>>,
    /// Shared permission lifecycle events for progress/action_log.
    ///
    /// Each entry is `(kind, detail)` where kind is one of
    /// `permission_denied`, `approval_requested`, `approval_resolved`.
    pub event_log: Arc<Mutex<Vec<(String, String)>>>,
}

impl SubagentPermissionContext {
    /// Headless defaults: Ask fails closed (no bridge).
    pub fn headless(workspace: impl Into<PathBuf>, agent_id: impl Into<String>) -> Self {
        Self {
            policy: ToolPermissionPolicy::new(workspace.into()),
            session_rules: Arc::new(RwLock::new(HashSet::new())),
            bridge: None,
            ask_strategy: SubagentAskStrategy::EscalateToUser,
            approval_timeout_secs: 60,
            timeout_decision: TimeoutDecision::Deny,
            guardian: Guardian::default(),
            agent_id: agent_id.into(),
            root_mode: RootPermissionMode::Normal,
            effective_mode: crate::sandbox::EffectiveMode::Normal,
            denial_log: Arc::new(Mutex::new(Vec::new())),
            event_log: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Format a one-line denial summary suffix for parent-visible results.
pub fn format_permission_summary(reasons: &[String]) -> String {
    if reasons.is_empty() {
        return String::new();
    }
    let n = reasons.len();
    let last: Vec<&str> = reasons.iter().rev().take(3).map(String::as_str).collect();
    let last_joined = last.into_iter().rev().collect::<Vec<_>>().join(", ");
    format!("[permissions: {n} denials; last: {last_joined}]")
}

/// ToolPort that enforces visibility + policy + guardian before registry execute.
pub struct GuardingToolPort<'a> {
    registry: &'a ToolRegistry,
    context: &'a AgentExecutionContext,
    allowed: HashSet<String>,
    workdir: Option<PathBuf>,
    permission: SubagentPermissionContext,
    /// Root turn id shared with the parent so subagent edits fold into the
    /// same per-turn checkpoint snapshot. `None` disables capture.
    origin_turn_id: Option<String>,
}

impl<'a> GuardingToolPort<'a> {
    pub fn new(
        registry: &'a ToolRegistry,
        context: &'a AgentExecutionContext,
        allowed: HashSet<String>,
        workdir: Option<PathBuf>,
        permission: SubagentPermissionContext,
    ) -> Self {
        Self {
            registry,
            context,
            allowed,
            workdir,
            permission,
            origin_turn_id: None,
        }
    }

    /// Fold subagent file edits into the root turn's checkpoint snapshot.
    pub fn with_origin_turn_id(mut self, turn_id: Option<String>) -> Self {
        self.origin_turn_id = turn_id;
        self
    }

    fn record_denial(&self, reason: &str) {
        if let Ok(mut log) = self.permission.denial_log.lock() {
            log.push(reason.to_string());
        }
        self.record_event("permission_denied", reason);
    }

    fn record_event(&self, kind: &str, detail: impl Into<String>) {
        if let Ok(mut log) = self.permission.event_log.lock() {
            log.push((kind.to_string(), detail.into()));
        }
    }

    fn fail(code: &str, message: impl Into<String>) -> ToolResponse {
        let message = message.into();
        ToolResponse {
            content: serde_json::json!({
                "success": false,
                "error": { "message": message, "code": code }
            })
            .to_string(),
            success: false,
        }
    }

    async fn resolve_ask(&self, perm: &PermissionRequest) -> Result<(), ToolResponse> {
        {
            let rules = self.permission.session_rules.read().await;
            if rules.contains(&perm.session_rule) {
                return Ok(());
            }
        }

        // Root agent runtime mode bypass: when the root agent is in Yolo or
        // AcceptEdits mode, auto-approve matching tools without blocking on the
        // approval bridge. Guardian still runs afterward for exec tools.
        if self.permission.root_mode.auto_approves(&perm.tool_name) {
            self.record_event(
                "root_mode_bypass",
                format!(
                    "tool={} mode={:?}",
                    perm.tool_name, self.permission.root_mode
                ),
            );
            return Ok(());
        }

        match self.permission.ask_strategy {
            SubagentAskStrategy::Deny => {
                self.record_denial("permission_denied");
                return Err(Self::fail(
                    "permission_denied",
                    format!(
                        "Permission denied for `{}`: {} (ask_strategy=deny)",
                        perm.tool_name, perm.reason
                    ),
                ));
            }
            SubagentAskStrategy::EscalateToUser => {}
        }

        let Some(bridge) = self.permission.bridge.as_ref() else {
            self.record_denial("approval_unavailable");
            return Err(Self::fail(
                "approval_unavailable",
                format!(
                    "Permission requires approval for `{}` but no approval bridge is available: {}",
                    perm.tool_name, perm.reason
                ),
            ));
        };

        let mut approval = StructuredApproval::policy_ask(
            Uuid::new_v4().to_string(),
            self.permission.agent_id.clone(),
            perm.tool_name.clone(),
            perm.reason.clone(),
            perm.session_rule.clone(),
        );
        // Prefer path-shaped session rules as paths for structured consumers.
        if let Some(path) = perm.session_rule.strip_prefix("path:") {
            approval.paths = vec![path.to_string()];
        } else if let Some(cmd) = perm.session_rule.strip_prefix("command:") {
            approval.command = Some(cmd.to_string());
        }

        self.record_event(
            "approval_requested",
            format!(
                "tool={} request_id={} rule={}",
                approval.tool, approval.request_id, approval.session_rule
            ),
        );

        // Best-effort observability via mailbox (resolution still uses the bridge).
        let structured_msg = TeamMessage::approval_request_from_structured(&approval);
        if let Ok(cwd) = std::env::current_dir() {
            let safe: String = self
                .permission
                .agent_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            // Parent-facing observability file under .team/inbox (non-blocking).
            let path = cwd
                .join(".team")
                .join("inbox")
                .join(format!("approval-obs-{safe}.jsonl"));
            let mailbox = crate::teams::mailbox::Mailbox::new(path);
            let _ = mailbox.send(&structured_msg).await;
        }

        let request_id = approval.request_id.clone();
        let timeout = Duration::from_secs(self.permission.approval_timeout_secs.max(1));
        let approved = bridge.request_with_timeout(approval, timeout).await;
        self.record_event(
            "approval_resolved",
            format!("request_id={request_id} approved={approved}"),
        );
        if approved {
            return Ok(());
        }

        let _ = self.permission.timeout_decision;
        self.record_denial("permission_denied");
        Err(Self::fail(
            "permission_denied",
            format!(
                "Permission denied for `{}`: {} (denied or timed out)",
                perm.tool_name, perm.reason
            ),
        ))
    }

    fn guardian_block(&self, tool_name: &str, args: &serde_json::Value) -> Option<ToolResponse> {
        if tool_name != "execute_command" && tool_name != "exec_command" {
            return None;
        }
        let cmd = args.get("command").and_then(|v| v.as_str())?;
        let decision = self.permission.guardian.check(tool_name, cmd);
        if !decision.allowed {
            return Some(Self::fail(
                "guardian_blocked",
                format!("Guardian blocked command: {}", decision.rationale),
            ));
        }
        None
    }
}

#[async_trait]
impl ToolPort for GuardingToolPort<'_> {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        if !self.allowed.contains(&req.name) {
            self.record_denial("tool_not_allowed");
            return Self::fail(
                "tool_not_allowed",
                format!(
                    "Tool `{}` is not in the allowed tool set for this subagent",
                    req.name
                ),
            );
        }

        let decision = {
            let rules = self.permission.session_rules.read().await;
            match validate_tool_call_shared(
                self.registry,
                &self.permission.policy,
                &rules,
                &req.name,
                &req.arguments,
            ) {
                Ok(d) => d,
                Err(e) => {
                    self.record_denial(e.code.as_deref().unwrap_or("policy_error"));
                    return Self::fail(e.code.as_deref().unwrap_or("policy_error"), e.message);
                }
            }
        };

        match decision {
            PolicyDecision::Allow => {}
            PolicyDecision::Ask(perm) => {
                if let Err(resp) = self.resolve_ask(&perm).await {
                    return resp;
                }
            }
        }

        if let Some(resp) = self.guardian_block(&req.name, &req.arguments) {
            self.record_denial("guardian_blocked");
            return resp;
        }

        let inv_id = req
            .invocation_id
            .clone()
            .unwrap_or_else(|| format!("{}-inv", req.name));
        // Prefer the root turn id (shared checkpoint) over the subagent's own
        // ToolRequest turn_id, which the shared loop may leave unset.
        let turn_id = self.origin_turn_id.as_deref().or(req.turn_id.as_deref());
        let tool_context = ToolContext {
            agent: self.context,
            invocation_id: ToolInvocationId::new(inv_id),
            origin_turn_id: turn_id,
            workdir: self.workdir.as_deref(),
            effective_mode: self.permission.effective_mode,
            checkpoint: Some(self.registry.checkpoint_store.as_ref()),
        };

        match self
            .registry
            .execute_with_context(&tool_context, &req.name, req.arguments)
            .await
        {
            Ok(output) => ToolResponse {
                content: output.content,
                success: true,
            },
            Err(e) => ToolResponse {
                content: format!("Error: {}", e.message),
                success: false,
            },
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry
            .list()
            .into_iter()
            .filter(|t| self.allowed.contains(t.name()))
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::ports::ToolPort;
    use crate::agent::{AgentExecutionContext, SessionId};
    use crate::config::SubagentAskStrategy;
    use std::time::Duration;

    fn write_req(path: &str) -> ToolRequest {
        ToolRequest {
            name: "file_write".into(),
            arguments: serde_json::json!({"path": path, "content": "secret"}),
            session_id: "s".into(),
            turn_id: None,
            invocation_id: Some("i1".into()),
            parallel: false,
        }
    }

    #[test]
    fn denial_summary_suffix_format() {
        let reasons = vec![
            "tool_not_allowed".to_string(),
            "permission_denied".to_string(),
        ];
        let s = format_permission_summary(&reasons);
        assert!(s.contains("2 denials"));
        assert!(s.contains("permission_denied"));
    }

    #[test]
    fn empty_denial_summary_is_empty() {
        assert_eq!(format_permission_summary(&[]), "");
    }

    #[tokio::test]
    async fn tool_not_allowed_returns_structured_error() {
        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_read".to_string());
        let perm = SubagentPermissionContext::headless(".", "child");
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);
        let resp = port
            .execute(ToolRequest {
                name: "file_write".into(),
                arguments: serde_json::json!({"path": "x", "content": "y"}),
                session_id: "s".into(),
                turn_id: None,
                invocation_id: Some("i1".into()),
                parallel: false,
            })
            .await;
        assert!(!resp.success);
        assert!(resp.content.contains("tool_not_allowed"));
    }

    #[tokio::test]
    async fn write_outside_workspace_is_not_silent_allow() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());

        // Headless: no bridge → Ask fails closed (approval_unavailable).
        let perm = SubagentPermissionContext::headless(&workspace, "child");
        let denial_log = Arc::clone(&perm.denial_log);
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);
        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;

        assert!(!resp.success, "outside write must not silently allow");
        assert!(
            resp.content.contains("approval_unavailable")
                || resp.content.contains("permission_denied"),
            "expected permission error, got {}",
            resp.content
        );
        assert!(!outside.exists(), "side effect must not execute");
        let denials = denial_log.lock().expect("denial_log");
        assert!(!denials.is_empty());
    }

    #[tokio::test]
    async fn ask_with_session_rule_allows_without_bridge() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("allowed-outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());

        let policy = ToolPermissionPolicy::new(workspace.clone());
        let rule = policy
            .path_rule_key(outside.to_str().expect("utf8"))
            .expect("rule key ok")
            .expect("outside path needs rule");
        let mut rules = HashSet::new();
        rules.insert(rule);

        let mut perm = SubagentPermissionContext::headless(&workspace, "child");
        perm.session_rules = Arc::new(RwLock::new(rules));
        perm.bridge = None;

        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);
        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;
        assert!(
            resp.success,
            "pre-approved rule should allow: {}",
            resp.content
        );
        assert!(outside.exists(), "approved write should execute");
    }

    #[tokio::test]
    async fn ask_deny_via_bridge_has_no_side_effect() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("timeout-outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());

        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let mut perm = SubagentPermissionContext::headless(&workspace, "child");
        perm.bridge = Some(Arc::clone(&bridge));
        perm.approval_timeout_secs = 5;
        let event_log = Arc::clone(&perm.event_log);
        let denial_log = Arc::clone(&perm.denial_log);
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);

        let bridge_for_resolve = Arc::clone(&bridge);
        let wait = tokio::spawn(async move {
            for _ in 0..100 {
                let pending = bridge_for_resolve.pending().await;
                if let Some(p) = pending.first() {
                    let _ = bridge_for_resolve.resolve(&p.request_id, false).await;
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("approval request never appeared");
        });

        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;
        wait.await.expect("resolver join");

        assert!(!resp.success);
        assert!(resp.content.contains("permission_denied"));
        assert!(!outside.exists());
        let denials = denial_log.lock().expect("denial_log");
        assert!(denials.iter().any(|d| d == "permission_denied"));
        let events = event_log.lock().expect("event_log");
        assert!(
            events.iter().any(|(k, _)| k == "approval_requested"),
            "expected approval_requested event: {events:?}"
        );
        assert!(
            events.iter().any(|(k, _)| k == "approval_resolved"),
            "expected approval_resolved event: {events:?}"
        );
    }

    #[tokio::test]
    async fn ask_approve_allows_and_executes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("approve-outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());

        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let mut perm = SubagentPermissionContext::headless(&workspace, "child");
        perm.bridge = Some(Arc::clone(&bridge));
        perm.ask_strategy = SubagentAskStrategy::EscalateToUser;
        perm.approval_timeout_secs = 5;
        let event_log = Arc::clone(&perm.event_log);
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);

        let bridge_for_resolve = Arc::clone(&bridge);
        let wait = tokio::spawn(async move {
            for _ in 0..100 {
                let pending = bridge_for_resolve.pending().await;
                if let Some(p) = pending.first() {
                    assert_eq!(p.tool, "file_write");
                    assert!(!p.session_rule.is_empty());
                    let _ = bridge_for_resolve.resolve(&p.request_id, true).await;
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("approval request never appeared");
        });

        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;
        wait.await.expect("resolver join");

        assert!(
            resp.success,
            "approved write should succeed: {}",
            resp.content
        );
        assert!(outside.exists());
        let events = event_log.lock().expect("event_log");
        assert!(events
            .iter()
            .any(|(k, d)| k == "approval_resolved" && d.contains("approved=true")));
    }

    #[tokio::test]
    async fn multiple_denials_surface_in_summary() {
        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_read".to_string());
        let mut perm = SubagentPermissionContext::headless(".", "child");
        let denial_log = Arc::clone(&perm.denial_log);
        perm.ask_strategy = SubagentAskStrategy::Deny;
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);

        // Two disallowed tools.
        for name in ["file_write", "file_edit"] {
            let resp = port
                .execute(ToolRequest {
                    name: name.into(),
                    arguments: serde_json::json!({"path": "x", "content": "y"}),
                    session_id: "s".into(),
                    turn_id: None,
                    invocation_id: Some("i".into()),
                    parallel: false,
                })
                .await;
            assert!(!resp.success);
        }
        let reasons = denial_log.lock().expect("denial_log").clone();
        assert!(reasons.len() >= 2);
        let summary = format_permission_summary(&reasons);
        assert!(summary.contains("denials"));
        assert!(summary.contains("tool_not_allowed"));
    }

    #[tokio::test]
    async fn yolo_mode_bypasses_ask_without_bridge() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("yolo-outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());

        // Headless (no bridge) but with root_mode = Yolo: Ask is bypassed.
        let mut perm = SubagentPermissionContext::headless(&workspace, "child");
        perm.root_mode = RootPermissionMode::Yolo;
        let event_log = Arc::clone(&perm.event_log);
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);

        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;

        // The Ask was bypassed - no approval_unavailable error.
        assert!(
            !resp.content.contains("approval_unavailable"),
            "Yolo mode should bypass Ask, got: {}",
            resp.content
        );

        // Verify the bypass event was recorded.
        let events = event_log.lock().expect("event_log");
        assert!(
            events.iter().any(|(k, _)| k == "root_mode_bypass"),
            "expected root_mode_bypass event, got: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn accept_edits_mode_bypasses_ask_for_file_tools_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let outside = temp.path().join("edits-outside.txt");

        let registry = ToolRegistry::new();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let mut allowed = HashSet::new();
        allowed.insert("file_write".to_string());
        allowed.insert("file_edit".to_string());

        // Headless (no bridge) with root_mode = AcceptEdits: file_write Ask is bypassed.
        let mut perm = SubagentPermissionContext::headless(&workspace, "child");
        perm.root_mode = RootPermissionMode::AcceptEdits;
        let event_log = Arc::clone(&perm.event_log);
        let port = GuardingToolPort::new(&registry, &root, allowed, None, perm);

        let resp = port
            .execute(write_req(outside.to_str().expect("utf8 path")))
            .await;

        // The Ask was bypassed for file_write - no approval_unavailable error.
        assert!(
            !resp.content.contains("approval_unavailable"),
            "AcceptEdits should bypass Ask for file_write, got: {}",
            resp.content
        );

        let events = event_log.lock().expect("event_log");
        assert!(
            events.iter().any(|(k, _)| k == "root_mode_bypass"),
            "expected root_mode_bypass event, got: {:?}",
            events
        );
    }

    #[test]
    fn structured_approval_serializes_with_legacy_payload() {
        let approval = StructuredApproval::policy_ask(
            "req-1",
            "child-a",
            "file_write",
            "write path is outside the workspace: /tmp/x",
            "path:/tmp/x",
        );
        let msg = TeamMessage::approval_request_from_structured(&approval);
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"type\":\"approval_request\""));
        assert!(json.contains("\"tool\":\"file_write\""));
        assert!(json.contains("\"session_rule\":\"path:/tmp/x\""));
        // Legacy free-text payload still present.
        assert!(json.contains("\"payload\""));

        // Legacy free-text-only still deserializes.
        let legacy = r#"{"type":"approval_request","from":"a","request_id":"r","kind":"generic","payload":"please"}"#;
        let parsed: TeamMessage = serde_json::from_str(legacy).expect("legacy parse");
        match parsed {
            TeamMessage::ApprovalRequest {
                payload,
                tool,
                session_rule,
                ..
            } => {
                assert_eq!(payload, "please");
                assert!(tool.is_none());
                assert!(session_rule.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
