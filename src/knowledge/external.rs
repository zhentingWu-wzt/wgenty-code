use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Origin of an external skill file, determining its priority and label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalSkillSource {
    /// Skill bundled in the project's `.wgenty-code/skills/` directory.
    ProjectWgentyCode { root: PathBuf },
    /// Skill from the user's home directory `.wgenty-code/skills/`.
    UserWgentyCode { root: PathBuf },
    /// Skill from the user's Claude Code skills directory `~/.claude/skills/`.
    UserClaude { root: PathBuf },
    /// Skill installed from a plugin cache.
    PluginCache {
        plugin_name: String,
        version: Option<String>,
        root: PathBuf,
    },
    /// Skill configured via an explicit path or label.
    Configured { label: String, root: PathBuf },
}

impl ExternalSkillSource {
    /// Returns a numeric rank for priority ordering (lower = higher priority).
    pub fn priority_rank(&self) -> u8 {
        match self {
            Self::ProjectWgentyCode { .. } => 0,
            Self::UserWgentyCode { .. } => 1,
            Self::UserClaude { .. } => 2,
            Self::PluginCache { .. } => 3,
            Self::Configured { .. } => 4,
        }
    }

    /// Returns a human-readable label identifying this source.
    pub fn label(&self) -> String {
        match self {
            Self::ProjectWgentyCode { root } => format!("project:{}", root.display()),
            Self::UserWgentyCode { root } => format!("user:{}", root.display()),
            Self::UserClaude { root } => format!("user-claude:{}", root.display()),
            Self::PluginCache {
                plugin_name,
                version,
                root,
            } => format!(
                "plugin:{}@{}:{}",
                plugin_name,
                version.as_deref().unwrap_or("unknown"),
                root.display()
            ),
            Self::Configured { label, root } => format!("configured:{}:{}", label, root.display()),
        }
    }

    /// Returns the root filesystem path of this source.
    pub fn root(&self) -> &Path {
        match self {
            Self::ProjectWgentyCode { root }
            | Self::UserWgentyCode { root }
            | Self::UserClaude { root }
            | Self::PluginCache { root, .. }
            | Self::Configured { root, .. } => root,
        }
    }
}

/// Parsed frontmatter of an external skill's SKILL.md file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    /// Skill name declared in the frontmatter (if present).
    pub name: Option<String>,
    /// Skill description declared in the frontmatter (if present).
    pub description: Option<String>,
    /// Raw frontmatter text between `---` markers.
    pub raw_frontmatter: String,
    /// Additional frontmatter fields (reserved for future extension).
    pub extra: HashMap<String, String>,
}

/// Parsed result of an external skill document (frontmatter + body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExternalSkillDocument {
    /// Skill name parsed from the frontmatter (if present).
    pub name: Option<String>,
    /// Skill description parsed from the frontmatter (if present).
    pub description: Option<String>,
    /// Raw frontmatter text between `---` markers.
    pub raw_frontmatter: String,
    /// Markdown body after the frontmatter.
    pub body: String,
}

/// Metadata for a skill that was shadowed (overridden) by a higher-priority source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowedSkillDefinition {
    /// Canonical name of the shadowed skill.
    pub canonical_name: String,
    /// Source from which the shadowed skill originated.
    pub source: ExternalSkillSource,
    /// Filesystem path to the shadowed skill file.
    pub source_path: PathBuf,
}

/// Fully resolved external skill definition loaded from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSkillDefinition {
    /// Canonical name derived from frontmatter or directory structure.
    pub canonical_name: String,
    /// Human-displayable name (populated when frontmatter `name` is present).
    pub display_name: String,
    /// Description of the skill's purpose.
    pub description: String,
    /// Markdown body of the skill document.
    pub body: String,
    /// Parsed frontmatter of the skill document.
    pub frontmatter: SkillFrontmatter,
    /// Origin source of this skill.
    pub source: ExternalSkillSource,
    /// Filesystem path to the SKILL.md file.
    pub source_path: PathBuf,
    /// Base directory under which this skill resides.
    pub base_dir: PathBuf,
    /// Previously-loaded skills that were shadowed by this definition.
    pub shadowed: Vec<ShadowedSkillDefinition>,
}

/// Errors that can occur when parsing external skill documents or deriving skill names.
#[derive(Error, Debug)]
pub enum ExternalSkillError {
    /// Frontmatter has a start `---` marker but no closing `---` marker.
    #[error("frontmatter has no closing marker")]
    UnclosedFrontmatter,
    /// The skill file path is not contained within the expected root directory.
    #[error("{0} is not under {1}")]
    PathNotUnderRoot(PathBuf, PathBuf),
    /// The skill file path does not match the expected `skills/<name>/SKILL.md`
    /// or `skills/<namespace>/<name>/SKILL.md` pattern.
    #[error("unsupported skill path {0}; expected skills/<name>/SKILL.md or skills/<namespace>/<name>/SKILL.md")]
    UnsupportedPath(PathBuf),
    /// I/O error during skill discovery or loading.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    /// A file path has no parent directory.
    #[error("{0} has no parent directory")]
    NoParentDirectory(PathBuf),
}

/// Parse an external skill document (SKILL.md) into its structured components.
///
/// Returns a [`ParsedExternalSkillDocument`] containing the extracted frontmatter
/// fields and the remaining markdown body. If no frontmatter is found (no `---`
/// marker at the start), the entire content is treated as body.
pub fn parse_external_skill_document(
    content: &str,
) -> Result<ParsedExternalSkillDocument, ExternalSkillError> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(ParsedExternalSkillDocument {
            name: None,
            description: None,
            raw_frontmatter: String::new(),
            body: content.to_string(),
        });
    }

    let rest = &trimmed[3..];
    let end = rest
        .find("---")
        .ok_or(ExternalSkillError::UnclosedFrontmatter)?;
    let raw_frontmatter = rest[..end].trim().to_string();
    let body = rest[end + 3..].trim_start().to_string();

    let mut name = None;
    let mut description = None;

    for line in raw_frontmatter.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().trim_matches('"').to_string());
        }
    }

    Ok(ParsedExternalSkillDocument {
        name,
        description,
        raw_frontmatter,
        body,
    })
}

/// Derive a canonical skill name from the frontmatter `name` field or the
/// directory structure.
///
/// If `frontmatter_name` is `Some` and non-empty, it is returned directly.
/// Otherwise the function falls back to the path relative to `skills_root`:
/// - `skills/<name>/SKILL.md` => `<name>`
/// - `skills/<namespace>/<name>/SKILL.md` => `<namespace>:<name>`
pub fn derive_canonical_skill_name(
    frontmatter_name: Option<&str>,
    skill_file: &Path,
    skills_root: &Path,
) -> Result<String, ExternalSkillError> {
    if let Some(name) = frontmatter_name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let relative = skill_file.strip_prefix(skills_root).map_err(|_| {
        ExternalSkillError::PathNotUnderRoot(skill_file.to_path_buf(), skills_root.to_path_buf())
    })?;
    let parent = relative
        .parent()
        .ok_or_else(|| ExternalSkillError::UnsupportedPath(relative.to_path_buf()))?;
    let parts: Vec<String> = parent
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .filter(|part| !part.is_empty())
        .collect();

    match parts.as_slice() {
        [name] => Ok(name.clone()),
        [namespace, name] => Ok(format!("{}:{}", namespace, name)),
        _ => Err(ExternalSkillError::UnsupportedPath(relative.to_path_buf())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_claude_variant_priority_rank() {
        let source = ExternalSkillSource::UserClaude {
            root: PathBuf::from("/home/user/.claude/skills"),
        };
        assert_eq!(source.priority_rank(), 2);
    }

    #[test]
    fn test_user_claude_variant_label() {
        let source = ExternalSkillSource::UserClaude {
            root: PathBuf::from("/home/user/.claude/skills"),
        };
        let label = source.label();
        assert!(label.starts_with("user-claude:"));
        assert!(label.contains("/.claude/skills"));
    }

    #[test]
    fn test_user_claude_variant_root() {
        let path = PathBuf::from("/home/user/.claude/skills");
        let source = ExternalSkillSource::UserClaude {
            root: path.clone(),
        };
        assert_eq!(source.root(), path.as_path());
    }

    #[test]
    fn test_user_claude_serialization_roundtrip() {
        let source = ExternalSkillSource::UserClaude {
            root: PathBuf::from("/home/user/.claude/skills"),
        };
        let json = serde_json::to_string(&source).unwrap();
        let deserialized: ExternalSkillSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, deserialized);
        assert_eq!(deserialized.priority_rank(), 2);
    }
}
