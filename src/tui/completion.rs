//! Completion engine for TUI input — skills (@) and commands (/) completion.

use std::collections::HashSet;
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
    pub category: String,
}

#[derive(Debug, Clone)]
pub struct CompletionMatch {
    pub text: String,
    pub description: String,
    pub args_hint: Option<String>,
    pub category: String,
}

pub struct CompletionEngine {
    pub skills: Vec<SkillEntry>,
    pub commands: Vec<CommandEntry>,
}

impl CompletionEngine {
    pub fn default_builtin_commands() -> Vec<CommandEntry> {
        vec![
            CommandEntry {
                name: "clear".to_string(),
                description: "Clear the conversation screen".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
            CommandEntry {
                name: "plan".to_string(),
                description: "Toggle plan mode".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
            CommandEntry {
                name: "continue".to_string(),
                description: "Continue after an interrupted turn".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
            CommandEntry {
                name: "undo".to_string(),
                description: "Ask the agent to undo the most recent operation".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
            CommandEntry {
                name: "init".to_string(),
                description: "Analyze the repository and generate project guidance".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
            CommandEntry {
                name: "help".to_string(),
                description: "Show available commands".to_string(),
                args_hint: None,
                category: "Built-in".to_string(),
            },
        ]
    }

    /// Scan skills directories for completion (both @skills and /commands).
    /// Scans project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`,
    /// and the legacy `~/.claude/skills/` directory.
    pub fn load(command_registry_commands: &[CommandEntry]) -> Self {
        let mut skills = Vec::new();
        let mut commands = command_registry_commands.to_vec();

        // Resolve all external skill roots through the central resolver.
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let project_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let roots = crate::knowledge::SkillRootResolver::roots_with(&home, &project_root);

        let scan_roots: Vec<std::path::PathBuf> =
            roots.iter().map(|r| r.skills_root.clone()).collect();

        let mut seen: HashSet<String> = HashSet::new();
        for root in &scan_roots {
            if !root.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if seen.contains(name) {
                                continue;
                            }
                            seen.insert(name.to_string());
                            let description = extract_skill_description(&path);
                            skills.push(SkillEntry {
                                name: name.to_string(),
                                description: description.clone(),
                                path: path.clone(),
                            });
                            // Also register as a slash command for / completion
                            commands.push(CommandEntry {
                                name: name.to_string(),
                                description,
                                args_hint: None,
                                category: slash_command_category(name).to_string(),
                            });
                        }
                    }
                }
            }
        }

        // Scan namespace directories (e.g. superpowers/brainstorming)
        for root in &scan_roots {
            if !root.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let ns_path = entry.path();
                    if !ns_path.is_dir() {
                        continue;
                    }
                    if let Some(ns_name) = ns_path.file_name().and_then(|n| n.to_str()) {
                        if let Ok(sub_entries) = std::fs::read_dir(&ns_path) {
                            for sub in sub_entries.flatten() {
                                let skill_path = sub.path();
                                if !skill_path.is_dir() {
                                    continue;
                                }
                                if let Some(skill_name) =
                                    skill_path.file_name().and_then(|n| n.to_str())
                                {
                                    let canonical = format!("{}:{}", ns_name, skill_name);
                                    if seen.contains(&canonical) {
                                        continue;
                                    }
                                    seen.insert(canonical.clone());
                                    let description = extract_skill_description(&skill_path);
                                    skills.push(SkillEntry {
                                        name: canonical.clone(),
                                        description: description.clone(),
                                        path: skill_path,
                                    });
                                    commands.push(CommandEntry {
                                        name: canonical,
                                        description,
                                        args_hint: None,
                                        category: slash_command_category(ns_name).to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by name for deterministic display
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        commands.sort_by(|a, b| a.name.cmp(&b.name));
        Self { skills, commands }
    }

    /// Replace all commands with a new list (e.g. after PluginRegistry loads).
    pub fn update_commands(&mut self, commands: Vec<CommandEntry>) {
        self.commands = commands;
    }

    /// Merge slash commands from a CommandRouter's entry_commands into the completion
    /// engine's command list. Entries that already exist (by name) are skipped.
    pub fn merge_from_entry_commands(&mut self, entry_commands: &[String]) {
        for cmd_name in entry_commands {
            if !self.commands.iter().any(|c| c.name == *cmd_name) {
                self.commands.push(CommandEntry {
                    name: cmd_name.clone(),
                    description: String::new(),
                    args_hint: None,
                    category: "Workflow".to_string(),
                });
            }
        }
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
                    category: "Skill".to_string(),
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
                    category: c.category.clone(),
                })
                .collect(),
            _ => vec![],
        }
    }
}

fn slash_command_category(name: &str) -> &'static str {
    if name == "comet" || name.starts_with("comet-") {
        "Comet"
    } else if name == "openspec" || name.starts_with("openspec-") {
        "OpenSpec"
    } else if name == "superpowers"
        || name.starts_with("superpowers-")
        || name.starts_with("superpowers:")
    {
        "Superpowers"
    } else {
        "Skill"
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
                    category: "Built-in".into(),
                },
                CommandEntry {
                    name: "clear".into(),
                    description: "Clear screen".into(),
                    args_hint: None,
                    category: "Built-in".into(),
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
            category: "Built-in".into(),
        }];
        let matches = e.filter('/', "code");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].args_hint, Some("<change-name>".to_string()));
    }

    #[test]
    fn test_default_commands_include_builtin_slash_commands_with_category() {
        let commands = CompletionEngine::default_builtin_commands();
        let command_names: Vec<&str> = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect();

        for expected in ["clear", "plan", "continue", "undo", "init", "help"] {
            assert!(
                command_names.contains(&expected),
                "missing builtin command: {expected}"
            );
        }

        let engine = CompletionEngine {
            skills: Vec::new(),
            commands,
        };
        let matches = engine.filter('/', "");

        assert!(matches.iter().all(|item| item.category == "Built-in"));
    }

    #[test]
    fn test_slash_command_category_groups_known_workflows() {
        assert_eq!(slash_command_category("comet"), "Comet");
        assert_eq!(slash_command_category("comet-build"), "Comet");
        assert_eq!(slash_command_category("openspec-propose"), "OpenSpec");
        assert_eq!(
            slash_command_category("superpowers:brainstorming"),
            "Superpowers"
        );
        assert_eq!(slash_command_category("brainstorming"), "Skill");
    }

    #[test]
    fn test_update_commands() {
        let mut e = test_engine();
        let new_commands = vec![CommandEntry {
            name: "new-cmd".into(),
            description: "New command".into(),
            args_hint: Some("<arg>".into()),
            category: "Plugin".into(),
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

    #[test]
    fn test_merge_from_entry_commands() {
        let mut e = test_engine();
        let entry_cmds = vec![
            "comet".to_string(),
            "comet-open".to_string(),
            "clear".to_string(),
        ];
        e.merge_from_entry_commands(&entry_cmds);
        // "clear" already exists (in test_engine commands), should not duplicate
        let matches = e.filter('/', "comet");
        assert_eq!(matches.len(), 2); // comet, comet-open
        assert_eq!(matches[0].text, "comet");
        assert_eq!(matches[0].category, "Workflow");
        assert_eq!(matches[1].text, "comet-open");
        assert_eq!(matches[1].category, "Workflow");
        // "clear" should still have original category "Built-in", not "Workflow"
        let clear_matches = e.filter('/', "clear");
        assert_eq!(clear_matches.len(), 1);
        assert_eq!(clear_matches[0].category, "Built-in");
    }
}
