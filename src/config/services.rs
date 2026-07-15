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
    #[serde(default)]
    pub codegraph: CodegraphSettings,
}

/// Per-project CodeGraph guidance state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodegraphSettings {
    /// Working dirs (canonical absolute paths, deduped) where the user has
    /// dismissed install/init guidance. Suppresses both the CLI startup notice
    /// and the agent's on-demand ask.
    #[serde(default)]
    pub dismissed_paths: Vec<PathBuf>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codegraph_settings_default_empty() {
        assert!(CodegraphSettings::default().dismissed_paths.is_empty());
    }

    #[test]
    fn integrations_default_has_codegraph() {
        assert!(IntegrationsConfig::default()
            .codegraph
            .dismissed_paths
            .is_empty());
    }

    #[test]
    fn serde_roundtrip_preserves_paths() {
        let mut cfg = IntegrationsConfig::default();
        cfg.codegraph.dismissed_paths.push(PathBuf::from("/tmp/a"));
        let json = serde_json::to_string(&cfg).unwrap();
        let back: IntegrationsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.codegraph.dismissed_paths,
            vec![PathBuf::from("/tmp/a")]
        );
    }

    #[test]
    fn serde_old_config_without_codegraph_field() {
        // A pre-existing settings.json carrying no `codegraph` key must still
        // deserialize via #[serde(default)].
        let json = r#"{"mcp_servers":[],"hooks":null}"#;
        let cfg: IntegrationsConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.codegraph.dismissed_paths.is_empty());
    }
}
