//! User-facing sandbox settings (`integrations.sandbox`).
//!
//! Kept free of `crate::sandbox` types so `config` does not depend on the
//! sandbox module. The policy resolver maps these settings onto
//! `SecurityLevel` / `FailMode`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Permission/sandbox effective mode key for settings maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeKey {
    Plan,
    Normal,
    AcceptEdits,
    Yolo,
}

/// Per-mode fail policy in settings.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailModeSetting {
    #[default]
    HardFail,
    DegradeWithMark,
}

/// Security level name in settings (mirrors sandbox `SecurityLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxLevelSetting {
    Minimal,
    Standard,
    High,
    Paranoid,
}

/// `integrations.sandbox` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// When false, all modes force DegradeWithMark + visible bypass marks.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional per-mode security level overrides.
    #[serde(default)]
    pub defaults_by_mode: HashMap<ModeKey, SandboxLevelSetting>,
    /// Optional per-mode fail-mode overrides.
    #[serde(default)]
    pub fail_mode_by_mode: HashMap<ModeKey, FailModeSetting>,
}

fn default_true() -> bool {
    true
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            defaults_by_mode: HashMap::new(),
            fail_mode_by_mode: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_settings_defaults() {
        let s = SandboxSettings::default();
        assert!(s.enabled);
        assert!(s.defaults_by_mode.is_empty());
        assert!(s.fail_mode_by_mode.is_empty());
    }

    #[test]
    fn sandbox_settings_serde_partial() {
        let json = r#"{
            "enabled": false,
            "defaults_by_mode": { "normal": "minimal" },
            "fail_mode_by_mode": { "yolo": "hard_fail" }
        }"#;
        let s: SandboxSettings = serde_json::from_str(json).unwrap();
        assert!(!s.enabled);
        assert_eq!(
            s.defaults_by_mode.get(&ModeKey::Normal),
            Some(&SandboxLevelSetting::Minimal)
        );
        assert_eq!(
            s.fail_mode_by_mode.get(&ModeKey::Yolo),
            Some(&FailModeSetting::HardFail)
        );
    }

    #[test]
    fn sandbox_level_setting_serde_snake() {
        assert_eq!(
            serde_json::to_string(&SandboxLevelSetting::Minimal).unwrap(),
            "\"minimal\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxLevelSetting::Standard).unwrap(),
            "\"standard\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxLevelSetting::High).unwrap(),
            "\"high\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxLevelSetting::Paranoid).unwrap(),
            "\"paranoid\""
        );
    }

    #[test]
    fn empty_object_deserializes_to_defaults() {
        let s: SandboxSettings = serde_json::from_str("{}").unwrap();
        assert!(s.enabled);
        assert!(s.defaults_by_mode.is_empty());
    }
}
