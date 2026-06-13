//! Daemon module — HTTP API server that exposes the agent as REST + SSE.
//!
//! Starts an Axum server providing:
//! - `POST /api/v1/chat/stream` — SSE streaming chat completions
//! - `POST /api/v1/tools/execute` — tool execution with permission checks
//! - `GET  /api/v1/mcp/servers` — MCP server management
//!
//! Launch via: `wgenty-code daemon --port 8371`

pub mod handlers;
pub mod models;
pub mod routes;
pub mod state;

use crate::state::AppState;
use state::DaemonState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

/// Start the daemon HTTP server. Blocks until the server exits.
pub async fn run(app_state: AppState, port: u16) -> anyhow::Result<()> {
    let daemon_state = Arc::new(DaemonState::new(app_state));

    // Spawn background task to evict stale subagent progress sessions (60s TTL).
    let cleanup_state = daemon_state.clone();
    tokio::spawn(async move {
        let ttl = std::time::Duration::from_secs(60);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            cleanup_state.cleanup_stale_subagent_sessions(ttl).await;
        }
    });

    let app = routes::create_router(daemon_state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("daemon listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
