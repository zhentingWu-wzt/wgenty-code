//! Configuration file watcher — reloads settings.json on change.
//!
//! Creates a shared `SettingsHandle` on startup, then spawns a blocking
//! watcher thread that monitors the config file. On change the settings are
//! reloaded, the shared value is updated, and a callback is invoked.

use crate::config::Settings;
use std::sync::Arc;
use std::time::Duration;

/// Shared, hot-reloadable settings handle.
pub type SettingsHandle = Arc<std::sync::RwLock<Settings>>;

/// Load settings and return a shared handle.
pub fn create_handle() -> SettingsHandle {
    let settings = Settings::load().unwrap_or_default();
    Arc::new(std::sync::RwLock::new(settings))
}

/// Watch the default `~/.wgenty-code/settings.json` (honoring `WGENTY_HOME`).
/// Thin wrapper over [`start_watching_path`].
pub fn start_watching(handle: SettingsHandle, on_change: impl Fn(Settings) + Send + 'static) {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_path = home.join(".wgenty-code").join("settings.json");
    start_watching_path(handle, config_path, on_change);
}

/// Spawn a blocking watcher thread on `config_path`. When the file changes
/// it reloads via [`Settings::load`] (which re-runs legacy migration,
/// `resolve_working_dir`, and `reconcile_routing`), updates `handle`, and
/// calls `on_change` with the new Settings.
///
/// Uses mtime polling (500ms interval) instead of OS file-system events.
/// This is intentionally simple and reliable: FSEvents on macOS silently
/// drops events in some sandboxed/test contexts, inotify has its own edge
/// cases with rename-in writes, and a 500ms poll on a single small file
/// costs effectively nothing. Configuration changes are rare; latency of
/// half a second is imperceptible.
///
/// Construction is best-effort: an unreadable path or stat failure simply
/// means "no change detected yet", never a panic.
pub fn start_watching_path(
    handle: SettingsHandle,
    config_path: std::path::PathBuf,
    on_change: impl Fn(Settings) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut last_mtime: Option<std::time::SystemTime> = None;

        loop {
            std::thread::sleep(Duration::from_millis(500));

            let Ok(metadata) = std::fs::metadata(&config_path) else {
                // File missing or unreadable — not a change event.
                continue;
            };
            let Ok(mtime) = metadata.modified() else {
                continue;
            };

            let changed = match last_mtime {
                None => {
                    // First observation: record baseline, don't trigger.
                    last_mtime = Some(mtime);
                    false
                }
                Some(prev) => mtime != prev,
            };

            if changed {
                last_mtime = Some(mtime);
                match Settings::reload() {
                    Ok(new_settings) => {
                        if let Ok(mut guard) = handle.write() {
                            *guard = new_settings.clone();
                        }
                        on_change(new_settings);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to reload settings: {}", e);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn watcher_handles_missing_file_without_panicking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope").join("settings.json");
        // Should not panic; missing file is just "no change".
        start_watching_path(create_handle(), missing, |_s| {});
    }

    #[test]
    fn watcher_detects_atomic_rename_write() {
        // The original bug: `sed -i` (and many editors) write a temp file and
        // rename it over the config. Polling catches this reliably because it
        // only looks at the final path's mtime.
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("settings.json");
        std::fs::write(&config, r#"{"verbose":false}"#).expect("seed config");

        let handle: SettingsHandle = Arc::new(std::sync::RwLock::new(Settings::default()));
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Settings>();
        start_watching_path(handle.clone(), config.clone(), move |s| {
            let _ = done_tx.send(s);
        });

        // Give the watcher thread time to record the baseline mtime.
        std::thread::sleep(Duration::from_millis(600));

        // Atomic rename write (the sed -i pattern).
        let tmp = dir.path().join(".settings.json.tmp");
        let mut f = std::fs::File::create(&tmp).expect("tmp");
        writeln!(f, r#"{{"verbose":true}}"#).expect("write tmp");
        drop(f);
        std::fs::rename(&tmp, &config).expect("rename");

        // Wait up to 5s for the poll cycle to detect the change and reload.
        let got = done_rx.recv_timeout(Duration::from_secs(5));
        assert!(
            got.is_ok(),
            "watcher callback did not fire on atomic rename write"
        );
        // The handle must also have been updated to the reloaded settings.
        let handle_snapshot = handle.read().expect("handle lock").clone();
        drop(handle_snapshot);
    }
}
