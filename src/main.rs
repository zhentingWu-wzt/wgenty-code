//! Claude Code Rust - Main Entry Point

use clap::Parser;
use claude_code_rs::cli::Cli;
use claude_code_rs::config::Settings;
use claude_code_rs::state::AppState;
use claude_code_rs::utils::logging;
use tracing::error;

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
