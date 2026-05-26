//! Logging initialization
//!
//! Logs go to `~/.claude-code/logs/claude-code.log`, never to the terminal.
//! If the log file cannot be opened, a sink layer is used as fallback.

use crate::utils::config_dir;
use std::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize the global tracing subscriber.
///
/// - Reads `RUST_LOG` for the filter level; defaults to `claude_code_rs=info`.
/// - Appends to `~/.claude-code/logs/claude-code.log`.
/// - Falls back to `/dev/null` sink if the log file is unavailable.
pub fn init() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("claude_code_rs=info"));

    let log_dir = config_dir().join("logs");

    if std::fs::create_dir_all(&log_dir).is_ok() {
        if let Ok(log_file) = std::fs::File::options()
            .create(true)
            .append(true)
            .open(log_dir.join("claude-code.log"))
        {
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(log_file))
                .with_ansi(false);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(file_layer)
                .init();
            return;
        }
    }

    // Fallback: discard logs silently
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::sink)
                .with_ansi(false),
        )
        .init();
}
