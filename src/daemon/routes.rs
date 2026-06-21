//! Axum router definition for the daemon API.
//!
//! Returns two routers so the daemon can apply auth middleware only to protected
//! routes while keeping `GET /api/v1/health` public.

use crate::daemon::auth;
use crate::daemon::handlers;
use crate::daemon::state::DaemonState;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

/// Return `(health_router, protected_router)` so callers can layer differently.
pub fn create_routers(state: Arc<DaemonState>, api_token: String) -> (Router, Router) {
    let health = Router::new()
        .route("/api/v1/health", get(handlers::health))
        .with_state(state.clone());

    let protected = Router::new()
        // Config
        .route("/api/v1/config", get(handlers::get_config))
        // Chat
        .route("/api/v1/chat/stream", post(handlers::chat_stream))
        // Tools
        .route("/api/v1/tools", get(handlers::list_tools))
        .route("/api/v1/tools/execute", post(handlers::execute_tool))
        .route("/api/v1/tools/approve", post(handlers::approve_tool))
        .route("/api/v1/tools/unapprove", post(handlers::unapprove_tool))
        // Tasks
        .route("/api/v1/tasks", get(handlers::list_tasks))
        // Todos (s03 TodoWrite state)
        .route("/api/v1/todos", get(handlers::get_todos))
        // Background tasks
        .route(
            "/api/v1/background/results",
            get(handlers::get_background_results),
        )
        // Subagent progress
        .route(
            "/api/v1/subagent/progress",
            get(handlers::get_subagent_progress),
        )
        // MCP
        .route("/api/v1/mcp/servers", get(handlers::list_mcp_servers))
        // Sessions
        .route(
            "/api/v1/sessions",
            get(handlers::list_sessions).post(handlers::create_session),
        )
        .route("/api/v1/sessions/search", get(handlers::search_sessions))
        .route(
            "/api/v1/sessions/:id",
            get(handlers::get_session)
                .put(handlers::update_session)
                .delete(handlers::delete_session),
        )
        .route_layer(middleware::from_fn_with_state(
            api_token,
            auth::require_auth,
        ))
        .with_state(state);

    (health, protected)
}
