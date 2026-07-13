//! Bundled skills embedded into the binary via rust-embed.
//!
//! When the `bundled-skills` feature is enabled, all SKILL.md files from
//! `.wgenty-code/skills/` are embedded at compile time. These can be
//! installed to the user's skills directory on first run or via
//! `wgenty-code skills install`.

#[cfg(feature = "bundled-skills")]
use rust_embed::RustEmbed;

#[cfg(feature = "bundled-skills")]
#[derive(RustEmbed)]
#[folder = ".wgenty-code/skills/"]
pub struct BundledSkills;

#[cfg(feature = "bundled-skills")]
impl BundledSkills {
    /// Install all bundled skills by mirroring the embedded tree into
    /// `target_dir/skills/`, preserving the full directory structure
    /// (namespaces like `superpowers/<name>/` and supporting files like
    /// `comet/scripts/*` and `comet/reference/*`).
    ///
    /// Existing files are never overwritten. Returns the canonical names of
    /// the skills whose `SKILL.md` was newly written.
    pub fn install_to(target_dir: &std::path::Path) -> std::io::Result<Vec<String>> {
        let skills_dir = target_dir.join("skills");
        std::fs::create_dir_all(&skills_dir)?;

        let mut installed: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for file_path in Self::iter() {
            let rel = std::path::Path::new(file_path.as_ref());
            let dest = skills_dir.join(rel);

            let created = !dest.exists();
            if created {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(asset) = Self::get(file_path.as_ref()) {
                    std::fs::write(&dest, &asset.data)?;
                    mark_executable_if_script(&dest);
                }
            }

            // Record the skill name when its SKILL.md is newly written so
            // re-install is idempotent (returns an empty list).
            if created && rel.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                if let Some((name, _)) = Self::skill_meta(rel) {
                    if !name.is_empty() && seen.insert(name.clone()) {
                        installed.push(name);
                    }
                }
            }
        }

        Ok(installed)
    }

    /// Check whether bundled skills are already installed to the given
    /// target directory.
    pub fn is_installed(target_dir: &std::path::Path) -> bool {
        // Check for the comet skill as a sentinel.
        let sentinel = target_dir.join("skills").join("comet").join("SKILL.md");
        sentinel.exists()
    }

    /// Report a summary of what is bundled (canonical name + description),
    /// suitable for listing in `skills install` output.
    pub fn list_bundled() -> Vec<(String, String)> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for file_path in Self::iter() {
            let rel = std::path::Path::new(file_path.as_ref());
            if rel.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
                continue;
            }
            if let Some((name, description)) = Self::skill_meta(rel) {
                if !name.is_empty() && seen.insert(name.clone()) {
                    result.push((name, description));
                }
            }
        }
        result
    }

    /// Count bundled skills (directories containing a `SKILL.md`).
    pub fn count() -> usize {
        let mut skills = std::collections::HashSet::new();
        for file_path in Self::iter() {
            let rel = std::path::Path::new(file_path.as_ref());
            if rel.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
                continue;
            }
            if let Some((name, _)) = Self::skill_meta(rel) {
                if !name.is_empty() {
                    skills.insert(name);
                }
            }
        }
        skills.len()
    }

    /// Parse a `SKILL.md` embedded at `file_path` and return its canonical
    /// name (frontmatter `name` preferred, falling back to the directory
    /// structure) and description. Returns `None` if the asset is missing.
    fn skill_meta(file_path: &std::path::Path) -> Option<(String, String)> {
        use super::loader::parse_frontmatter;
        let key = file_path.to_str()?;
        let asset = Self::get(key)?;
        let content = String::from_utf8_lossy(&asset.data);
        let (fm_name, description) = parse_frontmatter(&content);
        let name = fm_name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| skill_name_from_dirs(file_path))
            .unwrap_or_default();
        Some((name, description.unwrap_or_default()))
    }
}

/// Derive a canonical skill name from a `SKILL.md` path relative to the
/// embed root: `<name>/SKILL.md` => `<name>`,
/// `<namespace>/<name>/SKILL.md` => `<namespace>:<name>`.
#[cfg(feature = "bundled-skills")]
fn skill_name_from_dirs(path: &std::path::Path) -> Option<String> {
    let parent = path.parent()?;
    let parts: Vec<String> = parent
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        [name] => Some(name.clone()),
        [namespace, name] => Some(format!("{}:{}", namespace, name)),
        _ => None,
    }
}

/// Set the executable bit for files located under a `scripts/` directory so
/// bundled command scripts (e.g. comet's `comet-guard`) are runnable after
/// install. No-op on non-Unix platforms.
#[cfg(feature = "bundled-skills")]
fn mark_executable_if_script(dest: &std::path::Path) {
    let in_scripts = dest.components().any(|c| c.as_os_str() == "scripts");
    if !in_scripts {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(dest, perms);
        }
    }
}

// — Stubs when `bundled-skills` feature is disabled ——

#[cfg(not(feature = "bundled-skills"))]
pub struct BundledSkills;

#[cfg(not(feature = "bundled-skills"))]
impl BundledSkills {
    pub fn install_to(_target_dir: &std::path::Path) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn is_installed(_target_dir: &std::path::Path) -> bool {
        true // pretend installed to avoid noise
    }

    pub fn list_bundled() -> Vec<(String, String)> {
        Vec::new()
    }

    pub fn count() -> usize {
        0
    }
}

// — Convenience helper ————————————————————————————————————————

/// Install bundled skills to the user's home `.wgenty-code` directory.
/// Returns the list of skill names that were newly installed.
pub fn auto_install() -> Vec<String> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let base = home.join(".wgenty-code");
    match BundledSkills::install_to(&base) {
        Ok(installed) => installed,
        Err(e) => {
            tracing::warn!(error = %e, "failed to auto-install bundled skills");
            Vec::new()
        }
    }
}

/// Check whether bundled skills are already installed to the user's home
/// directory.
pub fn is_auto_installed() -> bool {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    BundledSkills::is_installed(&home.join(".wgenty-code"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skip-if-installed fast path in `run_repl` relies on `is_installed`
    /// correctly detecting a prior `install_to`. Lock that contract in: before
    /// install it reports false, after install it reports true, and re-install
    /// is idempotent (never overwrites).
    #[cfg(feature = "bundled-skills")]
    #[test]
    fn is_installed_detects_prior_install_to() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();

        // Sentinel absent before any install.
        assert!(
            !BundledSkills::is_installed(base),
            "should report not-installed before install_to"
        );

        let _installed = BundledSkills::install_to(base).expect("install_to");

        // Sentinel present after install -> the run_repl skip path can fire.
        assert!(
            BundledSkills::is_installed(base),
            "should report installed after install_to"
        );

        // Re-install must not overwrite (idempotent): no newly-written skills.
        let again = BundledSkills::install_to(base).expect("install_to again");
        assert!(
            again.is_empty(),
            "re-install should not write any new skills"
        );
    }

    /// Regression: `install_to` must mirror the full embedded tree, preserving
    /// namespaces (`superpowers/<name>/`) and supporting files
    /// (`comet/scripts/*`, `comet/reference/*`). The old flat-`<name>/SKILL.md`
    /// logic dropped namespaces and skipped supporting files entirely.
    #[cfg(feature = "bundled-skills")]
    #[test]
    fn install_to_preserves_namespaces_and_supporting_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();

        let installed = BundledSkills::install_to(base).expect("install_to");

        // `count` and `list_bundled` must agree on the number of skills, and
        // install_to must report the same count of newly-written skills.
        let listing = BundledSkills::list_bundled();
        assert_eq!(
            BundledSkills::count(),
            listing.len(),
            "count() and list_bundled() must agree"
        );
        assert_eq!(
            installed.len(),
            listing.len(),
            "install_to should report every bundled skill as newly installed"
        );

        // A namespaced skill lands at its nested path, not flattened.
        let namespaced = base
            .join("skills")
            .join("superpowers")
            .join("brainstorming")
            .join("SKILL.md");
        assert!(
            namespaced.exists(),
            "namespaced skill should be installed at {}",
            namespaced.display()
        );
        assert!(
            listing
                .iter()
                .any(|(name, _)| name == "superpowers:brainstorming"),
            "list_bundled should contain the superpowers:brainstorming skill"
        );

        // Supporting files under comet/scripts must be installed too.
        let comet_scripts = base.join("skills").join("comet").join("scripts");
        assert!(
            comet_scripts.is_dir(),
            "comet/scripts supporting directory should be installed"
        );
        assert!(
            std::fs::read_dir(&comet_scripts)
                .map(|mut it| it.next().is_some())
                .unwrap_or(false),
            "comet/scripts should contain at least one script file"
        );
    }
}
