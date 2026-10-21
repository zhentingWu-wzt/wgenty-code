//! MCP server management handlers.

use super::*;

pub async fn list_mcp_servers(
    State(state): State<Arc<DaemonState>>,
) -> Json<ListMcpServersResponse> {
    let servers: Vec<McpServerInfo> = state
        .mcp_manager
        .list_servers_for_settings(&state.app_state.settings)
        .await
        .into_iter()
        .map(|server| McpServerInfo {
            name: server.name,
            status: server.status.to_string(),
            tools_count: server.tools_count,
            resources_count: server.resources_count,
        })
        .collect();

    Json(ListMcpServersResponse { servers })
}

/// `POST /api/v1/mcp/servers` — add a new MCP server to settings + auto-start.
pub async fn add_mcp_server(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<AddMcpServerRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let config = crate::config::mcp_config::McpConfig {
        name: body.name,
        command: body.command,
        args: body.args,
        env: body.env,
        cwd: None,
        status: crate::config::mcp_config::McpServerStatus::Unknown,
        capabilities: vec![],
        auto_start: body.auto_start,
        filesystem_path: None,
    };
    state
        .mcp_manager
        .add_server(config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(StatusCode::CREATED)
}

/// `DELETE /api/v1/mcp/servers/:name` — stop + remove an MCP server.
pub async fn remove_mcp_server(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .mcp_manager
        .remove_server(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/mcp/servers/:name/start` — start (enable) a stopped server.
pub async fn start_mcp_server(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .mcp_manager
        .start_server(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(StatusCode::OK)
}

/// `POST /api/v1/mcp/servers/:name/stop` — stop (disable) a running server.
pub async fn stop_mcp_server(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .mcp_manager
        .stop_server(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(StatusCode::OK)
}

/// `POST /api/v1/mcp/servers/:name/restart` — restart a server.
pub async fn restart_mcp_server(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .mcp_manager
        .restart_server(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(StatusCode::OK)
}

// ── Sessions ──────────────────────────────────────────────────────────────────
