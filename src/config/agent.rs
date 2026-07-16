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

fn default_explore_readonly() -> bool {
    true
}

fn default_approval_timeout_secs() -> u64 {
    60
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
}
