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
pub(crate) mod web_ui;
pub mod workspace_files;
pub(crate) mod worktrees;
pub(crate) mod ws_push;

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

/// `spawned_by` marks the daemon as client-bound (web/tui/desktop pass
/// `--spawned-by`): instead of the 300s idle timeout it exits once its last
/// client has been gone for [`state::CLIENT_BOUND_GRACE_SECS`] — the daemon
/// follows its owner down instead of idling out from under it.
pub async fn run(
    app_state: AppState,
    port: u16,
    spawned_by: Option<String>,
) -> anyhow::Result<()> {
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
    // In-handler auth (WebSocket query token) reads the expected value from
    // the shared state; the middleware path keeps its own copy below.
    daemon_state.set_api_token(api_token.clone());
    eprintln!(
        "Daemon API token saved to: {}",
        crate::utils::daemon_token_path().display()
    );

    // Discovery file: write now, heartbeat every 30s, delete on clean exit.
    crate::utils::discovery::spawn_discovery_writer(port, api_token.clone());

    // Split the router: health stays public, everything else requires auth.
    // `cleanup_token` survives the move into `create_routers` so the shutdown
    // cleanup can delete the token file only when it still belongs to us.
    let cleanup_token = api_token.clone();
    let (health_router, protected_router) = routes::create_routers(daemon_state.clone(), api_token);

    let app = health_router
        .merge(protected_router)
        // Localhost daemon: disable Axum's default 2 MiB request body cap so
        // long-session chat/compaction POSTs are not rejected with 413.
        .layer(DefaultBodyLimit::disable())
        .layer(
            // Loopback-only daemon guarded by a per-boot bearer token: an
            // origin allowlist adds no security (callers must present the
            // token regardless of Origin) but breaks the web client whenever
            // vite picks a different port (5174…) or the page is opened via
            // LAN IP / hostname (`vite --host`). Private Network Access is
            // allowed for the same reason: Chrome sends
            // `Access-Control-Request-Private-Network` when a non-loopback
            // origin fetches 127.0.0.1, and without this header it fails the
            // fetch with a bare "Failed to fetch".
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ])
                .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE])
                .allow_private_network(true),
        );

    info!("daemon listening on http://{}", addr);

    // Thin-client shutdown monitor. Two policies:
    //   • unowned (manual `wgenty-code daemon`): exit when no thin client is
    //     connected AND no authenticated API request has arrived for
    //     THIN_CLIENT_IDLE_TIMEOUT_SECS. The window starts at daemon boot, so
    //     a daemon that never serves any client also exits instead of
    //     lingering forever; any activity pushes the deadline out.
    //   • client-bound (`--spawned-by web|tui|desktop`): exit once the last
    //     client has been gone for CLIENT_BOUND_GRACE_SECS — the daemon
    //     follows its owner down instead of idling out from under it.
    let active_clients = daemon_state.active_clients.clone();
    let shutdown_notify = daemon_state.shutdown_notify.clone();
    let spawn_owner = spawned_by.clone();
    let shutdown_signal = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let idle_timeout = if spawn_owner.is_some() {
            std::time::Duration::from_secs(crate::daemon::state::CLIENT_BOUND_GRACE_SECS)
        } else {
            std::time::Duration::from_secs(crate::daemon::state::THIN_CLIENT_IDLE_TIMEOUT_SECS)
        };

        loop {
            // Sleep until the idle deadline (with a small floor so the
            // overdue-but-clients-connected case re-checks periodically
            // instead of busy-spinning).
            let remaining = idle_timeout
                .saturating_sub(active_clients.idle_for())
                .max(std::time::Duration::from_millis(50));
            let deadline = tokio::time::sleep(remaining);
            tokio::pin!(deadline);

            tokio::select! {
                () = &mut deadline => {
                    // A connected thin client keeps the daemon alive even
                    // without requests (an idle-but-open web/desktop app only
                    // receives keepalives).
                    if active_clients.client_count() == 0
                        && active_clients.idle_for() >= idle_timeout
                    {
                        match spawn_owner.as_deref() {
                            Some(owner) => tracing::info!(
                                "client-bound daemon (spawned by {}): no clients for {}s; following owner down, initiating graceful shutdown",
                                owner,
                                idle_timeout.as_secs(),
                            ),
                            None => tracing::info!(
                                "idle timeout elapsed ({}s with no clients and no API activity); initiating graceful shutdown",
                                idle_timeout.as_secs(),
                            ),
                        }
                        active_clients.initiate_shutdown();
                        break;
                    }
                    // Activity during the wait pushed the deadline out —
                    // loop and recompute.
                }
                // Client count changed (last client left) — re-check now
                // instead of waiting out the current sleep.
                () = active_clients.clients_changed() => {}
                _ = &mut ctrl_c => {
                    tracing::info!("received SIGINT; initiating graceful shutdown");
                    active_clients.initiate_shutdown();
                    break;
                }
                // POST /api/v1/shutdown (`wgenty-code daemon stop`).
                () = shutdown_notify.notified() => {
                    tracing::info!("shutdown requested via API; initiating graceful shutdown");
                    active_clients.initiate_shutdown();
                    break;
                }
            }
        }
    };

    // Force-exit watchdog: hyper's graceful shutdown stops accepting new
    // connections but waits for in-flight ones to finish — and our SSE
    // streams (session events, subagent trace, client heartbeat) are
    // long-lived by design, so a shutdown while any client stream is open
    // would hang the process forever (observed as a zombie daemon that had
    // logged "initiating graceful shutdown" yet never exited). Give the
    // drain 5s, then bail out and let process teardown abort the rest.
    let shutdown_initiated = Arc::new(tokio::sync::Notify::new());
    let shutdown_signal = {
        let shutdown_initiated = shutdown_initiated.clone();
        async move {
            shutdown_signal.await;
            shutdown_initiated.notify_one();
        }
    };

    let server = std::future::IntoFuture::into_future(
        axum::serve(listener, app).with_graceful_shutdown(shutdown_signal),
    );
    tokio::pin!(server);
    tokio::select! {
        res = &mut server => {
            res?;
        }
        // `notified()` is polled from the start, and Notify stores a single
        // permit regardless, so the signal can't be missed.
        _ = shutdown_initiated.notified() => {
            tokio::select! {
                res = &mut server => {
                    res?;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    tracing::warn!(
                        "graceful shutdown did not finish within 5s (long-lived streams still open); forcing exit"
                    );
                }
            }
        }
    }

    // Clean up state files on daemon shutdown — but only if they still belong
    // to THIS instance. A newer daemon may already have started (e.g. this one
    // was idle-shutdown while a fresh launch took over the port); deleting
    // unconditionally would orphan the live daemon's auth token.
    let _ = crate::utils::remove_daemon_token_if_matches(&cleanup_token);
    let _ = crate::utils::discovery::remove_discovery_file_if_pid(std::process::id());

    Ok(())
}
