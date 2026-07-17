//! Mode → sandbox profile resolution (pure).
//!
//! Maps [`EffectiveMode`] + [`crate::config::SandboxSettings`] to a
//! [`ResolvedSandboxPolicy`] used by shell/exec tools. No process-global mode.

use crate::config::{
    FailModeSetting, ModeKey, SandboxLevelSetting, SandboxSettings,
};
use crate::sandbox::{NetworkPolicy, SandboxConfig, SandboxProfile, SecurityLevel};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime mode used for sandbox resolution (includes TUI Plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveMode {
    Plan,
    #[default]
    Normal,
    AcceptEdits,
    Yolo,
}

impl EffectiveMode {
    pub fn as_mode_key(self) -> ModeKey {
        match self {
            Self::Plan => ModeKey::Plan,
            Self::Normal => ModeKey::Normal,
            Self::AcceptEdits => ModeKey::AcceptEdits,
            Self::Yolo => ModeKey::Yolo,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Normal => "normal",
            Self::AcceptEdits => "accept_edits",
            Self::Yolo => "yolo",
        }
    }

    /// Map from root permission mode (Plan is not representable → use
    /// [`EffectiveMode::Plan`] explicitly from the TUI).
    pub fn from_root_permission_mode(mode: crate::config::RootPermissionMode) -> Self {
        use crate::config::RootPermissionMode;
        match mode {
            RootPermissionMode::Normal => Self::Normal,
            RootPermissionMode::AcceptEdits => Self::AcceptEdits,
            RootPermissionMode::Yolo => Self::Yolo,
        }
    }
}

/// How to behave when sandbox infrastructure fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailMode {
    /// Never direct-spawn; return ToolError.
    HardFail,
    /// Direct spawn allowed; must mark `sandbox_bypassed`.
    DegradeWithMark,
}

impl FailMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HardFail => "hard_fail",
            Self::DegradeWithMark => "degrade_with_mark",
        }
    }
}

/// Where the resolved policy came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    Default,
    SettingsOverride,
    Disabled,
}

/// Output of [`SandboxPolicyResolver::resolve`].
#[derive(Debug, Clone)]
pub struct ResolvedSandboxPolicy {
    pub level: SecurityLevel,
    pub fail_mode: FailMode,
    pub profile: SandboxProfile,
    pub enabled: bool,
    pub source: PolicySource,
}

/// Pure resolver: mode + settings + workspace → policy.
pub struct SandboxPolicyResolver;

impl SandboxPolicyResolver {
    pub fn resolve(
        mode: EffectiveMode,
        settings: &SandboxSettings,
        workspace: impl Into<PathBuf>,
    ) -> ResolvedSandboxPolicy {
        Self::resolve_with_network(mode, settings, workspace, None)
    }

    /// Like [`resolve`](Self::resolve), optionally forcing network policy
    /// (e.g. `run_test` with `allow_network=true`) without loosening level.
    pub fn resolve_with_network(
        mode: EffectiveMode,
        settings: &SandboxSettings,
        workspace: impl Into<PathBuf>,
        network: Option<NetworkPolicy>,
    ) -> ResolvedSandboxPolicy {
        let workspace = workspace.into();

        if !settings.enabled {
            let level = Self::default_level(mode);
            let profile = Self::build_profile(level, &workspace, network);
            return ResolvedSandboxPolicy {
                level,
                fail_mode: FailMode::DegradeWithMark,
                profile,
                enabled: false,
                source: PolicySource::Disabled,
            };
        }

        let key = mode.as_mode_key();
        let (level, level_overridden) = match settings.defaults_by_mode.get(&key) {
            Some(l) => (Self::level_from_setting(*l), true),
            None => (Self::default_level(mode), false),
        };
        let (fail_mode, fail_overridden) = match settings.fail_mode_by_mode.get(&key) {
            Some(FailModeSetting::HardFail) => (FailMode::HardFail, true),
            Some(FailModeSetting::DegradeWithMark) => (FailMode::DegradeWithMark, true),
            None => (Self::default_fail_mode(mode), false),
        };
        let source = if level_overridden || fail_overridden {
            PolicySource::SettingsOverride
        } else {
            PolicySource::Default
        };
        let profile = Self::build_profile(level, &workspace, network);
        ResolvedSandboxPolicy {
            level,
            fail_mode,
            profile,
            enabled: true,
            source,
        }
    }

    fn default_level(mode: EffectiveMode) -> SecurityLevel {
        match mode {
            EffectiveMode::Plan => SecurityLevel::High,
            EffectiveMode::Normal | EffectiveMode::AcceptEdits => SecurityLevel::Standard,
            EffectiveMode::Yolo => SecurityLevel::Minimal,
        }
    }

    fn default_fail_mode(mode: EffectiveMode) -> FailMode {
        match mode {
            EffectiveMode::Yolo => FailMode::DegradeWithMark,
            _ => FailMode::HardFail,
        }
    }

    fn level_from_setting(level: SandboxLevelSetting) -> SecurityLevel {
        match level {
            SandboxLevelSetting::Minimal => SecurityLevel::Minimal,
            SandboxLevelSetting::Standard => SecurityLevel::Standard,
            SandboxLevelSetting::High => SecurityLevel::High,
            SandboxLevelSetting::Paranoid => SecurityLevel::Paranoid,
        }
    }

    fn build_profile(
        level: SecurityLevel,
        workspace: &Path,
        network: Option<NetworkPolicy>,
    ) -> SandboxProfile {
        let mut b = SandboxConfig::builder(workspace.to_path_buf()).security_level(level);
        if let Some(n) = network {
            b = b.network(n);
        }
        // Match execute_command: allow HOME read for toolchains.
        if let Ok(home) = std::env::var("HOME") {
            b = b.readable_path(home);
        }
        let mut profile = b.build();
        profile.workdir = Some(workspace.to_path_buf());
        profile
    }
}

impl SecurityLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::High => "high",
            Self::Paranoid => "paranoid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SandboxSettings;

    #[test]
    fn resolve_plan_is_high_hard_fail() {
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::Plan,
            &SandboxSettings::default(),
            PathBuf::from("/tmp/ws"),
        );
        assert_eq!(p.level, SecurityLevel::High);
        assert_eq!(p.fail_mode, FailMode::HardFail);
        assert!(p.enabled);
        assert_eq!(p.source, PolicySource::Default);
    }

    #[test]
    fn resolve_normal_is_standard_hard_fail() {
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::Normal,
            &SandboxSettings::default(),
            PathBuf::from("/tmp/ws"),
        );
        assert_eq!(p.level, SecurityLevel::Standard);
        assert_eq!(p.fail_mode, FailMode::HardFail);
        // Package managers under Normal: Standard defaults to Full network.
        assert_eq!(p.profile.network, NetworkPolicy::Full);
    }

    #[test]
    fn resolve_accept_edits_shell_standard_hard_fail() {
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::AcceptEdits,
            &SandboxSettings::default(),
            PathBuf::from("/tmp/ws"),
        );
        assert_eq!(p.level, SecurityLevel::Standard);
        assert_eq!(p.fail_mode, FailMode::HardFail);
    }

    #[test]
    fn resolve_yolo_is_minimal_degrade() {
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::Yolo,
            &SandboxSettings::default(),
            PathBuf::from("/tmp/ws"),
        );
        assert_eq!(p.level, SecurityLevel::Minimal);
        assert_eq!(p.fail_mode, FailMode::DegradeWithMark);
    }

    #[test]
    fn settings_override_level() {
        let mut s = SandboxSettings::default();
        s.defaults_by_mode
            .insert(ModeKey::Normal, SandboxLevelSetting::Minimal);
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::Normal,
            &s,
            PathBuf::from("/tmp/ws"),
        );
        assert_eq!(p.level, SecurityLevel::Minimal);
        assert_eq!(p.source, PolicySource::SettingsOverride);
    }

    #[test]
    fn enabled_false_forces_degrade() {
        let mut s = SandboxSettings::default();
        s.enabled = false;
        let p = SandboxPolicyResolver::resolve(
            EffectiveMode::Plan,
            &s,
            PathBuf::from("/tmp/ws"),
        );
        assert!(!p.enabled);
        assert_eq!(p.fail_mode, FailMode::DegradeWithMark);
        assert_eq!(p.source, PolicySource::Disabled);
    }

    #[test]
    fn missing_mode_defaults_normal() {
        assert_eq!(EffectiveMode::default(), EffectiveMode::Normal);
    }

    #[test]
    fn run_test_network_keeps_mode_level() {
        let p = SandboxPolicyResolver::resolve_with_network(
            EffectiveMode::Normal,
            &SandboxSettings::default(),
            PathBuf::from("/tmp/ws"),
            Some(NetworkPolicy::Full),
        );
        assert_eq!(p.level, SecurityLevel::Standard);
        assert_eq!(p.profile.network, NetworkPolicy::Full);
        assert_eq!(p.fail_mode, FailMode::HardFail);
    }

    #[test]
    fn security_level_serde_snake() {
        assert_eq!(
            serde_json::to_string(&SecurityLevel::Minimal).unwrap(),
            "\"minimal\""
        );
        assert_eq!(
            serde_json::to_string(&SecurityLevel::Standard).unwrap(),
            "\"standard\""
        );
    }
}
