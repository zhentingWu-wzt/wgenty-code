use super::defaults::{default_max_transcript_age_days, default_transcript_db_path};
use super::guardian::GuardianSettings;
use super::mcp_config::McpConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub auto_update: bool,
    #[serde(default)]
    pub enabled_map: std::collections::HashMap<String, bool>,
    #[serde(default)]
    pub marketplaces: Option<serde_json::Value>,
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
    #[serde(default = "default_transcript_db_path")]
    pub db_path: String,
    #[serde(default = "default_max_transcript_age_days")]
    pub max_age_days: u32,
}

impl Default for TranscriptConfig {
    fn default() -> Self {
        Self {
            db_path: default_transcript_db_path(),
            max_age_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub working_dir: PathBuf,
    pub memory: MemorySettings,
    #[serde(default)]
    pub transcript: TranscriptConfig,
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
                importance_threshold: default_importance_threshold(),
                age_threshold_hours: default_age_threshold_hours(),
                enable_auto_consolidation: default_enable_auto_consolidation(),
                recall_top_n: default_recall_top_n(),
                recall_similarity_threshold: default_recall_similarity_threshold(),
            },
            transcript: TranscriptConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub mcp_servers: Vec<McpConfig>,
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
    #[serde(default)]
    pub voice: VoiceSettings,
    #[serde(default)]
    pub guardian: GuardianSettings,
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
    /// Minimum importance for a memory to survive consolidation (0.0–1.0).
    #[serde(default = "default_importance_threshold")]
    pub importance_threshold: f32,
    /// Memories older than this (in hours) and below the importance threshold
    /// are eligible for removal during consolidation.
    #[serde(default = "default_age_threshold_hours")]
    pub age_threshold_hours: u64,
    /// Whether auto-consolidation is enabled.
    #[serde(default = "default_enable_auto_consolidation")]
    pub enable_auto_consolidation: bool,
    /// Top-N memories to inject per recall (default 5).
    #[serde(default = "default_recall_top_n")]
    pub recall_top_n: usize,
    /// Topic overlap threshold (Jaccard) for triggering re-retrieval
    /// during per-turn smart recall. Range 0.0–1.0, default 0.3.
    #[serde(default = "default_recall_similarity_threshold")]
    pub recall_similarity_threshold: f32,
}

fn default_importance_threshold() -> f32 {
    0.3
}
fn default_age_threshold_hours() -> u64 {
    24
}
fn default_enable_auto_consolidation() -> bool {
    true
}
fn default_recall_top_n() -> usize {
    5
}
fn default_recall_similarity_threshold() -> f32 {
    0.3
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
        Self {
            enabled: false,
            push_to_talk: false,
            silence_threshold: 0.01,
            sample_rate: 16000,
        }
    }
}
