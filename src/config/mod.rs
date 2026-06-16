//! Configuration Module

pub mod api_config;
pub mod cc_mapping;
pub mod mcp_config;
pub mod watcher;

pub use api_config::ApiConfig;
pub use mcp_config::{McpConfig, McpServerStatus};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// API configuration
    pub api: ApiConfig,
    /// MCP server configurations
    pub mcp_servers: Vec<McpConfig>,
    /// Model selection
    pub model: String,
    /// Small model for delegating simple tasks (e.g., "haiku", "gpt-4o-mini").
    /// When set, the agent can call the `task` tool with `use_small_model: true`
    /// to spawn subagents with this model instead of the main model.
    #[serde(default)]
    pub small_model: Option<String>,
    /// Base URL for the small model API. Falls back to api.base_url if not set.
    #[serde(default)]
    pub small_model_base_url: Option<String>,
    /// API key for the small model. Falls back to api.get_api_key() if not set.
    #[serde(default)]
    pub small_model_api_key: Option<String>,
    /// App key for the small model API (provider-specific, e.g., OpenRouter).
    /// When absent, falls back to small_model_api_key.
    #[serde(default)]
    pub small_model_appkey: Option<String>,
    /// Maximum subagent nesting depth. Subagents cannot spawn further
    /// subagents once this depth is reached. Default: 3.
    #[serde(default = "default_subagent_depth")]
    pub max_subagent_depth: usize,
    /// Maximum concurrent subagents. The task tool will refuse new
    /// subagent spawns when this many are already running. Default: 5.
    #[serde(default = "default_max_concurrent_subagents")]
    pub max_concurrent_subagents: usize,
    /// Maximum wall-clock seconds for a single subagent execution.
    /// Subagent loops that exceed this duration are aborted. Default: 240.
    #[serde(default = "default_subagent_timeout")]
    pub subagent_timeout_secs: u64,
    /// RLM (Recursive Language Model) pipeline settings.
    #[serde(default)]
    pub rlm: RlmSettings,
    /// Token budget in thousands (k). When cumulative token usage across
    /// all models exceeds this limit, the agent stops and signals budget
    /// exhaustion. 0 = unlimited. Default: 0.
    #[serde(default)]
    pub token_budget_k: usize,
    /// Default token budget for subagents in thousands (0 = unlimited).
    #[serde(default)]
    pub default_subagent_token_budget_k: usize,
    /// Jaccard similarity threshold for RLM claim deduplication (0.0–1.0).
    /// Claims with Jaccard index above this threshold are considered duplicates.
    /// Default: 0.8.
    #[serde(default = "default_rlm_jaccard_threshold")]
    pub rlm_jaccard_threshold: f64,
    /// Maximum LLM rounds per turn. None = use internal default (100).
    #[serde(default)]
    pub max_rounds: Option<usize>,
    /// Planner model name. When set and PlanMode is active, this model is
    /// used for plan generation while the main model handles execution.
    /// Falls back to main `model` if not configured.
    #[serde(default)]
    pub planner_model: Option<String>,
    /// Base URL for the planner model API. Falls back to api.base_url.
    #[serde(default)]
    pub planner_model_base_url: Option<String>,
    /// API key for the planner model. Falls back to api.get_api_key().
    #[serde(default)]
    pub planner_model_api_key: Option<String>,
    /// Enable plan mode: agent generates a plan before executing tools.
    /// User reviews and approves the plan before execution begins.
    #[serde(default)]
    pub plan_mode: bool,
    /// Enable verbose logging
    pub verbose: bool,
    /// Working directory
    pub working_dir: PathBuf,
    /// Memory settings
    pub memory: MemorySettings,
    /// Voice settings
    pub voice: VoiceSettings,
    /// Plugin settings
    pub plugins: PluginSettings,
    /// Hook definitions for lifecycle events
    /// Format: { "PreToolUse": [{ "command": "...", "timeout_secs": 30 }] }
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
    /// CC compatible: enabledPlugins — maps "name@publisher" to bool.
    /// Takes priority over plugins.enabled_map when both are set.
    #[serde(default, alias = "enabledPlugins")]
    pub enabled_plugins: Option<std::collections::HashMap<String, bool>>,
    /// CC compatible: pluginMarketplaces — marketplace source configuration.
    /// Merged with existing marketplace registry.
    #[serde(default, alias = "pluginMarketplaces")]
    pub plugin_marketplaces: Option<serde_json::Value>,
    /// User-defined developer instructions injected into the system prompt.
    /// When set and non-empty, wraps in <developer_instructions> tags.
    #[serde(default)]
    pub developer_instructions: Option<String>,
    /// Collaboration mode: "default", "plan", "execute", or "pair_programming".
    /// When set, injects the corresponding collaboration instructions.
    #[serde(default)]
    pub collaboration_mode: Option<String>,
    /// Include permissions instructions (sandbox mode + approval policy) in system prompt.
    #[serde(default = "default_true")]
    pub include_permissions_instructions: bool,
    /// Include developer instructions in system prompt.
    #[serde(default = "default_true")]
    pub include_developer_instructions: bool,
    /// Include collaboration mode instructions in system prompt.
    #[serde(default = "default_true")]
    pub include_collaboration_instructions: bool,
    /// Include environment context (cwd, shell, date, timezone) in system prompt.
    #[serde(default = "default_true")]
    pub include_environment_context: bool,
    /// Include skill instructions in system prompt.
    #[serde(default = "default_true")]
    pub include_skill_instructions: bool,
    /// Path to a file containing model instructions that override base instructions.
    #[serde(default)]
    pub model_instructions_file: Option<String>,
    /// Guardian (security review) configuration.
    #[serde(default)]
    pub guardian: GuardianSettings,
    /// Maximum age in days for stored transcripts. Older records are cleaned up.
    /// 0 = unlimited retention.
    #[serde(default = "default_max_transcript_age_days")]
    pub max_transcript_age_days: u32,
    /// Path to the SQLite database for subagent transcript persistence.
    /// Defaults to `~/.wgenty-code/subagent_transcripts.db`.
    #[serde(default = "default_transcript_db_path")]
    pub transcript_db_path: String,
}

/// Default helper for serde: returns true.
fn default_subagent_depth() -> usize {
    3
}

fn default_max_concurrent_subagents() -> usize {
    5
}
fn default_subagent_timeout() -> u64 {
    240
}
fn default_rlm_max_replan() -> usize {
    2
}

fn default_rlm_jaccard_threshold() -> f64 { 0.8 }

fn default_true() -> bool {
    true
}

fn default_max_transcript_age_days() -> u32 { 30 }

fn default_transcript_db_path() -> String {
    let home = dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    format!("{}/.wgenty-code/subagent_transcripts.db", home)
}

// ===== New grouped sub-config types (Task 1; will replace flat Settings in Task 2) =====

/// One model endpoint: name + optional override of base_url/api_key/appkey.
/// On `models.small` / `models.planner`, `None` for url/key/appkey means inherit from `models.main`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelEndpoint {
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub appkey: Option<String>,
}

impl ModelEndpoint {
    /// Resolve the effective base_url for this endpoint. If `self.base_url` is None,
    /// fall back to env var `API_BASE_URL`, then "https://api.anthropic.com".
    pub fn endpoint_base_url(&self) -> String {
        if let Some(u) = &self.base_url { return u.clone(); }
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string())
    }

    /// Resolve the effective api_key for this endpoint, checking env first.
    pub fn endpoint_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY").ok()
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .or_else(|| self.api_key.clone())
    }
}

/// HTTP/SSE transport-layer config shared by all model endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
    pub beta_headers: Vec<String>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            timeout: 120,
            streaming: true,
            beta_headers: vec![],
        }
    }
}

/// All model endpoints + shared transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub transport: TransportConfig,
    pub main: ModelEndpoint,
    #[serde(default)]
    pub small: Option<ModelEndpoint>,
    #[serde(default)]
    pub planner: Option<ModelEndpoint>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            transport: TransportConfig::default(),
            main: ModelEndpoint {
                name: "sonnet".to_string(),
                base_url: std::env::var("API_BASE_URL").ok(),
                api_key: std::env::var("ANTHROPIC_API_KEY").ok()
                    .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
                    .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok()),
                appkey: None,
            },
            small: None,
            planner: None,
        }
    }
}

/// Token budgets for main agent and subagents (units of 1000 tokens; 0 = unlimited).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenBudget {
    #[serde(default)]
    pub main_k: usize,
    #[serde(default)]
    pub subagent_default_k: usize,
}

/// Per-field overrides that subagents can specify. None on every field = inherit
/// the corresponding main-agent value. Resolution: see Settings::resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentRlmOverride {
    #[serde(default)] pub enabled: Option<bool>,
    #[serde(default)] pub delegate_tool: Option<bool>,
    #[serde(default)] pub auto_routing: Option<bool>,
    #[serde(default)] pub retry_enabled: Option<bool>,
    #[serde(default)] pub max_replan_cycles: Option<usize>,
    #[serde(default)] pub jaccard_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptIncludesOverride {
    #[serde(default)] pub permissions: Option<bool>,
    #[serde(default)] pub developer: Option<bool>,
    #[serde(default)] pub collaboration: Option<bool>,
    #[serde(default)] pub environment: Option<bool>,
    #[serde(default)] pub skills: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptOverride {
    #[serde(default)] pub include: SubagentPromptIncludesOverride,
    #[serde(default)] pub developer_instructions: Option<String>,
    #[serde(default)] pub collaboration_mode: Option<String>,
    #[serde(default)] pub model_instructions_file: Option<String>,
}

/// Subagent runtime limits + overrides.
/// max_depth/max_concurrent/timeout_secs are subagent-only (no main-agent counterpart).
/// The remaining fields are overrides; None = inherit from agent.* — see resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub timeout_secs: u64,

    #[serde(default)] pub token_budget_k: Option<usize>,
    #[serde(default)] pub max_rounds: Option<usize>, // Some(0) = unlimited
    #[serde(default)] pub plan_mode: Option<bool>,
    #[serde(default)] pub rlm: SubagentRlmOverride,
    #[serde(default)] pub prompt: SubagentPromptOverride,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)] pub plan_mode: bool,
    #[serde(default)] pub max_rounds: Option<usize>,
    #[serde(default)] pub token_budget: TokenBudget,
    #[serde(default)] pub subagent: SubagentLimits,
    #[serde(default)] pub rlm: RlmSettings,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            plan_mode: false,
            max_rounds: None,
            token_budget: TokenBudget::default(),
            subagent: SubagentLimits::default(),
            rlm: RlmSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptIncludes {
    #[serde(default = "default_true")] pub permissions: bool,
    #[serde(default = "default_true")] pub developer: bool,
    #[serde(default = "default_true")] pub collaboration: bool,
    #[serde(default = "default_true")] pub environment: bool,
    #[serde(default = "default_true")] pub skills: bool,
}

impl Default for PromptIncludes {
    fn default() -> Self {
        Self { permissions: true, developer: true, collaboration: true, environment: true, skills: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptConfig {
    #[serde(default)] pub include: PromptIncludes,
    #[serde(default)] pub developer_instructions: Option<String>,
    #[serde(default)] pub collaboration_mode: Option<String>,
    #[serde(default)] pub model_instructions_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub auto_update: bool,
    #[serde(default)] pub enabled_map: std::collections::HashMap<String, bool>,
    #[serde(default)] pub marketplaces: Option<serde_json::Value>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        Self {
            enabled: true,
            dir: config_dir.join("plugins"),
            auto_update: true,
            enabled_map: std::collections::HashMap::new(),
            marketplaces: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptConfig {
    #[serde(default = "default_transcript_db_path")] pub db_path: String,
    #[serde(default = "default_max_transcript_age_days")] pub max_age_days: u32,
}

impl Default for TranscriptConfig {
    fn default() -> Self {
        Self { db_path: default_transcript_db_path(), max_age_days: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub working_dir: PathBuf,
    pub memory: MemorySettings,
    #[serde(default)] pub transcript: TranscriptConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        Self {
            working_dir: PathBuf::from("."),
            memory: MemorySettings {
                enabled: true,
                path: config_dir.join("memory.json"),
                consolidation_interval: 24,
                max_memories: 1000,
            },
            transcript: TranscriptConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    #[serde(default)] pub mcp_servers: Vec<McpConfig>,
    #[serde(default)] pub hooks: Option<serde_json::Value>,
    #[serde(default)] pub voice: VoiceSettings,
    #[serde(default)] pub guardian: GuardianSettings,
}

// ===== End new sub-config types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    /// Enable memory persistence
    pub enabled: bool,
    /// Memory file path
    pub path: PathBuf,
    /// Auto-consolidation interval (hours)
    pub consolidation_interval: u64,
    /// Maximum memories to keep
    pub max_memories: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    /// Enable voice input
    pub enabled: bool,
    /// Push-to-talk mode
    pub push_to_talk: bool,
    /// Silence detection threshold
    pub silence_threshold: f32,
    /// Sample rate
    pub sample_rate: u32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self { enabled: false, push_to_talk: false, silence_threshold: 0.01, sample_rate: 16000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSettings {
    /// Enable plugin system
    pub enabled: bool,
    /// Plugin directory
    pub plugin_dir: PathBuf,
    /// Auto-update plugins
    pub auto_update: bool,
    /// CC-compatible: enabled plugins map (keyed by "name@publisher")
    #[serde(default)]
    pub enabled_map: std::collections::HashMap<String, bool>,
}

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

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");

        Self {
            api: ApiConfig::default(),
            mcp_servers: Vec::new(),
            model: "sonnet".to_string(),
            small_model: None,
            small_model_base_url: None,
            small_model_api_key: None,
            small_model_appkey: None,
            max_subagent_depth: 3,
            max_concurrent_subagents: 5,
            subagent_timeout_secs: 240,
            rlm: RlmSettings::default(),
            token_budget_k: 0,
            default_subagent_token_budget_k: 0,
            rlm_jaccard_threshold: 0.8,
            max_rounds: None,
            planner_model: None,
            planner_model_base_url: None,
            planner_model_api_key: None,
            plan_mode: false,
            verbose: false,
            working_dir: PathBuf::from("."),
            memory: MemorySettings {
                enabled: true,
                path: config_dir.join("memory.json"),
                consolidation_interval: 24,
                max_memories: 1000,
            },
            voice: VoiceSettings {
                enabled: false,
                push_to_talk: false,
                silence_threshold: 0.01,
                sample_rate: 16000,
            },
            plugins: PluginSettings {
                enabled: true,
                plugin_dir: config_dir.join("plugins"),
                auto_update: true,
                enabled_map: std::collections::HashMap::new(),
            },
            hooks: None,
            enabled_plugins: None,
            plugin_marketplaces: None,
            developer_instructions: None,
            collaboration_mode: None,
            include_permissions_instructions: true,
            include_developer_instructions: true,
            include_collaboration_instructions: true,
            include_environment_context: true,
            include_skill_instructions: true,
            model_instructions_file: None,
            guardian: GuardianSettings::default(),
            max_transcript_age_days: 30,
            transcript_db_path: default_transcript_db_path(),
        }
    }
}

impl Settings {
    /// Load settings from file
    pub fn load() -> anyhow::Result<Self> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_path = home.join(".wgenty-code").join("settings.json");

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let mut settings: Settings = serde_json::from_str(&content)?;
            // Migrate legacy flat RLM keys into the rlm group.
            Self::migrate_rlm_settings(&content, &mut settings);
            cc_mapping::CcConfigMapper::apply_mappings(&mut settings);
            Ok(settings)
        } else {
            let settings = Settings::default();
            settings.save()?;
            Ok(settings)
        }
    }

    /// Migrate legacy flat `rlm_retry_enabled` / `rlm_max_replan_cycles` keys
    /// from the raw JSON into `Settings.rlm`. Only touch rlm fields when the
    /// raw JSON contains the legacy key AND the rlm group was not provided.
    fn migrate_rlm_settings(raw_json: &str, settings: &mut Settings) {
        let Ok(raw) = serde_json::from_str::<serde_json::Value>(raw_json) else {
            return;
        };
        // If the new "rlm" group is present, legacy keys are ignored.
        if raw.get("rlm").is_some() {
            return;
        }
        let mut migrated = false;
        if let Some(val) = raw.get("rlm_retry_enabled").and_then(|v| v.as_bool()) {
            settings.rlm.retry_enabled = val;
            migrated = true;
        }
        if let Some(val) = raw.get("rlm_max_replan_cycles").and_then(|v| v.as_u64()) {
            settings.rlm.max_replan_cycles = val as usize;
            migrated = true;
        }
        if migrated {
            tracing::info!(
                target: "config",
                rlm_retry = settings.rlm.retry_enabled,
                rlm_replan = settings.rlm.max_replan_cycles,
                "Migrated legacy RLM config keys into rlm group"
            );
        }
    }

    /// Save settings to file
    pub fn save(&self) -> anyhow::Result<()> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("settings.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    /// Reload settings from file, returning a new instance.
    /// This is intentionally a full reload rather than merge to avoid stale partial state.
    pub fn reload() -> anyhow::Result<Self> {
        Self::load()
    }

    /// Build settings for the small model. Falls back to main model config when
    /// small_model fields are absent.
    pub fn small_model_settings(&self) -> Self {
        let mut s = self.clone();
        if let Some(ref m) = self.small_model {
            s.model = m.clone();
        }
        s.api.max_tokens = 2048;
        if let Some(ref url) = self.small_model_base_url {
            s.api.base_url = url.clone();
        }
        if let Some(ref key) = self.small_model_api_key {
            s.api.api_key = Some(key.clone());
        }
        s
    }

    /// Set a configuration value
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        let mut settings = Self::load()?;

        match key {
            "model" => settings.model = value.to_string(),
            "verbose" => settings.verbose = value.parse().unwrap_or(false),
            "api_key" => settings.api.api_key = Some(value.to_string()),
            "base_url" => settings.api.base_url = value.to_string(),
            "small_model" => settings.small_model = Some(value.to_string()),
            "small_model_base_url" => settings.small_model_base_url = Some(value.to_string()),
            "small_model_api_key" => settings.small_model_api_key = Some(value.to_string()),
            "small_model_appkey" => settings.small_model_appkey = Some(value.to_string()),
            "max_subagent_depth" => settings.max_subagent_depth = value.parse().unwrap_or(3),
            "max_concurrent_subagents" => {
                settings.max_concurrent_subagents = value.parse().unwrap_or(5)
            }
            "subagent_timeout_secs" => {
                settings.subagent_timeout_secs = value.parse().unwrap_or(240)
            }
            // rlm group — new canonical keys
            "rlm.enabled" => settings.rlm.enabled = value.parse().unwrap_or(true),
            "rlm.delegate_tool" => settings.rlm.delegate_tool = value.parse().unwrap_or(true),
            "rlm.auto_routing" => settings.rlm.auto_routing = value.parse().unwrap_or(true),
            "rlm.retry_enabled" => settings.rlm.retry_enabled = value.parse().unwrap_or(true),
            "rlm.max_replan_cycles" => {
                settings.rlm.max_replan_cycles = value.parse().unwrap_or(2)
            }
            // legacy aliases (backward compatible)
            "rlm_retry_enabled" => settings.rlm.retry_enabled = value.parse().unwrap_or(true),
            "rlm_max_replan_cycles" => {
                settings.rlm.max_replan_cycles = value.parse().unwrap_or(2)
            }
            "token_budget_k" => settings.token_budget_k = value.parse().unwrap_or(0),
            "default_subagent_token_budget_k" => settings.default_subagent_token_budget_k = value.parse().unwrap_or(0),
            "rlm_jaccard_threshold" => settings.rlm_jaccard_threshold = value.parse().unwrap_or(0.8),
            "planner_model" => settings.planner_model = Some(value.to_string()),
            "planner_model_base_url" => settings.planner_model_base_url = Some(value.to_string()),
            "planner_model_api_key" => settings.planner_model_api_key = Some(value.to_string()),
            "plan_mode" => settings.plan_mode = value.parse().unwrap_or(false),
            "max_tokens" => settings.api.max_tokens = value.parse().unwrap_or(4096),
            "timeout" => settings.api.timeout = value.parse().unwrap_or(120),
            "streaming" => settings.api.streaming = value.parse().unwrap_or(true),
            "memory.enabled" => settings.memory.enabled = value.parse().unwrap_or(true),
            "voice.enabled" => settings.voice.enabled = value.parse().unwrap_or(false),
            // CC-compatible: enabledPlugins.<plugin@publisher>
            _ if key.starts_with("enabledPlugins.") => {
                let plugin_key = key.strip_prefix("enabledPlugins.").unwrap();
                let enabled = value.parse().unwrap_or(true);
                if let Some(ref mut map) = settings.enabled_plugins {
                    map.insert(plugin_key.to_string(), enabled);
                } else {
                    let mut map = std::collections::HashMap::new();
                    map.insert(plugin_key.to_string(), enabled);
                    settings.enabled_plugins = Some(map);
                }
            }
            // CC-compatible: pluginMarketplaces.<name>
            _ if key.starts_with("pluginMarketplaces.") => {
                // Store as a nested JSON value
                let mkt_name = key.strip_prefix("pluginMarketplaces.").unwrap();
                let parsed: serde_json::Value = serde_json::from_str(value)
                    .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
                if let Some(ref mut map) = settings.plugin_marketplaces {
                    if let Some(obj) = map.as_object_mut() {
                        obj.insert(mkt_name.to_string(), parsed);
                    }
                } else {
                    let mut map = serde_json::Map::new();
                    map.insert(mkt_name.to_string(), parsed);
                    settings.plugin_marketplaces = Some(serde_json::Value::Object(map));
                }
            }
            "max_transcript_age_days" => settings.max_transcript_age_days = value.parse().unwrap_or(30),
            "transcript_db_path" => settings.transcript_db_path = value.to_string(),
            _ => return Err(anyhow::anyhow!("Unknown setting: {}", key)),
        }

        settings.save()?;
        Ok(())
    }

    /// Reset settings to defaults
    pub fn reset() -> anyhow::Result<()> {
        let settings = Settings::default();
        settings.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlm_settings_default_all_enabled() {
        let rlm = RlmSettings::default();
        assert!(rlm.enabled);
        assert!(rlm.delegate_tool);
        assert!(rlm.auto_routing);
        assert!(rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 2);
    }

    #[test]
    fn test_rlm_settings_deserialize_partial() {
        let json = r#"{"enabled": false}"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        // Other fields should use defaults
        assert!(rlm.delegate_tool);
        assert!(rlm.auto_routing);
        assert!(rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 2);
    }

    #[test]
    fn test_rlm_settings_deserialize_full() {
        let json = r#"{
            "enabled": false,
            "delegate_tool": false,
            "auto_routing": false,
            "retry_enabled": false,
            "max_replan_cycles": 0
        }"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        assert!(!rlm.delegate_tool);
        assert!(!rlm.auto_routing);
        assert!(!rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 0);
    }

    #[test]
    fn test_migrate_rlm_legacy_keys() {
        // Simulate old config format with flat keys
        let old_json = r#"{
            "model": "sonnet",
            "rlm_retry_enabled": false,
            "rlm_max_replan_cycles": 5
        }"#;
        let mut settings = Settings::default();
        Settings::migrate_rlm_settings(old_json, &mut settings);
        // Legacy values should be copied into rlm group
        assert!(!settings.rlm.retry_enabled);
        assert_eq!(settings.rlm.max_replan_cycles, 5);
        // Fields not in old JSON stay at defaults
        assert!(settings.rlm.enabled);
        assert!(settings.rlm.delegate_tool);
    }

    #[test]
    fn test_migrate_rlm_no_override_when_group_present() {
        // When the new "rlm" group is present, legacy flat keys are ignored.
        // migrate_rlm_settings returns early without touching anything.
        let json = r#"{
            "rlm": {"enabled": false, "retry_enabled": true},
            "rlm_retry_enabled": false
        }"#;
        let mut settings = Settings::default();
        Settings::migrate_rlm_settings(json, &mut settings);
        // rlm group present -> migration returns early, legacy key is ignored
        // settings.rlm fields remain at their default values
        assert!(settings.rlm.enabled);
        assert!(settings.rlm.delegate_tool);
        assert!(settings.rlm.retry_enabled); // legacy rlm_retry_enabled:false is ignored
    }

    #[test]
    fn test_settings_default_includes_rlm() {
        let settings = Settings::default();
        assert!(settings.rlm.enabled);
        assert!(settings.rlm.delegate_tool);
        assert!(settings.rlm.auto_routing);
    }

    #[test]
    fn test_rlm_deserialize_in_settings() {
        let json = r#"{
            "api": {"base_url": "http://localhost", "max_tokens": 4096, "timeout": 120, "streaming": true, "beta_headers": []},
            "mcp_servers": [],
            "model": "test",
            "verbose": false,
            "working_dir": ".",
            "memory": {"enabled": false, "path": ".", "consolidation_interval": 24, "max_memories": 100},
            "voice": {"enabled": false, "push_to_talk": false, "silence_threshold": 0.0, "sample_rate": 16000},
            "plugins": {"enabled": false, "plugin_dir": ".", "auto_update": false},
            "rlm": {"enabled": false, "delegate_tool": false}
        }"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert!(!settings.rlm.enabled);
        assert!(!settings.rlm.delegate_tool);
        // Unspecified rlm fields use defaults
        assert!(settings.rlm.auto_routing);
        assert!(settings.rlm.retry_enabled);
        assert_eq!(settings.rlm.max_replan_cycles, 2);
    }
}
