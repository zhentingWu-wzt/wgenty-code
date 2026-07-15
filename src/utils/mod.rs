//! Utility functions — logging, project helpers, stress testing.
//!
//! Non-domain utilities that don't fit into a specific harness mechanism.

pub mod http;
pub mod lenient_json;
pub mod logging;
pub mod project;
pub mod startup_timing;
pub mod stress_tests;
pub mod stuck_detector;

pub use stress_tests::{run_stress_test, StressTestResult, StressTestRunner};

use std::path::PathBuf;

/// Get the home directory
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Get the config directory
pub fn config_dir() -> PathBuf {
    home_dir().join(".wgenty-code")
}

/// Get the data directory
pub fn data_dir() -> PathBuf {
    home_dir().join(".wgenty-code").join("data")
}

/// Ensure a directory exists
pub fn ensure_dir(path: &std::path::Path) -> anyhow::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Format bytes to human readable
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format duration to human readable
pub fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

/// Rough token estimate for a string (≈ chars / 4 for English, ≈ chars / 2 for CJK).
/// Conservative: uses chars / 3 to avoid underestimating.
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        return 0;
    }
    // Count CJK characters (they consume more tokens per char)
    let cjk = text.chars().filter(|c| is_cjk(*c)).count();
    let non_cjk = chars - cjk;
    // CJK: ~1.5 chars/token, non-CJK: ~4 chars/token
    #[allow(clippy::cast_possible_truncation)]
    // token estimate: small value proportional to text length
    let tokens = (cjk as f64 / 1.5 + non_cjk as f64 / 4.0).ceil() as usize;
    tokens
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
        | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
        | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
        | '\u{FF00}'..='\u{FFEF}' // Halfwidth and Fullwidth Forms
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

// ── Daemon token file management ────────────────────────────────────────────

/// Path to the daemon auth token file (`~/.wgenty-code/daemon.token`).
pub fn daemon_token_path() -> PathBuf {
    config_dir().join("daemon.token")
}

/// Write the daemon API token to the token file with restricted permissions.
/// Creates parent directories if they don't exist.
pub fn write_daemon_token(token: &str) -> anyhow::Result<()> {
    let path = daemon_token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Read the daemon auth token from the token file.
/// Returns `None` if the file doesn't exist or can't be read.
pub fn read_daemon_token() -> Option<String> {
    let path = daemon_token_path();
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

/// Remove the daemon token file. Succeeds silently if the file doesn't exist.
pub fn remove_daemon_token() -> anyhow::Result<()> {
    let path = daemon_token_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}
