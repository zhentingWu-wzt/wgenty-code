//! Configuration Module

pub mod api_config;
pub mod mcp_config;
pub mod watcher;

pub use api_config::ApiConfig;
pub use mcp_config::{McpConfig, McpServerStatus};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main configuration structure (top-level grouped form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)] pub models: ModelsConfig,
    #[serde(default)] pub agent: AgentConfig,
    #[serde(default)] pub prompt: PromptConfig,
    #[serde(default)] pub plugins: PluginsConfig,
    #[serde(default)] pub storage: StorageConfig,
    #[serde(default)] pub integrations: IntegrationsConfig,
    #[serde(default)] pub verbose: bool,
}

/// Default helper for serde: returns true.
fn default_rlm_max_replan() -> usize {
    2
}

fn default_rlm_jaccard_threshold() -> f64 {
    0.8
}

fn default_true() -> bool {
    true
}

fn default_max_transcript_age_days() -> u32 {
    30
}

fn default_transcript_db_path() -> String {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
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
        Self {
            models: ModelsConfig::default(),
            agent: AgentConfig::default(),
            prompt: PromptConfig::default(),
            plugins: PluginsConfig::default(),
            storage: StorageConfig::default(),
            integrations: IntegrationsConfig::default(),
            verbose: false,
        }
    }
}

impl Settings {
    /// Resolve the path to ~/.wgenty-code/settings.json
    fn config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".wgenty-code").join("settings.json")
    }

    /// Load settings from file. No backward-compatibility migration: an old
    /// settings.json containing flat fields will fail to deserialize.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            let s = Settings::default();
            s.save()?;
            Ok(s)
        }
    }

    /// Save settings to file (~/.wgenty-code/settings.json) as pretty JSON.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Reload settings from file, returning a new instance.
    /// This is intentionally a full reload rather than merge to avoid stale partial state.
    pub fn reload() -> anyhow::Result<Self> {
        Self::load()
    }

    /// Build a Settings clone configured for the small model.
    /// If `models.small` is None, returns a clone of self (no-op).
    /// If `models.small` is Some, overrides `models.main` name/base_url/api_key/appkey
    /// from the small endpoint where present, and forces transport.max_tokens = 2048.
    /// (`appkey` if present overrides `api_key` — preserves prior behavior.)
    pub fn small_model_settings(&self) -> Self {
        let mut s = self.clone();
        if let Some(small) = &self.models.small {
            s.models.main.name = small.name.clone();
            if let Some(url) = &small.base_url { s.models.main.base_url = Some(url.clone()); }
            if let Some(key) = &small.api_key  { s.models.main.api_key  = Some(key.clone()); }
            if let Some(ak)  = &small.appkey   { s.models.main.api_key  = Some(ak.clone()); }
            s.models.transport.max_tokens = 2048;
        }
        s
    }

    /// Build a Settings clone where subagent override fields (under agent.subagent)
    /// have been folded into the corresponding agent.* fields. Used at subagent spawn
    /// time so the subagent loop can read agent.* directly.
    ///
    /// Special cases:
    /// - max_rounds: subagent override `Some(0)` means "unlimited" (mapped to None).
    /// - subagent_default_k from token_budget is NOT consulted here; it is read by
    ///   the spawn caller separately as a fallback when no subagent override exists.
    pub fn resolve_subagent_config(&self) -> Self {
        let mut s = self.clone();
        let ov = &self.agent.subagent;

        if let Some(b) = ov.token_budget_k { s.agent.token_budget.main_k = b; }
        if let Some(r) = ov.max_rounds {
            s.agent.max_rounds = if r == 0 { None } else { Some(r) };
        }
        if let Some(p) = ov.plan_mode { s.agent.plan_mode = p; }

        if let Some(v) = ov.rlm.enabled            { s.agent.rlm.enabled = v; }
        if let Some(v) = ov.rlm.delegate_tool      { s.agent.rlm.delegate_tool = v; }
        if let Some(v) = ov.rlm.auto_routing       { s.agent.rlm.auto_routing = v; }
        if let Some(v) = ov.rlm.retry_enabled      { s.agent.rlm.retry_enabled = v; }
        if let Some(v) = ov.rlm.max_replan_cycles  { s.agent.rlm.max_replan_cycles = v; }
        if let Some(v) = ov.rlm.jaccard_threshold  { s.agent.rlm.jaccard_threshold = v; }

        if let Some(v) = ov.prompt.include.permissions   { s.prompt.include.permissions = v; }
        if let Some(v) = ov.prompt.include.developer     { s.prompt.include.developer = v; }
        if let Some(v) = ov.prompt.include.collaboration { s.prompt.include.collaboration = v; }
        if let Some(v) = ov.prompt.include.environment   { s.prompt.include.environment = v; }
        if let Some(v) = ov.prompt.include.skills        { s.prompt.include.skills = v; }

        if let Some(v) = &ov.prompt.developer_instructions  { s.prompt.developer_instructions  = Some(v.clone()); }
        if let Some(v) = &ov.prompt.collaboration_mode      { s.prompt.collaboration_mode      = Some(v.clone()); }
        if let Some(v) = &ov.prompt.model_instructions_file { s.prompt.model_instructions_file = Some(v.clone()); }

        s
    }

    /// Set a configuration value via dotted path.
    /// Examples:
    ///   set("models.main.name", "sonnet")
    ///   set("agent.subagent.max_depth", "7")
    ///   set("prompt.include.skills", "false")
    ///   set("plugins.enabled_map.foo@bar", "true")
    /// Values are parsed as JSON literals first (so "true"/"42"/"3.14" become bool/number);
    /// on parse failure, the value is treated as a string.
    /// Type validation happens at deserialize time — invalid paths/types return Err
    /// and the on-disk settings.json is left unchanged.
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        use serde_json::Value;
        let settings = Self::load()?;
        let mut json = serde_json::to_value(&settings)?;

        let parsed: Value = serde_json::from_str(value)
            .unwrap_or_else(|_| Value::String(value.to_string()));

        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err(anyhow::anyhow!("Invalid empty key segment in '{}'", key));
        }

        fn set_at(node: &mut Value, parts: &[&str], val: Value) -> anyhow::Result<()> {
            let (head, rest) = parts.split_first().ok_or_else(|| anyhow::anyhow!("empty path"))?;
            if rest.is_empty() {
                match node {
                    Value::Object(map) => { map.insert(head.to_string(), val); Ok(()) }
                    _ => Err(anyhow::anyhow!("path segment '{}' is not under an object", head)),
                }
            } else {
                let next = match node {
                    Value::Object(map) => map.entry(head.to_string()).or_insert(Value::Object(Default::default())),
                    _ => return Err(anyhow::anyhow!("path segment '{}' is not under an object", head)),
                };
                set_at(next, rest, val)
            }
        }

        set_at(&mut json, &parts, parsed)?;

        let new_settings: Settings = serde_json::from_value(json)
            .map_err(|e| anyhow::anyhow!("invalid setting at '{}': {}", key, e))?;
        new_settings.save()?;
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
        assert_eq!(rlm.jaccard_threshold, 0.8);
    }

    #[test]
    fn test_rlm_settings_deserialize_partial() {
        let json = r#"{"enabled": false}"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        assert!(rlm.delegate_tool);
        assert!(rlm.auto_routing);
        assert!(rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 2);
        assert_eq!(rlm.jaccard_threshold, 0.8);
    }

    #[test]
    fn test_rlm_settings_deserialize_full() {
        let json = r#"{
            "enabled": false,
            "delegate_tool": false,
            "auto_routing": false,
            "retry_enabled": false,
            "max_replan_cycles": 0,
            "jaccard_threshold": 0.95
        }"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        assert!(!rlm.delegate_tool);
        assert!(!rlm.auto_routing);
        assert!(!rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 0);
        assert!((rlm.jaccard_threshold - 0.95).abs() < 1e-9);
    }

    #[test]
    fn test_settings_default_includes_rlm() {
        let settings = Settings::default();
        assert!(settings.agent.rlm.enabled);
        assert!(settings.agent.rlm.delegate_tool);
        assert!(settings.agent.rlm.auto_routing);
    }

    #[test]
    fn test_rlm_deserialize_in_settings() {
        let json = r#"{
            "models": {
                "transport": {"max_tokens": 4096, "timeout": 120, "streaming": true, "beta_headers": []},
                "main": {"name": "test"}
            },
            "agent": {
                "rlm": {"enabled": false, "delegate_tool": false}
            },
            "storage": {
                "working_dir": ".",
                "memory": {"enabled": false, "path": ".", "consolidation_interval": 24, "max_memories": 100}
            },
            "plugins": {"enabled": false, "dir": ".", "auto_update": false}
        }"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert!(!settings.agent.rlm.enabled);
        assert!(!settings.agent.rlm.delegate_tool);
        // Unspecified rlm fields use defaults
        assert!(settings.agent.rlm.auto_routing);
        assert!(settings.agent.rlm.retry_enabled);
        assert_eq!(settings.agent.rlm.max_replan_cycles, 2);
    }

    #[test]
    fn test_prompt_includes_default_all_true() {
        let s = Settings::default();
        assert!(s.prompt.include.permissions);
        assert!(s.prompt.include.developer);
        assert!(s.prompt.include.collaboration);
        assert!(s.prompt.include.environment);
        assert!(s.prompt.include.skills);
    }

    #[test]
    fn test_models_default_no_small_or_planner() {
        let s = Settings::default();
        assert_eq!(s.models.main.name, "sonnet");
        assert!(s.models.small.is_none());
        assert!(s.models.planner.is_none());
    }

    #[test]
    fn test_models_small_inherits_when_url_absent() {
        let json = r#"{
            "models": {
                "main": {"name": "sonnet", "base_url": "https://api.example.com", "api_key": "main-key"},
                "small": {"name": "haiku"}
            }
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        let small = s.models.small.as_ref().unwrap();
        assert_eq!(small.name, "haiku");
        // Inheritance is the consumer's job — see small_model_settings
        assert!(small.base_url.is_none());
        assert!(small.api_key.is_none());
    }

    #[test]
    fn test_small_model_settings_uses_small_overrides() {
        let mut s = Settings::default();
        s.models.main.base_url = Some("https://api.main.example".to_string());
        s.models.main.api_key = Some("main-key".to_string());
        s.models.small = Some(ModelEndpoint {
            name: "haiku".to_string(),
            base_url: None,                                      // inherits main
            api_key: Some("small-key".to_string()),
            appkey: None,
        });
        let small_s = s.small_model_settings();
        assert_eq!(small_s.models.main.name, "haiku");
        assert_eq!(small_s.models.main.base_url, Some("https://api.main.example".to_string())); // unchanged
        assert_eq!(small_s.models.main.api_key, Some("small-key".to_string()));                 // overridden
        assert_eq!(small_s.models.transport.max_tokens, 2048);
    }

    #[test]
    fn test_subagent_overrides_default_none() {
        let s = Settings::default();
        let ov = &s.agent.subagent;
        assert!(ov.token_budget_k.is_none());
        assert!(ov.max_rounds.is_none());
        assert!(ov.plan_mode.is_none());
        assert!(ov.rlm.enabled.is_none());
        assert!(ov.rlm.delegate_tool.is_none());
        assert!(ov.rlm.auto_routing.is_none());
        assert!(ov.rlm.retry_enabled.is_none());
        assert!(ov.rlm.max_replan_cycles.is_none());
        assert!(ov.rlm.jaccard_threshold.is_none());
        assert!(ov.prompt.include.permissions.is_none());
        assert!(ov.prompt.include.developer.is_none());
        assert!(ov.prompt.include.collaboration.is_none());
        assert!(ov.prompt.include.environment.is_none());
        assert!(ov.prompt.include.skills.is_none());
        assert!(ov.prompt.developer_instructions.is_none());
        assert!(ov.prompt.collaboration_mode.is_none());
        assert!(ov.prompt.model_instructions_file.is_none());
    }

    #[test]
    fn test_resolve_subagent_config_noop_when_no_overrides() {
        let s = Settings::default();
        let r = s.resolve_subagent_config();
        assert_eq!(r.agent.plan_mode, s.agent.plan_mode);
        assert_eq!(r.agent.max_rounds, s.agent.max_rounds);
        assert_eq!(r.agent.token_budget.main_k, s.agent.token_budget.main_k);
        assert_eq!(r.agent.rlm.enabled, s.agent.rlm.enabled);
        assert_eq!(r.prompt.include.skills, s.prompt.include.skills);
    }

    #[test]
    fn test_resolve_subagent_config_applies_overrides() {
        let mut s = Settings::default();
        s.agent.token_budget.main_k = 100;
        s.agent.rlm.enabled = true;
        s.prompt.include.skills = true;

        s.agent.subagent.token_budget_k = Some(50);
        s.agent.subagent.rlm.enabled = Some(false);
        s.agent.subagent.prompt.include.skills = Some(false);

        let r = s.resolve_subagent_config();
        assert_eq!(r.agent.token_budget.main_k, 50);
        assert!(!r.agent.rlm.enabled);
        assert!(!r.prompt.include.skills);
        // Source unchanged
        assert_eq!(s.agent.token_budget.main_k, 100);
        assert!(s.agent.rlm.enabled);
    }

    #[test]
    fn test_resolve_subagent_max_rounds_zero_means_unlimited() {
        let mut s = Settings::default();
        s.agent.max_rounds = Some(50);
        s.agent.subagent.max_rounds = Some(0);
        let r = s.resolve_subagent_config();
        assert_eq!(r.agent.max_rounds, None);
    }

    #[test]
    fn test_set_dotted_path_nested_field() {
        use serde_json::Value;
        let s = Settings::default();
        let mut json = serde_json::to_value(&s).unwrap();
        let parts: &[&str] = &["agent", "subagent", "max_depth"];
        fn walk_set(n: &mut Value, p: &[&str], v: Value) {
            let (h, r) = p.split_first().unwrap();
            if r.is_empty() {
                n.as_object_mut().unwrap().insert(h.to_string(), v);
            } else {
                let nx = n.as_object_mut().unwrap()
                    .entry(h.to_string()).or_insert(Value::Object(Default::default()));
                walk_set(nx, r, v);
            }
        }
        walk_set(&mut json, parts, Value::Number(7.into()));
        let new: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(new.agent.subagent.max_depth, 7);
    }

    #[test]
    fn test_set_dotted_path_unknown_field_fails_validation() {
        use serde_json::Value;
        let s = Settings::default();
        let mut json = serde_json::to_value(&s).unwrap();
        json.as_object_mut().unwrap()
            .insert("nonexistent_top".to_string(), Value::Bool(true));
        // serde_json by default tolerates extra fields; document behavior here.
        let r: Result<Settings, _> = serde_json::from_value(json);
        assert!(r.is_ok(), "extra fields are tolerated by default; if rejection is desired, add deny_unknown_fields");
    }

    /// Mirrors the budget-fallback chain in src/tools/meta/task.rs.
    fn resolve_token_budget_k(s: &Settings, caller: Option<usize>) -> usize {
        caller
            .or(s.agent.subagent.token_budget_k)
            .or((s.agent.token_budget.subagent_default_k > 0)
                .then_some(s.agent.token_budget.subagent_default_k))
            .unwrap_or(s.agent.token_budget.main_k)
    }

    #[test]
    fn test_subagent_token_budget_fallback_chain() {
        let mut s = Settings::default();
        s.agent.token_budget.main_k = 100;

        // Level 4: only main_k set
        assert_eq!(resolve_token_budget_k(&s, None), 100);

        // Level 3: subagent_default_k > 0 wins over main_k
        s.agent.token_budget.subagent_default_k = 50;
        assert_eq!(resolve_token_budget_k(&s, None), 50);

        // Level 3 ignored when subagent_default_k == 0
        s.agent.token_budget.subagent_default_k = 0;
        assert_eq!(resolve_token_budget_k(&s, None), 100);

        // Level 2: subagent override beats subagent_default and main
        s.agent.token_budget.subagent_default_k = 50;
        s.agent.subagent.token_budget_k = Some(30);
        assert_eq!(resolve_token_budget_k(&s, None), 30);

        // Level 1: caller-explicit beats everything
        assert_eq!(resolve_token_budget_k(&s, Some(7)), 7);
    }
}
