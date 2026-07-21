use serde::{Deserialize, Serialize};

use super::guardian::RlmSettings;
use super::models::TokenBudget;

/// Per-field overrides that subagents can specify. None on every field = inherit
/// the corresponding main-agent value. Resolution: see Settings::resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentRlmOverride {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub delegate_tool: Option<bool>,
    #[serde(default)]
    pub auto_routing: Option<bool>,
    #[serde(default)]
    pub retry_enabled: Option<bool>,
    #[serde(default)]
    pub max_replan_cycles: Option<usize>,
    #[serde(default)]
    pub jaccard_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptIncludesOverride {
    #[serde(default)]
    pub permissions: Option<bool>,
    #[serde(default)]
    pub developer: Option<bool>,
    #[serde(default)]
    pub collaboration: Option<bool>,
    #[serde(default)]
    pub environment: Option<bool>,
    #[serde(default)]
    pub skills: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptOverride {
    #[serde(default)]
    pub include: SubagentPromptIncludesOverride,
    #[serde(default)]
    pub developer_instructions: Option<String>,
    #[serde(default)]
    pub collaboration_mode: Option<String>,
    #[serde(default)]
    pub model_instructions_file: Option<String>,
}

/// How a subagent resolves policy `Ask` when no session rule matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentAskStrategy {
    /// Escalate to the user via the root permission UI / bridge.
    #[default]
    EscalateToUser,
    /// Fail closed without prompting.
    Deny,
}

/// Decision applied when an escalated approval times out.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutDecision {
    /// Deny the tool call (fail closed).
    #[default]
    Deny,
}

/// Where subagent trace events are emitted (design D6).
///
/// - `file`: append JSONL to `<trace_dir>/<session_id>.jsonl` (default).
/// - `daemon`: broadcast to the daemon SSE trace stream (no file).
/// - `both`: file + daemon.
/// - `off`: disable streaming entirely.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TraceSinkMode {
    #[default]
    File,
    Daemon,
    Both,
    Off,
}

impl TraceSinkMode {
    /// Whether this mode writes the project-local JSONL trace file.
    pub fn writes_file(self) -> bool {
        matches!(self, TraceSinkMode::File | TraceSinkMode::Both)
    }

    /// Whether this mode feeds the daemon broadcast/SSE channel.
    pub fn writes_daemon(self) -> bool {
        matches!(self, TraceSinkMode::Daemon | TraceSinkMode::Both)
    }
}

/// Root agent's runtime permission mode, mirrored to subagents so they can
/// short-circuit policy `Ask` without blocking on the approval bridge.
///
/// This is a runtime (TUI) concept, not a static setting: the TUI pushes its
/// current mode to the daemon, which forwards it to each spawned subagent's
/// [`crate::teams::guarding_tool_port::SubagentPermissionContext`].
///
/// - `Normal`: no short-circuit; `Ask` follows `ask_strategy` (escalate/deny).
/// - `AcceptEdits`: auto-approve `Ask` for mutating filesystem tools only.
/// - `Yolo`: auto-approve every `Ask` (guardian still runs afterwards).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RootPermissionMode {
    #[default]
    Normal,
    AcceptEdits,
    Yolo,
}

impl RootPermissionMode {
    /// Whether this mode auto-approves a policy `Ask` for the given tool.
    ///
    /// `Yolo` approves everything; `AcceptEdits` approves only mutating
    /// filesystem tools; `Normal` approves nothing.
    pub fn auto_approves(&self, tool_name: &str) -> bool {
        match self {
            RootPermissionMode::Yolo => true,
            RootPermissionMode::AcceptEdits => {
                matches!(tool_name, "file_write" | "file_edit" | "apply_patch")
            }
            RootPermissionMode::Normal => false,
        }
    }
}

fn default_explore_readonly() -> bool {
    true
}

fn default_approval_timeout_secs() -> u64 {
    60
}

/// Subagent trace streaming config (design D3/D6).
///
/// `dir = None` resolves to `<project_root>/.wgenty-code/traces` at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceConfig {
    #[serde(default)]
    pub sink: TraceSinkMode,
    /// Optional override for the trace directory. `None` => project-local
    /// `.wgenty-code/traces` (resolved at the dispatch site).
    #[serde(default)]
    pub dir: Option<std::path::PathBuf>,
    /// Char-boundary truncation threshold for the failing round's
    /// assistant text + final tool output captured in `FailedRoundContext`.
    /// Default 2000 (design D6 / Q4).
    #[serde(default = "default_context_char_limit")]
    pub context_char_limit: usize,
}

fn default_context_char_limit() -> usize {
    2000
}

impl Default for SubagentTraceConfig {
    fn default() -> Self {
        Self {
            sink: TraceSinkMode::default(),
            dir: None,
            context_char_limit: default_context_char_limit(),
        }
    }
}

/// Subagent runtime limits + overrides.
/// max_depth/max_concurrent/timeout_secs are subagent-only (no main-agent counterpart).
/// The remaining fields are overrides; None = inherit from agent.* — see resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub timeout_secs: u64,

    #[serde(default)]
    pub token_budget_k: Option<usize>,
    #[serde(default)]
    pub max_rounds: Option<usize>, // Some(0) = unlimited
    #[serde(default)]
    pub plan_mode: Option<bool>,
    #[serde(default)]
    pub rlm: SubagentRlmOverride,
    #[serde(default)]
    pub prompt: SubagentPromptOverride,

    /// Optional permission mode override. `None` means follow the root session's
    /// shared policy + session_rules (design: mode follow).
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// How policy Ask is resolved for subagents.
    #[serde(default)]
    pub ask_strategy: SubagentAskStrategy,
    /// When true, explore/plan agents cannot see mutating FS tools.
    #[serde(default = "default_explore_readonly")]
    pub explore_readonly: bool,
    /// Timeout for escalated user approvals.
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    /// Decision when approval wait times out.
    #[serde(default)]
    pub timeout_decision: TimeoutDecision,
    /// Ordered list of fallback model names for model-unavailable failures.
    /// On a `subagent_model_unavailable` failure, the first entry different
    /// from the failed child's model is selected and swapped in (reusing the
    /// original endpoint). Empty (default) => model failures degrade to the
    /// parent model (current behavior).
    #[serde(default)]
    pub fallback_models: Vec<String>,
    /// Subagent trace streaming config (JSONL file + daemon SSE).
    #[serde(default)]
    pub trace: SubagentTraceConfig,
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self {
            max_depth: 1,
            max_concurrent: 5,
            timeout_secs: 1800,
            token_budget_k: None,
            max_rounds: None,
            plan_mode: None,
            rlm: SubagentRlmOverride::default(),
            prompt: SubagentPromptOverride::default(),
            permission_mode: None,
            ask_strategy: SubagentAskStrategy::default(),
            explore_readonly: default_explore_readonly(),
            approval_timeout_secs: default_approval_timeout_secs(),
            timeout_decision: TimeoutDecision::default(),
            fallback_models: Vec::new(),
            trace: SubagentTraceConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub max_rounds: Option<usize>,
    #[serde(default)]
    pub token_budget: TokenBudget,
    #[serde(default)]
    pub subagent: SubagentLimits,
    #[serde(default)]
    pub rlm: RlmSettings,
    /// Autonomous worker (s11): background claimer for ready task-groups.
    #[serde(default)]
    pub autonomous: AutonomousConfig,
    /// Per-turn file checkpoint settings (non-destructive snapshots).
    #[serde(default)]
    pub checkpoint: CheckpointSettings,
    /// ExecutionSession inner layer (turn chain + verify-gate). When enabled,
    /// frontends construct a `SessionCoordinator`, wire it into the agent loop
    /// turn hook, and register the `verify_and_complete` tool. Prepared in
    /// Task 7; frontend wiring lands in a follow-up. Default: `true`.
    #[serde(default)]
    pub exec_session: ExecSessionSettings,
}

/// ExecutionSession inner-layer settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecSessionSettings {
    /// Master switch for the exec_session inner layer (Task 7). When `false`,
    /// frontends skip coordinator construction and the agent loop runs without
    /// turn-boundary snapshots or the `verify_and_complete` tool.
    #[serde(default = "default_exec_session_enabled")]
    pub enabled: bool,
    /// Maximum verify retries per node before escalating (outer layer node
    /// state machine). Default: 2.
    #[serde(default = "default_auto_retry_max")]
    pub auto_retry_max: u32,
}

fn default_exec_session_enabled() -> bool {
    true
}

fn default_auto_retry_max() -> u32 {
    2
}

impl Default for ExecSessionSettings {
    fn default() -> Self {
        Self {
            enabled: default_exec_session_enabled(),
            auto_retry_max: default_auto_retry_max(),
        }
    }
}

/// Retention settings for per-turn file checkpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSettings {
    /// Number of most-recent turn snapshots to keep on disk. Older turns are
    /// pruned when a new turn begins. Default: 10.
    #[serde(default = "default_checkpoint_keep_n")]
    pub keep_n: usize,
}

fn default_checkpoint_keep_n() -> usize {
    10
}

impl Default for CheckpointSettings {
    fn default() -> Self {
        Self {
            keep_n: default_checkpoint_keep_n(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Poll interval in seconds (default 2).
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Max consecutive idle polls before stopping (default 30).
    #[serde(default = "default_max_idle_polls")]
    pub max_idle_polls: u16,
}

fn default_poll_interval_secs() -> u64 {
    2
}
fn default_max_idle_polls() -> u16 {
    30
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: default_poll_interval_secs(),
            max_idle_polls: default_max_idle_polls(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_permission_defaults() {
        let limits = SubagentLimits::default();
        assert!(limits.explore_readonly);
        assert_eq!(limits.ask_strategy, SubagentAskStrategy::EscalateToUser);
        assert_eq!(limits.approval_timeout_secs, 60);
        assert_eq!(limits.timeout_decision, TimeoutDecision::Deny);
        assert!(limits.permission_mode.is_none());
    }

    #[test]
    fn subagent_permission_serde_defaults_when_omitted() {
        let json = r#"{
            "max_depth": 1,
            "max_concurrent": 5,
            "timeout_secs": 1800
        }"#;
        let limits: SubagentLimits = serde_json::from_str(json).expect("deserialize");
        assert!(limits.explore_readonly);
        assert_eq!(limits.ask_strategy, SubagentAskStrategy::EscalateToUser);
        assert_eq!(limits.approval_timeout_secs, 60);
        assert_eq!(limits.timeout_decision, TimeoutDecision::Deny);
        assert!(limits.permission_mode.is_none());
    }

    #[test]
    fn root_permission_mode_auto_approves() {
        // Normal: never auto-approves.
        assert!(!RootPermissionMode::Normal.auto_approves("file_write"));
        assert!(!RootPermissionMode::Normal.auto_approves("execute_command"));

        // AcceptEdits: only mutating filesystem tools.
        assert!(RootPermissionMode::AcceptEdits.auto_approves("file_write"));
        assert!(RootPermissionMode::AcceptEdits.auto_approves("file_edit"));
        assert!(RootPermissionMode::AcceptEdits.auto_approves("apply_patch"));
        assert!(!RootPermissionMode::AcceptEdits.auto_approves("execute_command"));
        assert!(!RootPermissionMode::AcceptEdits.auto_approves("file_read"));

        // Yolo: everything.
        assert!(RootPermissionMode::Yolo.auto_approves("file_write"));
        assert!(RootPermissionMode::Yolo.auto_approves("execute_command"));
        assert!(RootPermissionMode::Yolo.auto_approves("anything"));
    }

    #[test]
    fn root_permission_mode_serde_roundtrip() {
        for mode in [
            RootPermissionMode::Normal,
            RootPermissionMode::AcceptEdits,
            RootPermissionMode::Yolo,
        ] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let back: RootPermissionMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, back);
        }
        // Snake-case serialization.
        assert_eq!(
            serde_json::to_string(&RootPermissionMode::AcceptEdits).unwrap(),
            "\"accept_edits\""
        );
    }

    #[test]
    fn exec_session_auto_retry_max_defaults_to_2() {
        let settings = ExecSessionSettings::default();
        assert_eq!(settings.auto_retry_max, 2);
    }

    #[test]
    fn exec_session_auto_retry_max_serde_default_when_omitted() {
        let json = r#"{"enabled": true}"#;
        let settings: ExecSessionSettings = serde_json::from_str(json).expect("deserialize");
        assert_eq!(settings.auto_retry_max, 2);
        assert!(settings.enabled);
    }
}
