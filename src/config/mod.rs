//! Configuration Module

pub mod api_config;
pub mod mcp_config;

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
}

/// Default helper for serde: returns true.
fn default_true() -> bool {
    true
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSettings {
    /// Enable plugin system
    pub enabled: bool,
    /// Plugin directory
    pub plugin_dir: PathBuf,
    /// Auto-update plugins
    pub auto_update: bool,
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

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");

        Self {
            api: ApiConfig::default(),
            mcp_servers: Vec::new(),
            model: "sonnet".to_string(),
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
            },
            hooks: None,
            developer_instructions: None,
            collaboration_mode: None,
            include_permissions_instructions: true,
            include_developer_instructions: true,
            include_collaboration_instructions: true,
            include_environment_context: true,
            include_skill_instructions: true,
            model_instructions_file: None,
            guardian: GuardianSettings::default(),
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
            let settings: Settings = serde_json::from_str(&content)?;
            Ok(settings)
        } else {
            let settings = Settings::default();
            settings.save()?;
            Ok(settings)
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

    /// Set a configuration value
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        let mut settings = Self::load()?;

        match key {
            "model" => settings.model = value.to_string(),
            "verbose" => settings.verbose = value.parse().unwrap_or(false),
            "api_key" => settings.api.api_key = Some(value.to_string()),
            "base_url" => settings.api.base_url = value.to_string(),
            "max_tokens" => settings.api.max_tokens = value.parse().unwrap_or(4096),
            "timeout" => settings.api.timeout = value.parse().unwrap_or(120),
            "streaming" => settings.api.streaming = value.parse().unwrap_or(true),
            "memory.enabled" => settings.memory.enabled = value.parse().unwrap_or(true),
            "voice.enabled" => settings.voice.enabled = value.parse().unwrap_or(false),
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
