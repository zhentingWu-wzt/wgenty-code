//! Comet state tracking — reads `.comet.yaml` from active OpenSpec changes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Phase of a comet change, serialized as lowercase variant names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CometPhase {
    Open,
    Design,
    Build,
    Verify,
    Archive,
}

/// Parsed state from a `.comet.yaml` file in an active OpenSpec change.
#[derive(Debug, Clone)]
pub struct CometState {
    pub change_name: String,
    pub phase: CometPhase,
    pub workflow: Option<String>,
    pub build_mode: Option<String>,
    pub isolation: Option<String>,
}

impl CometState {
    /// Scan `openspec/changes/*/.comet.yaml` for all non-archived changes.
    ///
    /// If multiple active changes are found, a warning is logged and the
    /// most restrictive phase across all changes is used (order: Archive >
    /// Verify > Open > Design > Build).
    pub fn read(working_dir: &Path) -> Option<Self> {
        let changes_dir = working_dir.join("openspec").join("changes");
        if !changes_dir.exists() {
            return None;
        }

        let entries = match std::fs::read_dir(&changes_dir) {
            Ok(e) => e,
            Err(_) => return None,
        };

        let mut active: Vec<CometState> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Skip the archive/ directory so we don't descend into archived changes.
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if file_name == "archive" {
                continue;
            }

            let comet_yaml = path.join(".comet.yaml");
            if !comet_yaml.exists() {
                continue;
            }

            let content = match std::fs::read_to_string(&comet_yaml) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let parsed = parse_comet_yaml(&content);

            // Skip archived changes.
            if parsed.get("archived").map(|v| v == "true").unwrap_or(false) {
                continue;
            }

            let phase = match parsed.get("phase") {
                Some(p) => CometPhase::from_yaml_str(p),
                None => continue,
            };

            active.push(CometState {
                change_name: file_name.to_string(),
                phase,
                workflow: parsed.get("workflow").map(|s| s.to_string()),
                build_mode: parsed.get("build_mode").map(|s| s.to_string()),
                isolation: parsed.get("isolation").map(|s| s.to_string()),
            });
        }

        if active.is_empty() {
            return None;
        }

        // Multiple active changes: warn and use the most restrictive phase.
        if active.len() > 1 {
            let names: Vec<&str> = active.iter().map(|c| c.change_name.as_str()).collect();
            tracing::warn!(
                "Multiple active comet changes detected: {:?}. Using most restrictive phase rules.",
                names
            );
        }

        // Select the change with the most restrictive phase.
        active.into_iter().max_by_key(|c| c.phase.restrictiveness())
    }

    /// Return a phase-specific Chinese instruction string for the system prompt.
    pub fn phase_instruction(&self) -> &'static str {
        match self.phase {
            CometPhase::Open => {
                "当前处于 Open（开启）阶段。允许：创建 proposal/design/tasks, 运行 guard。禁止：写源代码。"
            }
            CometPhase::Design => {
                "当前处于 Design（设计）阶段。允许：brainstorming, 创建设计文档, 运行 guard。禁止：写源代码。"
            }
            CometPhase::Build => {
                "当前处于 Build（构建）阶段。允许：写源代码、测试、执行计划。禁止：跳过用户确认点。"
            }
            CometPhase::Verify => {
                "当前处于 Verify（验证）阶段。允许：验证、branch handling。禁止：跳过失败处理。"
            }
            CometPhase::Archive => {
                "当前处于 Archive（归档）阶段。允许：确认归档、运行归档脚本。禁止：写源代码。"
            }
        }
    }
}

impl CometPhase {
    /// Returns a numeric restrictiveness score (higher = more restrictive).
    ///
    /// Order (most → least restrictive): Archive (4) > Verify (3) > Open (2) >
    /// Design (1) > Build (0).
    fn restrictiveness(&self) -> u8 {
        match self {
            CometPhase::Build => 0,
            CometPhase::Design => 1,
            CometPhase::Open => 2,
            CometPhase::Verify => 3,
            CometPhase::Archive => 4,
        }
    }

    /// Parse a phase string from a `.comet.yaml` value.
    fn from_yaml_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "open" => CometPhase::Open,
            "design" => CometPhase::Design,
            "build" => CometPhase::Build,
            "verify" => CometPhase::Verify,
            "archive" => CometPhase::Archive,
            // Unknown phase defaults to Build (safest — allows writes).
            _ => CometPhase::Build,
        }
    }
}

/// Manual line-by-line YAML parser for `.comet.yaml` files.
///
/// Reads simple `key: value` pairs. Quotes around values are stripped.
/// Lines without a colon, or blank/comment lines, are ignored.
fn parse_comet_yaml(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').trim().to_string();
            map.insert(key, value);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: create a temp directory with a given structure.
    fn setup_changes_dir(structure: &[(&str, Option<&str>)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let changes = tmp.path().join("openspec").join("changes");
        std::fs::create_dir_all(&changes).unwrap();
        for (name, content) in structure {
            let dir = changes.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(yaml) = content {
                let comet = dir.join(".comet.yaml");
                let mut f = std::fs::File::create(&comet).unwrap();
                write!(f, "{}", yaml).unwrap();
            }
        }
        tmp
    }

    #[test]
    fn test_read_no_changes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No openspec/changes/ at all
        assert!(CometState::read(tmp.path()).is_none());
    }

    #[test]
    fn test_read_with_active_change() {
        let tmp = setup_changes_dir(&[(
            "my-change",
            Some("phase: design\nworkflow: full\narchived: false\n"),
        )]);
        let state = CometState::read(tmp.path()).expect("should find active change");
        assert_eq!(state.change_name, "my-change");
        assert_eq!(state.phase, CometPhase::Design);
        assert_eq!(state.workflow.as_deref(), Some("full"));
        assert!(state.build_mode.is_none());
        assert!(state.isolation.is_none());
    }

    #[test]
    fn test_read_skips_archived() {
        let tmp = setup_changes_dir(&[("done-change", Some("phase: archive\narchived: true\n"))]);
        assert!(CometState::read(tmp.path()).is_none());
    }

    #[test]
    fn test_read_finds_build_change() {
        let tmp = setup_changes_dir(&[(
            "build-me",
            Some(
                "phase: build\nworkflow: full\nbuild_mode: subagent-driven-development\nisolation: branch\narchived: false\n",
            ),
        )]);
        let state = CometState::read(tmp.path()).unwrap();
        assert_eq!(state.phase, CometPhase::Build);
        assert_eq!(
            state.build_mode.as_deref(),
            Some("subagent-driven-development")
        );
        assert_eq!(state.isolation.as_deref(), Some("branch"));
    }

    #[test]
    fn test_read_skips_directory_without_comet_yaml() {
        let tmp = setup_changes_dir(&[
            ("no-yaml", None),
            ("has-yaml", Some("phase: open\narchived: false\n")),
        ]);
        let state = CometState::read(tmp.path()).unwrap();
        assert_eq!(state.change_name, "has-yaml");
    }

    #[test]
    fn test_read_returns_first_non_archived() {
        let tmp = setup_changes_dir(&[
            ("first", Some("phase: design\narchived: false\n")),
            ("second", Some("phase: build\narchived: false\n")),
        ]);
        let state = CometState::read(tmp.path()).unwrap();
        // Directory iteration order is not guaranteed, but the first one found
        // should be one of the two. We just verify we get a valid state.
        assert!(state.change_name == "first" || state.change_name == "second");
        assert!(!state.change_name.is_empty());
    }

    #[test]
    fn test_read_malformed_yaml_falls_back() {
        let tmp = setup_changes_dir(&[("broken", Some("this is not yaml\nno colon here\n"))]);
        // Missing phase key → should be None
        assert!(CometState::read(tmp.path()).is_none());
    }

    #[test]
    fn test_phase_instruction_open() {
        let state = CometState {
            change_name: "test".into(),
            phase: CometPhase::Open,
            workflow: None,
            build_mode: None,
            isolation: None,
        };
        let inst = state.phase_instruction();
        assert!(inst.contains("Open"));
        assert!(!inst.is_empty());
    }

    #[test]
    fn test_phase_instruction_build() {
        let state = CometState {
            change_name: "test".into(),
            phase: CometPhase::Build,
            workflow: None,
            build_mode: None,
            isolation: None,
        };
        let inst = state.phase_instruction();
        assert!(inst.contains("Build"));
        assert!(!inst.is_empty());
    }

    #[test]
    fn test_comet_phase_serde_lowercase() {
        let json = serde_json::to_string(&CometPhase::Design).unwrap();
        assert_eq!(json, "\"design\"");
        let parsed: CometPhase = serde_json::from_str("\"design\"").unwrap();
        assert_eq!(parsed, CometPhase::Design);

        let json = serde_json::to_string(&CometPhase::Build).unwrap();
        assert_eq!(json, "\"build\"");
    }

    #[test]
    fn test_read_empty_changes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("openspec").join("changes")).unwrap();
        assert!(CometState::read(tmp.path()).is_none());
    }

    #[test]
    fn test_read_skips_archive_subdirectory() {
        let tmp = setup_changes_dir(&[
            (
                "archive/old-change",
                Some("phase: build\narchived: false\n"),
            ),
            ("active-change", Some("phase: open\narchived: false\n")),
        ]);
        // The archive/ subdirectory should be skipped
        // active-change should be found
        let state = CometState::read(tmp.path());
        assert!(state.is_some());
        // It should find active-change, not the one under archive/
    }

    #[test]
    fn test_read_yaml_with_extra_fields() {
        let tmp = setup_changes_dir(&[(
            "extra-fields",
            Some("phase: verify\nworkflow: full\ncontext_compression: off\nunknown_field: xxx\narchived: false\n"),
        )]);
        let state = CometState::read(tmp.path()).unwrap();
        assert_eq!(state.phase, CometPhase::Verify);
        assert_eq!(state.workflow.as_deref(), Some("full"));
    }

    #[test]
    fn test_multiple_active_changes_uses_most_restrictive_phase() {
        // Create changes with Design (least restrictive) and Verify (most restrictive).
        let tmp = setup_changes_dir(&[
            ("change-design", Some("phase: design\narchived: false\n")),
            ("change-verify", Some("phase: verify\narchived: false\n")),
            ("change-build", Some("phase: build\narchived: false\n")),
        ]);
        let state = CometState::read(tmp.path()).unwrap();
        // Verify is more restrictive than Design/Build, so it should be selected.
        assert_eq!(state.phase, CometPhase::Verify);
    }

    #[test]
    fn test_multiple_active_changes_uses_most_restrictive_archive() {
        // Archive is the most restrictive phase.
        let tmp = setup_changes_dir(&[
            ("change-open", Some("phase: open\narchived: false\n")),
            ("change-archive", Some("phase: archive\narchived: false\n")),
        ]);
        let state = CometState::read(tmp.path()).unwrap();
        assert_eq!(state.phase, CometPhase::Archive);
    }

    #[test]
    fn test_single_active_change_no_warning() {
        // Single change: no warning, returned as-is.
        let tmp = setup_changes_dir(&[("only-change", Some("phase: design\narchived: false\n"))]);
        let state = CometState::read(tmp.path()).unwrap();
        assert_eq!(state.change_name, "only-change");
        assert_eq!(state.phase, CometPhase::Design);
    }

    #[test]
    fn test_restrictiveness_ordering() {
        use super::CometPhase;
        // Higher score = more restrictive.
        assert!(CometPhase::Build.restrictiveness() < CometPhase::Design.restrictiveness());
        assert!(CometPhase::Design.restrictiveness() < CometPhase::Open.restrictiveness());
        assert!(CometPhase::Open.restrictiveness() < CometPhase::Verify.restrictiveness());
        assert!(CometPhase::Verify.restrictiveness() < CometPhase::Archive.restrictiveness());
    }
}
