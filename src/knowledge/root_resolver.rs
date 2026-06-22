//! Unified skill root resolution for Comet workflow compatibility.
//!
//! Provides a single source of truth for external skill root directories,
//! resolving project, user wgenty-code, and user Claude Code skills in
//! priority order.

use super::external::ExternalSkillSource;
use super::external_registry::ExternalSkillRoot;
use std::path::{Path, PathBuf};

/// Central resolver for external skill root directories.
///
/// Use [`SkillRootResolver::roots()`] in production to auto-detect home
/// and project directories. Use [`SkillRootResolver::roots_with()`] for
/// deterministic paths (e.g. in tests).
pub struct SkillRootResolver;

impl SkillRootResolver {
    /// Returns the canonical ordered list of external skill roots, using
    /// the current working directory and the user's home directory.
    ///
    /// Priority (highest to lowest):
    /// 1. Project `.wgenty-code/skills/`
    /// 2. User `~/.wgenty-code/skills/`
    /// 3. User `~/.claude/skills/` (legacy Claude Code)
    pub fn roots() -> Vec<ExternalSkillRoot> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let project_root =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::roots_with(&home, &project_root)
    }

    /// Returns the canonical ordered list of external skill roots with
    /// explicit `home` and `project_root` paths.
    pub fn roots_with(home: &Path, project_root: &Path) -> Vec<ExternalSkillRoot> {
        vec![
            ExternalSkillRoot::new(
                project_root.join(".wgenty-code").join("skills"),
                ExternalSkillSource::ProjectWgentyCode {
                    root: project_root.join(".wgenty-code").join("skills"),
                },
            ),
            ExternalSkillRoot::new(
                home.join(".wgenty-code").join("skills"),
                ExternalSkillSource::UserWgentyCode {
                    root: home.join(".wgenty-code").join("skills"),
                },
            ),
            ExternalSkillRoot::new(
                home.join(".claude").join("skills"),
                ExternalSkillSource::UserClaude {
                    root: home.join(".claude").join("skills"),
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roots_returns_three_entries() {
        let home = Path::new("/tmp/test-home");
        let project = Path::new("/tmp/test-project");
        let roots = SkillRootResolver::roots_with(home, project);
        assert_eq!(roots.len(), 3, "should return exactly 3 roots");
    }

    #[test]
    fn test_roots_priority_order() {
        let home = Path::new("/tmp/test-home");
        let project = Path::new("/tmp/test-project");
        let roots = SkillRootResolver::roots_with(home, project);
        assert_eq!(roots.len(), 3);

        // First priority: project
        assert_eq!(
            roots[0].source.priority_rank(),
            0,
            "first root should be project (rank 0)"
        );
        assert!(matches!(
            roots[0].source,
            ExternalSkillSource::ProjectWgentyCode { .. }
        ));

        // Second priority: user wgenty-code
        assert_eq!(
            roots[1].source.priority_rank(),
            1,
            "second root should be user wgenty-code (rank 1)"
        );
        assert!(matches!(
            roots[1].source,
            ExternalSkillSource::UserWgentyCode { .. }
        ));

        // Third priority: user claude
        assert_eq!(
            roots[2].source.priority_rank(),
            2,
            "third root should be user claude (rank 2)"
        );
        assert!(matches!(
            roots[2].source,
            ExternalSkillSource::UserClaude { .. }
        ));
    }

    #[test]
    fn test_roots_paths_are_correct() {
        let home = Path::new("/tmp/test-home");
        let project = Path::new("/tmp/test-project");
        let roots = SkillRootResolver::roots_with(home, project);

        assert_eq!(
            roots[0].skills_root,
            Path::new("/tmp/test-project/.wgenty-code/skills")
        );
        assert_eq!(
            roots[1].skills_root,
            Path::new("/tmp/test-home/.wgenty-code/skills")
        );
        assert_eq!(
            roots[2].skills_root,
            Path::new("/tmp/test-home/.claude/skills")
        );
    }
}
