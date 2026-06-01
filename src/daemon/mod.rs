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
