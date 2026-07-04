use super::defaults::{default_rlm_jaccard_threshold, default_rlm_max_replan, default_true};
use serde::{Deserialize, Serialize};

/// Guardian (security review) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianSettings {
    /// Enable the guardian security review layer.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Enable LLM-based review for medium+ risk commands.
    #[serde(default)]
    pub llm_review: bool,
    /// Auto-deny commands classified as Critical risk.
    #[serde(default = "default_true")]
    pub auto_deny_critical: bool,
}

impl Default for GuardianSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            llm_review: false,
            auto_deny_critical: true,
        }
    }
}

/// RLM (Recursive Language Model) pipeline settings.
/// Controls the delegate tool, auto-routing in task, and pipeline behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSettings {
    /// Master kill switch: when false, RLM is completely unavailable
    /// regardless of other flags.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether the `delegate` tool is registered and visible to the model.
    #[serde(default = "default_true")]
    pub delegate_tool: bool,
    /// Whether `task` tool auto-routes complex tasks to the RLM pipeline.
    #[serde(default = "default_true")]
    pub auto_routing: bool,
    /// Whether RLM pipeline retries failed sub-tasks.
    #[serde(default = "default_true")]
    pub retry_enabled: bool,
    /// Max re-plan cycles when RLM executor failure rate exceeds 50%.
    /// 0 = disabled (no feedback loop). Default: 2.
    #[serde(default = "default_rlm_max_replan")]
    pub max_replan_cycles: usize,
    /// Jaccard similarity threshold for RLM claim deduplication (0.0–1.0).
    /// Claims with Jaccard index above this threshold are considered duplicates.
    /// Default: 0.8.
    #[serde(default = "default_rlm_jaccard_threshold")]
    pub jaccard_threshold: f64,
}

impl Default for RlmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            delegate_tool: true,
            auto_routing: true,
            retry_enabled: true,
            max_replan_cycles: 2,
            jaccard_threshold: 0.8,
        }
    }
}
