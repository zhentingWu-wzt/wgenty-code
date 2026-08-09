//! Daemon module — HTTP API server that exposes the agent as REST + SSE.
//!
//! Starts an Axum server providing:
//! - `POST /api/v1/chat/stream` — SSE streaming chat completions
//! - `POST /api/v1/tools/execute` — tool execution with permission checks
//! - `GET  /api/v1/mcp/servers` — MCP server management
//!
//! Launch via: `wgenty-code daemon --port 8371`

pub mod auth;
pub(crate) mod fs;
pub mod global_events;
pub mod handlers;
pub mod interaction_bridge;
pub mod memory_router;
pub mod models;
pub mod projects;
pub mod routes;
pub mod run_loop;
pub(crate) mod session_admin;
pub(crate) mod skills_api;
pub mod state;
pub(crate) mod worktrees;

use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use state::DaemonState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing::info;

// No explicit body-size ceiling: the daemon listens on loopback only and chat
// / compaction requests legitimately carry full conversation history. Axum's
// default 2 MiB limit previously caused `413 Payload Too Large` on long sessions.

/// Start the daemon HTTP server. Blocks until the server exits.
pub async fn run(app_state: AppState, port: u16) -> anyhow::Result<()> {
    let daemon_state = Arc::new(DaemonState::new(app_state).await);

    // Recover persisted sessions as lightweight index entries so the
    // `list_sessions` API returns historical sessions quickly. Full message
    // history is hydrated on demand via `load(id)` / `get(id)`.
    if let Err(e) = daemon_state.session_manager.load_index().await {
        tracing::warn!(error = %e, "Failed to load persisted sessions into daemon");
    }

    // Restore session → worktree bindings persisted in session metadata.
    session_admin::reconcile_worktree_bindings(&daemon_state).await;

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
            let session_id = daemon_state
                .app_state
                .settings
                .storage
                .working_dir
                .to_string_lossy()
                .to_string();
            let notify_id = "root".to_string();
            tokio::spawn(async move {
                worker.run(&session_id, &notify_id).await;
            });
            info!("autonomous worker enabled (s11)");
        }
    }

    // Bind the TCP listener BEFORE writing the token/discovery files. If the
    // port is already in use (another daemon instance), we want to fail fast
    // without clobbering the existing daemon's token — writing the token first
    // and then failing on bind would leave a stale token on disk that doesn't
    // match the running daemon, breaking all clients. This mirrors the order
    // in `src/tui/util.rs::start_daemon`.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("daemon binding to http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Generate a random API token — saved to a restricted-permission file.
    // Written only after the port is successfully bound, so a bind failure
    // never overwrites an existing daemon's token.
    let api_token = auth::generate_api_token();
    crate::utils::write_daemon_token(&api_token)?;
    eprintln!(
        "Daemon API token saved to: {}",
        crate::utils::daemon_token_path().display()
    );

    // Discovery file: write now, heartbeat every 30s, delete on clean exit.
    crate::utils::discovery::spawn_discovery_writer(port, api_token.clone());

    // Split the router: health stays public, everything else requires auth.
    let (health_router, protected_router) = routes::create_routers(daemon_state.clone(), api_token);

    let app = health_router
        .merge(protected_router)
        // Localhost daemon: disable Axum's default 2 MiB request body cap so
        // long-session chat/compaction POSTs are not rejected with 413.
        .layer(DefaultBodyLimit::disable())
        .layer(
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:3000"
                        .parse()
                        .expect("invalid hardcoded URL literal"),
                    "http://localhost:5173"
                        .parse()
                        .expect("invalid hardcoded URL literal"),
                    "http://127.0.0.1:3000"
                        .parse()
                        .expect("invalid hardcoded URL literal"),
                    "http://127.0.0.1:5173"
                        .parse()
                        .expect("invalid hardcoded URL literal"),
                ])
                .allow_methods([
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ])
                .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]),
        );

    info!("daemon listening on http://{}", addr);

    // Thin-client idle-shutdown monitor: when the last thin client
    // disconnects, start a grace-period timer. If no client reconnects
    // within that window, signal the Axum server to shut down gracefully.
    let active_clients = daemon_state.active_clients.clone();
    let shutdown_signal = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        loop {
            tokio::select! {
                // wait_for_zero resolves when: (a) the first client connects, or
                // (b) count reaches zero after having had clients.
                () = active_clients.wait_for_zero() => {
                    // Only proceed with shutdown when count is actually zero.
                    if active_clients.client_count() != 0 {
                        continue;
                    }

                    let timeout = std::time::Duration::from_secs(
                        crate::daemon::state::THIN_CLIENT_IDLE_TIMEOUT_SECS,
                    );
                    tracing::info!(
                        "all thin clients disconnected; waiting {}s before shutdown",
                        timeout.as_secs(),
                    );

                    // Sleep through the grace period. If a client reconnects
                    // during this time, the count will be >0 when we wake up.
                    tokio::time::sleep(timeout).await;

                    if active_clients.client_count() == 0 {
                        tracing::info!("idle timeout elapsed; initiating graceful shutdown");
                        active_clients.initiate_shutdown();
                        break;
                    }

                    tracing::info!("client reconnected during grace period; cancelling shutdown");
                }
                _ = &mut ctrl_c => {
                    tracing::info!("received SIGINT; initiating graceful shutdown");
                    active_clients.initiate_shutdown();
                    break;
                }
            }
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    // Clean up token file on daemon shutdown.
    let _ = crate::utils::remove_daemon_token();
    let _ = crate::utils::discovery::remove_discovery_file();

    Ok(())
}
