//! Axum router definition for the daemon API.

use crate::daemon::handlers;
use crate::daemon::state::DaemonState;
use axum::{routing::{delete, get, post, put}, Router};
use std::sync::Arc;

pub fn create_router(state: Arc<DaemonState>) -> Router {
    Router::new()
        // Health & Config
        .route("/api/v1/health", get(handlers::health))
        .route("/api/v1/config", get(handlers::get_config))
        // Chat
        .route("/api/v1/chat/stream", post(handlers::chat_stream))
        // Tools
        .route("/api/v1/tools", get(handlers::list_tools))
        .route("/api/v1/tools/execute", post(handlers::execute_tool))
        .route("/api/v1/tools/approve", post(handlers::approve_tool))
        // Tasks
        .route("/api/v1/tasks", get(handlers::list_tasks))
        // Todos (s03 TodoWrite state)
        .route("/api/v1/todos", get(handlers::get_todos))
        // Background tasks
        .route("/api/v1/background/results", get(handlers::get_background_results))
        // MCP
        .route("/api/v1/mcp/servers", get(handlers::list_mcp_servers))
        // Sessions
        .route("/api/v1/sessions", get(handlers::list_sessions))
        .route("/api/v1/sessions", post(handlers::create_session))
        .route("/api/v1/sessions/search", get(handlers::search_sessions))
        .route("/api/v1/sessions/{id}", get(handlers::get_session))
        .route("/api/v1/sessions/{id}", put(handlers::update_session))
        .route("/api/v1/sessions/{id}", delete(handlers::delete_session))
        .with_state(state)
}
