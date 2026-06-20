//! Skill Loader — scans skills directories for SKILL.md files.
//!
//! Two-layer skill injection:
//!   Layer 1: Skill names + descriptions (listed via tool definitions or system prompt)
//!   Layer 2: Full SKILL.md body returned by `load_skill` tool (loaded on demand)

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source_path: PathBuf,
}

pub struct SkillLoader {
    skills: HashMap<String, SkillInfo>,
}

impl SkillLoader {
    /// Scan multiple directories for skills/ subdirectories containing SKILL.md files.
    /// Later directories in the slice take lower priority — skills already loaded from an
    /// earlier directory are not overwritten.
    pub fn load_from_dirs(base_dirs: &[PathBuf]) -> Self {
        let mut skills = HashMap::new();

        for base_dir in base_dirs {
            let skills_dir = base_dir.join("skills");
            Self::scan_skills_dir(&skills_dir, &mut skills);
        }

        Self { skills }
    }

    /// Scan a single directory for skills/ subdirectories.
    pub fn load_from_dir(base_dir: &std::path::Path) -> Self {
        Self::load_from_dirs(&[base_dir.to_path_buf()])
    }

    fn scan_skills_dir(skills_dir: &std::path::Path, skills: &mut HashMap<String, SkillInfo>) {
        if !skills_dir.exists() {
            return;
        }

        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let skill_md = path.join("SKILL.md");
                    if skill_md.exists() {
                        if let Ok(content) = std::fs::read_to_string(&skill_md) {
                            let (name, description) = parse_frontmatter(&content);
                            let skill_name = name.unwrap_or_else(|| {
                                path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default()
                            });

                            // Only insert if not already loaded from a prior directory
                            if !skills.contains_key(&skill_name) {
                                skills.insert(
                                    skill_name.clone(),
                                    SkillInfo {
                                        name: skill_name,
                                        description: description.unwrap_or_default(),
                                        body: content,
                                        source_path: skill_md,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get brief listing for system prompt (Layer 1).
    pub fn get_skill_listing(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut lines = vec!["\n## Available Skills\n".to_string()];
        for skill in self.skills.values() {
            lines.push(format!("- **{}**: {}", skill.name, skill.description));
        }
        lines.push(
            "\nUse `load_skill` to read a skill's full instructions when needed.".to_string(),
        );
        lines.join("\n")
    }

    /// Load a skill by name (Layer 2).
    pub fn load_skill(&self, name: &str) -> Option<&SkillInfo> {
        self.skills.get(name)
    }

    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// Parse YAML frontmatter (--- ... ---) from SKILL.md content.
pub(crate) fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>) {
    let content = content.trim();
    if !content.starts_with("---") {
        return (None, None);
    }

    let rest = &content[3..];
    let end = rest.find("---").unwrap_or(0);
    let frontmatter = &rest[..end];

    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }

    (name, description)
}
