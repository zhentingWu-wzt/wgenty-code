//! Axum router definition for the daemon API.

use crate::daemon::handlers;
use crate::daemon::state::DaemonState;
use axum::{routing::get, routing::post, Router};
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
        .with_state(state)
}
