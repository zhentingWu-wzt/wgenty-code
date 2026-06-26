use super::external::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillDefinition,
    ExternalSkillError, ExternalSkillSource, ShadowedSkillDefinition, SkillFrontmatter,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A root directory that serves as a source of external skills.
#[derive(Debug, Clone)]
pub struct ExternalSkillRoot {
    /// Root filesystem path where skills are stored.
    pub skills_root: PathBuf,
    /// Source type indicating the origin (project, user, plugin, configured).
    pub source: ExternalSkillSource,
}

impl ExternalSkillRoot {
    /// Creates a new `ExternalSkillRoot` from a filesystem path and a source type.
    pub fn new(skills_root: PathBuf, source: ExternalSkillSource) -> Self {
        Self {
            skills_root,
            source,
        }
    }
}

/// Registry that discovers, resolves, and manages external skill definitions
/// from multiple root directories with priority-based conflict resolution.
#[derive(Debug, Clone, Default)]
pub struct ExternalSkillRegistry {
    /// Resolved skills indexed by canonical name.
    skills: HashMap<String, ExternalSkillDefinition>,
    /// Diagnostic messages collected during discovery.
    diagnostics: Vec<String>,
}

impl ExternalSkillRegistry {
    /// Discovers all external skills from the given list of root directories.
    ///
    /// Roots are scanned in order of `ExternalSkillSource::priority_rank()`.
    /// If two roots contain a skill with the same canonical name, the lower-ranked
    /// (higher priority) source wins, and the shadowed definition is recorded.
    pub fn discover(roots: Vec<ExternalSkillRoot>) -> Result<Self, ExternalSkillError> {
        let mut discovered = Vec::new();
        let mut diagnostics = Vec::new();

        for root in &roots {
            if !root.skills_root.exists() {
                continue;
            }
            scan_root(&root.skills_root, root, &mut discovered, &mut diagnostics)?;
        }

        discovered.sort_by_key(|skill| skill.source.priority_rank());

        let mut skills: HashMap<String, ExternalSkillDefinition> = HashMap::new();
        for skill in discovered {
            if let Some(existing) = skills.get_mut(&skill.canonical_name) {
                diagnostics.push(format!(
                    "skill '{}' from {} shadowed by {}",
                    skill.canonical_name,
                    skill.source_path.display(),
                    existing.source_path.display()
                ));
                existing.shadowed.push(ShadowedSkillDefinition {
                    canonical_name: skill.canonical_name.clone(),
                    source: skill.source.clone(),
                    source_path: skill.source_path.clone(),
                });
            } else {
                skills.insert(skill.canonical_name.clone(), skill);
            }
        }

        Ok(Self {
            skills,
            diagnostics,
        })
    }

    /// Resolves a skill by its canonical name.
    pub fn resolve(&self, name: &str) -> Option<&ExternalSkillDefinition> {
        self.skills.get(name)
    }

    /// Returns all resolved skills, sorted alphabetically by canonical name.
    pub fn list(&self) -> Vec<&ExternalSkillDefinition> {
        let mut values: Vec<_> = self.skills.values().collect();
        values.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
        values
    }

    /// Returns the diagnostic messages collected during discovery.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// Suggests similar skill names based on Levenshtein distance.
    ///
    /// Returns up to `limit` suggestions with a distance of 3 or less,
    /// sorted by closest match first.
    pub fn suggest(&self, name: &str, limit: usize) -> Vec<String> {
        let mut candidates: Vec<(usize, String)> = self
            .skills
            .keys()
            .map(|candidate| (levenshtein(name, candidate), candidate.clone()))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .filter(|(distance, _)| *distance <= 3)
            .take(limit)
            .map(|(_, candidate)| candidate)
            .collect()
    }
}

/// Scans a single skills root directory for skill directories.
fn scan_root(
    skills_root: &Path,
    root: &ExternalSkillRoot,
    discovered: &mut Vec<ExternalSkillDefinition>,
    diagnostics: &mut Vec<String>,
) -> Result<(), ExternalSkillError> {
    let entries = std::fs::read_dir(skills_root)?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        collect_skill_files(&path, skills_root, root, discovered, diagnostics)?;
    }
    Ok(())
}

/// Collects skill files from a single skill directory (or checks subdirectories).
fn collect_skill_files(
    directory: &Path,
    skills_root: &Path,
    root: &ExternalSkillRoot,
    discovered: &mut Vec<ExternalSkillDefinition>,
    diagnostics: &mut Vec<String>,
) -> Result<(), ExternalSkillError> {
    let skill_file = directory.join("SKILL.md");
    if skill_file.exists() {
        match load_skill_file(&skill_file, skills_root, root) {
            Ok(s) => discovered.push(s),
            Err(e) => diagnostics.push(e.to_string()),
        }
        return Ok(());
    }

    // Check subdirectories that contain a SKILL.md
    let entries = std::fs::read_dir(directory)?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("SKILL.md").exists() {
            match load_skill_file(&path.join("SKILL.md"), skills_root, root) {
                Ok(s) => discovered.push(s),
                Err(e) => diagnostics.push(e.to_string()),
            }
        }
    }
    Ok(())
}

/// Loads and parses a single SKILL.md file into an `ExternalSkillDefinition`.
fn load_skill_file(
    skill_file: &Path,
    skills_root: &Path,
    root: &ExternalSkillRoot,
) -> Result<ExternalSkillDefinition, ExternalSkillError> {
    let content = std::fs::read_to_string(skill_file)?;

    let parsed = parse_external_skill_document(&content)?;

    let canonical_name =
        derive_canonical_skill_name(parsed.name.as_deref(), skill_file, skills_root)?;

    let description = parsed.description.clone().unwrap_or_default();
    let base_dir = skill_file
        .parent()
        .ok_or_else(|| ExternalSkillError::NoParentDirectory(skill_file.to_path_buf()))?
        .to_path_buf();

    Ok(ExternalSkillDefinition {
        display_name: parsed
            .name
            .clone()
            .unwrap_or_else(|| canonical_name.clone()),
        canonical_name,
        description,
        body: content,
        frontmatter: SkillFrontmatter {
            name: parsed.name,
            description: parsed.description,
            raw_frontmatter: parsed.raw_frontmatter,
            extra: HashMap::new(),
        },
        source: root.source.clone(),
        source_path: skill_file.to_path_buf(),
        base_dir,
        shadowed: Vec::new(),
    })
}

/// Computes the Levenshtein distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev = (0..=b_chars.len()).collect::<Vec<_>>();
    for (i, ca) in a_chars.iter().enumerate() {
        let mut curr = vec![i + 1];
        for (j, cb) in b_chars.iter().enumerate() {
            curr.push(if ca == cb {
                prev[j]
            } else {
                1 + prev[j].min(curr[j]).min(prev[j + 1])
            });
        }
        prev = curr;
    }
    prev[b_chars.len()]
}
