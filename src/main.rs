//! Wgenty Code Rust - Main Entry Point

use clap::Parser;
use tracing::error;
use wgenty_code::cli::Cli;
use wgenty_code::config::Settings;
use wgenty_code::state::AppState;
use wgenty_code::utils::logging;
use wgenty_code::utils::startup_timing;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Record the startup baseline BEFORE anything else (including logging
    // init) so every milestone below is measured from true process entry.
    startup_timing::init();
    logging::init();
    startup_timing::mark("logging initialized");

    let cli = Cli::parse();
    startup_timing::mark("cli parsed");
    let settings = Settings::load()?;
    startup_timing::mark("settings loaded");
    let state = AppState::new(settings);
    startup_timing::mark("app state created");

    match cli.run_async(state).await {
        Ok(_) => {}
        Err(e) => {
            error!(error = ?e, "application failed");
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
