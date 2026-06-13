//! Configuration file watcher — reloads settings.json on change.
//!
//! Creates a shared `SettingsHandle` on startup, then spawns a blocking
//! watcher thread that monitors the config file. On change the settings are
//! reloaded, the shared value is updated, and a callback is invoked.

use crate::config::Settings;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};

/// Shared, hot-reloadable settings handle.
pub type SettingsHandle = Arc<std::sync::RwLock<Settings>>;

/// Load settings and return a shared handle.
pub fn create_handle() -> SettingsHandle {
    let settings = Settings::load().unwrap_or_default();
    Arc::new(std::sync::RwLock::new(settings))
}

/// Spawn a blocking watcher thread. When settings.json changes it reloads the
/// file, updates `handle`, and calls `on_change` with the new Settings.
pub fn start_watching(handle: SettingsHandle, on_change: impl Fn(Settings) + Send + 'static) {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_path = home.join(".wgenty-code").join("settings.json");

    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel::<()>();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        let _ = tx.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("Failed to create config watcher: {}", e);
                    return;
                }
            };

        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            tracing::warn!("Failed to watch config file {:?}: {}", config_path, e);
            return;
        }

        loop {
            if rx.recv().is_err() {
                break;
            }
            // Drain burst events
            while rx.try_recv().is_ok() {}
            // Let file write complete
            std::thread::sleep(Duration::from_millis(300));

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
    });
}
