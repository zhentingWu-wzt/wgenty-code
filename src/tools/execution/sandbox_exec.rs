//! Shared sandbox policy resolution + fail-mode helpers for shell tools.

use crate::config::{SandboxSettings, Settings};
use crate::sandbox::{
    EffectiveMode, FailMode, NetworkPolicy, ResolvedSandboxPolicy, SandboxPolicyResolver,
    SecurityLevel,
};
use crate::tools::ToolError;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Load `integrations.sandbox` from user settings (defaults if missing).
pub fn load_sandbox_settings() -> SandboxSettings {
    Settings::load()
        .map(|s| s.integrations.sandbox)
        .unwrap_or_default()
}

/// Resolve policy for a tool call: mode + settings + workdir (+ optional network).
pub fn resolve_for_context(
    mode: EffectiveMode,
    workdir: Option<&Path>,
    network_override: Option<NetworkPolicy>,
) -> ResolvedSandboxPolicy {
    let settings = load_sandbox_settings();
    let cwd = workdir
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    if network_override.is_some() {
        SandboxPolicyResolver::resolve_with_network(mode, &settings, cwd, network_override)
    } else {
        SandboxPolicyResolver::resolve(mode, &settings, cwd)
    }
}

/// Whether FailMode allows direct (unsandboxed) spawn after infrastructure failure.
pub fn should_degrade_to_direct(fail_mode: FailMode) -> bool {
    matches!(fail_mode, FailMode::DegradeWithMark)
}

/// Map sandbox infrastructure failure under HardFail to a tool error.
pub fn sandbox_infra_tool_error(backend: &str, err: impl std::fmt::Display) -> ToolError {
    ToolError {
        message: format!("sandbox unavailable ({}): {}", backend, err),
        code: Some("sandbox_spawn_failed".to_string()),
    }
}

/// Backend enforcement fidelity (profile intent vs real OS isolation).
///
/// - `full`: FS + network isolation roughly match profile intent (e.g. macOS seatbelt)
/// - `partial`: kernel limits exist but not full FS/network (e.g. Windows job objects)
/// - `none`: no hardware isolation (NoneBackend / bypassed)
pub fn enforcement_fidelity(
    backend: &str,
    hardware_enforced: bool,
    bypassed: bool,
) -> &'static str {
    if bypassed || !hardware_enforced || backend == "none" || backend == "windows-stub" {
        return "none";
    }
    match backend {
        "seatbelt" => "full",
        // Linux namespaces + cgroups without real seccomp → partial.
        "seccomp+ns" => "partial",
        // Job Objects: resource/process limits, no FS/network isolation yet.
        "job-object" => "partial",
        _ => {
            if hardware_enforced {
                "partial"
            } else {
                "none"
            }
        }
    }
}

/// Metadata keys describing sandbox outcome for tool results / UI.
pub fn sandbox_metadata(
    mode: EffectiveMode,
    level: SecurityLevel,
    backend: &str,
    bypassed: bool,
    enforced: bool,
    fail_mode: FailMode,
) -> HashMap<String, serde_json::Value> {
    let mut m = HashMap::new();
    m.insert("permission_mode".to_string(), json!(mode.as_str()));
    m.insert("sandbox_level".to_string(), json!(level.as_str()));
    m.insert("sandbox_backend".to_string(), json!(backend));
    m.insert("sandbox_bypassed".to_string(), json!(bypassed));
    m.insert("sandbox_enforced".to_string(), json!(enforced && !bypassed));
    m.insert("sandbox_fail_mode".to_string(), json!(fail_mode.as_str()));
    m.insert(
        "sandbox_enforcement_fidelity".to_string(),
        json!(enforcement_fidelity(backend, enforced, bypassed)),
    );
    m
}

/// Decide whether to hard-fail or degrade after a sandbox infrastructure error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFailDecision {
    HardFail,
    Degrade,
}

pub fn decide_after_sandbox_err(fail_mode: FailMode) -> SpawnFailDecision {
    if should_degrade_to_direct(fail_mode) {
        SpawnFailDecision::Degrade
    } else {
        SpawnFailDecision::HardFail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_fail_maps_to_tool_error_code() {
        let e = sandbox_infra_tool_error("seatbelt", "spawn failed");
        assert_eq!(e.code.as_deref(), Some("sandbox_spawn_failed"));
        assert!(e.message.contains("seatbelt"));
    }

    #[test]
    fn degrade_allows_direct_flag() {
        assert!(should_degrade_to_direct(FailMode::DegradeWithMark));
        assert!(!should_degrade_to_direct(FailMode::HardFail));
        assert_eq!(
            decide_after_sandbox_err(FailMode::HardFail),
            SpawnFailDecision::HardFail
        );
        assert_eq!(
            decide_after_sandbox_err(FailMode::DegradeWithMark),
            SpawnFailDecision::Degrade
        );
    }

    #[test]
    fn metadata_includes_bypass() {
        let m = sandbox_metadata(
            EffectiveMode::Yolo,
            SecurityLevel::Minimal,
            "none",
            true,
            false,
            FailMode::DegradeWithMark,
        );
        assert_eq!(
            m.get("sandbox_bypassed").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            m.get("permission_mode").and_then(|v| v.as_str()),
            Some("yolo")
        );
        assert_eq!(
            m.get("sandbox_level").and_then(|v| v.as_str()),
            Some("minimal")
        );
        assert_eq!(
            m.get("sandbox_fail_mode").and_then(|v| v.as_str()),
            Some("degrade_with_mark")
        );
        assert_eq!(
            m.get("sandbox_enforcement_fidelity")
                .and_then(|v| v.as_str()),
            Some("none")
        );
    }

    #[test]
    fn fidelity_seatbelt_full_when_enforced() {
        assert_eq!(enforcement_fidelity("seatbelt", true, false), "full");
        assert_eq!(enforcement_fidelity("job-object", true, false), "partial");
        assert_eq!(enforcement_fidelity("seccomp+ns", true, false), "partial");
        assert_eq!(enforcement_fidelity("seatbelt", true, true), "none");
    }
}
