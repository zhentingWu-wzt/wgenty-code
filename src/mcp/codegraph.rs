//! CodeGraph availability probe + guidance text.
//!
//! Detects whether the third-party `codegraph` CLI is installed and whether the
//! current repo has been indexed (`.codegraph/` marker), so the agent can guide
//! the user to install vs. initialize. The probe is synchronous and cheap
//! (one PATH scan + one `exists()` stat), safe to run at startup without
//! waiting for the non-blocking MCP handshake.

use crate::config::Settings;
use std::path::{Path, PathBuf};

/// Sync-determinable CodeGraph availability. Drives all guidance channels
/// (CLI startup notice, prompt injection, TUI status indicator).
///
/// Note: the binary-present + index-present case is reported as `Ready`. A
/// distinct `ConnectionError` state (handshake failure despite a ready
/// install) is logged by the background MCP connect task and is not surfaced
/// as a separate variant here -- guidance only needs the actionable states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegraphInstallState {
    /// Binary installed and `.codegraph/` index present.
    Ready,
    /// `codegraph` not found on PATH.
    NotInstalled,
    /// Binary present, but no `.codegraph/` index dir in the working dir.
    NotInitialized,
    /// User dismissed guidance for this working dir.
    Dismissed,
}

impl CodegraphInstallState {
    /// Short human-readable hint injected into the prompt environment layer.
    pub fn guidance_hint(&self) -> &'static str {
        match self {
            Self::Ready => "ready (code navigation active)",
            Self::NotInstalled => {
                "not_installed (install: npm i -g @colbymchenry/codegraph; fallback grep/lsp)"
            }
            Self::NotInitialized => "not_initialized (run `codegraph init`; fallback grep/lsp)",
            Self::Dismissed => "dismissed (fallback grep/lsp)",
        }
    }
}

/// Canonicalize a path for dismissed-set comparison; falls back to the raw
/// path when canonicalization fails (e.g. path no longer exists).
fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// True if `working_dir` (canonicalized) is in the dismissed set.
fn is_dismissed(settings: &Settings, working_dir: &Path) -> bool {
    let target = canon(working_dir);
    settings
        .integrations
        .codegraph
        .dismissed_paths
        .iter()
        .any(|p| canon(p) == target)
}

/// Pure classifier (no `which` call) -- unit-testable with a synthetic
/// `binary_present` flag so tests never depend on the host having `codegraph`
/// installed.
fn classify_install_state(settings: &Settings, binary_present: bool) -> CodegraphInstallState {
    let working_dir = &settings.storage.working_dir;
    if is_dismissed(settings, working_dir) {
        return CodegraphInstallState::Dismissed;
    }
    if !binary_present {
        return CodegraphInstallState::NotInstalled;
    }
    if !working_dir.join(".codegraph").exists() {
        return CodegraphInstallState::NotInitialized;
    }
    CodegraphInstallState::Ready
}

/// Full probe: checks PATH for the `codegraph` binary, then classifies.
pub fn probe_install_state(settings: &Settings) -> CodegraphInstallState {
    let binary_present = which::which("codegraph").is_ok();
    classify_install_state(settings, binary_present)
}

/// One-line CLI notice text for actionable states; `None` when silent
/// (`Ready` / `Dismissed`).
pub fn install_state_notice(state: CodegraphInstallState) -> Option<String> {
    match state {
        CodegraphInstallState::NotInstalled => Some(
            "⚠ CodeGraph 未安装，代码导航已降级到 grep/lsp。安装: npm i -g @colbymchenry/codegraph"
                .to_string(),
        ),
        CodegraphInstallState::NotInitialized => {
            Some("⚠ CodeGraph 已安装但当前仓库未初始化。在项目根运行: codegraph init".to_string())
        }
        CodegraphInstallState::Ready | CodegraphInstallState::Dismissed => None,
    }
}

/// Whether the MCP connect path should skip spawning the codegraph server.
/// True when the binary is absent or the user dismissed guidance -- in both
/// cases launching `codegraph serve --mcp` is pointless or unwanted.
pub fn should_skip_codegraph(state: CodegraphInstallState) -> bool {
    matches!(
        state,
        CodegraphInstallState::NotInstalled | CodegraphInstallState::Dismissed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CodegraphSettings, Settings, StorageConfig};

    /// Build a Settings whose working_dir points at `dir`.
    fn settings_in(dir: &Path) -> Settings {
        Settings {
            storage: StorageConfig {
                working_dir: dir.to_path_buf(),
                ..Settings::default().storage
            },
            ..Default::default()
        }
    }

    #[test]
    fn classify_dismissed_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings_in(tmp.path());
        s.integrations.codegraph = CodegraphSettings {
            dismissed_paths: vec![canon(tmp.path())],
        };
        // Dismissed takes precedence even when binary present + indexed.
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::Dismissed
        );
    }

    #[test]
    fn classify_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(
            classify_install_state(&s, false),
            CodegraphInstallState::NotInstalled
        );
    }

    #[test]
    fn classify_not_initialized() {
        let tmp = tempfile::tempdir().unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::NotInitialized
        );
    }

    #[test]
    fn classify_ready() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".codegraph")).unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::Ready
        );
    }

    #[test]
    fn classify_dismissed_by_raw_path() {
        // A dismissed entry stored as a raw (non-canonicalized) path that
        // canonicalizes to the working dir must still match.
        let tmp = tempfile::tempdir().unwrap();
        let raw = tmp.path().to_path_buf();
        let mut s = settings_in(&raw);
        s.integrations.codegraph = CodegraphSettings {
            dismissed_paths: vec![raw.clone()],
        };
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::Dismissed
        );
    }

    #[test]
    fn notice_text_not_installed() {
        assert!(install_state_notice(CodegraphInstallState::NotInstalled)
            .unwrap()
            .contains("npm i -g @colbymchenry/codegraph"));
    }

    #[test]
    fn notice_text_not_initialized() {
        assert!(install_state_notice(CodegraphInstallState::NotInitialized)
            .unwrap()
            .contains("codegraph init"));
    }

    #[test]
    fn notice_none_for_ready_and_dismissed() {
        assert!(install_state_notice(CodegraphInstallState::Ready).is_none());
        assert!(install_state_notice(CodegraphInstallState::Dismissed).is_none());
    }

    #[test]
    fn should_skip_true_for_not_installed_and_dismissed() {
        assert!(should_skip_codegraph(CodegraphInstallState::NotInstalled));
        assert!(should_skip_codegraph(CodegraphInstallState::Dismissed));
    }

    #[test]
    fn should_skip_false_for_ready_and_not_initialized() {
        assert!(!should_skip_codegraph(CodegraphInstallState::Ready));
        assert!(!should_skip_codegraph(
            CodegraphInstallState::NotInitialized
        ));
    }
}
