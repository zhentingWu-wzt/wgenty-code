//! Daemon module — HTTP API server that exposes the agent as REST + SSE.
//!
//! Starts an Axum server providing:
//! - `POST /api/v1/chat/stream` — SSE streaming chat completions
//! - `POST /api/v1/tools/execute` — tool execution with permission checks
//! - `GET  /api/v1/mcp/servers` — MCP server management
//!
//! Launch via: `wgenty-code daemon --port 8371`

pub mod auth;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod state;

use crate::state::AppState;
use state::DaemonState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing::info;

/// Start the daemon HTTP server. Blocks until the server exits.
pub async fn run(app_state: AppState, port: u16) -> anyhow::Result<()> {
    let daemon_state = Arc::new(DaemonState::new(app_state).await);

    // Recover persisted sessions as lightweight index entries so the
    // `list_sessions` API returns historical sessions quickly. Full message
    // history is hydrated on demand via `load(id)` / `get(id)`.
    if let Err(e) = daemon_state.session_manager.load_index().await {
        tracing::warn!(error = %e, "Failed to load persisted sessions into daemon");
    }

    // Spawn background task to evict stale subagent progress sessions (60s TTL).
    let cleanup_state = daemon_state.clone();
    tokio::spawn(async move {
        let ttl = std::time::Duration::from_secs(60);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            cleanup_state.cleanup_stale_subagent_sessions(ttl).await;
        }
    });

    // s11 autonomous worker: background claimer for ready task-groups.
    // Disabled by default; enabled via settings.agent.autonomous.enabled.
    {
        let cfg = &daemon_state.app_state.settings.agent.autonomous;
        if cfg.enabled {
            let worker = std::sync::Arc::new(crate::services::AutonomousWorker::new(
                daemon_state.coordinator.clone(),
                crate::services::AutonomousWorkerConfig {
                    poll_interval: std::time::Duration::from_secs(cfg.poll_interval_secs),
                    max_idle_polls: cfg.max_idle_polls,
                    enabled: true,
                },
            ));
            let session_id = daemon_state.app_state.settings.storage.working_dir
                .to_string_lossy()
                .to_string();
            let notify_id = "root".to_string();
            tokio::spawn(async move {
                worker.run(&session_id, &notify_id).await;
            });
            info!("autonomous worker enabled (s11)");
        }
    }

    // Generate a random API token — saved to a restricted-permission file.
    let api_token = auth::generate_api_token();
    crate::utils::write_daemon_token(&api_token)?;
    eprintln!(
        "Daemon API token saved to: {}",
        crate::utils::daemon_token_path().display()
    );

    // Split the router: health stays public, everything else requires auth.
    let (health_router, protected_router) = routes::create_routers(daemon_state, api_token);

    let app = health_router.merge(protected_router).layer(
        CorsLayer::new()
            .allow_origin([
                "http://localhost:3000".parse().unwrap(),
                "http://localhost:5173".parse().unwrap(),
                "http://127.0.0.1:3000".parse().unwrap(),
                "http://127.0.0.1:5173".parse().unwrap(),
            ])
            .allow_methods([
                http::Method::GET,
                http::Method::POST,
                http::Method::PUT,
                http::Method::DELETE,
            ])
            .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("daemon listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    // Clean up token file on daemon shutdown.
    let _ = crate::utils::remove_daemon_token();

    Ok(())
}
