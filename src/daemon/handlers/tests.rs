use super::agents::{assemble_local_view, resolve_navigation_context};
use super::*;

#[tokio::test]
async fn daemon_registers_contextual_work_graph_tools() {
    use crate::config::Settings;
    use crate::state::AppState;

    let temp = tempfile::tempdir().expect("temp directory");
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let state = DaemonState::new(AppState::new(settings)).await;

    for name in [
        "begin_node",
        "verify_node",
        "rollback_node",
        "verify_and_complete",
        "submit_specialist_report",
    ] {
        assert!(
            state.tool_registry.get(name).is_some(),
            "missing contextual work-graph tool {name}"
        );
    }
}

#[tokio::test]
async fn daemon_specialist_tool_accepts_a_trusted_root_cause_child() {
    use crate::agent::{SpawnChildRequest, ToolContext, ToolInvocationId};
    use crate::config::Settings;
    use crate::org_graph::NodeType;
    use crate::state::AppState;

    let temp = tempfile::tempdir().expect("temp directory");
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let state = DaemonState::new(AppState::new(settings)).await;
    let root = state
        .root_context("specialist-tool-session")
        .await
        .expect("root context");
    let child = state
        .coordinator
        .reserve_child(
            &root,
            SpawnChildRequest::new("diagnose").with_node_type(NodeType::RootCause),
        )
        .await
        .expect("reserve root-cause child");
    state
        .work_graph_runtime_store
        .ensure_turn(&root.session_id)
        .expect("start graph turn");
    let root_tool_context = ToolContext {
        agent: &root,
        invocation_id: ToolInvocationId::new("begin-specialist-node"),
        origin_turn_id: None,
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::Normal,
        checkpoint: None,
    };
    state
        .tool_registry
        .execute_with_context(
            &root_tool_context,
            "begin_node",
            serde_json::json!({
                "goal": "diagnose defect",
                "verify_commands": [],
                "expected_files": []
            }),
        )
        .await
        .expect("start graph node through daemon registry");
    state
        .work_graph_runtime_store
        .seed_root_cause_route_for_test(&root.session_id);
    assert!(matches!(
        state
            .work_graph_runtime_store
            .prepare_root_cause_dispatch(&root.session_id)
            .expect("prepare root-cause dispatch"),
        crate::exec_session::RootCauseDispatchState::Ready(_)
    ));
    state
        .work_graph_runtime_store
        .bind_root_cause_child(
            &root.session_id,
            child.context.agent_id.as_str().to_string(),
        )
        .expect("bind root-cause child");
    let context = ToolContext {
        agent: &child.context,
        invocation_id: ToolInvocationId::new("specialist-tool"),
        origin_turn_id: None,
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::Normal,
        checkpoint: None,
    };

    let output = state
            .tool_registry
            .execute_with_context(
                &context,
                "submit_specialist_report",
                serde_json::json!({
                    "kind": "root_cause",
                    "summary": "A guard is bypassed.",
                    "evidence": [{ "path": "src/guard.rs", "detail": "A branch returns before validation." }],
                    "suspected_files": ["src/guard.rs"],
                    "recommended_actions": ["Validate before branching."]
                }),
            )
            .await
            .expect("submit specialist report through daemon registry");

    assert!(output.content.contains("recorded"));
}

#[tokio::test]
async fn work_graph_sessions_are_isolated() {
    use crate::config::Settings;
    use crate::state::AppState;

    let temp = tempfile::tempdir().expect("temp directory");
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let root_a = state.root_context("graph-session-a").await.expect("root a");
    let root_b = state.root_context("graph-session-b").await.expect("root b");
    let context_a = crate::agent::ToolContext {
        agent: &root_a,
        invocation_id: crate::agent::ToolInvocationId::new("graph-a"),
        origin_turn_id: None,
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::Normal,
        checkpoint: None,
    };
    let context_b = crate::agent::ToolContext {
        agent: &root_b,
        invocation_id: crate::agent::ToolInvocationId::new("graph-b"),
        origin_turn_id: None,
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::Normal,
        checkpoint: None,
    };
    let begin = serde_json::json!({
        "goal": "isolated node",
        "verify_commands": ["true"],
        "expected_files": []
    });

    state
        .tool_registry
        .execute_with_context(&context_a, "begin_node", begin.clone())
        .await
        .expect("begin session a node");
    state
        .tool_registry
        .execute_with_context(&context_b, "begin_node", begin.clone())
        .await
        .expect("begin session b node");
    state
        .tool_registry
        .execute_with_context(&context_a, "verify_node", serde_json::json!({}))
        .await
        .expect("verify session a node");
    let second_a = state
        .tool_registry
        .execute_with_context(&context_a, "begin_node", begin)
        .await
        .expect("session a may advance after its own verification");
    assert!(second_a.content.contains("n2"));

    let second_b = state
        .tool_registry
        .execute_with_context(
            &context_b,
            "begin_node",
            serde_json::json!({
                "goal": "must remain running",
                "verify_commands": ["true"],
                "expected_files": []
            }),
        )
        .await
        .expect_err("session b must not observe session a verification");
    assert_eq!(second_b.code.as_deref(), Some("begin_node_failed"));
    for session_id in ["graph-session-a", "graph-session-b"] {
        assert!(
            temp.path()
                .join(".wgenty-code")
                .join("snapshots")
                .join(session_id)
                .join("session.json")
                .is_file(),
            "missing isolated session record for {session_id}"
        );
    }
}

#[tokio::test]
async fn daemon_state_saves_sessions_under_project_local_dir() {
    use crate::config::Settings;
    use crate::daemon::models::{SessionResponse, UpdateSessionRequest};
    use crate::state::AppState;
    use crate::utils::project_sessions_dir;
    use axum::extract::{Path, State};
    use axum::Json;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    // Do not override session_manager — this asserts DaemonState::new wires
    // MemorySessionManager::with_project_root(working_dir).
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);

    let fixed_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();
    let body = UpdateSessionRequest {
        name: Some("project-local".to_string()),
        messages: Some(vec![crate::context::memory_session::SessionMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: chrono::Utc::now(),
            metadata: Default::default(),
        }]),
        ui_messages: None,
        expected_version: None,
    };
    let Json(resp): Json<SessionResponse> =
        update_session(State(state.clone()), Path(fixed_id.clone()), Json(body))
            .await
            .expect("update_session should succeed");
    assert_eq!(resp.id, fixed_id);

    let expected_path = project_sessions_dir(temp.path()).join(format!("{fixed_id}.json"));
    assert!(
        expected_path.is_file(),
        "session must be written under project-local dir, expected {}",
        expected_path.display()
    );

    // Must not land only in the global home sessions dir for this working_dir.
    let home_sessions = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".wgenty-code")
        .join("sessions")
        .join(format!("{fixed_id}.json"));
    assert!(
            !home_sessions.is_file(),
            "session must not be written to global ~/.wgenty-code/sessions when project dir is writable"
        );
}

#[tokio::test]
async fn multi_project_session_routing() {
    use crate::config::Settings;
    use crate::daemon::models::CreateSessionRequest;
    use crate::state::AppState;
    use axum::extract::{Path, State};
    use axum::Json;

    let main = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = main.path().to_path_buf();
    let mut state = DaemonState::new(AppState::new(settings)).await;
    // Isolate the registry from the developer's real projects.json.
    let reg_store = tempfile::tempdir().unwrap();
    state.projects = crate::daemon::projects::ProjectRegistry::load(
        main.path().to_path_buf(),
        reg_store.path().join("projects.json"),
    );
    let state = Arc::new(state);

    // Register project B and create a session in it.
    let proj_b = tempfile::tempdir().unwrap();
    let b_canon = proj_b.path().canonicalize().unwrap();
    state.projects.add(proj_b.path().to_str().unwrap()).unwrap();
    let Json(created) = create_session(
        State(state.clone()),
        Json(CreateSessionRequest {
            name: Some("in-b".to_string()),
            project_path: Some(b_canon.to_string_lossy().to_string()),
        }),
    )
    .await
    .expect("create_session into project B");
    let sid = created.id;

    // The session file lands under project B's store, not the main one.
    assert!(crate::utils::project_sessions_dir(&b_canon)
        .join(format!("{sid}.json"))
        .is_file());
    assert!(!crate::utils::project_sessions_dir(main.path())
        .join(format!("{sid}.json"))
        .is_file());

    // list_sessions aggregates both projects and tags the entry.
    let Json(listed) = list_sessions(State(state.clone())).await.unwrap();
    let entry = listed.iter().find(|s| s.id == sid).expect("listed");
    assert_eq!(
        entry.project_path.as_deref(),
        Some(b_canon.to_string_lossy().as_ref())
    );

    // get/update/delete route across projects.
    let Json(got) = get_session(State(state.clone()), Path(sid.clone()))
        .await
        .expect("get_session routes to project B");
    assert_eq!(got.name, "in-b");

    // The effective working root follows the session's project.
    assert_eq!(state.effective_session_root(&sid).await, b_canon);

    // Unknown project path is rejected.
    let rejected = create_session(
        State(state.clone()),
        Json(CreateSessionRequest {
            name: None,
            project_path: Some("/no/such/project".to_string()),
        }),
    )
    .await;
    assert!(rejected.is_err());

    // delete routes to the owning store.
    let _ = delete_session(State(state.clone()), Path(sid.clone()))
        .await
        .expect("delete routes to project B");
    assert!(state.resolve_session(&sid).await.is_none());
}

#[tokio::test]
async fn update_session_upsert_preserves_path_id_across_saves() {
    use crate::config::Settings;
    use crate::daemon::models::{SessionResponse, UpdateSessionRequest};
    use crate::state::AppState;
    use axum::extract::{Path, State};
    use axum::Json;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    // Isolate session files for this test before wrapping in Arc.
    let sessions_dir = temp.path().join("sessions-test");
    let mut state = DaemonState::new(AppState::new(settings)).await;
    state.session_manager =
        crate::context::memory_session::SessionManager::with_dir(sessions_dir.clone());
    let state = Arc::new(state);

    let fixed_id = "11111111-2222-3333-4444-555555555555".to_string();
    for i in 0..3 {
        let body = UpdateSessionRequest {
            name: Some("duplicate-name".to_string()),
            messages: Some(vec![crate::context::memory_session::SessionMessage {
                role: "user".to_string(),
                content: format!("turn-{i}"),
                tool_call_id: None,
                tool_calls: None,
                timestamp: chrono::Utc::now(),
                metadata: Default::default(),
            }]),
            ui_messages: None,
            expected_version: None,
        };
        let Json(resp): Json<SessionResponse> =
            update_session(State(state.clone()), Path(fixed_id.clone()), Json(body))
                .await
                .expect("update_session should succeed");
        assert_eq!(resp.id, fixed_id, "response id must match path id");
        assert_eq!(resp.name, "duplicate-name");
    }

    let mut entries = tokio::fs::read_dir(&sessions_dir).await.unwrap();
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        files.push(entry.file_name().to_string_lossy().into_owned());
    }
    assert_eq!(
        files,
        vec![format!("{fixed_id}.json")],
        "must not mint a new file per save"
    );

    let loaded = state
        .session_manager
        .load(&fixed_id)
        .await
        .unwrap()
        .expect("session file exists");
    assert_eq!(loaded.id, fixed_id);
    assert_eq!(loaded.messages.len(), 1); // last write replaces messages
}

#[tokio::test]
async fn update_session_version_matrix() {
    use crate::config::Settings;
    use crate::daemon::models::UpdateSessionRequest;
    use crate::state::AppState;
    use axum::extract::{Path, State};
    use axum::Json;

    // Keep the tempdir alive for the whole test: sessions persist under
    // `working_dir`'s project-local dir (mirrors the first test above).
    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let id = "ver-matrix".to_string();

    let body = |expected_version: Option<u64>| UpdateSessionRequest {
        name: None,
        messages: None,
        ui_messages: None,
        expected_version,
    };

    // First upsert (no expected_version) → version 0 -> 1.
    let Json(r1) = update_session(State(state.clone()), Path(id.clone()), Json(body(None)))
        .await
        .expect("upsert ok");
    assert_eq!(r1.version, 1);

    // Some(0) no longer matches the current version 1 → 409 + current_version.
    let err = update_session(State(state.clone()), Path(id.clone()), Json(body(Some(0))))
        .await
        .expect_err("stale expected_version conflicts");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert_eq!(err.1 .0["current_version"], 1);

    // Some(1) matches → success, version advances to 2.
    let Json(r3) = update_session(State(state.clone()), Path(id.clone()), Json(body(Some(1))))
        .await
        .expect("matching expected_version ok");
    assert_eq!(r3.version, 2);

    // None → legacy last-write-wins path, still succeeds.
    let Json(r4) = update_session(State(state.clone()), Path(id.clone()), Json(body(None)))
        .await
        .expect("no expected_version stays compatible");
    assert_eq!(r4.version, 3);
}

#[tokio::test]
async fn agent_routes_support_recursive_generation_bound_navigation() {
    use crate::agent::capability::CapabilityGrant;
    use crate::agent::SpawnChildRequest;
    use crate::config::Settings;
    use crate::daemon::models::{CreateViewerResponse, LocalAgentViewResponse};
    use crate::state::AppState;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.storage.transcript.db_path = temp
        .path()
        .join("subagent-transcripts.db")
        .to_string_lossy()
        .into_owned();
    // This test exercises a root -> child -> grandchild chain (depth 3),
    // but the product default disables subagent recursion (max_depth=1).
    // Raise the limit here so the navigation scenario under test can run.
    settings.agent.subagent.max_depth = 3;
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let root = state.root_context("session").await.unwrap();
    assert_eq!(
        state.coordinator.advance_generation(&root.session_id).await,
        1
    );
    let child = state
        .coordinator
        .reserve_child(&root, SpawnChildRequest::new("child"))
        .await
        .unwrap()
        .context;
    let grandchild = state
        .coordinator
        .reserve_child(&child, SpawnChildRequest::new("grandchild"))
        .await
        .unwrap()
        .context;

    let app = crate::daemon::routes::agent_routes().with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();

    let viewer = client
        .post(format!("{base_url}/api/v1/ui/viewers"))
        .send()
        .await
        .unwrap()
        .json::<CreateViewerResponse>()
        .await
        .unwrap()
        .viewer_token;
    let root_response = client
        .get(format!("{base_url}/api/v1/agents/self?session_id=session"))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(root_response.status(), StatusCode::OK);
    let root_view = root_response
        .json::<LocalAgentViewResponse>()
        .await
        .unwrap();
    assert_eq!(root_view.self_view.agent_id, root.agent_id.as_str());
    assert_eq!(root_view.children.len(), 1);
    assert_eq!(root_view.children[0].agent_id, child.agent_id.as_str());

    let child_capability = &root_view.children[0].navigation_capability;
    let child_response = client
        .get(format!(
            "{base_url}/api/v1/agents/children/{child_capability}?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(child_response.status(), StatusCode::OK);
    let child_view = child_response
        .json::<LocalAgentViewResponse>()
        .await
        .unwrap();
    assert_eq!(child_view.self_view.agent_id, child.agent_id.as_str());
    assert_eq!(child_view.children.len(), 1);
    assert_eq!(
        child_view.children[0].agent_id,
        grandchild.agent_id.as_str()
    );

    let grandchild_capability = &child_view.children[0].navigation_capability;
    let grandchild_response = client
        .get(format!(
            "{base_url}/api/v1/agents/children/{grandchild_capability}?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(grandchild_response.status(), StatusCode::OK);
    let grandchild_view = grandchild_response
        .json::<LocalAgentViewResponse>()
        .await
        .unwrap();
    assert_eq!(
        grandchild_view.self_view.agent_id,
        grandchild.agent_id.as_str()
    );
    assert!(grandchild_view.children.is_empty());

    let wrong_viewer = client
        .post(format!("{base_url}/api/v1/ui/viewers"))
        .send()
        .await
        .unwrap()
        .json::<CreateViewerResponse>()
        .await
        .unwrap()
        .viewer_token;
    for (capability, denied_viewer, denied_session) in [
        (
            grandchild_capability.as_str(),
            wrong_viewer.as_str(),
            "session",
        ),
        (grandchild_capability.as_str(), viewer.as_str(), "other"),
        ("forged-capability", viewer.as_str(), "session"),
    ] {
        let response = client
            .get(format!(
                "{base_url}/api/v1/agents/children/{capability}?session_id={denied_session}"
            ))
            .header(VIEWER_TOKEN_HEADER, denied_viewer)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    let stale_capability = state
        .capability_service
        .issue(&CapabilityGrant::navigate(
            viewer.as_str(),
            "session",
            grandchild.agent_id.as_str(),
            0,
        ))
        .await;
    let stale_response = client
        .get(format!(
            "{base_url}/api/v1/agents/children/{stale_capability}?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(stale_response.status(), StatusCode::NOT_FOUND);

    server.abort();
}

/// Minimal [`crate::agent::SubagentProgress`] for directory cross-fill
/// tests: only the fields the handler reads are populated.
fn directory_progress(
    agent_id: &str,
    status: crate::agent::SubagentStatus,
    started_at: i64,
    elapsed_ms: u64,
    cumulative_tokens: u64,
    round: Option<usize>,
    max_rounds: Option<usize>,
) -> crate::agent::SubagentProgress {
    crate::agent::SubagentProgress {
        node_id: agent_id.to_string(),
        parent_id: None,
        label: agent_id.to_string(),
        status,
        round,
        max_rounds,
        current_tool: None,
        current_params: None,
        action_log: Vec::new(),
        text_snapshot: None,
        started_at,
        elapsed_ms,
        metadata: None,
        progress_delta: None,
        token_budget_k: None,
        cumulative_tokens,
        error_details: None,
        events: Vec::new(),
        messages: Vec::new(),
    }
}

#[tokio::test]
async fn agent_directory_endpoint_serves_recursive_tree_with_progress_crossfill() {
    use crate::agent::SpawnChildRequest;
    use crate::config::Settings;
    use crate::daemon::models::{AgentDirectoryResponse, CreateViewerResponse};
    use crate::state::AppState;
    use std::collections::HashMap;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.storage.transcript.db_path = temp
        .path()
        .join("subagent-transcripts.db")
        .to_string_lossy()
        .into_owned();
    // This test exercises a root -> child -> grandchild chain (depth 3),
    // but the product default disables subagent recursion (max_depth=1).
    // Raise the limit here so the directory scenario under test can run.
    settings.agent.subagent.max_depth = 3;
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let root = state.root_context("session").await.unwrap();
    assert_eq!(
        state.coordinator.advance_generation(&root.session_id).await,
        1
    );
    let child = state
        .coordinator
        .reserve_child(&root, SpawnChildRequest::new("child"))
        .await
        .unwrap()
        .context;
    let grandchild = state
        .coordinator
        .reserve_child(&child, SpawnChildRequest::new("grandchild"))
        .await
        .unwrap()
        .context;
    // Complete the child through the public coordinator surface; the
    // directory must keep terminal agents visible.
    state
        .coordinator
        .finish_child(
            &child,
            crate::agent::ChildTerminal::Completed {
                summary: "child finished".to_string(),
            },
        )
        .await
        .unwrap();
    // Cross-fill source: seed the legacy progress store with entries for
    // the terminal child (stored elapsed wins) and the running grandchild
    // (elapsed recomputed live from started_at).
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut entries = HashMap::new();
    entries.insert(
        child.agent_id.as_str().to_string(),
        directory_progress(
            child.agent_id.as_str(),
            crate::agent::SubagentStatus::Completed,
            now_ms - 60_000,
            12_000,
            4_321,
            Some(3),
            Some(5),
        ),
    );
    entries.insert(
        grandchild.agent_id.as_str().to_string(),
        directory_progress(
            grandchild.agent_id.as_str(),
            crate::agent::SubagentStatus::Running,
            now_ms - 10_000,
            0,
            999,
            Some(2),
            Some(7),
        ),
    );
    state
        .subagent_progress
        .write()
        .await
        .insert(root.session_id.as_str().to_string(), entries);

    let app = crate::daemon::routes::agent_routes().with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();

    let viewer = client
        .post(format!("{base_url}/api/v1/ui/viewers"))
        .send()
        .await
        .unwrap()
        .json::<CreateViewerResponse>()
        .await
        .unwrap()
        .viewer_token;
    let response = client
        .get(format!(
            "{base_url}/api/v1/agents/directory?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let directory = response.json::<AgentDirectoryResponse>().await.unwrap();
    assert_eq!(directory.session_id, "session");
    assert_eq!(directory.root.agent_id, root.agent_id.as_str());
    assert_eq!(directory.root.depth, 0);
    // The root agent has no progress entry: defaults, not fake metrics.
    assert_eq!(directory.root.cumulative_tokens, 0);
    assert_eq!(directory.root.started_at, 0);
    assert_eq!(directory.root.round, None);
    assert_eq!(directory.root.children.len(), 1);

    let child_entry = &directory.root.children[0];
    assert_eq!(child_entry.agent_id, child.agent_id.as_str());
    assert_eq!(child_entry.depth, 1);
    assert_eq!(
        child_entry.status,
        crate::agent::AgentLifecycleStatus::Completed
    );
    assert_eq!(child_entry.summary.as_deref(), Some("child finished"));
    // Progress cross-fill from the legacy store.
    assert_eq!(child_entry.cumulative_tokens, 4_321);
    assert_eq!(child_entry.round, Some(3));
    assert_eq!(child_entry.max_rounds, Some(5));
    // Terminal agents keep their stored elapsed value.
    assert_eq!(child_entry.elapsed_ms, 12_000);

    assert_eq!(child_entry.children.len(), 1);
    let grandchild_entry = &child_entry.children[0];
    assert_eq!(grandchild_entry.agent_id, grandchild.agent_id.as_str());
    assert_eq!(grandchild_entry.depth, 2);
    assert_eq!(grandchild_entry.cumulative_tokens, 999);
    assert_eq!(grandchild_entry.round, Some(2));
    assert_eq!(grandchild_entry.max_rounds, Some(7));
    // Running agents get a live-recomputed timer from started_at.
    assert!(grandchild_entry.elapsed_ms >= 10_000);
    assert!(grandchild_entry.children.is_empty());

    // Missing session_id is a client bug, not a denial: 400.
    let missing = client
        .get(format!("{base_url}/api/v1/agents/directory"))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    // Unknown viewer token maps to the same stable 404 as the other
    // scoped endpoints.
    let unknown_viewer = client
        .get(format!(
            "{base_url}/api/v1/agents/directory?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, "not-a-viewer")
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_viewer.status(), StatusCode::NOT_FOUND);

    server.abort();
}

#[tokio::test]
async fn agent_directory_keeps_terminal_children_navigable_after_generation_reset() {
    use crate::agent::SpawnChildRequest;
    use crate::config::Settings;
    use crate::daemon::models::{
        AgentDirectoryResponse, CreateViewerResponse, LocalAgentViewResponse,
    };
    use crate::state::AppState;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.storage.transcript.db_path = temp
        .path()
        .join("subagent-transcripts.db")
        .to_string_lossy()
        .into_owned();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let root = state.root_context("session").await.unwrap();
    let child = state
        .coordinator
        .reserve_child(&root, SpawnChildRequest::new("child"))
        .await
        .unwrap()
        .context;
    state
        .coordinator
        .finish_child(
            &child,
            crate::agent::ChildTerminal::Completed {
                summary: "child finished".to_string(),
            },
        )
        .await
        .unwrap();

    let app = crate::daemon::routes::agent_routes().with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();

    let viewer = client
        .post(format!("{base_url}/api/v1/ui/viewers"))
        .send()
        .await
        .unwrap()
        .json::<CreateViewerResponse>()
        .await
        .unwrap()
        .viewer_token;

    // A new turn advances the generation, cancelling obsolete subtrees —
    // but terminal records must stay visible and navigable (regression:
    // detail panels must keep opening completed subagents after the turn
    // ends; the records are not deleted by the reset).
    let reset = client
        .post(format!("{base_url}/api/v1/agents/generation/reset"))
        .json(&serde_json::json!({"session_id": "session"}))
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::OK);

    let response = client
        .get(format!(
            "{base_url}/api/v1/agents/directory?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let directory = response.json::<AgentDirectoryResponse>().await.unwrap();
    assert_eq!(directory.root.children.len(), 1);
    assert_eq!(
        directory.root.children[0].status,
        crate::agent::AgentLifecycleStatus::Completed
    );
    assert_eq!(directory.root.children[0].agent_id, child.agent_id.as_str());

    // Fresh navigation capability for the terminal child still resolves:
    // /agents/self re-issues it and the scoped children endpoint accepts
    // it, so detail tabs keep working past the turn boundary.
    let self_response = client
        .get(format!("{base_url}/api/v1/agents/self?session_id=session"))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(self_response.status(), StatusCode::OK);
    let view = self_response
        .json::<LocalAgentViewResponse>()
        .await
        .unwrap();
    assert_eq!(view.children.len(), 1);
    let cap = view.children[0].navigation_capability.clone();
    let nav = client
        .get(format!(
            "{base_url}/api/v1/agents/children/{cap}?session_id=session"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(nav.status(), StatusCode::OK);
    let nav_view = nav.json::<LocalAgentViewResponse>().await.unwrap();
    assert_eq!(nav_view.self_view.agent_id, child.agent_id.as_str());

    server.abort();
}

#[tokio::test]
async fn agent_directory_endpoint_returns_empty_tree_for_root_only_session() {
    use crate::config::Settings;
    use crate::daemon::models::AgentDirectoryResponse;
    use crate::state::AppState;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.storage.transcript.db_path = temp
        .path()
        .join("subagent-transcripts.db")
        .to_string_lossy()
        .into_owned();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let root = state.root_context("empty").await.unwrap();

    let app = crate::daemon::routes::agent_routes().with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();

    let viewer = client
        .post(format!("{base_url}/api/v1/ui/viewers"))
        .send()
        .await
        .unwrap()
        .json::<crate::daemon::models::CreateViewerResponse>()
        .await
        .unwrap()
        .viewer_token;
    let response = client
        .get(format!(
            "{base_url}/api/v1/agents/directory?session_id=empty"
        ))
        .header(VIEWER_TOKEN_HEADER, &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = response.text().await.unwrap();
    let directory: AgentDirectoryResponse = serde_json::from_str(&raw).unwrap();
    assert_eq!(directory.session_id, "empty");
    assert_eq!(directory.root.agent_id, root.agent_id.as_str());
    assert!(directory.root.children.is_empty());
    // The polling payload is hierarchy + metrics only: no messages or
    // trace ids anywhere in the wire format.
    assert!(!raw.contains("messages"));
    assert!(!raw.contains("trace_id"));

    server.abort();
}

#[tokio::test]
async fn navigation_resolves_recursive_targets_with_canonical_hierarchy() {
    use crate::agent::capability::{CapabilityGrant, CapabilityService, ViewerId};
    use crate::agent::{AgentCoordinator, SessionId, SpawnChildRequest};
    use std::collections::HashMap;

    let coordinator = AgentCoordinator::new(8, 4);
    let root = coordinator
        .ensure_root(SessionId::new("session"))
        .await
        .unwrap();
    assert_eq!(coordinator.advance_generation(&root.session_id).await, 1);
    let child = coordinator
        .reserve_child(&root, SpawnChildRequest::new("child"))
        .await
        .unwrap()
        .context;
    let grandchild = coordinator
        .reserve_child(&child, SpawnChildRequest::new("grandchild"))
        .await
        .unwrap()
        .context;
    let service = CapabilityService::new([7; 32]);
    let viewer = ViewerId::new("viewer");
    let progress = HashMap::new();

    let root_response = assemble_local_view(&coordinator, &service, &progress, &root, &viewer)
        .await
        .unwrap();
    assert_eq!(root_response.self_view.agent_id, root.agent_id.as_str());
    assert_eq!(root_response.children.len(), 1);
    assert!(!root_response
        .children
        .iter()
        .any(|record| record.agent_id == grandchild.agent_id.as_str()));

    let child_capability = &root_response.children[0].navigation_capability;
    let child_context =
        resolve_navigation_context(&coordinator, &service, child_capability, &viewer, "session")
            .await
            .unwrap();
    assert_eq!(child_context.agent_id, child.agent_id);
    assert_eq!(child_context.parent_id, Some(root.agent_id.clone()));
    assert_eq!(child_context.depth, 1);
    let child_response =
        assemble_local_view(&coordinator, &service, &progress, &child_context, &viewer)
            .await
            .unwrap();
    assert_eq!(child_response.self_view.agent_id, child.agent_id.as_str());
    assert_eq!(child_response.children.len(), 1);
    assert_eq!(
        child_response.children[0].agent_id,
        grandchild.agent_id.as_str()
    );

    let grandchild_capability = &child_response.children[0].navigation_capability;
    let grandchild_context = resolve_navigation_context(
        &coordinator,
        &service,
        grandchild_capability,
        &viewer,
        "session",
    )
    .await
    .unwrap();
    assert_eq!(grandchild_context.agent_id, grandchild.agent_id);
    assert_eq!(grandchild_context.parent_id, Some(child.agent_id.clone()));
    assert_eq!(grandchild_context.depth, 2);
    let grandchild_response = assemble_local_view(
        &coordinator,
        &service,
        &progress,
        &grandchild_context,
        &viewer,
    )
    .await
    .unwrap();
    assert_eq!(
        grandchild_response.self_view.agent_id,
        grandchild.agent_id.as_str()
    );
    assert!(grandchild_response.children.is_empty());

    let stale_capability = service
        .issue(&CapabilityGrant::navigate(
            viewer.as_str(),
            "session",
            grandchild.agent_id.as_str(),
            0,
        ))
        .await;
    assert_eq!(
        resolve_navigation_context(
            &coordinator,
            &service,
            &stale_capability,
            &viewer,
            "session",
        )
        .await
        .unwrap_err(),
        StatusCode::NOT_FOUND
    );

    for (capability, denied_viewer, denied_session) in [
        (
            grandchild_capability.as_str(),
            ViewerId::new("wrong-viewer"),
            "session",
        ),
        (
            grandchild_capability.as_str(),
            viewer.clone(),
            "wrong-session",
        ),
        ("forged-capability", viewer.clone(), "session"),
    ] {
        assert_eq!(
            resolve_navigation_context(
                &coordinator,
                &service,
                capability,
                &denied_viewer,
                denied_session,
            )
            .await
            .unwrap_err(),
            StatusCode::NOT_FOUND
        );
    }
}

#[test]
fn scoped_coordinator_error_preserves_not_found_boundary() {
    assert_eq!(
        map_scoped_coordinator_error(crate::agent::CoordinatorError::NotVisible),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        map_scoped_coordinator_error(crate::agent::CoordinatorError::Storage(
            "invariant".to_string()
        )),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

// ── Subagent trace SSE (Task 3.4 / 3.5) ──────────────────────────────────

#[tokio::test]
async fn replay_session_events_filters_by_session_and_since() {
    use crate::transcript::{SubagentTranscript, SubagentTranscriptStore, TranscriptStatus};

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("replay.db");
    let store = SubagentTranscriptStore::open(&db).unwrap();

    let mk = |id: &str, sid: &str, started_at: i64, status: TranscriptStatus| SubagentTranscript {
        id: id.into(),
        session_id: sid.into(),
        parent_id: None,
        label: format!("label-{id}"),
        status,
        system_prompt: None,
        user_prompt: "u".into(),
        started_at,
        finished_at: Some(started_at + 1000),
        total_tokens: 100,
        max_rounds: None,
        actual_rounds: 3,
        token_budget_k: None,
        error_message: None,
        summary: None,
        failure_diagnostics: None,
        project_path: None,
        node_type: None,
        events: vec![],
    };

    store
        .save(&mk("a", "alpha", 1000, TranscriptStatus::Completed), None)
        .unwrap();
    store
        .save(&mk("b", "alpha", 2000, TranscriptStatus::Failed), None)
        .unwrap();
    store
        .save(&mk("c", "beta", 3000, TranscriptStatus::Completed), None)
        .unwrap();

    // No since filter: both alpha events, ascending by started_at
    // (list_by_session returns DESC; replay reverses to ASC).
    let evs = replay_session_events(&store, "alpha", 0);
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0].node_id, "a");
    assert_eq!(evs[0].ts, 1000);
    assert_eq!(evs[1].node_id, "b");
    assert_eq!(evs[1].ts, 2000);

    // since=1000 skips the first (ts <= since is inclusive-skip).
    let evs = replay_session_events(&store, "alpha", 1000);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].node_id, "b");

    // beta only.
    let evs = replay_session_events(&store, "beta", 0);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].node_id, "c");

    // unknown session -> empty.
    assert!(replay_session_events(&store, "gamma", 0).is_empty());
}

#[test]
fn should_emit_live_filters_session_and_since() {
    use crate::teams::trace_sink::TraceEvent;

    let mk = |ts: i64, sid: &str| TraceEvent {
        ts,
        session_id: sid.into(),
        node_id: "n".into(),
        parent_id: None,
        label: "l".into(),
        status: "Running".into(),
        round: None,
        current_tool: None,
        current_params: None,
        elapsed_ms: 0,
        progress_delta: None,
        token_budget_k: None,
        cumulative_tokens: 0,
        error: None,
        result: None,
        kind: crate::teams::trace_sink::TraceEventKind::Progress,
        permission: None,
        question: None,
    };

    // session filter keeps matching, drops non-matching.
    assert!(should_emit_live(&mk(100, "alpha"), Some("alpha"), 0));
    assert!(!should_emit_live(&mk(100, "beta"), Some("alpha"), 0));
    // no session filter (global) keeps all sessions.
    assert!(should_emit_live(&mk(100, "alpha"), None, 0));
    // since: ts > since passes; ts <= since dropped.
    assert!(should_emit_live(&mk(101, "alpha"), Some("alpha"), 100));
    assert!(!should_emit_live(&mk(100, "alpha"), Some("alpha"), 100));
}

#[tokio::test]
async fn sse_trace_stream_requires_bearer_auth() {
    use crate::config::Settings;
    use crate::state::AppState;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.storage.transcript.db_path = temp
        .path()
        .join("sse-auth.db")
        .to_string_lossy()
        .into_owned();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let token = "sse-auth-token".to_string();
    let (health, protected) = crate::daemon::routes::create_routers(state, token);
    let app = health.merge(protected);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    // No bearer -> 401 (middleware short-circuits before the handler).
    let resp = client
        .get(format!("http://{addr}/api/v1/subagents/trace/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // Wrong bearer -> 401.
    let resp = client
        .get(format!("http://{addr}/api/v1/subagents/trace/stream"))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sse_trace_stream_cold_start_replays_persisted_session() {
    use crate::config::Settings;
    use crate::state::AppState;
    use crate::transcript::{SubagentTranscript, SubagentTranscriptStore, TranscriptStatus};
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let db_path = temp.path().join("sse-replay.db");
    settings.storage.transcript.db_path = db_path.to_string_lossy().into_owned();

    // Seed a persisted transcript for session "alpha-cs" before starting the
    // daemon so the SSE cold-start path has history to replay.
    {
        let store = SubagentTranscriptStore::open(&db_path).unwrap();
        let t = SubagentTranscript {
            id: "node-cs-1".into(),
            session_id: "alpha-cs".into(),
            parent_id: None,
            label: "cold-start-seed".into(),
            status: TranscriptStatus::Completed,
            system_prompt: None,
            user_prompt: "u".into(),
            started_at: 5_000,
            finished_at: Some(6_000),
            total_tokens: 42,
            max_rounds: None,
            actual_rounds: 2,
            token_budget_k: None,
            error_message: None,
            summary: None,
            failure_diagnostics: None,
            project_path: None,
            node_type: None,
            events: vec![],
        };
        store.save(&t, None).unwrap();
    }

    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
    let token = "sse-replay-token".to_string();
    let (health, protected) = crate::daemon::routes::create_routers(state, token);
    let app = health.merge(protected);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/api/v1/subagents/trace/stream?session_id=alpha-cs"
        ))
        .bearer_auth("sse-replay-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read the SSE body stream; cold-start replay emits the seeded header
    // immediately. Collect with a deadline (the live loop never ends).
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(Ok(chunk))) =
            tokio::time::timeout(Duration::from_millis(250), stream.next()).await
        {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("\"node_id\":\"node-cs-1\"") {
                break;
            }
        }
    }
    assert!(
        buf.contains("\"node_id\":\"node-cs-1\""),
        "cold-start replay event missing; got: {buf}"
    );
    assert!(
        buf.contains("\"session_id\":\"alpha-cs\""),
        "session-scoped replay missing; got: {buf}"
    );
}

#[tokio::test]
async fn memory_list_filters_by_scope_and_importance() {
    use crate::config::Settings;
    use crate::context::{MemoryEntry, MemoryOrigin, MemoryType};
    use crate::daemon::models::MemoryListQuery;
    use crate::state::AppState;
    use axum::extract::{Query, State};
    use axum::Json;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let mut state = DaemonState::new(AppState::new(settings)).await;
    // Isolate the global memory pool: the default manager writes global
    // memories to the real `~/.wgenty-code/memory/` regardless of
    // working_dir, so swap in a test manager backed by the tempdir.
    state.memory_manager = Arc::new(crate::context::MemoryManager::new_for_test(
        temp.path().to_path_buf(),
        temp.path().join("global-memory"),
    ));
    let state = Arc::new(state);

    // Seed: one high-importance project memory, one low project, one global.
    let mut hi = MemoryEntry::new(MemoryType::Knowledge, "important project fact");
    hi.importance = 0.9;
    let mut lo = MemoryEntry::new(MemoryType::Session, "trivial project note");
    lo.importance = 0.1;
    let mut gl = MemoryEntry::new(MemoryType::Preference, "global pref");
    gl.importance = 0.8;
    state
        .memory_manager
        .add_memory(hi, MemoryOrigin::Project)
        .await
        .unwrap();
    state
        .memory_manager
        .add_memory(lo, MemoryOrigin::Project)
        .await
        .unwrap();
    state
        .memory_manager
        .add_memory(gl, MemoryOrigin::Global)
        .await
        .unwrap();

    // No filter → all three.
    let Json(all) = list_memory(State(state.clone()), Query(MemoryListQuery::default()))
        .await
        .expect("list all");
    assert_eq!(all.total, 3, "default list should return all 3 memories");

    // Scope=project → only the two project entries.
    let Json(proj) = list_memory(
        State(state.clone()),
        Query(MemoryListQuery {
            scope: Some("project".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("list project");
    assert_eq!(proj.total, 2, "project scope should return 2");
    assert!(proj.items.iter().all(|m| m.origin == "project"));

    // Scope=global → one global entry.
    let Json(glob) = list_memory(
        State(state.clone()),
        Query(MemoryListQuery {
            scope: Some("global".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("list global");
    assert_eq!(glob.total, 1, "global scope should return 1");
    assert_eq!(glob.items[0].origin, "global");

    // min_importance=0.5 → only the two >= 0.5 entries (hi + global).
    let Json(imp) = list_memory(
        State(state.clone()),
        Query(MemoryListQuery {
            min_importance: Some(0.5),
            ..Default::default()
        }),
    )
    .await
    .expect("list by importance");
    assert_eq!(imp.total, 2, "importance>=0.5 should return 2");
    assert!(imp.items.iter().all(|m| m.entry.importance >= 0.5));
}

#[tokio::test]
async fn memory_prune_dry_run_is_noop() {
    use crate::config::Settings;
    use crate::context::{MemoryEntry, MemoryOrigin, MemoryType};
    use crate::daemon::models::PruneRequest;
    use crate::state::AppState;
    use axum::extract::State;
    use axum::Json;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let state = Arc::new(DaemonState::new(AppState::new(settings)).await);

    let entry = MemoryEntry::new(MemoryType::Knowledge, "kept");
    state
        .memory_manager
        .add_memory(entry, MemoryOrigin::Project)
        .await
        .unwrap();

    // Dry-run must not remove anything: before == after.
    let Json(result) = prune_memory(
        State(state.clone()),
        Query(MemoryProjectQuery { project: None }),
        Some(Json(PruneRequest { dry_run: true })),
    )
    .await
    .expect("dry-run prune");
    assert_eq!(result.removed, 0, "dry-run must remove nothing");
    assert_eq!(result.before, result.after);

    // The memory is still there.
    let Json(still) = crate::daemon::handlers::memory_status(
        State(state.clone()),
        Query(MemoryProjectQuery { project: None }),
    )
    .await
    .expect("status");
    assert!(
        still.total_memories >= 1,
        "memory must survive a dry-run prune"
    );
}

// ── Global event producers (daemon-session-orchestration Task 7) ─────────

/// Tempdir-backed DaemonState for global-event tests. Construction-time
/// I/O finishes inside `DaemonState::new`, so the tempdir may drop once
/// the helper returns (mirrors global_events.rs::test_daemon_state).
async fn global_event_test_state() -> DaemonState {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut settings = crate::config::Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    DaemonState::new(crate::state::AppState::new(settings)).await
}

#[tokio::test]
async fn apply_todos_update_broadcasts_full_snapshot() {
    let state = global_event_test_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state
        .apply_todos_update(
            "test-session",
            vec![crate::tasks::TodoItem {
                content: "write plan".into(),
                status: "in_progress".into(),
                active_form: String::new(),
                subagent: None,
            }],
        )
        .await;
    let ev = rx.recv().await.expect("todos event");
    assert_eq!(
        ev.kind,
        crate::daemon::global_events::GlobalEventKind::TodosChanged
    );
    assert_eq!(ev.data["items"][0]["content"], "write plan");
    // Multi-project dimension: clients filter by `project`.
    assert!(ev.data["project"].is_string(), "data: {}", ev.data);
    assert_eq!(ev.data["has_open_items"], true);
    // 快照与 GET /todos 读取同源（route through the same session path）。
    let todo_state = state.todo_state_for_session("test-session").await;
    let todos = todo_state.read().await;
    assert_eq!(todos.items.len(), 1);
}

#[tokio::test]
async fn set_permission_mode_broadcasts_mode_changed() {
    use axum::extract::State;
    use axum::Json;

    let state = Arc::new(global_event_test_state().await);
    let mut rx = state.global_event_hub.subscribe();
    let Json(resp) = set_permission_mode(
        State(state.clone()),
        Json(crate::daemon::models::SetPermissionModeRequest {
            mode: crate::config::agent::RootPermissionMode::Yolo,
            effective_mode: None,
            session_id: Some("s1".to_string()),
        }),
    )
    .await
    .expect("set_permission_mode with session_id");
    assert_eq!(resp["success"], true);

    let ev = rx.recv().await.expect("mode event");
    assert_eq!(
        ev.kind,
        crate::daemon::global_events::GlobalEventKind::ModeChanged
    );
    assert_eq!(ev.data["session_id"], "s1");
    assert_eq!(ev.data["mode"], "yolo");
    // Derived from the root mode when effective_mode is omitted.
    assert_eq!(ev.data["effective_mode"], "yolo");
}

/// `switch_model` persists to `~/.wgenty-code/settings.json`, so the test
/// scopes a fake `$HOME` (serial, mirroring tui token-budget tests) to
/// never touch the developer's real config.
#[tokio::test]
#[serial_test::serial]
async fn switch_model_broadcasts_model_changed() {
    use axum::extract::State;
    use axum::Json;

    let temp = tempfile::tempdir().unwrap();
    let fake_home = tempfile::tempdir().unwrap();
    let mut settings = crate::config::Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    settings.models.profiles.insert(
        "p2".to_string(),
        crate::config::models::ModelEndpoint {
            name: "gpt-test".to_string(),
            provider: Some("openai".to_string()),
            ..Default::default()
        },
    );
    // Pre-seed the on-disk settings (same profile) so the handler's
    // load-from-disk + switch + save path succeeds under the fake home.
    let cfg_dir = fake_home.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();

    // Point the config path at the fake home via WGENTY_HOME. This is the
    // only reliable cross-platform override: dirs::home_dir() reads `HOME`
    // on Unix but prefers the Known Folder API (COM) over USERPROFILE on
    // Windows, so env injection through HOME/USERPROFILE is not hermetic.
    let prev_wgenty_home = std::env::var_os("WGENTY_HOME");
    std::env::set_var("WGENTY_HOME", fake_home.path());
    let run = async {
        let state = Arc::new(DaemonState::new(crate::state::AppState::new(settings)).await);
        let mut rx = state.global_event_hub.subscribe();
        let resp = switch_model(
            State(state.clone()),
            Json(SwitchModelRequest {
                profile: "p2".to_string(),
            }),
        )
        .await
        .expect("switch_model should succeed");
        assert!(resp.success);

        let ev = rx.recv().await.expect("model event");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::ModelChanged
        );
        assert_eq!(ev.data["profile"], "p2");
        assert_eq!(ev.data["model_name"], "gpt-test");
        assert_eq!(ev.data["provider"], "openai");
    }
    .await;
    match prev_wgenty_home {
        Some(v) => std::env::set_var("WGENTY_HOME", v),
        None => std::env::remove_var("WGENTY_HOME"),
    }
    run
}

#[tokio::test]
async fn claim_task_group_broadcasts_task_group_result() {
    use axum::extract::State;
    use axum::Json;

    let state = Arc::new(global_event_test_state().await);
    // Produce a ready root group through the coordinator (same path as
    // coordinator.rs tests): reserve a child in the group, finish it.
    let root = state.root_context("s").await.expect("root context");
    let group = state
        .coordinator
        .create_root_task_group(
            &root,
            "turn-1",
            tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        )
        .await
        .expect("root group");
    let child = state
        .coordinator
        .reserve_child_in_group(
            &root,
            crate::agent::SpawnChildRequest::new("work"),
            group.clone(),
        )
        .await
        .expect("reserve child");
    state
        .coordinator
        .finish_child(
            &child.context,
            crate::agent::ChildTerminal::completed("done"),
        )
        .await
        .expect("finish child");

    let mut rx = state.global_event_hub.subscribe();
    let Json(resp) = claim_task_group(
        State(state.clone()),
        Json(ClaimTaskGroupRequest {
            session_id: "s".to_string(),
            generation: 0,
        }),
    )
    .await
    .expect("claim should deliver the ready group");
    assert_eq!(resp.results.len(), 1);

    let ev = rx.recv().await.expect("task-group event");
    assert_eq!(
        ev.kind,
        crate::daemon::global_events::GlobalEventKind::TaskGroupResult
    );
    assert_eq!(ev.data["task_group_id"], group.as_str());
    assert_eq!(ev.data["session_id"], "s");
    assert_eq!(ev.data["generation"], 0);
    assert_eq!(ev.data["result_count"], 1);
    // Multi-project dimension: clients filter by `project`.
    assert!(ev.data["project"].is_string(), "data: {}", ev.data);
}

/// The global events SSE endpoint lives on the protected router: no (or
/// wrong) bearer token must short-circuit with 401 before the handler.
#[tokio::test]
async fn global_events_stream_requires_bearer_auth() {
    let state = Arc::new(global_event_test_state().await);
    let (health, protected) =
        crate::daemon::routes::create_routers(state, "events-auth-token".to_string());
    let app = health.merge(protected);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    // No bearer -> 401 (middleware short-circuits before the handler).
    let resp = client
        .get(format!("http://{addr}/api/v1/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // Wrong bearer -> 401.
    let resp = client
        .get(format!("http://{addr}/api/v1/events"))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Background results retention (daemon-session-orchestration Task 8) ────

fn sample_bg_result(
    task_id: &str,
    session_id: Option<&str>,
) -> crate::tools::execution::background::BackgroundResult {
    crate::tools::execution::background::BackgroundResult {
        task_id: task_id.to_string(),
        session_id: session_id.map(str::to_string),
        result_type: "command".to_string(),
        command: "true".to_string(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
        sandbox_bypassed: false,
        permission_mode: None,
        sandbox_level: None,
    }
}

#[tokio::test]
async fn background_results_are_retained_per_session() {
    let state = global_event_test_state().await;
    state
        .record_background_result(sample_bg_result("a-1", Some("session-a")))
        .await;
    state
        .record_background_result(sample_bg_result("a-2", Some("session-a")))
        .await;

    let snapshot = state
        .background_results_snapshot_for_session("session-a")
        .await;
    let task_ids: Vec<&str> = snapshot
        .iter()
        .map(|result| result.task_id.as_str())
        .collect();
    assert_eq!(task_ids, vec!["a-1", "a-2"]);
}

#[tokio::test]
async fn background_results_deduplicate_task_ids_before_publication() {
    let state = global_event_test_state().await;
    let mut rx = state.global_event_hub.subscribe();
    let first = sample_bg_result("same-task", Some("session-a"));
    let mut duplicate = first.clone();
    duplicate.command = "must not replace first result".to_string();

    state.record_background_result(first).await;
    rx.recv().await.expect("first result broadcast");
    state.record_background_result(duplicate).await;

    let snapshot = state
        .background_results_snapshot_for_session("session-a")
        .await;
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].command, "true");
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn background_results_are_isolated_from_foreign_and_unowned_sessions() {
    let state = global_event_test_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state
        .record_background_result(sample_bg_result("shared-task", Some("session-a")))
        .await;
    rx.recv().await.expect("owned result broadcast");
    let mut foreign = sample_bg_result("shared-task", Some("session-b"));
    foreign.command = "foreign command".to_string();
    state.record_background_result(foreign).await;
    rx.recv().await.expect("foreign owned result broadcast");
    state
        .record_background_result(sample_bg_result("legacy", None))
        .await;

    let session_a = state
        .background_results_snapshot_for_session("session-a")
        .await;
    assert_eq!(session_a.len(), 1);
    assert_eq!(session_a[0].task_id, "shared-task");
    assert_eq!(session_a[0].command, "true");
    let session_b = state
        .background_results_snapshot_for_session("session-b")
        .await;
    assert_eq!(session_b.len(), 1);
    assert_eq!(session_b[0].task_id, "shared-task");
    assert_eq!(session_b[0].command, "foreign command");
    assert!(state
        .background_results_snapshot()
        .await
        .iter()
        .all(|result| result.task_id != "legacy"));
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert!(state
        .background_results_snapshot_for_session("missing-session")
        .await
        .is_empty());
}

#[tokio::test]
async fn background_results_are_retained_not_drained() {
    let state = global_event_test_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state
        .record_background_result(sample_bg_result("r1", Some("session-a")))
        .await;
    state
        .record_background_result(sample_bg_result("r2", Some("session-a")))
        .await;
    // 广播在入队之后到达。
    let ev = rx.recv().await.expect("broadcast");
    assert_eq!(
        ev.kind,
        crate::daemon::global_events::GlobalEventKind::BackgroundResult
    );
    assert_eq!(ev.data["result"]["session_id"], "session-a");
    // 两次快照读取内容一致（不再先到先得）。
    let first = state.background_results_snapshot().await;
    let second = state.background_results_snapshot().await;
    assert_eq!(first.len(), 2);
    assert_eq!(first.len(), second.len());
}

#[tokio::test]
async fn background_results_evict_oldest_beyond_capacity() {
    let state = global_event_test_state().await;
    let capacity = crate::daemon::state::BACKGROUND_RESULTS_CAPACITY;
    // capacity + 1 results: the oldest ("r0") must be evicted.
    for i in 0..=capacity {
        state
            .record_background_result(sample_bg_result(&format!("r{i}"), Some("session-a")))
            .await;
    }
    let snapshot = state
        .background_results_snapshot_for_session("session-a")
        .await;
    assert_eq!(snapshot.len(), capacity);
    assert_eq!(snapshot[0].task_id, "r1");
    assert_eq!(
        snapshot.last().expect("non-empty snapshot").task_id,
        format!("r{capacity}")
    );
}

#[tokio::test]
async fn get_background_results_returns_snapshot_without_draining() {
    use axum::extract::State;
    use axum::Json;

    let state = Arc::new(global_event_test_state().await);
    state
        .record_background_result(sample_bg_result("r1", Some("session-a")))
        .await;
    state
        .record_background_result(sample_bg_result("r2", Some("session-a")))
        .await;
    // Two consecutive reads see the same results (no first-come-first-served
    // drain): old polling clients keep working, results are not stolen.
    let Json(first) = get_background_results(State(state.clone())).await;
    let Json(second) = get_background_results(State(state.clone())).await;
    assert_eq!(first["results"].as_array().expect("results array").len(), 2);
    assert_eq!(first, second);
}

/// End-to-end wiring: results completed in the tool-layer background
/// manager flow into the daemon's retained queue (hook replaces the
/// drain queue daemon-side) and are broadcast on the global event bus.
#[tokio::test]
async fn background_manager_results_flow_into_retained_queue() {
    let state = global_event_test_state().await;
    let mut rx = state.global_event_hub.subscribe();
    state
        .background_manager
        .spawn_for_session(
            "true",
            300,
            crate::sandbox::EffectiveMode::Yolo,
            None,
            Some("session-a".to_string()),
        )
        .await
        .expect("background command starts");
    // Retain-before-broadcast: once the event arrives the result is
    // guaranteed queryable via the snapshot.
    let ev = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("event within timeout")
        .expect("broadcast");
    assert_eq!(
        ev.kind,
        crate::daemon::global_events::GlobalEventKind::BackgroundResult
    );
    let snapshot = state
        .background_results_snapshot_for_session("session-a")
        .await;
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].result_type, "command");
    // The tool-layer drain queue stays empty: the retained queue is the
    // single source of truth daemon-side.
    assert!(state.background_manager.drain_results().await.is_empty());
}

// ── session_id required on server-side paths (daemon-session-orchestration Task 12) ──

/// Viewer-token headers for the agents/* handlers (they resolve the
/// viewer before touching session_id).
async fn viewer_headers(state: &DaemonState) -> axum::http::HeaderMap {
    let token = state.create_viewer().await;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        VIEWER_TOKEN_HEADER,
        axum::http::HeaderValue::from_str(&token).expect("token is header-safe"),
    );
    headers
}

#[tokio::test]
async fn set_permission_mode_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let result = set_permission_mode(
        State(state),
        Json(crate::daemon::models::SetPermissionModeRequest {
            mode: crate::config::agent::RootPermissionMode::Normal,
            effective_mode: None,
            session_id: None,
        }),
    )
    .await;
    let Err((status, Json(body))) = result else {
        panic!("missing session_id must be rejected with 400");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, serde_json::json!({"error": "session_id required"}));
}

#[tokio::test]
async fn get_permission_mode_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let result = get_permission_mode(
        State(state),
        Query(crate::daemon::models::PermissionModeQuery { session_id: None }),
    )
    .await;
    let Err((status, Json(body))) = result else {
        panic!("missing session_id must be rejected with 400");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, serde_json::json!({"error": "session_id required"}));
}

#[tokio::test]
async fn get_agent_self_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let headers = viewer_headers(&state).await;
    let result = get_agent_self(State(state), Query(HashMap::new()), headers).await;
    assert!(
        matches!(result, Err(StatusCode::BAD_REQUEST)),
        "missing session_id must be rejected with 400"
    );
}

#[tokio::test]
async fn navigate_agent_view_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let headers = viewer_headers(&state).await;
    let result = navigate_agent_view(
        State(state),
        Path("cap".to_string()),
        Query(HashMap::new()),
        headers,
    )
    .await;
    assert!(
        matches!(result, Err(StatusCode::BAD_REQUEST)),
        "missing session_id must be rejected with 400"
    );
}

#[tokio::test]
async fn get_child_transcript_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let headers = viewer_headers(&state).await;
    let result = get_child_transcript(
        State(state),
        Path("cap".to_string()),
        Query(HashMap::new()),
        headers,
    )
    .await;
    assert!(
        matches!(result, Err(StatusCode::BAD_REQUEST)),
        "missing session_id must be rejected with 400"
    );
}

#[tokio::test]
async fn cancel_child_without_session_id_is_rejected() {
    let state = Arc::new(global_event_test_state().await);
    let headers = viewer_headers(&state).await;
    let result = cancel_child(
        State(state),
        Path("cap".to_string()),
        Query(HashMap::new()),
        headers,
    )
    .await;
    assert!(
        matches!(result, Err(StatusCode::BAD_REQUEST)),
        "missing session_id must be rejected with 400"
    );
}
