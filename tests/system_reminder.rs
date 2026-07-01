//! Integration tests for the <system-reminder> injection channel.
//!
//! Strategy: rather than spinning up the full agent loop, we invoke
//! `build_user_turn_reminder` against a configured PromptContext with
//! the same inputs the AgentLoop would pass, then assert the structure.
//! Full end-to-end tests with hook injection (§5) will mock HookManager.

use tempfile::TempDir;

use wgenty_code::prompts::{build_user_turn_reminder, PromptContext};

/// Test helper: scope a fake `$HOME` for one closure.
/// Tests using this must be `#[serial]` to avoid races.
fn with_fake_home<F: FnOnce() -> R, R>(home: &std::path::Path, f: F) -> R {
    let prev = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let r = f();
    match prev {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    r
}

/// I1 — first turn user message contains the reminder block.
#[test]
#[serial_test::serial]
fn first_turn_user_message_contains_reminder() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["PROJECT-WGENTY".into()])
        .with_project_root(project_root);

    let reminder = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[]).expect("project section present → reminder Some")
    });

    // Simulate AgentLoop's prepend: reminder + "\n\n" + user input.
    let user_input = "What is 2 + 2?";
    let user_content = format!("{}\n\n{}", reminder.to_model, user_input);

    // The user message sent to the model must START with the reminder block.
    assert!(
        user_content.starts_with("<system-reminder>\n"),
        "user message must start with reminder opener"
    );
    assert!(
        user_content.contains("# wgentyMd"),
        "reminder must contain # wgentyMd marker"
    );
    assert!(
        user_content.contains("PROJECT-WGENTY"),
        "reminder must contain project WGENTY content"
    );
    assert!(
        user_content.contains("</system-reminder>"),
        "reminder must be closed"
    );
    assert!(
        user_content.ends_with(user_input),
        "user input must be preserved at the end"
    );

    // Reminder block ends before user input starts (separator: \n\n)
    let reminder_end = user_content.rfind("</system-reminder>").unwrap();
    let input_start = user_content.rfind(user_input).unwrap();
    assert!(
        reminder_end < input_start,
        "reminder must precede user input"
    );
}

/// I2 — second turn reminder reappears (per-turn injection, no caching).
#[test]
#[serial_test::serial]
fn second_turn_reminder_reappears() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["P".into()])
        .with_project_root(project_root);

    let (turn_a, turn_b) = with_fake_home(tmp.path(), || {
        let a = build_user_turn_reminder(&ctx, &[]).expect("turn1 → Some");
        let b = build_user_turn_reminder(&ctx, &[]).expect("turn2 → Some");
        (a, b)
    });

    assert_eq!(
        turn_a.to_model, turn_b.to_model,
        "reminder must be deterministic across turns when sources are unchanged"
    );
    assert!(turn_a.to_model.contains("<system-reminder>"));
    assert!(turn_b.to_model.contains("<system-reminder>"));
}

/// I3-prep — verifies runtime file modification path (deferred to Task 8.x manual test,
/// but covered here cheaply by writing two different WGENTY.md contents and asserting
/// reminder reflects the change.
#[test]
#[serial_test::serial]
fn reminder_reflects_runtime_file_change() {
    let tmp = TempDir::new().unwrap();
    let wgenty_dir = tmp.path().join(".wgenty-code");
    std::fs::create_dir_all(&wgenty_dir).unwrap();
    let user_wgenty = wgenty_dir.join("WGENTY.md");

    let ctx = PromptContext::new();

    let (before, after) = with_fake_home(tmp.path(), || {
        std::fs::write(&user_wgenty, "VERSION_ONE").unwrap();
        let r1 = build_user_turn_reminder(&ctx, &[]).expect("user WGENTY → Some");

        std::fs::write(&user_wgenty, "VERSION_TWO").unwrap();
        let r2 = build_user_turn_reminder(&ctx, &[]).expect("user WGENTY → Some");

        (r1, r2)
    });

    assert!(before.to_model.contains("VERSION_ONE"));
    assert!(!before.to_model.contains("VERSION_TWO"));
    assert!(after.to_model.contains("VERSION_TWO"));
    assert!(!after.to_model.contains("VERSION_ONE"));
}

// ── §5: Hook injection end-to-end ─────────────────────────────────────────

use wgenty_code::runtime::hooks::{
    collect_injections, ContextSource, HookAction, HookContext, HookDefinition, HookEvent,
    HookManager, HookOutcome, LayerVisibility,
};

/// I5.3 — hook injection flows through to reminder.to_model.
#[tokio::test]
#[serial_test::serial]
async fn hook_inject_content_end_to_end() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("EXTRA".into()),
            priority: 50,
            visibility: LayerVisibility::Visible,
        }],
    }]);

    let hook_ctx = HookContext {
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
        .fire(&HookEvent::UserPromptSubmit, &hook_ctx, None, None)
        .await;
    let injections = collect_injections(&outcomes);

    // Build reminder with the hook injections — no file sources needed.
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();

    let reminder = with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &injections))
        .expect("hook injection → reminder Some");

    // The model-facing output must contain the injected content.
    assert!(
        reminder.to_model.contains("EXTRA"),
        "to_model should include hook-injected content; got: {}",
        reminder.to_model
    );
}

/// I5.4 — multiple hooks rendered in priority order (lower priority first).
#[tokio::test]
#[serial_test::serial]
async fn two_hooks_render_in_priority_order_in_reminder() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("FROM-LOW-PRIO".into()),
                priority: 90,
                visibility: LayerVisibility::Visible,
            }],
        },
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("FROM-HIGH-PRIO".into()),
                priority: 5,
                visibility: LayerVisibility::Visible,
            }],
        },
    ]);

    let hook_ctx = HookContext {
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
        .fire(&HookEvent::UserPromptSubmit, &hook_ctx, None, None)
        .await;
    let injections = collect_injections(&outcomes);

    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let reminder =
        with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &injections)).expect("Some");

    let pos_high = reminder
        .to_model
        .find("FROM-HIGH-PRIO")
        .expect("high-prio present");
    let pos_low = reminder
        .to_model
        .find("FROM-LOW-PRIO")
        .expect("low-prio present");
    assert!(
        pos_high < pos_low,
        "priority 5 must render before priority 90; got high={}, low={}",
        pos_high,
        pos_low
    );
}

/// Hook-only scenario: no file sources, single hook injection.
///
/// Per spec (both system-reminder-injection §"All file sources missing but hook
/// injections present" and hook-lifecycle-complete §"Only hook content"):
/// the reminder block IS emitted, wrapping the hook content between the standard
/// preambles. No orphan file-source attribution headers.
#[tokio::test]
#[serial_test::serial]
async fn hook_only_yields_wrapped_reminder() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("HOOK-ONLY-PAYLOAD".into()),
            priority: 50,
            visibility: LayerVisibility::Visible,
        }],
    }]);

    let hook_ctx = HookContext {
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
        .fire(&HookEvent::UserPromptSubmit, &hook_ctx, None, None)
        .await;
    let injections = collect_injections(&outcomes);

    // Empty home → no user-global sources; empty ctx → no project sections.
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let reminder = with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &injections))
        .expect("hook present → Some");

    // Wrapped with reminder tags + preambles
    assert!(
        reminder.to_model.starts_with("<system-reminder>\n"),
        "hook-only output must start with reminder opener"
    );
    assert!(
        reminder.to_model.contains("# wgentyMd"),
        "opening preamble must be present"
    );
    assert!(
        reminder
            .to_model
            .contains("IMPORTANT: this context may or may not be relevant"),
        "closing preamble must be present"
    );
    assert!(
        reminder.to_model.trim_end().ends_with("</system-reminder>"),
        "hook-only output must close with reminder closer"
    );

    // Hook content present
    assert!(
        reminder.to_model.contains("HOOK-ONLY-PAYLOAD"),
        "hook content must appear inside the reminder block"
    );

    // No orphan file-source attribution headers (the 3 description constants)
    assert!(
        !reminder
            .to_model
            .contains("user's private global instructions for all projects"),
        "no user-source attribution header should appear (no user files exist)"
    );
    assert!(
        !reminder
            .to_model
            .contains("project instructions, checked into the codebase"),
        "no project-WGENTY attribution header should appear (no sections set)"
    );
    assert!(
        !reminder
            .to_model
            .contains("project agent conventions, checked into the codebase"),
        "no project-AGENTS attribution header should appear (no sections set)"
    );
}

/// I5.5 — a hook that blocks the turn (`continue_execution: false`) still has
/// its `injected_content` collected for the next user turn.
///
/// The turn-blocking itself is pre-existing hook semantics outside this
/// change's scope; this test verifies the INJECTION PERSISTENCE part, which
/// `collect_injections` owns. `collect_injections` filters only on
/// `injected_content` being `None`/empty — it does NOT consult
/// `continue_execution` — so a blocking hook's context still produces an
/// `InjectedFragment`.
///
/// Constructs a `HookOutcome` directly (no `HookManager::fire` needed, no
/// `$HOME` access), so `#[serial]` is unnecessary.
#[test]
fn hook_with_continue_execution_false_still_injects() {
    let outcome = HookOutcome {
        def: HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![],
        },
        continue_execution: false,
        reason: Some("blocked by guard".into()),
        injected_content: Some("blocked context".into()),
        user_answer: None,
        injection_priority: Some(50),
        injection_visibility: Some(LayerVisibility::Visible),
    };

    let injections = collect_injections(&[outcome]);

    assert_eq!(
        injections.len(),
        1,
        "continue_execution=false must NOT suppress injection collection"
    );
    assert_eq!(injections[0].content, "blocked context");
    assert_eq!(injections[0].priority, 50);
}

/// I5.6 — two hooks with identical priority preserve declaration order.
///
/// Per spec scenario "Two hooks with identical priorities": when two hooks
/// both inject with `priority: 5` and hook A is declared before hook B in
/// settings.json, hook A's content renders before hook B's.
/// `collect_injections` uses a stable sort (`sort_by_key`), so ties preserve
/// the input (declaration) order. `register_workflow_hooks` preserves Vec
/// order into the per-event hook list, and `fire()` iterates that list in
/// order, so declaration order flows end-to-end through to the reminder.
#[tokio::test]
#[serial_test::serial]
async fn two_hooks_identical_priority_preserve_declaration_order() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("FIRST_DECLARED".into()),
                priority: 5,
                visibility: LayerVisibility::Visible,
            }],
        },
        HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("SECOND_DECLARED".into()),
                priority: 5,
                visibility: LayerVisibility::Visible,
            }],
        },
    ]);

    let hook_ctx = HookContext {
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
        .fire(&HookEvent::UserPromptSubmit, &hook_ctx, None, None)
        .await;
    let injections = collect_injections(&outcomes);

    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let reminder = with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &injections))
        .expect("hook injections → reminder Some");

    let pos_first = reminder
        .to_model
        .find("FIRST_DECLARED")
        .expect("FIRST_DECLARED present");
    let pos_second = reminder
        .to_model
        .find("SECOND_DECLARED")
        .expect("SECOND_DECLARED present");
    assert!(
        pos_first < pos_second,
        "ties (priority 5) must preserve declaration order: FIRST_DECLARED should render \
         before SECOND_DECLARED; got first={}, second={}",
        pos_first,
        pos_second
    );
}
