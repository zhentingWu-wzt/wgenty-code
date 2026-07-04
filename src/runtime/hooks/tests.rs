use super::matching::shell_escape;
use super::*;

#[test]
fn test_matches_matcher_empty() {
    assert!(matches_matcher(
        &None,
        &HookEvent::PreToolUse,
        Some("TaskCreate"),
        None
    ));
    assert!(matches_matcher(
        &Some("".into()),
        &HookEvent::PreToolUse,
        Some("TaskCreate"),
        None
    ));
}

#[test]
fn test_matches_matcher_single_tool() {
    let matcher = Some("TaskCreate".to_string());
    assert!(matches_matcher(
        &matcher,
        &HookEvent::PreToolUse,
        Some("TaskCreate"),
        None
    ));
    assert!(!matches_matcher(
        &matcher,
        &HookEvent::PreToolUse,
        Some("TaskUpdate"),
        None
    ));
}

#[test]
fn test_matches_matcher_pipe_separated() {
    let matcher = Some("TaskCreate|TaskUpdate".to_string());
    assert!(matches_matcher(
        &matcher,
        &HookEvent::PreToolUse,
        Some("TaskCreate"),
        None
    ));
    assert!(matches_matcher(
        &matcher,
        &HookEvent::PreToolUse,
        Some("TaskUpdate"),
        None
    ));
    assert!(!matches_matcher(
        &matcher,
        &HookEvent::PreToolUse,
        Some("Read"),
        None
    ));
}

#[test]
fn test_matches_matcher_notification() {
    let matcher = Some("permission_prompt".to_string());
    assert!(matches_matcher(
        &matcher,
        &HookEvent::Notification,
        None,
        Some("permission_prompt")
    ));
    assert!(!matches_matcher(
        &matcher,
        &HookEvent::Notification,
        None,
        Some("other")
    ));
}

#[test]
fn test_expand_hook_variables_tool() {
    let result = expand_hook_variables("echo %tool%", Some("TaskCreate"), None);
    assert!(result.contains("TaskCreate"));
    assert!(!result.contains("%tool%"));
}

#[test]
fn test_expand_hook_variables_input() {
    let result = expand_hook_variables("echo %input%", None, Some(r#"{"key":"value"}"#));
    assert!(result.contains(r#"{"key":"value"}"#));
    assert!(!result.contains("%input%"));
}

#[test]
fn test_shell_escape_single_quotes() {
    let escaped = shell_escape("it's working");
    assert_eq!(escaped, "'it'\\''s working'");
}

#[test]
fn test_deserialize_hookevent_slash_command() {
    // GREEN: SlashCommand variant now exists, deserialization should succeed.
    let result: Result<HookEvent, _> = serde_json::from_str("\"SlashCommand\"");
    assert!(
        result.is_ok(),
        "SlashCommand should deserialize as a HookEvent variant"
    );
    assert_eq!(result.unwrap(), HookEvent::SlashCommand);
}

#[test]
fn test_hook_action_inject_context_serde() {
    // GREEN: HookAction::InjectContext should serialize/deserialize correctly.
    let action = HookAction::InjectContext {
        source: ContextSource::Inline("hello".to_string()),
        priority: 10,
        visibility: LayerVisibility::Internal,
    };
    let json = serde_json::to_string(&action).expect("serialize InjectContext");
    let parsed: HookAction = serde_json::from_str(&json).expect("deserialize InjectContext");
    match parsed {
        HookAction::InjectContext {
            source,
            priority,
            visibility,
        } => {
            match source {
                ContextSource::Inline(s) => assert_eq!(s, "hello"),
                _ => panic!("expected Inline source"),
            }
            assert_eq!(priority, 10);
            match visibility {
                LayerVisibility::Internal => {}
                _ => panic!("expected Internal visibility"),
            }
        }
        _ => panic!("expected InjectContext variant"),
    }
}

#[test]
fn test_hook_definition_new_actions_format() {
    // GREEN: New HookDefinition format with 'actions' should now deserialize.
    let json = serde_json::json!({
        "event": "PreToolUse",
        "matcher": "TaskCreate",
        "when_state": "build",
        "actions": [
            {"Command": {"command": "echo hello", "timeout_secs": 10}}
        ]
    });
    let result: Result<HookDefinition, _> = serde_json::from_value(json);
    assert!(
        result.is_ok(),
        "New actions format should deserialize after refactor"
    );
    let def = result.unwrap();
    assert_eq!(def.event, HookEvent::PreToolUse);
    assert_eq!(def.matcher.as_deref(), Some("TaskCreate"));
    assert_eq!(def.when_state.as_deref(), Some("build"));
    assert_eq!(def.actions.len(), 1);
}

#[test]
fn test_hook_definition_backward_compat_command_format() {
    // GREEN: Old format with 'command' field must continue to work after refactor.
    let json = serde_json::json!({
        "event": "PreToolUse",
        "command": "echo hello",
        "timeout_secs": 30
    });
    let result: Result<HookDefinition, _> = serde_json::from_value(json);
    assert!(
        result.is_ok(),
        "Old command format should deserialize into new HookDefinition"
    );
    let def = result.unwrap();
    assert_eq!(def.event, HookEvent::PreToolUse);
    assert_eq!(def.actions.len(), 1);
    match &def.actions[0] {
        HookAction::Command {
            command,
            timeout_secs,
        } => {
            assert_eq!(command, "echo hello");
            assert_eq!(*timeout_secs, 30);
        }
        _ => panic!("expected Command action"),
    }
}

#[test]
fn test_user_answer_struct() {
    // RED: UserAnswer struct should exist with selected field.
    let answer = UserAnswer {
        selected: vec!["opt1".to_string(), "opt2".to_string()],
    };
    assert_eq!(answer.selected.len(), 2);
    assert_eq!(answer.selected[0], "opt1");
    assert_eq!(answer.selected[1], "opt2");
}

#[test]
fn test_hook_outcome_new_fields() {
    // RED: HookOutcome should have def, continue_execution, reason,
    // injected_content, and user_answer fields.
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: None,
        actions: vec![],
    };
    let outcome = HookOutcome {
        def: def.clone(),
        continue_execution: true,
        reason: Some("test reason".to_string()),
        injected_content: Some("injected text".to_string()),
        user_answer: Some(UserAnswer {
            selected: vec!["a".to_string()],
        }),
        injection_priority: None,
        injection_visibility: None,
    };
    assert!(outcome.continue_execution);
    assert_eq!(outcome.reason.as_deref(), Some("test reason"));
    assert_eq!(outcome.injected_content.as_deref(), Some("injected text"));
    assert!(outcome.user_answer.is_some());
    assert_eq!(outcome.user_answer.as_ref().unwrap().selected, vec!["a"]);
    assert_eq!(outcome.def.event, HookEvent::PreToolUse);
}

#[test]
fn test_hook_context_workflow_state_and_variables() {
    // RED: HookContext should serialize workflow_state and variables fields.
    let ctx = HookContext {
        event: "PreToolUse".to_string(),
        tool_name: Some("test".to_string()),
        tool_input: None,
        tool_result: None,
        session_id: None,
        working_directory: "/tmp".to_string(),
        timestamp: "2024-01-01T00:00:00Z".to_string(),
        comet_phase: None,
        workflow_state: Some("build".to_string()),
        variables: {
            let mut m = HashMap::new();
            m.insert("key".to_string(), "value".to_string());
            m
        },
    };
    let json = serde_json::to_value(&ctx).expect("serialize HookContext");
    assert_eq!(json["workflow_state"], "build");
    assert_eq!(json["variables"]["key"], "value");
}

// ── Step 3: execute_action tests (via fire()) ──────────────────────

#[tokio::test]
async fn test_fire_inject_context_inline() {
    // RED → GREEN: InjectContext with Inline source produces injected_content.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("hello world".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].injected_content.as_deref(), Some("hello world"));
    assert!(outcomes[0].continue_execution);
}

#[tokio::test]
async fn test_fire_inject_context_template() {
    // RED → GREEN: InjectContext with Template source renders variables.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Template("Tool: {tool_name}".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("MyTool", &serde_json::json!({}), None);
    let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].injected_content.as_deref(),
        Some("Tool: MyTool")
    );
}

#[tokio::test]
async fn test_fire_ask_user_placeholder() {
    // RED → GREEN: AskUser returns a UserAnswer with empty selected (placeholder).
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::AskUser {
            question: "Proceed?".to_string(),
            options: vec![UserOption {
                label: "Yes".to_string(),
                value: "yes".to_string(),
                description: None,
            }],
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].user_answer.is_some());
    assert!(outcomes[0]
        .user_answer
        .as_ref()
        .unwrap()
        .selected
        .is_empty());
    assert!(outcomes[0].continue_execution);
}

// ── Step 4: register_workflow_hooks tests ────────────────────────

#[test]
fn test_register_workflow_hooks_adds_hooks() {
    // RED → GREEN: register_workflow_hooks makes hooks visible via has_hooks.
    let mut hm = HookManager::default();
    assert!(!hm.has_hooks(&HookEvent::PreToolUse));
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::Command {
            command: "echo test".to_string(),
            timeout_secs: 30,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    assert!(hm.has_hooks(&HookEvent::PreToolUse));
}

#[test]
fn test_register_workflow_hooks_multiple_events() {
    // RED → GREEN: register_workflow_hooks supports multiple events.
    let mut hm = HookManager::default();
    let hooks = vec![
        HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::Command {
                command: "echo a".to_string(),
                timeout_secs: 30,
            }],
        },
        HookDefinition {
            event: HookEvent::PostToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::Command {
                command: "echo b".to_string(),
                timeout_secs: 30,
            }],
        },
    ];
    hm.register_workflow_hooks(hooks);
    assert!(hm.has_hooks(&HookEvent::PreToolUse));
    assert!(hm.has_hooks(&HookEvent::PostToolUse));
    assert!(!hm.has_hooks(&HookEvent::SessionStart));
}

// ── Step 2: when_state filtering tests ───────────────────────────

#[tokio::test]
async fn test_fire_when_state_filter_matches() {
    // RED → GREEN: fire() with state matches when_state and fires the hook.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: Some("build".to_string()),
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("matched".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm
        .fire(&HookEvent::PreToolUse, &ctx, Some("build"), None)
        .await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].injected_content.as_deref(), Some("matched"));
}

#[tokio::test]
async fn test_fire_when_state_filter_skips() {
    // RED → GREEN: fire() with non-matching state skips the hook.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: Some("build".to_string()),
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("should not fire".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm
        .fire(&HookEvent::PreToolUse, &ctx, Some("design"), None)
        .await;
    assert!(outcomes.is_empty());
}

#[tokio::test]
async fn test_fire_when_state_none_fires_all() {
    // RED → GREEN: fire() with state=None fires hooks regardless of when_state.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: Some("build".to_string()),
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("always fires".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].injected_content.as_deref(),
        Some("always fires")
    );
}

// ── Step 3: Claude Code exit-code compatibility ───────────────

#[tokio::test]
async fn test_run_shell_command_exit_2_blocks() {
    // CC-compat: a hook exiting with code 2 signals "block" (PreToolUse deny).
    // stderr carries the human-readable reason. This is the protocol
    // comet-hook-guard.sh uses; without this mapping the block is swallowed.
    let hm = HookManager::default();
    let ctx = HookManager::pre_tool_context(
        "file_write",
        &serde_json::json!({"path": "/tmp/comet-hook-test"}),
        None,
    );
    let result = hm.run_shell_command("exit 2", 10, &ctx).await;
    assert!(!result.continue_execution, "exit 2 must block");
    assert!(result.reason.is_some(), "block reason must be populated");
}

#[tokio::test]
async fn test_run_shell_command_non_two_exit_allows() {
    // Non-zero, non-2 exits (e.g., 1) are hook errors, not blocks:
    // preserve original "proceed" behavior so a crashing hook can't
    // silently hard-lock the tool call.
    let hm = HookManager::default();
    let ctx = HookManager::pre_tool_context(
        "file_write",
        &serde_json::json!({"path": "/tmp/comet-hook-test"}),
        None,
    );
    let result = hm.run_shell_command("exit 1", 10, &ctx).await;
    assert!(result.continue_execution, "exit 1 must not block");
}

#[tokio::test]
async fn test_fire_when_state_pipe_separated() {
    // RED → GREEN: when_state with pipe-separated values matches any.
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: None,
        when_state: Some("build|design".to_string()),
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("pipe matched".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
    let outcomes = hm
        .fire(&HookEvent::PreToolUse, &ctx, Some("design"), None)
        .await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].injected_content.as_deref(),
        Some("pipe matched")
    );
}

// ── Step 5: with_state builder test ──────────────────────────────

#[test]
fn test_hook_context_with_state_builder() {
    // RED → GREEN: with_state sets workflow_state and comet_phase.
    let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None)
        .with_state(Some("build".to_string()));
    assert_eq!(ctx.workflow_state.as_deref(), Some("build"));
    assert_eq!(ctx.comet_phase.as_deref(), Some("build"));
}

// ── Notification matcher integration test ─────────────────────────

#[tokio::test]
async fn test_fire_notification_matcher() {
    // RED: Notification hook with matcher should work through fire().
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::Notification,
        matcher: Some("test_subtype".to_string()),
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("notification fired".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register_workflow_hooks(vec![def]);
    let ctx = HookManager::notification_context(Some("test message"), None);

    // With matching notification_subtype, hook should fire
    let outcomes = hm
        .fire(&HookEvent::Notification, &ctx, None, Some("test_subtype"))
        .await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].injected_content.as_deref(),
        Some("notification fired")
    );

    // With non-matching notification_subtype, hook should not fire
    let outcomes = hm
        .fire(&HookEvent::Notification, &ctx, None, Some("other_subtype"))
        .await;
    assert!(outcomes.is_empty());
}

#[test]
fn collect_injections_empty_outcomes_returns_empty() {
    assert!(collect_injections(&[]).is_empty());
}

#[test]
fn collect_injections_single_outcome_extracts_fragment() {
    let outcome = HookOutcome {
        def: HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![],
        },
        continue_execution: true,
        reason: None,
        injected_content: Some("hello".into()),
        user_answer: None,
        injection_priority: Some(20),
        injection_visibility: Some(LayerVisibility::Internal),
    };
    let frags = collect_injections(&[outcome]);
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].content, "hello");
    assert_eq!(frags[0].priority, 20);
    assert_eq!(frags[0].source_label, "hook:UserPromptSubmit:0");
    matches!(frags[0].visibility, LayerVisibility::Internal);
}

#[test]
fn collect_injections_sorts_by_priority_stable() {
    let mk = |content: &str, prio: u8| HookOutcome {
        def: HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![],
        },
        continue_execution: true,
        reason: None,
        injected_content: Some(content.into()),
        user_answer: None,
        injection_priority: Some(prio),
        injection_visibility: Some(LayerVisibility::Visible),
    };
    let outcomes = vec![mk("low2", 30), mk("high", 10), mk("low1", 30)];
    let frags = collect_injections(&outcomes);
    assert_eq!(
        frags.iter().map(|f| f.content.as_str()).collect::<Vec<_>>(),
        vec!["high", "low2", "low1"]
    );
}

#[tokio::test]
async fn multiple_inject_hooks_sort_by_priority_after_collect() {
    // End-to-end: register two InjectContext hooks, fire(), then collect_injections().
    // Validates Task 5.1 wiring — priority/visibility flow from action → outcome → fragment.
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("LOW".into()),
                priority: 30,
                visibility: LayerVisibility::Visible,
            }],
        },
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("HIGH".into()),
                priority: 5,
                visibility: LayerVisibility::Visible,
            }],
        },
    ]);

    let ctx = HookContext {
        event: "UserPromptSubmit".into(),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        session_id: None,
        working_directory: String::new(),
        timestamp: String::new(),
        comet_phase: None,
        workflow_state: None,
        variables: Default::default(),
    };

    let outcomes = hm
        .fire(&HookEvent::UserPromptSubmit, &ctx, None, None)
        .await;
    let injections = collect_injections(&outcomes);

    // After collect_injections sorts by priority asc: HIGH (5) before LOW (30).
    assert_eq!(injections.len(), 2);
    assert_eq!(injections[0].content, "HIGH");
    assert_eq!(injections[0].priority, 5);
    assert_eq!(injections[1].content, "LOW");
    assert_eq!(injections[1].priority, 30);
}
