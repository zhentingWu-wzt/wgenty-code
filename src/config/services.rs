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
                enabled: true, // ← 这里可以改成 false 关闭总开关
                path: config_dir.join("memory.json"),
                consolidation_interval: 24,
                max_memories: default_max_memories(),
                importance_threshold: default_importance_threshold(),
                age_threshold_hours: default_age_threshold_hours(),
                enable_auto_consolidation: default_enable_auto_consolidation(),
                recall_top_n: default_recall_top_n(),
                recall_min_effective_importance: default_recall_min_effective_importance(),
                write_importance_threshold: default_write_importance_threshold(),
                max_extract_per_compaction: default_max_extract_per_compaction(),
                exploration_epsilon: default_exploration_epsilon(),
                staleness_check: default_staleness_check(),
                staleness_penalty: default_staleness_penalty(),
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
    #[serde(default)]
    pub sandbox: super::SandboxSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodegraphSettings {
    #[serde(default)]
    pub dismissed_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    pub enabled: bool,
    pub path: PathBuf,
    pub consolidation_interval: u64,
    #[serde(default = "default_max_memories")]
    pub max_memories: usize,
    #[serde(default = "default_importance_threshold")]
    pub importance_threshold: f32,
    #[serde(default = "default_age_threshold_hours")]
    pub age_threshold_hours: u64,
    #[serde(default = "default_enable_auto_consolidation")]
    pub enable_auto_consolidation: bool,
    #[serde(default = "default_recall_top_n")]
    pub recall_top_n: usize,
    /// Minimum **effective importance** floor for recall injection. Memories
    /// whose effective importance falls below this are not injected into the
    /// `<memory-context>` block.
    ///
    /// Serde alias keeps the legacy `recall_similarity_threshold` name working
    /// for existing config files — the field was renamed because it filters by
    /// effective importance (decay × hit-rate × staleness), not by Jaccard
    /// text similarity as the old name implied.
    #[serde(
        default = "default_recall_min_effective_importance",
        alias = "recall_similarity_threshold"
    )]
    pub recall_min_effective_importance: f32,
    #[serde(default = "default_write_importance_threshold")]
    pub write_importance_threshold: f32,
    #[serde(default = "default_max_extract_per_compaction")]
    pub max_extract_per_compaction: usize,
    #[serde(default = "default_exploration_epsilon")]
    pub exploration_epsilon: f32,
    #[serde(default = "default_staleness_check")]
    pub staleness_check: bool,
    #[serde(default = "default_staleness_penalty")]
    pub staleness_penalty: f32,
}

impl Default for MemorySettings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        Self {
            enabled: true,
            path: config_dir.join("memory.json"),
            consolidation_interval: 24,
            max_memories: default_max_memories(),
            importance_threshold: default_importance_threshold(),
            age_threshold_hours: default_age_threshold_hours(),
            enable_auto_consolidation: default_enable_auto_consolidation(),
            recall_top_n: default_recall_top_n(),
            recall_min_effective_importance: default_recall_min_effective_importance(),
            write_importance_threshold: default_write_importance_threshold(),
            max_extract_per_compaction: default_max_extract_per_compaction(),
            exploration_epsilon: default_exploration_epsilon(),
            staleness_check: default_staleness_check(),
            staleness_penalty: default_staleness_penalty(),
        }
    }
}

fn default_max_memories() -> usize {
    200
}
fn default_importance_threshold() -> f32 {
    0.6
}
fn default_age_threshold_hours() -> u64 {
    48
}
fn default_enable_auto_consolidation() -> bool {
    true
}
fn default_recall_top_n() -> usize {
    3
}
fn default_recall_min_effective_importance() -> f32 {
    0.3
}
fn default_write_importance_threshold() -> f32 {
    0.6
}
fn default_max_extract_per_compaction() -> usize {
    3
}
fn default_exploration_epsilon() -> f32 {
    0.0
}
fn default_staleness_check() -> bool {
    true
}
fn default_staleness_penalty() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub push_to_talk: bool,
    pub silence_threshold: f32,
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
    fn memory_settings_serde_defaults_exploration_and_staleness() {
        // Legacy configs omit the new keys; serde defaults must apply.
        let json = r#"{
            "enabled": true,
            "path": "/tmp/memory.json",
            "consolidation_interval": 24
        }"#;
        let settings: MemorySettings = serde_json::from_str(json).unwrap();
        assert!((settings.exploration_epsilon - 0.0).abs() < f32::EPSILON);
        assert!(settings.staleness_check);
        assert!((settings.staleness_penalty - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn memory_settings_default_values_exploration_and_staleness() {
        let settings = MemorySettings::default();
        assert!((settings.exploration_epsilon - 0.0).abs() < f32::EPSILON);
        assert!(settings.staleness_check);
        assert!((settings.staleness_penalty - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn memory_settings_legacy_recall_similarity_threshold_alias() {
        // Existing config files use the pre-rename key `recall_similarity_threshold`.
        // The serde alias must keep them deserializing into the renamed field.
        let json = r#"{
            "enabled": true,
            "path": "/tmp/memory.json",
            "consolidation_interval": 24,
            "recall_similarity_threshold": 0.45
        }"#;
        let settings: MemorySettings = serde_json::from_str(json).unwrap();
        assert!(
            (settings.recall_min_effective_importance - 0.45).abs() < f32::EPSILON,
            "legacy alias should populate renamed field"
        );
    }

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
        let json = r#"{"mcp_servers":[],"hooks":null}"#;
        let cfg: IntegrationsConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.codegraph.dismissed_paths.is_empty());
    }
}
