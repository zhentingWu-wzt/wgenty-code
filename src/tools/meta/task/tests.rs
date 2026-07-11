use super::*;

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
        .and_then(|v| if v == 0 { None } else { Some(v) })
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
        .and_then(|v| if v == 0 { None } else { Some(v) })
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
        .and_then(|v| if v == 0 { None } else { Some(v) })
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
        .and_then(|v| if v == 0 { None } else { Some(v) })
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
        .and_then(|v| if v == 0 { None } else { Some(v) })
        .or(if default_k > 0 { Some(default_k) } else { None });
    assert_eq!(
        result,
        Some(20),
        "explicit token_budget=0 with non-zero default should use the default"
    );
}

// ── Coordinator-owned children: forge-field and depth tests ─────────────

use crate::agent::{AgentCoordinator, SessionId, ToolContext, ToolInvocationId};

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
    let err = result.expect_err("expected depth-limit error");
    assert_eq!(err.code.as_deref(), Some("depth_limit_reached"));
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
