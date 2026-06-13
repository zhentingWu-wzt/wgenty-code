//! Wgenty Code Rust - Main Entry Point

use clap::Parser;
use tracing::error;
use wgenty_code::cli::Cli;
use wgenty_code::config::Settings;
use wgenty_code::state::AppState;
use wgenty_code::utils::logging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();

    let cli = Cli::parse();
    let settings = Settings::load()?;
    let state = AppState::new(settings);

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
