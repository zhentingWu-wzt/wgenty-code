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
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_concurrent: 5,
            timeout_secs: 240,
            token_budget_k: None,
            max_rounds: None,
            plan_mode: None,
            rlm: SubagentRlmOverride::default(),
            prompt: SubagentPromptOverride::default(),
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
}
