//! Web Server Main Entry Point - Plugin Marketplace Web Interface

use wgenty_code::web::{server::start_server, WebServer};
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    info!("Starting Wgenty Code Plugin Marketplace Web Server");
    info!("Server will be available at http://127.0.0.1:{}", port);

    start_server(port).await
}
