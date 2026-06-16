//! Completion engine for TUI input — skills (@) and commands (/) completion.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub name: String,
    pub description: String,
    pub args_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionMatch {
    pub text: String,
    pub description: String,
    pub args_hint: Option<String>,
}

pub struct CompletionEngine {
    pub skills: Vec<SkillEntry>,
    pub commands: Vec<CommandEntry>,
}

impl CompletionEngine {
    /// Scan ~/.claude/skills/ for skills, load from PluginRegistry for commands.
    pub fn load(skills_dir: &std::path::Path, command_registry_commands: &[CommandEntry]) -> Self {
        let mut skills = Vec::new();
        if skills_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(skills_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            let description = extract_skill_description(&path);
                            skills.push(SkillEntry {
                                name: name.to_string(),
                                description,
                                path,
                            });
                        }
                    }
                }
            }
        }
        // Sort skills by name for deterministic display
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            skills,
            commands: command_registry_commands.to_vec(),
        }
    }

    /// Replace all commands with a new list (e.g. after PluginRegistry loads).
    pub fn update_commands(&mut self, commands: Vec<CommandEntry>) {
        self.commands = commands;
    }

    pub fn filter(&self, prefix: char, partial: &str) -> Vec<CompletionMatch> {
        let partial_lower = partial.to_lowercase();
        match prefix {
            '@' => self
                .skills
                .iter()
                .filter(|s| s.name.to_lowercase().contains(&partial_lower))
                .map(|s| CompletionMatch {
                    text: s.name.clone(),
                    description: s.description.clone(),
                    args_hint: None,
                })
                .collect(),
            '/' => self
                .commands
                .iter()
                .filter(|c| c.name.to_lowercase().starts_with(&partial_lower))
                .map(|c| CompletionMatch {
                    text: c.name.clone(),
                    description: c.description.clone(),
                    args_hint: c.args_hint.clone(),
                })
                .collect(),
            _ => vec![],
        }
    }
}

fn extract_skill_description(skill_dir: &std::path::Path) -> String {
    let skill_md = skill_dir.join("SKILL.md");
    if let Ok(content) = std::fs::read_to_string(&skill_md) {
        // Try frontmatter description first
        if let Some(desc) = content
            .lines()
            .find(|l| l.trim().starts_with("description:"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().trim_matches('"').to_string())
        {
            if !desc.is_empty() {
                return desc;
            }
        }
        // Fallback to first non-empty, non-frontmatter line
        if let Some(line) = content
            .lines()
            .skip_while(|l| l.trim().starts_with("---"))
            .skip(1)
            .find(|l| !l.trim().is_empty() && !l.trim().starts_with("---"))
        {
            return line.trim().to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> CompletionEngine {
        CompletionEngine {
            skills: vec![
                SkillEntry {
                    name: "comet-design".into(),
                    description: "Design phase".into(),
                    path: PathBuf::new(),
                },
                SkillEntry {
                    name: "comet-build".into(),
                    description: "Build phase".into(),
                    path: PathBuf::new(),
                },
                SkillEntry {
                    name: "comet-open".into(),
                    description: "Open change".into(),
                    path: PathBuf::new(),
                },
            ],
            commands: vec![
                CommandEntry {
                    name: "code-review".into(),
                    description: "Review code".into(),
                    args_hint: None,
                },
                CommandEntry {
                    name: "clear".into(),
                    description: "Clear screen".into(),
                    args_hint: None,
                },
            ],
        }
    }

    #[test]
    fn test_skills_filter_by_at() {
        let e = test_engine();
        let matches = e.filter('@', "comet");
        assert_eq!(matches.len(), 3);
        assert!(matches.iter().all(|m| m.text.starts_with("comet")));
    }

    #[test]
    fn test_skills_filter_case_insensitive() {
        let e = test_engine();
        let matches = e.filter('@', "COMET");
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_skills_filter_empty_partial() {
        let e = test_engine();
        let matches = e.filter('@', "");
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_commands_filter_by_slash() {
        let e = test_engine();
        let matches = e.filter('/', "code");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "code-review");
    }

    #[test]
    fn test_commands_prefix_match_only() {
        let e = test_engine();
        let matches = e.filter('/', "review");
        assert_eq!(matches.len(), 0); // "code-review" doesn't start with "review"
    }

    #[test]
    fn test_unknown_prefix_returns_empty() {
        let e = test_engine();
        let matches = e.filter('!', "anything");
        assert!(matches.is_empty());
    }

    #[test]
    fn test_commands_filter_includes_args_hint() {
        let mut e = test_engine();
        // Override commands with one that has an args_hint
        e.commands = vec![CommandEntry {
            name: "code-review".into(),
            description: "Review code".into(),
            args_hint: Some("<change-name>".into()),
        }];
        let matches = e.filter('/', "code");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].args_hint, Some("<change-name>".to_string()));
    }

    #[test]
    fn test_update_commands() {
        let mut e = test_engine();
        let new_commands = vec![CommandEntry {
            name: "new-cmd".into(),
            description: "New command".into(),
            args_hint: Some("<arg>".into()),
        }];
        e.update_commands(new_commands);
        // Old commands should be replaced
        let matches_old = e.filter('/', "code");
        assert_eq!(matches_old.len(), 0);
        // New command should be found
        let matches_new = e.filter('/', "new");
        assert_eq!(matches_new.len(), 1);
        assert_eq!(matches_new[0].text, "new-cmd");
        assert_eq!(matches_new[0].args_hint, Some("<arg>".to_string()));
    }
}
