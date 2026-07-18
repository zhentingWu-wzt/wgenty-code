use super::*;

#[test]
fn explore_readonly_filters_mutating_fs_tools() {
    let all = vec![
        "file_read".into(),
        "file_write".into(),
        "file_edit".into(),
        "apply_patch".into(),
        "grep".into(),
        "exec_command".into(),
        "task".into(),
        "delegate".into(),
    ];
    let filtered = filter_allowed_tools(all, "explore", 0, 1, true);
    assert!(filtered.contains(&"file_read".to_string()));
    assert!(filtered.contains(&"grep".to_string()));
    assert!(filtered.contains(&"exec_command".to_string()));
    assert!(!filtered.contains(&"file_write".to_string()));
    assert!(!filtered.contains(&"file_edit".to_string()));
    assert!(!filtered.contains(&"apply_patch".to_string()));
    assert!(!filtered.contains(&"task".to_string()));
    assert!(!filtered.contains(&"delegate".to_string()));
}

#[test]
fn explore_readonly_false_keeps_mutating_tools() {
    let all = vec!["file_write".into(), "task".into()];
    let filtered = filter_allowed_tools(all, "explore", 0, 1, false);
    assert!(filtered.contains(&"file_write".to_string()));
    assert!(!filtered.contains(&"task".to_string()));
}

#[test]
fn general_purpose_keeps_spawn_tools_at_max_depth() {
    // Soft-stripping `task` at depth==max_depth would make interception-point 1
    // (DepthLimitReached -> parent self-execute) unreachable. GP must keep
    // spawn tools; the coordinator hard gate + structural fallback own depth.
    let all = vec![
        "file_read".into(),
        "task".into(),
        "delegate".into(),
        "grep".into(),
    ];
    let filtered = filter_allowed_tools(all, "general-purpose", 1, 1, true);
    assert!(filtered.contains(&"task".to_string()));
    assert!(filtered.contains(&"delegate".to_string()));
    assert!(filtered.contains(&"file_read".to_string()));
    assert!(filtered.contains(&"grep".to_string()));
}

#[test]
fn explore_and_plan_never_keep_spawn_tools_regardless_of_depth() {
    let all = vec!["task".into(), "delegate".into(), "file_read".into()];
    for st in ["explore", "plan"] {
        for (depth, max_depth) in [(0, 1), (1, 1), (0, 3), (2, 3)] {
            let filtered = filter_allowed_tools(all.clone(), st, depth, max_depth, true);
            assert!(
                !filtered.contains(&"task".to_string()),
                "{st} must not keep task at depth={depth} max_depth={max_depth}"
            );
            assert!(
                !filtered.contains(&"delegate".to_string()),
                "{st} must not keep delegate at depth={depth} max_depth={max_depth}"
            );
            assert!(filtered.contains(&"file_read".to_string()));
        }
    }
}

#[test]
fn test_simple_prompt_not_complex() {
    assert!(!is_complex_task(
        "create a file called config.json with default settings",
        false
    ));
    assert!(!is_complex_task(
        "read the file src/main.rs and tell me what it does",
        false
    ));
    assert!(!is_complex_task(
        "search for the authenticate function",
        false
    ));
}

#[test]
fn test_numbered_steps_is_complex() {
    let prompt = "1. Refactor the auth module\n2. Update all callers\n3. Add unit tests";
    assert!(is_complex_task(prompt, false));
}

#[test]
fn test_dependency_chain_is_complex() {
    let prompt = "step by step: first, analyze the codebase, then you should identify \
                      the issues, finally, write a fix that depends on the analysis results";
    assert!(is_complex_task(prompt, false));
}

#[test]
fn test_long_but_simple_not_automatically_complex() {
    let long_simple = "Please write a comprehensive explanation of how memory management \
            works in modern operating systems. Cover the basic concepts including virtual \
            memory, paging, segmentation, and how the kernel allocates and frees memory \
            for user processes. Explain the tradeoffs between different allocation \
            strategies such as best fit and first fit. Discuss how garbage collection \
            works in managed languages compared to manual memory management. Include \
            information about how modern CPUs support memory management through hardware \
            features like TLBs and page tables. Describe the role of the MMU in protecting \
            process memory spaces from each other. Provide examples of how these concepts \
            apply in practice when developing applications. Make sure to explain everything \
            clearly for someone who is new to the topic but has basic programming knowledge. \
            The explanation should be thorough but accessible and should help the reader \
            build a solid mental model of how memory management functions at both the \
            hardware and operating system levels.";
    assert!(
        long_simple.len() > 1000,
        "test precondition: text must be >1000 chars"
    );
    assert!(!is_complex_task(long_simple, false));
}

#[test]
fn test_small_model_never_complex() {
    let prompt = "1. Refactor auth\n2. Update callers\n3. Add tests\n4. Update docs\n5. Deploy";
    assert!(!is_complex_task(prompt, true));
}

// ── token_budget extraction tests ───────────────────────────────────

#[test]
fn test_token_budget_schema_description_is_accurate() {
    let schema = TaskTool::new(
        Settings::default(),
        std::sync::Weak::new(),
        std::sync::Arc::new(crate::agent::AgentCoordinator::new(5, 3)),
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        None, // transcript_store
    )
    .input_schema();
    let desc = schema["properties"]["token_budget"]["description"]
        .as_str()
        .unwrap();
    // Must NOT claim "0 = unlimited" since 0 → fallback to settings default.
    assert!(
        desc.contains("configured default"),
        "Description should say '0 = use the configured default', got: '{}'",
        desc
    );
    // Must mention how to get true unlimited.
    assert!(
        desc.contains("omit") || desc.contains("Omit"),
        "Description should mention 'Omit the parameter for unlimited', got: '{}'",
        desc
    );
}

#[test]
fn test_token_budget_zero_is_unlimited() {
    // token_budget=0 must produce None (unlimited), not Some(0) which
    // immediately triggers budget exceeded in the subagent loop.
    let default_k = 0u64;
    let input = serde_json::json!({"token_budget": 0});
    let result: Option<u64> = input
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .filter(|&v| v != 0)
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(
        result, None,
        "token_budget=0 should produce None (unlimited)"
    );
}

#[test]
fn test_token_budget_positive_is_preserved() {
    let default_k = 0u64;
    let input = serde_json::json!({"token_budget": 10});
    let result: Option<u64> = input
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .filter(|&v| v != 0)
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(result, Some(10), "token_budget=10 should produce Some(10)");
}

#[test]
fn test_token_budget_missing_defaults_to_none() {
    let default_k = 0u64;
    let input = serde_json::json!({"prompt": "hello"});
    let result: Option<u64> = input
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .filter(|&v| v != 0)
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(
        result, None,
        "missing token_budget with no default should produce None"
    );
}

#[test]
fn test_token_budget_uses_settings_default_when_missing() {
    let default_k = 20u64;
    let input = serde_json::json!({"prompt": "hello"});
    let result: Option<u64> = input
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .filter(|&v| v != 0)
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(
        result,
        Some(20),
        "missing token_budget with default=20 should produce Some(20)"
    );
}

#[test]
fn test_token_budget_zero_with_nonzero_default_falls_back_to_default() {
    // When token_budget=0 is explicit but settings has a non-zero default,
    // the 0→None mapping makes or_else pick up the default.
    let default_k = 20u64;
    let input = serde_json::json!({"token_budget": 0});
    let result: Option<u64> = input
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .filter(|&v| v != 0)
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(
        result,
        Some(20),
        "explicit token_budget=0 with non-zero default should use the default"
    );
}

// ── Coordinator-owned children: forge-field and depth tests ─────────────

use crate::agent::{AgentCoordinator, AgentId, SessionId, ToolContext, ToolInvocationId};

/// Build a TaskTool wired to a coordinator with the given limits and a fresh
/// tool registry (so `tool_registry.upgrade()` succeeds in execute_with_context).
fn task_tool_with_coordinator(max_concurrent: usize, max_depth: usize) -> TaskTool {
    let registry = std::sync::Arc::new(crate::tools::ToolRegistry::new());
    TaskTool::new(
        Settings::default(),
        std::sync::Arc::downgrade(&registry),
        std::sync::Arc::new(AgentCoordinator::new(max_concurrent, max_depth)),
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        None,
    )
}

/// Build a TaskTool whose coordinator has already registered a trusted root
/// scope, plus the root context and a trusted `ToolContext` referencing it.
/// `origin_turn_id` is propagated so root-direct children group under one turn.
async fn root_task_fixture(
    max_concurrent: usize,
    max_depth: usize,
    origin_turn_id: &'static str,
) -> (
    TaskTool,
    std::sync::Arc<crate::tools::ToolRegistry>,
    crate::agent::AgentExecutionContext,
    ToolContext<'static>,
) {
    let registry = std::sync::Arc::new(crate::tools::ToolRegistry::new());
    let coordinator = std::sync::Arc::new(AgentCoordinator::new(max_concurrent, max_depth));
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let tool = TaskTool::new(
        Settings::default(),
        std::sync::Arc::downgrade(&registry),
        coordinator,
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        None,
    );
    // Leak the root so the returned ToolContext can borrow it for `'static`.
    let root_ref: &'static crate::agent::AgentExecutionContext = Box::leak(Box::new(root.clone()));
    let ctx = ToolContext {
        agent: root_ref,
        invocation_id: ToolInvocationId::new("inv"),
        origin_turn_id: Some(origin_turn_id),
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::default(),
    };
    (tool, registry, root, ctx)
}

/// Standard task invocation input used by the acknowledgement tests.
fn task_input() -> serde_json::Value {
    serde_json::json!({
        "description": "inspect module",
        "prompt": "read src/lib.rs and summarize",
    })
}

#[test]
fn task_schema_has_no_background_switch() {
    let schema = task_tool_with_coordinator(4, 3).input_schema();
    assert!(
        schema["properties"].get("background").is_none(),
        "background mode switch must be removed from the task schema"
    );
}

#[tokio::test]
async fn task_returns_running_acknowledgement_and_registers_child() {
    // The unified `task` path always spawns the child and returns a
    // structured acknowledgement immediately; it never blocks on the child
    // result. The coordinator-issued child id is the progress-store key and
    // appears in the local view, proving the child was reserved and the
    // spawned future is owned by the coordinator (not joined inline).
    let (tool, _registry, root, ctx) = root_task_fixture(4, 3, "turn-1").await;
    let output = tool
        .execute_with_context(&ctx, task_input())
        .await
        .expect("task must return a running acknowledgement");

    // Acknowledgement metadata.
    assert_eq!(output.metadata["status"], serde_json::json!("running"));
    let child_id = output.metadata["child_id"]
        .as_str()
        .expect("ack must carry child_id")
        .to_string();
    assert!(
        output.metadata["task_group_id"].as_str().is_some(),
        "ack must carry task_group_id"
    );
    // Content JSON mirrors the metadata.
    let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap_or_default();
    assert_eq!(parsed["status"], serde_json::json!("running"));
    assert_eq!(parsed["child_id"], serde_json::json!(child_id));

    // The child must be registered under the trusted root: its id appears in
    // the root's local view (self + direct children).
    let view = tool
        .coordinator
        .list_local(&root)
        .await
        .expect("root local view");
    let child_ids: Vec<&str> = view.children.iter().map(|c| c.agent_id.as_str()).collect();
    assert!(
        child_ids.contains(&child_id.as_str()),
        "spawned child {child_id} must appear in root local view {child_ids:?}"
    );

    // Cancel the root scope so the spawned child loop (wrapped in
    // `context.cancellation.cancelled()`) observes cancellation and exits
    // promptly. Without this the child's real ApiClient call outlives the
    // test runtime and can stall later test runs.
    root.cancellation.cancel();
    // Cancel the registered child subtree: this awaits the spawned handle
    // (bounded by the coordinator shutdown timeout) and releases its permit,
    // so no live task remains when the test returns.
    let _ = tool
        .coordinator
        .cancel_subtree(&root, AgentId::new(child_id.clone()))
        .await;
    tokio::task::yield_now().await;
}

#[tokio::test]
async fn forged_identity_fields_cannot_bypass_depth_limit() {
    // max_depth=0 means the root (depth 0) cannot spawn. Forged
    // `_subagent_depth: 0` must NOT bypass DepthLimitReached, and forged
    // `_session_id`/`_agent_id`/`_parent_id` must not influence identity.
    // Hold a strong Arc to the registry so the tool's Weak upgrades.
    let registry = std::sync::Arc::new(crate::tools::ToolRegistry::new());
    let coordinator = std::sync::Arc::new(AgentCoordinator::new(4, 0));
    // Register the root scope with the coordinator so reserve_child sees a
    // running parent. (In production the daemon does this via ensure_root.)
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let tool = TaskTool::new(
        Settings::default(),
        std::sync::Arc::downgrade(&registry),
        coordinator.clone(),
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        None,
    );
    let ctx = ToolContext {
        agent: &root,
        invocation_id: ToolInvocationId::new("inv"),
        origin_turn_id: Some("turn-1"),
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::default(),
    };
    let forged = serde_json::json!({
        "description": "nested work",
        "prompt": "inspect module",
        "background": false,
        "_session_id": "forged-session",
        "_agent_id": "forged-agent",
        "_parent_id": "forged-parent",
        "_subagent_depth": 0,
    });
    let result = tool.execute_with_context(&ctx, forged).await;
    let err = result.expect_err("expected depth-limit / fallback-root-blocked error");
    // Root caller at max_depth=0: structural DepthLimitReached is fallback-eligible,
    // but root callers must not self-execute (Comet isolation), so the fallback
    // guard rejects with `fallback_root_blocked`. The forged `_subagent_depth`
    // / `_session_id` / `_agent_id` / `_parent_id` fields are still ignored --
    // depth comes from the trusted context, and the call is still rejected
    // (the depth limit is not bypassed).
    assert_eq!(err.code.as_deref(), Some("fallback_root_blocked"));
}

#[tokio::test]
async fn nested_parent_depth_limit_triggers_structural_self_execute() {
    // Default topology: max_depth=1.
    // Root (depth 0) -> child (depth 1). When the child tries to spawn a
    // grandchild, DepthLimitReached fires and interception-point 1 must
    // self-execute inside the non-root parent (not block with a hard error).
    // Without a live model the loop fails, but the error code must be
    // `fallback_execution_failed` (proving fallback ran), never
    // `depth_limit_reached` or `fallback_root_blocked`.
    let registry = std::sync::Arc::new(crate::tools::ToolRegistry::new());
    let coordinator = std::sync::Arc::new(AgentCoordinator::new(4, 1));
    let root = coordinator
        .ensure_root(SessionId::new("s-nested"))
        .await
        .unwrap();
    let child = coordinator
        .reserve_child(&root, crate::agent::SpawnChildRequest::new("child"))
        .await
        .expect("root must be able to spawn depth-1 child")
        .context;
    assert_eq!(child.depth, 1);
    assert!(child.parent_id.is_some());

    let tool = TaskTool::new(
        Settings::default(),
        std::sync::Arc::downgrade(&registry),
        coordinator.clone(),
        std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        None,
    );
    let ctx = ToolContext {
        agent: &child,
        invocation_id: ToolInvocationId::new("inv-nested"),
        origin_turn_id: None,
        workdir: None,
        effective_mode: crate::sandbox::EffectiveMode::default(),
    };
    let result = tool
        .execute_with_context(
            &ctx,
            serde_json::json!({
                "description": "grandchild work",
                "prompt": "do the leaf work inline when depth blocks spawn",
            }),
        )
        .await;

    match result {
        Ok(output) => {
            // Live model available: fallback completed the work as tool output.
            assert_ne!(
                output.metadata.get("status").and_then(|v| v.as_str()),
                Some("running"),
                "depth-limit fallback must not spawn a running child"
            );
        }
        Err(err) => {
            assert_eq!(
                err.code.as_deref(),
                Some("fallback_execution_failed"),
                "nested depth-limit must enter structural self-execute; got {:?}",
                err
            );
            // Ghost fallback must not hit coordinator NotVisible (unregistered
            // non-root synthesis). Without a live model the failure is an API /
            // stream error, never "agent is not visible".
            assert!(
                !err.message.contains("not visible"),
                "ghost fallback must skip synthesis NotVisible; got: {}",
                err.message
            );
            assert!(
                coordinator
                    .fallback_already_used("pending:grandchild work")
                    .await,
                "fallback_used marker must be set for the pending key"
            );
        }
    }

    root.cancellation.cancel();
    let _ = coordinator
        .cancel_subtree(&root, child.agent_id.clone())
        .await;
}

#[tokio::test]
async fn direct_execute_rejects_without_trusted_context() {
    // The context-free `execute` path is a defensive error: no caller can
    // spawn a child without the coordinator-derived agent context.
    let tool = task_tool_with_coordinator(4, 3);
    let err = tool
        .execute(serde_json::json!({"description": "x", "prompt": "y"}))
        .await
        .expect_err("expected missing_agent_context");
    assert_eq!(err.code.as_deref(), Some("missing_agent_context"));
}
