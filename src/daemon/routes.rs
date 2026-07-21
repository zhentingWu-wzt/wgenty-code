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

/// Builds the scoped agent routes. Production and boundary tests share this
/// exact route table so navigation tests exercise the public Axum handlers.
pub(crate) fn agent_routes() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/api/v1/ui/viewers", post(handlers::create_viewer))
        .route("/api/v1/agents/self", get(handlers::get_agent_self))
        .route("/api/v1/agents/children", get(handlers::get_agent_children))
        .route(
            "/api/v1/agents/children/:capability",
            get(handlers::navigate_agent_view),
        )
        .route(
            "/api/v1/agents/children/:capability/transcript",
            get(handlers::get_child_transcript),
        )
        .route(
            "/api/v1/agents/children/:capability/cancel",
            post(handlers::cancel_child),
        )
        .route(
            "/api/v1/agents/task-groups/claim",
            post(handlers::claim_task_group),
        )
        .route(
            "/api/v1/agents/generation/reset",
            post(handlers::reset_agent_generation),
        )
        .route(
            "/api/v1/agents/session/cancel",
            post(handlers::cancel_agent_session),
        )
}

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
        .route(
            "/api/v1/tools/pending-permissions",
            get(handlers::list_pending_permissions),
        )
        .route(
            "/api/v1/tools/resolve-permission",
            post(handlers::resolve_subagent_permission),
        )
        // Permission mode (root agent runtime mode: Yolo/AcceptEdits/Normal)
        .route(
            "/api/v1/permission-mode",
            get(handlers::get_permission_mode).post(handlers::set_permission_mode),
        )
        // Tasks
        .route("/api/v1/tasks", get(handlers::list_tasks))
        .route("/api/v1/tasks/progress", get(handlers::task_progress))
        // Todos (s03 TodoWrite state)
        .route("/api/v1/todos", get(handlers::get_todos))
        // Background tasks
        .route(
            "/api/v1/background/results",
            get(handlers::get_background_results),
        )
        // Scoped agent APIs (strict subagent isolation). The flat
        // /api/v1/subagent/progress endpoint is retired in favor of these
        // capability-scoped local views.
        .merge(agent_routes())
        // Subagent trace stream (SSE): live + cold-start replay. Daemon-only.
        .route(
            "/api/v1/subagents/trace/stream",
            get(handlers::subagent_trace_stream),
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
