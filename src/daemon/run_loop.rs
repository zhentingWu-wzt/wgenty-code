//! Daemon-side agent run loop — session event layer and root tool port.
//!
//! Task 1 scope: the `SessionEvent` envelope, a broadcast hub, and a
//! `DaemonEventSink` that maps runtime events onto the hub so HTTP/SSE
//! handlers (later tasks) can fan them out to web clients.
//!
//! Task 3 scope: [`RootToolPort`], the fully-owned [`ToolPort`] the daemon's
//! root run loop executes tools through (policy → session rules → indefinite
//! bridge escalation → guardian → registry).

use crate::agent::runtime::{EventSink, RuntimeEvent};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A single event in a daemon-run session, envelope for SSE fan-out.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    /// Monotonically increasing sequence number within one run, starting at 1.
    pub seq: u64,
    pub session_id: String,
    pub run_id: String,
    pub kind: SessionEventKind,
    /// Kind-specific payload (see [`DaemonEventSink::emit`] mapping).
    pub data: serde_json::Value,
}

/// The subset of runtime events worth broadcasting to clients (v1).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    ContentDelta,
    ReasoningDelta,
    ToolStart,
    ToolResult,
    TurnDone,
    TurnError,
    Save,
}

/// Broadcast channel over which one daemon run publishes [`SessionEvent`]s.
/// Subscribers use `subscribe()`; lagging receivers see `RecvError::Lagged`.
pub type SessionEventHub = tokio::sync::broadcast::Sender<SessionEvent>;

/// [`EventSink`] that translates runtime events into [`SessionEvent`]s and
/// broadcasts them on a per-run hub. Connection noise (Connecting,
/// PreparingTools, CompactionStarted, …) is intentionally dropped in v1.
// Constructed by the run-loop spawner in a later task; allow dead_code until then.
#[allow(dead_code)]
pub struct DaemonEventSink {
    session_id: String,
    run_id: String,
    hub: SessionEventHub,
    next_seq: Arc<AtomicU64>,
}

#[allow(dead_code)] // used once the run-loop spawner lands (later task)
impl DaemonEventSink {
    pub fn new(session_id: String, run_id: String, hub: SessionEventHub) -> Self {
        Self {
            session_id,
            run_id,
            hub,
            next_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    fn publish(&self, kind: SessionEventKind, data: serde_json::Value) {
        let event = SessionEvent {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            kind,
            data,
        };
        // No subscribers is normal (e.g. run without an attached SSE client);
        // broadcast::Sender::send only errors in that case, so ignore it.
        let _ = self.hub.send(event);
    }
}

impl EventSink for DaemonEventSink {
    fn emit(&self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::ContentDelta(text) => {
                self.publish(
                    SessionEventKind::ContentDelta,
                    serde_json::json!({ "text": text }),
                );
            }
            RuntimeEvent::ReasoningDelta(text) => {
                self.publish(
                    SessionEventKind::ReasoningDelta,
                    serde_json::json!({ "text": text }),
                );
            }
            RuntimeEvent::ToolStart { name, args } => {
                self.publish(
                    SessionEventKind::ToolStart,
                    serde_json::json!({ "name": name, "args": args }),
                );
            }
            RuntimeEvent::ToolResult {
                name,
                args,
                content,
            } => {
                self.publish(
                    SessionEventKind::ToolResult,
                    serde_json::json!({ "name": name, "args": args, "content": content }),
                );
            }
            RuntimeEvent::StreamDone { finish_reason } => {
                self.publish(
                    SessionEventKind::TurnDone,
                    serde_json::json!({ "finish_reason": finish_reason }),
                );
            }
            RuntimeEvent::StreamError(message) => {
                self.publish(
                    SessionEventKind::TurnError,
                    serde_json::json!({ "message": message }),
                );
            }
            RuntimeEvent::SaveSession => {
                self.publish(SessionEventKind::Save, serde_json::json!({}));
            }
            // v1: connection noise and UI-only signals are not broadcast.
            _ => {}
        }
    }
}

// ── RootToolPort (Task 3) ────────────────────────────────────────────────────

use crate::agent::runtime::ports::{ToolPort, ToolRequest, ToolResponse};
use crate::agent::{AgentExecutionContext, SessionId, ToolContext, ToolInvocationId};
use crate::api::ToolDefinition;
use crate::daemon::state::DaemonState;
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::teams::permission_bridge::{PermissionBridge, StructuredApproval};
use crate::tools::executor::validate_tool_call_shared;
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Outcome for a policy `Ask` before touching the permission bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskDecision {
    /// Approved session rule or auto-approving root mode: run without asking.
    Execute,
    /// Escalate to the shared permission bridge and wait indefinitely.
    Escalate,
}

/// Pure Ask-branch decision (mirrors the `/tools/execute` handler ordering).
fn decide_ask(rule_approved: bool, mode_auto: bool) -> AskDecision {
    if rule_approved || mode_auto {
        AskDecision::Execute
    } else {
        AskDecision::Escalate
    }
}

/// Fully-owned [`ToolPort`] for the daemon's root run loop.
///
/// Pipeline (modeled on `GuardingToolPort` + the headless `RegistryToolPort`):
/// policy validation → shared session rules → root-mode auto-approve →
/// indefinite [`PermissionBridge`] escalation → guardian pre-check →
/// registry execution. Unlike `GuardingToolPort` everything here is owned
/// (`Arc<ToolRegistry>`, owned root [`AgentExecutionContext`]) so the port
/// outlives the stack frame that built it.
///
/// The shared `session_rules` handle comes from `DaemonState::tool_executor`,
/// so a rule approved here ("AlwaysAllow") is also visible to `/tools/execute`
/// and subagent ports, and vice versa.
// Constructed by the run-loop spawner in a later task; allow dead_code until then.
#[allow(dead_code)]
pub struct RootToolPort {
    registry: Arc<ToolRegistry>,
    policy: ToolPermissionPolicy,
    session_rules: Arc<RwLock<HashSet<String>>>,
    bridge: Arc<PermissionBridge>,
    root_mode: Arc<std::sync::RwLock<crate::config::RootPermissionMode>>,
    effective_mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    agent: AgentExecutionContext,
    workdir: Option<PathBuf>,
    session_id: String,
}

#[allow(dead_code)] // used once the run-loop spawner lands (later task)
impl RootToolPort {
    /// Build the port for a server-side root run. `workdir` is the session's
    /// bound worktree (`None` = main working dir); it is injected into every
    /// [`ToolContext`] so relative tool paths resolve inside the worktree.
    pub fn new(state: &DaemonState, session_id: &str, workdir: Option<PathBuf>) -> Self {
        Self {
            registry: Arc::clone(&state.tool_registry),
            policy: state.tool_executor.policy().clone(),
            session_rules: state.tool_executor.session_rules_handle(),
            bridge: Arc::clone(&state.permission_bridge),
            root_mode: Arc::clone(&state.root_mode),
            effective_mode: Arc::clone(&state.effective_mode),
            agent: AgentExecutionContext::root(SessionId::new(session_id)),
            workdir,
            session_id: session_id.to_string(),
        }
    }

    /// Test-only constructor: a full in-memory `DaemonState` is too heavy, so
    /// tests inject the registry / rules / bridge directly. The policy is
    /// rooted at `workdir` (falls back to the current directory).
    #[cfg(test)]
    fn new_for_test(
        registry: Arc<ToolRegistry>,
        session_rules: Arc<RwLock<HashSet<String>>>,
        bridge: Arc<PermissionBridge>,
        workdir: Option<PathBuf>,
    ) -> Self {
        let policy_root = workdir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self {
            registry,
            policy: ToolPermissionPolicy::new(policy_root),
            session_rules,
            bridge,
            root_mode: Arc::new(std::sync::RwLock::new(
                crate::config::RootPermissionMode::Normal,
            )),
            effective_mode: Arc::new(std::sync::RwLock::new(
                crate::sandbox::EffectiveMode::Normal,
            )),
            agent: AgentExecutionContext::root(SessionId::new("test")),
            workdir,
            session_id: "test".to_string(),
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

    /// Resolve a policy `Ask`: approved rule / auto-approving mode short-
    /// circuit; otherwise escalate to the bridge and wait indefinitely (the
    /// human decides when to answer). On approval the session rule is
    /// persisted into the shared rule set.
    async fn resolve_ask(
        &self,
        perm: &crate::permissions::policy::PermissionRequest,
    ) -> Result<(), ToolResponse> {
        let rule_approved = self.session_rules.read().await.contains(&perm.session_rule);
        let mode_auto = self
            .root_mode
            .read()
            .map(|m| m.auto_approves(&perm.tool_name))
            .unwrap_or(false);
        if decide_ask(rule_approved, mode_auto) == AskDecision::Execute {
            return Ok(());
        }

        let mut approval = StructuredApproval::policy_ask(
            Uuid::new_v4().to_string(),
            self.session_id.clone(),
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

        if !self.bridge.request_indefinite(approval).await {
            return Err(Self::fail(
                "permission_denied",
                format!(
                    "Permission denied for `{}`: {}",
                    perm.tool_name, perm.reason
                ),
            ));
        }

        // Approved: persist the rule so future calls (and /tools/execute,
        // which shares this rule set) skip the prompt.
        self.session_rules
            .write()
            .await
            .insert(perm.session_rule.clone());
        Ok(())
    }

    /// Guardian pre-check, copied from the headless `RegistryToolPort`:
    /// critical-risk shell commands are blocked before execution.
    fn guardian_block(&self, req: &ToolRequest) -> Option<ToolResponse> {
        if req.name != "execute_command" && req.name != "exec_command" {
            return None;
        }
        let cmd = req.arguments.get("command").and_then(|v| v.as_str())?;
        let risk = crate::runtime::guardian::classify_risk(cmd);
        if risk >= crate::runtime::guardian::RiskLevel::Critical {
            let content = format!(
                r#"{{"success":false,"error":"GUARDIAN BLOCK: critical-risk command rejected. {}"}}"#,
                cmd
            );
            return Some(ToolResponse {
                content,
                success: false,
            });
        }
        None
    }
}

#[allow(dead_code)] // used once the run-loop spawner lands (later task)
#[async_trait]
impl ToolPort for RootToolPort {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        // 1. Policy validation against the shared session rules.
        let decision = {
            let rules = self.session_rules.read().await;
            validate_tool_call_shared(
                &self.registry,
                &self.policy,
                &rules,
                &req.name,
                &req.arguments,
            )
        };
        match decision {
            Ok(PolicyDecision::Allow) => {}
            Ok(PolicyDecision::Ask(perm)) => {
                // 2. Ask → shared rules / root mode / indefinite bridge.
                if let Err(resp) = self.resolve_ask(&perm).await {
                    return resp;
                }
            }
            Err(e) => {
                return Self::fail(e.code.as_deref().unwrap_or("policy_error"), e.message);
            }
        }

        // 3. Guardian pre-check for shell tools.
        if let Some(resp) = self.guardian_block(&req) {
            return resp;
        }

        // 4. Registry execution with the session-bound workdir.
        let inv_id = req
            .invocation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        // Ensure the turn snapshot exists when the caller supplies a turn id
        // (mirrors the /tools/execute handler; Plan mode skips capture inside
        // maybe_capture_pre_edit via EffectiveMode::Plan).
        if let Some(turn_id) = req.turn_id.as_deref() {
            if let Err(e) = self.registry.checkpoint_manager.begin_turn(turn_id) {
                tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
            }
        }
        let effective_mode = self.effective_mode.read().map(|m| *m).unwrap_or_default();
        let tool_context = ToolContext {
            agent: &self.agent,
            invocation_id: ToolInvocationId::new(inv_id),
            origin_turn_id: req.turn_id.as_deref(),
            workdir: self.workdir.as_deref(),
            effective_mode,
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
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::ports::ToolPort;
    use crate::agent::runtime::{EventSink, RuntimeEvent};
    use crate::teams::permission_bridge::PermissionBridge;
    use crate::tools::ToolRegistry;
    use std::collections::HashSet;
    use std::time::Duration;
    use tokio::sync::RwLock;

    #[test]
    fn maps_runtime_events_to_session_events() {
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
        sink.emit(RuntimeEvent::ContentDelta("hi".into()));
        sink.emit(RuntimeEvent::ReasoningDelta("think".into()));
        sink.emit(RuntimeEvent::ToolStart {
            name: "file_read".into(),
            args: serde_json::json!({"path":"a"}),
        });
        sink.emit(RuntimeEvent::SaveSession);
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.kind, SessionEventKind::ContentDelta);
        assert_eq!(e1.data["text"], "hi");
        assert_eq!(e1.seq, 1);
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.kind, SessionEventKind::ReasoningDelta);
        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.kind, SessionEventKind::ToolStart);
        assert_eq!(e3.data["name"], "file_read");
        let e4 = rx.try_recv().unwrap();
        assert_eq!(e4.kind, SessionEventKind::Save);
        // seq 单调递增
        assert_eq!(e4.seq, 4);
    }

    #[test]
    fn unmapped_variants_are_skipped() {
        // Connecting/PreparingTools/StreamDone/CompactionStarted 等不产生事件
        //（v1 不广播连接噪声）；StreamError → TurnError；StreamDone → TurnDone
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
        sink.emit(RuntimeEvent::Connecting {
            attempt: 1,
            max_retries: 2,
        });
        sink.emit(RuntimeEvent::StreamDone {
            finish_reason: "stop".into(),
        });
        sink.emit(RuntimeEvent::StreamError("boom".into()));
        assert!(rx
            .try_recv()
            .is_ok_and(|e| e.kind == SessionEventKind::TurnDone));
        assert!(rx
            .try_recv()
            .is_ok_and(|e| e.kind == SessionEventKind::TurnError));
        assert!(rx.try_recv().is_err());
    }

    // ── RootToolPort ───────────────────────────────────────────────────────

    fn write_req(path: &str) -> ToolRequest {
        ToolRequest {
            name: "file_write".into(),
            arguments: serde_json::json!({"path": path, "content": "x"}),
            session_id: "test".into(),
            turn_id: None,
            invocation_id: Some("i1".into()),
            parallel: false,
        }
    }

    #[test]
    fn decide_ask_short_circuits_on_rule_or_mode() {
        assert_eq!(decide_ask(true, false), AskDecision::Execute);
        assert_eq!(decide_ask(true, true), AskDecision::Execute);
        assert_eq!(decide_ask(false, true), AskDecision::Execute);
        assert_eq!(decide_ask(false, false), AskDecision::Escalate);
    }

    #[tokio::test]
    async fn executes_in_bound_workdir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = Arc::new(ToolRegistry::with_project_root(temp.path(), 5));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let port =
            RootToolPort::new_for_test(registry, rules, bridge, Some(temp.path().to_path_buf()));

        // file_write with a relative path is inside the workspace (policy
        // Allow) and must land under the session-bound workdir.
        let resp = port.execute(write_req("a.txt")).await;
        assert!(resp.success, "write should succeed: {}", resp.content);
        let written = std::fs::read_to_string(temp.path().join("a.txt")).expect("a.txt exists");
        assert_eq!(written, "x");
    }

    #[tokio::test]
    async fn ask_suspends_until_bridge_resolve() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        // Absolute path outside the workspace → policy Ask.
        let outside = temp.path().join("outside.txt");

        let registry = Arc::new(ToolRegistry::with_project_root(&workspace, 5));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let port = Arc::new(RootToolPort::new_for_test(
            registry,
            Arc::clone(&rules),
            Arc::clone(&bridge),
            Some(workspace),
        ));

        let outside_str = outside.to_str().expect("utf8 path").to_string();
        let executing = {
            let port = Arc::clone(&port);
            tokio::spawn(async move { port.execute(write_req(&outside_str)).await })
        };

        // The execute call must suspend on a pending bridge approval.
        let mut request_id = None;
        for _ in 0..100 {
            let pending = bridge.pending().await;
            if let Some(p) = pending.first() {
                assert_eq!(p.tool, "file_write");
                assert!(p.session_rule.starts_with("path:"));
                request_id = Some(p.request_id.clone());
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let request_id = request_id.expect("approval request never appeared");
        assert!(
            !executing.is_finished(),
            "execute must stay suspended while the approval is pending"
        );
        assert!(!outside.exists(), "no side effect before approval");

        assert!(bridge.resolve(&request_id, true).await);
        let resp = executing.await.expect("execute join");
        assert!(resp.success, "approved write should run: {}", resp.content);
        assert_eq!(
            std::fs::read_to_string(&outside).expect("outside.txt written"),
            "x"
        );

        // The approved rule is persisted into the shared session rules so
        // /tools/execute and later server runs stay coherent.
        let approved_rules = rules.read().await;
        assert!(
            approved_rules.iter().any(|r| r.starts_with("path:")),
            "approved rule must be recorded: {approved_rules:?}"
        );
    }
}
