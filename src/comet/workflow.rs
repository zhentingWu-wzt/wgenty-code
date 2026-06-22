//! Comet workflow — discovers active OpenSpec changes.

use std::path::{Path, PathBuf};

/// Information about an active OpenSpec change.
#[derive(Debug, Clone)]
pub struct ChangeInfo {
    pub name: String,
    pub dir: PathBuf,
}

/// Scan `openspec/changes/` for non-archived changes with a `.comet.yaml`.
pub fn active_changes(working_dir: &Path) -> Vec<ChangeInfo> {
    let changes_dir = working_dir.join("openspec").join("changes");
    if !changes_dir.exists() {
        return vec![];
    }

    let entries = match std::fs::read_dir(&changes_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut changes = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Skip the archive/ directory.
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if file_name == "archive" {
            continue;
        }

        let comet_yaml = path.join(".comet.yaml");
        if !comet_yaml.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&comet_yaml) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Skip archived changes (simple check — "archived: true" anywhere).
        if content.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == "archived: true" || trimmed == "archived:true"
        }) {
            continue;
        }

        changes.push(ChangeInfo {
            name: file_name.to_string(),
            dir: path,
        });
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_change(tmp: &tempfile::TempDir, name: &str, phase: &str, archived: bool) {
        let dir = tmp
            .path()
            .join("openspec")
            .join("changes")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(".comet.yaml")).unwrap();
        writeln!(f, "phase: {}", phase).unwrap();
        writeln!(f, "archived: {}", archived).unwrap();
    }

    #[test]
    fn test_active_changes_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("openspec").join("changes")).unwrap();
        let changes = active_changes(tmp.path());
        assert!(changes.is_empty());
    }

    #[test]
    fn test_active_changes_finds_non_archived() {
        let tmp = tempfile::tempdir().unwrap();
        make_change(&tmp, "active-one", "design", false);
        make_change(&tmp, "active-two", "build", false);

        let changes = active_changes(tmp.path());
        assert_eq!(changes.len(), 2);
        let names: Vec<&str> = changes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"active-one"));
        assert!(names.contains(&"active-two"));
    }

    #[test]
    fn test_active_changes_skips_archived() {
        let tmp = tempfile::tempdir().unwrap();
        make_change(&tmp, "archived-one", "archive", true);
        make_change(&tmp, "active-one", "build", false);

        let changes = active_changes(tmp.path());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "active-one");
    }

    #[test]
    fn test_active_changes_skips_no_comet_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp
            .path()
            .join("openspec")
            .join("changes")
            .join("no-yaml");
        std::fs::create_dir_all(&dir).unwrap();
        // No .comet.yaml

        make_change(&tmp, "has-yaml", "open", false);

        let changes = active_changes(tmp.path());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "has-yaml");
    }

    #[test]
    fn test_active_changes_skips_archive_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        // Create an entry inside the archive/ directory — should be skipped.
        make_change(&tmp, "archive/old-thing", "build", false);
        make_change(&tmp, "real-change", "design", false);

        let changes = active_changes(tmp.path());
        // archive/ is skipped entirely, so old-thing is never visited.
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "real-change");
    }

    #[test]
    fn test_active_changes_dir_field() {
        let tmp = tempfile::tempdir().unwrap();
        make_change(&tmp, "my-change", "build", false);

        let changes = active_changes(tmp.path());
        assert_eq!(changes.len(), 1);
        assert!(changes[0].dir.ends_with("my-change"));
        assert!(changes[0].dir.join(".comet.yaml").exists());
    }
}
