use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalSkillSource {
    ProjectWgentyCode { root: PathBuf },
    UserWgentyCode { root: PathBuf },
    PluginCache {
        plugin_name: String,
        version: Option<String>,
        root: PathBuf,
    },
    Configured { label: String, root: PathBuf },
}

impl ExternalSkillSource {
    pub fn priority_rank(&self) -> u8 {
        match self {
            Self::ProjectWgentyCode { .. } => 0,
            Self::UserWgentyCode { .. } => 1,
            Self::PluginCache { .. } => 2,
            Self::Configured { .. } => 3,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::ProjectWgentyCode { root } => format!("project:{}", root.display()),
            Self::UserWgentyCode { root } => format!("user:{}", root.display()),
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

    pub fn root(&self) -> &Path {
        match self {
            Self::ProjectWgentyCode { root }
            | Self::UserWgentyCode { root }
            | Self::PluginCache { root, .. }
            | Self::Configured { root, .. } => root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub raw: String,
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExternalSkillDocument {
    pub name: Option<String>,
    pub description: Option<String>,
    pub raw_frontmatter: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowedSkillDefinition {
    pub canonical_name: String,
    pub source: ExternalSkillSource,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSkillDefinition {
    pub canonical_name: String,
    pub display_name: String,
    pub description: String,
    pub body: String,
    pub frontmatter: SkillFrontmatter,
    pub source: ExternalSkillSource,
    pub source_path: PathBuf,
    pub base_dir: PathBuf,
    pub shadowed: Vec<ShadowedSkillDefinition>,
}

pub fn parse_external_skill_document(
    content: &str,
) -> Result<ParsedExternalSkillDocument, String> {
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
        .ok_or_else(|| "frontmatter start marker has no closing marker".to_string())?;
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

pub fn derive_canonical_skill_name(
    frontmatter_name: Option<&str>,
    skill_file: &Path,
    skills_root: &Path,
) -> Result<String, String> {
    if let Some(name) = frontmatter_name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let relative = skill_file
        .strip_prefix(skills_root)
        .map_err(|_| format!("{} is not under {}", skill_file.display(), skills_root.display()))?;
    let parent = relative
        .parent()
        .ok_or_else(|| format!("{} has no skill directory", relative.display()))?;
    let parts: Vec<String> = parent
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .filter(|part| !part.is_empty())
        .collect();

    match parts.as_slice() {
        [name] => Ok(name.clone()),
        [namespace, name] => Ok(format!("{}:{}", namespace, name)),
        _ => Err(format!(
            "unsupported skill path {}; expected skills/<name>/SKILL.md or skills/<namespace>/<name>/SKILL.md",
            relative.display()
        )),
    }
}
