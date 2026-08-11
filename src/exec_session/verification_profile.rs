//! Deterministic verification profiles for project-specific command anchors.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Project profile used to derive required verification command anchors.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationProfile {
    /// No project-specific verification anchors.
    #[default]
    None,
    /// Rust project verification anchors.
    Rust,
}

impl VerificationProfile {
    /// Detect the profile from files located at the project root.
    pub fn detect(project_root: &Path) -> Self {
        if project_root.join("Cargo.toml").is_file() {
            Self::Rust
        } else {
            Self::None
        }
    }

    /// Resolve explicit commands with any profile-required command anchors.
    pub fn resolve(
        self,
        compile_commands: Vec<String>,
        test_commands: Vec<String>,
        verify_commands: Vec<String>,
    ) -> ResolvedVerificationCommands {
        let required_compile_commands = match self {
            Self::None => Vec::new(),
            Self::Rust => vec!["cargo check".to_string()],
        };
        let required_test_commands = match self {
            Self::None => Vec::new(),
            Self::Rust => vec!["cargo test --all".to_string()],
        };

        ResolvedVerificationCommands {
            compile_commands: deduplicate_commands(required_compile_commands, compile_commands),
            test_commands: deduplicate_commands(required_test_commands, test_commands),
            verify_commands: deduplicate_commands(Vec::new(), verify_commands),
        }
    }
}

/// Fully resolved command vectors for a node verification run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedVerificationCommands {
    /// Compile command anchors.
    pub compile_commands: Vec<String>,
    /// Test command anchors.
    pub test_commands: Vec<String>,
    /// Additional verification commands.
    pub verify_commands: Vec<String>,
}

/// Append commands in first-seen order, eliminating only exact duplicates.
fn deduplicate_commands(required: Vec<String>, supplied: Vec<String>) -> Vec<String> {
    let mut commands = required;
    commands.extend(supplied);
    let mut seen = HashSet::new();
    commands
        .iter()
        .filter(|command| seen.insert(command.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_profile_prepends_required_anchors_and_deduplicates_final_commands() {
        let resolved = VerificationProfile::Rust.resolve(
            vec!["custom compile".into()],
            vec!["custom test".into()],
            vec![
                "cargo clippy --all-targets -- -D warnings".into(),
                "cargo test --doc".into(),
            ],
        );
        assert_eq!(resolved.compile_commands, ["cargo check", "custom compile"]);
        assert_eq!(resolved.test_commands, ["cargo test --all", "custom test"]);
        assert_eq!(
            resolved.verify_commands,
            [
                "cargo clippy --all-targets -- -D warnings",
                "cargo test --doc",
            ]
        );
    }

    #[test]
    fn detect_rust_only_when_manifest_exists() {
        let dir = tempfile::tempdir().expect("temporary project");
        assert_eq!(
            VerificationProfile::detect(dir.path()),
            VerificationProfile::None
        );
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"p\"\n")
            .expect("write manifest");
        assert_eq!(
            VerificationProfile::detect(dir.path()),
            VerificationProfile::Rust
        );
    }
}
