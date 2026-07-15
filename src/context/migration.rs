//! Legacy data migration for project-local state.
//!
//! One-time migration that tags pre-existing session files with a
//! `project_path` so they are associated with the current project after the
//! dual-storage (project + global) feature is introduced.
//!
//! - **Sessions**: `project_path: null` (or missing) → CWD, in-place JSON
//!   update in `~/.wgenty-code/sessions/*.json`.
//! - **Memories**: No migration needed; existing `~/.wgenty-code/memory/`
//!   files are naturally read as global memories by `MemoryManager`'s
//!   `global_storage`.
//! - **Command history**: Stays global, unchanged.
//!
//! Idempotent via a marker file at `~/.wgenty-code/.migrated-project-local`.

use std::path::Path;

/// Marker file name written to the config directory after migration.
const MARKER_FILE: &str = ".migrated-project-local";

/// Run the one-time legacy session migration using the real config directory
/// (`~/.wgenty-code/`) and the current working directory as the project root.
///
/// Errors are logged as warnings and never propagated—migration failure must
/// not block application startup.
pub fn migrate_legacy_sessions() {
    let config_dir = crate::utils::config_dir();
    let project_root = crate::utils::current_project_root();
    migrate_sessions_in_dir(&config_dir, &project_root);
}

/// Core migration logic operating on an explicit config directory and project
/// root. Separated from [`migrate_legacy_sessions`] so tests can use temp
/// directories without touching the real `~/.wgenty-code/`.
fn migrate_sessions_in_dir(config_dir: &Path, project_root: &Path) {
    let marker = config_dir.join(MARKER_FILE);

    // Idempotency: skip if already migrated.
    if marker.exists() {
        tracing::debug!("legacy session migration already applied; skipping");
        return;
    }

    let sessions_dir = config_dir.join("sessions");
    if !sessions_dir.is_dir() {
        // No sessions to migrate; still write marker so we don't check again.
        let _ = std::fs::write(&marker, "");
        tracing::debug!("no sessions directory found; writing migration marker");
        return;
    }

    let entries = match std::fs::read_dir(&sessions_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                error = %e,
                sessions_dir = %sessions_dir.display(),
                "failed to read sessions directory for migration; skipping"
            );
            return;
        }
    };

    let project_path_str = project_root.display().to_string();
    let mut migrated_count = 0usize;
    let mut total_count = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        total_count += 1;

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to read session file during migration; skipping"
                );
                continue;
            }
        };

        let mut json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse session JSON during migration; skipping"
                );
                continue;
            }
        };

        // Migrate only when project_path is missing or null.
        let needs_migration = json.get("project_path").is_none_or(|v| v.is_null());

        if !needs_migration {
            continue;
        }

        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "project_path".to_string(),
                serde_json::Value::String(project_path_str.clone()),
            );
        }

        let serialized = match serde_json::to_string_pretty(&json) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to serialize migrated session JSON; skipping"
                );
                continue;
            }
        };

        if let Err(e) = std::fs::write(&path, serialized) {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "failed to write migrated session file; skipping"
            );
            continue;
        }

        migrated_count += 1;
    }

    // Write marker file for idempotency.
    if let Err(e) = std::fs::write(&marker, "") {
        tracing::warn!(
            error = %e,
            marker = %marker.display(),
            "failed to write migration marker file; migration will re-run next startup"
        );
    }

    tracing::info!(
        total_sessions = total_count,
        migrated = migrated_count,
        "legacy session migration complete"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn migrate_skips_when_marker_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path();
        let sessions = config.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        // Create a session file that would need migration.
        std::fs::write(
            sessions.join("abc.json"),
            r#"{"id":"abc","name":"test","project_path":null}"#,
        )
        .unwrap();
        // Create marker file to signal migration already ran.
        std::fs::write(config.join(MARKER_FILE), "").unwrap();

        migrate_sessions_in_dir(config, Path::new("/my/project"));

        // File should be unchanged (marker prevented migration).
        let content = std::fs::read_to_string(sessions.join("abc.json")).unwrap();
        assert!(
            content.contains(r#""project_path":null"#),
            "session should be unchanged when marker exists"
        );
    }

    #[test]
    fn migrate_sets_project_path_for_null_and_missing_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path();
        let sessions = config.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();

        // Session with null project_path → should be migrated.
        std::fs::write(
            sessions.join("abc.json"),
            r#"{"id":"abc","name":"test","project_path":null}"#,
        )
        .unwrap();
        // Session with missing project_path → should be migrated.
        std::fs::write(sessions.join("def.json"), r#"{"id":"def","name":"test2"}"#).unwrap();
        // Session with existing project_path → should be untouched.
        std::fs::write(
            sessions.join("ghi.json"),
            r#"{"id":"ghi","name":"test3","project_path":"/existing/path"}"#,
        )
        .unwrap();

        let project_root = PathBuf::from("/my/project");
        migrate_sessions_in_dir(config, &project_root);

        // abc.json: null → project_root
        let abc = std::fs::read_to_string(sessions.join("abc.json")).unwrap();
        assert!(
            abc.contains("/my/project"),
            "null project_path should be migrated: {}",
            abc
        );
        assert!(!abc.contains("null"));

        // def.json: missing → project_root
        let def = std::fs::read_to_string(sessions.join("def.json")).unwrap();
        assert!(
            def.contains("/my/project"),
            "missing project_path should be migrated: {}",
            def
        );

        // ghi.json: existing → unchanged
        let ghi = std::fs::read_to_string(sessions.join("ghi.json")).unwrap();
        assert!(
            ghi.contains("/existing/path"),
            "existing project_path should be untouched: {}",
            ghi
        );
        assert!(!ghi.contains("/my/project"));

        // Marker file should exist.
        assert!(
            config.join(MARKER_FILE).exists(),
            "marker file should be created after migration"
        );
    }

    #[test]
    fn migrate_creates_marker_when_no_sessions_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path();

        // No sessions directory exists.
        migrate_sessions_in_dir(config, Path::new("/my/project"));

        assert!(
            config.join(MARKER_FILE).exists(),
            "marker should be created even when no sessions dir exists"
        );
    }

    #[test]
    fn migrate_skips_non_json_files() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path();
        let sessions = config.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("readme.txt"), "not a session").unwrap();
        std::fs::write(
            sessions.join("real.json"),
            r#"{"id":"real","project_path":null}"#,
        )
        .unwrap();

        migrate_sessions_in_dir(config, Path::new("/proj"));

        // txt file should be untouched.
        assert_eq!(
            std::fs::read_to_string(sessions.join("readme.txt")).unwrap(),
            "not a session"
        );
        // json file should be migrated.
        let real = std::fs::read_to_string(sessions.join("real.json")).unwrap();
        assert!(real.contains("/proj"));
    }

    #[test]
    fn migrate_is_idempotent_on_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path();
        let sessions = config.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("abc.json"),
            r#"{"id":"abc","project_path":null}"#,
        )
        .unwrap();

        // First run: migrates and writes marker.
        migrate_sessions_in_dir(config, Path::new("/first"));
        let after_first = std::fs::read_to_string(sessions.join("abc.json")).unwrap();
        assert!(after_first.contains("/first"));

        // Second run: marker exists, should be a no-op.
        migrate_sessions_in_dir(config, Path::new("/second"));
        let after_second = std::fs::read_to_string(sessions.join("abc.json")).unwrap();
        assert!(
            after_second.contains("/first"),
            "second run should not re-migrate"
        );
        assert!(!after_second.contains("/second"));
    }
}
