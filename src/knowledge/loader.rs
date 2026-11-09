//! Skill Loader — scans skills directories for SKILL.md files.
//!
//! Two-layer skill injection:
//!   Layer 1: Skill names + descriptions (listed via tool definitions or system prompt)
//!   Layer 2: Full SKILL.md body returned by `load_skill` tool (loaded on demand)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// How long a loaded skill snapshot stays fresh. [`SkillLoaderHandle`] uses
/// this TTL so read paths (HTTP listing, `load_skill` tool) transparently
/// rescan the skills directories when the snapshot predates a recent install
/// — a long-running daemon no longer serves a startup-only snapshot forever.
pub const SKILL_SNAPSHOT_TTL: Duration = Duration::from_secs(10);

/// Shared, hot-swappable skill snapshot for long-running processes.
///
/// `SkillLoader` itself is an immutable startup scan. This handle wraps one
/// behind a lock and re-derives it on demand:
/// - [`SkillLoaderHandle::reload`] replaces the snapshot (manual refresh via
///   `POST /api/v1/skills/reload` or the TUI `/skills` command).
/// - [`SkillLoaderHandle::current`] returns the cached snapshot, rescanning
///   first when it is older than [`SKILL_SNAPSHOT_TTL`] — so sessions of an
///   already-running daemon see newly installed skills without any manual
///   action.
///
/// Scanning is synchronous disk I/O over a handful of directories (each
/// containing a `SKILL.md`); on an async context callers should wrap calls in
/// `tokio::task::block_in_place` or `spawn_blocking`.
pub struct SkillLoaderHandle {
    inner: Mutex<SkillLoaderHandleInner>,
    base_dirs: Vec<PathBuf>,
}

struct SkillLoaderHandleInner {
    snapshot: Arc<SkillLoader>,
    loaded_at: Instant,
}

impl SkillLoaderHandle {
    /// Build a handle over a fresh scan of `base_dirs` (same convention as
    /// [`SkillLoader::load_from_dirs`]: each entry gets `/skills` appended).
    pub fn load_from_dirs(base_dirs: &[PathBuf]) -> Self {
        Self {
            inner: Mutex::new(SkillLoaderHandleInner {
                snapshot: Arc::new(SkillLoader::load_from_dirs(base_dirs)),
                loaded_at: Instant::now(),
            }),
            base_dirs: base_dirs.to_vec(),
        }
    }

    /// Current snapshot; rescans when the cached one is older than
    /// [`SKILL_SNAPSHOT_TTL`]. A rescan that finds nothing new simply
    /// replaces the snapshot; skills are best-effort and must never take a
    /// session down.
    pub fn current(&self) -> Arc<SkillLoader> {
        let mut guard = self.inner.lock().expect("skill loader lock poisoned");
        if guard.loaded_at.elapsed() >= SKILL_SNAPSHOT_TTL {
            guard.snapshot = Arc::new(SkillLoader::load_from_dirs(&self.base_dirs));
            guard.loaded_at = Instant::now();
        }
        guard.snapshot.clone()
    }

    /// Force a rescan now, returning the new snapshot and how many skills it
    /// contains. The manual counterpart to [`Self::current`]'s TTL refresh.
    pub fn reload(&self) -> (Arc<SkillLoader>, usize) {
        let mut guard = self.inner.lock().expect("skill loader lock poisoned");
        let snapshot = Arc::new(SkillLoader::load_from_dirs(&self.base_dirs));
        let count = snapshot.skill_names().len();
        guard.snapshot = snapshot.clone();
        guard.loaded_at = Instant::now();
        (snapshot, count)
    }

    /// Direct access without TTL logic — for callers that must not rescan
    /// (e.g. tests, or paths that already hold a fresh snapshot).
    pub fn peek(&self) -> Arc<SkillLoader> {
        self.inner
            .lock()
            .expect("skill loader lock poisoned")
            .snapshot
            .clone()
    }

    /// The base dirs this handle rescans (each plus `/skills`).
    pub fn base_dirs(&self) -> &[PathBuf] {
        &self.base_dirs
    }
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

#[cfg(test)]
mod handle_tests {
    use super::*;

    fn write_skill(base: &std::path::Path, name: &str) {
        let dir = base.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d-{name}\n---\nbody\n"),
        )
        .unwrap();
    }

    #[test]
    fn reload_picks_up_newly_installed_skills() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(tmp.path(), "start-here");
        let handle = SkillLoaderHandle::load_from_dirs(&[tmp.path().to_path_buf()]);

        assert_eq!(handle.peek().skill_names().len(), 1);

        // Install another skill AFTER the handle was built.
        write_skill(tmp.path(), "propose");
        assert_eq!(handle.peek().skill_names().len(), 1, "peek must not rescan");

        let (loader, count) = handle.reload();
        assert_eq!(count, 2);
        assert!(loader.load_skill("propose").is_some());
        // peek observes the reloaded snapshot without rescanning again.
        assert_eq!(handle.peek().skill_names().len(), 2);
    }

    #[test]
    fn reload_drops_removed_skills() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(tmp.path(), "start-here");
        let handle = SkillLoaderHandle::load_from_dirs(&[tmp.path().to_path_buf()]);

        std::fs::remove_dir_all(tmp.path().join("skills").join("start-here")).unwrap();
        let (loader, count) = handle.reload();
        assert_eq!(count, 0);
        assert!(loader.load_skill("start-here").is_none());
    }

    #[test]
    fn current_rescans_after_ttl_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(tmp.path(), "start-here");
        let handle = SkillLoaderHandle::load_from_dirs(&[tmp.path().to_path_buf()]);

        // Fresh snapshot: no rescan, even though the dir gained a skill.
        write_skill(tmp.path(), "propose");
        assert_eq!(handle.current().skill_names().len(), 1);

        // Force TTL expiry by backdating `loaded_at`.
        {
            let mut guard = handle.inner.lock().unwrap();
            guard.loaded_at = Instant::now() - SKILL_SNAPSHOT_TTL - Duration::from_millis(1);
        }
        assert_eq!(handle.current().skill_names().len(), 2);
    }

    #[test]
    fn base_dirs_are_reported() {
        let handle = SkillLoaderHandle::load_from_dirs(&[PathBuf::from("/tmp/x")]);
        assert_eq!(handle.base_dirs(), &[PathBuf::from("/tmp/x")]);
    }
}
