//! Axum router definition for the daemon API.
//!
//! Returns two routers so the daemon can apply auth middleware only to protected
//! routes while keeping `GET /api/v1/health` public.

use crate::daemon::auth;
use crate::daemon::fs;
use crate::daemon::global_events;
use crate::daemon::handlers;
use crate::daemon::projects;
use crate::daemon::run_loop;
use crate::daemon::session_admin;
use crate::daemon::skills_api;
use crate::daemon::state::DaemonState;
use crate::daemon::worktrees;
use axum::{
    middleware,
    routing::{get, post, put},
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
        // Model profiles (switchable via /model in the TUI)
        .route("/api/v1/models", get(handlers::list_models))
        .route("/api/v1/model/switch", post(handlers::switch_model))
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
        // Interaction (ask_user_question) resolve — server-side loop prompts
        .route(
            "/api/v1/interactions/:id/resolve",
            post(handlers::resolve_interaction),
        )
        // Undo (per-turn file checkpoint rollback)
        .route("/api/v1/tools/undo-turn", post(handlers::undo_turn_range))
        .route("/api/v1/checkpoints", get(handlers::list_checkpoints))
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
        // Global cross-project event stream (SSE, live-only v1)
        .route("/api/v1/events", get(global_events::get_global_events))
        // Memory ops (web-ops-console Tier 2): wrap MemoryManager
        .route("/api/v1/memory/status", get(handlers::memory_status))
        .route("/api/v1/memory", get(handlers::list_memory))
        .route("/api/v1/memory/:id", get(handlers::get_memory))
        .route("/api/v1/memory/prune", post(handlers::prune_memory))
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
        // Skills (web command center, read-only)
        .route("/api/v1/skills", get(skills_api::list_skills))
        // Sessions
        .route(
            "/api/v1/sessions",
            get(handlers::list_sessions).post(handlers::create_session),
        )
        // Filesystem browsing (web directory picker — read-only sub-dir listing)
        .route("/api/v1/fs/dirs", get(fs::list_dirs))
        // Projects (multi-project registry; main project = daemon working_dir)
        .route(
            "/api/v1/projects",
            get(projects::list_projects)
                .post(projects::add_project)
                .delete(projects::remove_project),
        )
        // Worktrees (web command center)
        .route(
            "/api/v1/worktrees",
            get(worktrees::list_worktrees)
                .post(worktrees::create_worktree)
                .delete(worktrees::delete_worktree),
        )
        .route("/api/v1/sessions/search", get(handlers::search_sessions))
        .route(
            "/api/v1/sessions/:id",
            get(handlers::get_session)
                .put(handlers::update_session)
                .delete(handlers::delete_session),
        )
        // Session server-side runs (spawn / cancel an agent turn, live SSE events)
        .route("/api/v1/sessions/:id/run", post(run_loop::post_run))
        .route("/api/v1/sessions/:id/cancel", post(run_loop::post_cancel))
        .route(
            "/api/v1/sessions/:id/events",
            get(run_loop::get_session_events),
        )
        // Session worktree binding + archive (project v1)
        .route(
            "/api/v1/sessions/:id/worktree",
            put(session_admin::bind_worktree).delete(session_admin::unbind_worktree),
        )
        .route(
            "/api/v1/sessions/:id/archive",
            put(session_admin::set_archived),
        )
        .route_layer(middleware::from_fn_with_state(
            api_token,
            auth::require_auth,
        ))
        .with_state(state);

    (health, protected)
}
