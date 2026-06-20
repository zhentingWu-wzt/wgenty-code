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
    /// Install all bundled skills into `target_dir/skills/<name>/SKILL.md`.
    /// Does NOT overwrite existing files. Returns the list of newly installed
    /// skill names.
    pub fn install_to(target_dir: &std::path::Path) -> std::io::Result<Vec<String>> {
        let skills_dir = target_dir.join("skills");
        std::fs::create_dir_all(&skills_dir)?;

        let mut installed = Vec::new();

        for file_path in Self::iter() {
            let path = std::path::Path::new(file_path.as_ref());
            // Each embedded item is like "comet/SKILL.md"
            // Extract skill name from the parent directory.
            let skill_name = match path.parent().and_then(|p| p.file_name()) {
                Some(name) => name.to_string_lossy().to_string(),
                None => continue,
            };
            if skill_name.is_empty() {
                continue;
            }

            let dest_dir = skills_dir.join(&skill_name);
            let dest_file = dest_dir.join("SKILL.md");

            if dest_file.exists() {
                continue; // Never overwrite user skills
            }

            std::fs::create_dir_all(&dest_dir)?;

            if let Some(asset) = Self::get(file_path.as_ref()) {
                std::fs::write(&dest_file, asset.data)?;
                installed.push(skill_name);
            }
        }

        Ok(installed)
    }

    /// Check whether bundled skills are already installed to the given
    /// target directory.
    pub fn is_installed(target_dir: &std::path::Path) -> bool {
        // Check for the first comet skill as a sentinel.
        let sentinel = target_dir.join("skills").join("comet").join("SKILL.md");
        sentinel.exists()
    }

    /// Report a summary of what is bundled (name + description), suitable
    /// for listing in `skills install` output.
    pub fn list_bundled() -> Vec<(String, String)> {
        use super::loader::parse_frontmatter;
        let mut result = Vec::new();

        let mut seen = std::collections::HashSet::new();
        for file_path in Self::iter() {
            let path = std::path::Path::new(file_path.as_ref());
            let skill_name = match path.parent().and_then(|p| p.file_name()) {
                Some(name) => name.to_string_lossy().to_string(),
                None => continue,
            };
            if skill_name.is_empty() || !seen.insert(skill_name.clone()) {
                continue;
            }
            if let Some(asset) = Self::get(file_path.as_ref()) {
                let content = String::from_utf8_lossy(&asset.data);
                let (_, description) = parse_frontmatter(&content);
                result.push((skill_name, description.unwrap_or_default()));
            }
        }
        result
    }

    /// Count bundled skill directories.
    pub fn count() -> usize {
        use std::collections::HashSet;
        let mut dirs = HashSet::new();
        for file_path in Self::iter() {
            let path = std::path::Path::new(file_path.as_ref());
            if let Some(parent) = path.parent() {
                if let Some(name) = parent.file_name() {
                    dirs.insert(name.to_string_lossy().to_string());
                }
            }
        }
        dirs.len()
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
