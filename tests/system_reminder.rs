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
    HookManager, LayerVisibility,
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
    let reminder = with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &injections))
        .expect("Some");

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
