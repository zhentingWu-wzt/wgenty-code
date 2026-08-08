//! Acceptance: global event bus (brief Step 2) and the todos wire format.
//!
//! Covers `GET /events` multi-subscriber identical sequences, background
//! result retention without preemption, and the `activeForm` field name in
//! `todos_changed` payloads (daemon `tasks::TodoItem` serde rename).

use crate::daemon_harness::{create_session, spawn_daemon, SseReader};
use wgenty_code::daemon::global_events::GlobalEventKind;
use wgenty_code::tools::execution::background::BackgroundResult;

fn bg_result(task_id: &str) -> BackgroundResult {
    BackgroundResult {
        task_id: task_id.to_string(),
        result_type: "command".to_string(),
        command: "echo hi".to_string(),
        stdout: "hi".to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
        sandbox_bypassed: false,
        permission_mode: None,
        sandbox_level: None,
    }
}

/// Brief Step 2.1: two clients subscribed to `GET /events` observe the same
/// seq sequence for permission-mode switches, todos updates, background
/// results, and model changes.
///
/// The model-switch HTTP endpoint itself (settings persistence included) is
/// covered by the `switch_model_broadcasts_model_changed` unit test; here the
/// ModelChanged event is published through the same `broadcast_global` entry
/// point so this test never touches the developer's real `settings.json`.
#[tokio::test]
async fn global_events_two_subscribers_observe_identical_sequence() {
    let d = spawn_daemon().await;
    let sid = create_session(&d, "global-bus").await;

    let url = format!("{}/events", d.base);
    let mut a = SseReader::connect(&d.client, &url).await;
    let mut b = SseReader::connect(&d.client, &url).await;

    // 1. permission-mode switch → ModeChanged.
    let resp = d
        .client
        .post(format!("{}/permission-mode", d.base))
        .json(&serde_json::json!({ "mode": "yolo", "session_id": sid }))
        .send()
        .await
        .expect("set permission mode");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // 2. todos update → TodosChanged (also asserts the activeForm wire name).
    d.state
        .apply_todos_update(vec![wgenty_code::tasks::TodoItem {
            content: "write tests".to_string(),
            status: "in_progress".to_string(),
            active_form: "Writing tests".to_string(),
            subagent: None,
        }])
        .await;

    // 3. background result → BackgroundResult.
    d.state.record_background_result(bg_result("bg-1")).await;

    // 4. model change → ModelChanged (via the shared broadcast entry point).
    d.state.broadcast_global(
        GlobalEventKind::ModelChanged,
        serde_json::json!({ "profile": "p2", "model_name": "gpt-test" }),
    );

    let expected_kinds = [
        "mode_changed",
        "todos_changed",
        "background_result",
        "model_changed",
    ];
    let mut seqs_a = Vec::new();
    let mut seqs_b = Vec::new();
    for expected in expected_kinds {
        let ea = a.next_json().await;
        let eb = b.next_json().await;
        assert_eq!(ea["kind"], expected, "subscriber A kind");
        assert_eq!(eb["kind"], expected, "subscriber B kind");
        assert_eq!(ea, eb, "both subscribers see identical envelopes");
        seqs_a.push(ea["seq"].as_u64().expect("seq"));
        seqs_b.push(eb["seq"].as_u64().expect("seq"));
        if expected == "todos_changed" {
            // The streamed snapshot carries the daemon's `activeForm` rename
            // (the field the T9 TUI fix parses).
            assert_eq!(ea["data"]["items"][0]["activeForm"], "Writing tests");
        }
    }
    assert_eq!(seqs_a, vec![1, 2, 3, 4], "monotonic from 1");
    assert_eq!(seqs_a, seqs_b, "both subscribers see the same sequence");
}

/// Brief Step 2.2: a result produced while client C is offline is still
/// visible via `GET /background/results` when C comes back; online
/// subscribers both receive the broadcast (no preemption), and reads are
/// snapshots — repeated GETs keep returning the results (no drain).
#[tokio::test]
async fn background_result_retained_and_broadcast_without_preemption() {
    let d = spawn_daemon().await;

    // Produced while "C" is offline (no subscriber, no reader yet).
    d.state
        .record_background_result(bg_result("bg-offline"))
        .await;

    // C comes online: the retained result is queryable.
    let results_url = format!("{}/background/results", d.base);
    let body: serde_json::Value = d
        .client
        .get(&results_url)
        .send()
        .await
        .expect("GET background results")
        .json()
        .await
        .expect("results body");
    let ids: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["task_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["bg-offline"], "retained while offline");

    // Two online subscribers both get the next broadcast (no preemption).
    let url = format!("{}/events", d.base);
    let mut a = SseReader::connect(&d.client, &url).await;
    let mut b = SseReader::connect(&d.client, &url).await;
    d.state.record_background_result(bg_result("bg-live")).await;
    for reader in [&mut a, &mut b] {
        let ev = reader.next_json().await;
        assert_eq!(ev["kind"], "background_result");
        assert_eq!(ev["data"]["result"]["task_id"], "bg-live");
    }

    // Snapshot semantics: a repeated GET still returns everything (no drain).
    let body: serde_json::Value = d
        .client
        .get(&results_url)
        .send()
        .await
        .expect("GET background results again")
        .json()
        .await
        .expect("results body");
    let ids: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["task_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["bg-offline", "bg-live"], "no drain on read");
}

/// Wire-format guard for the T9 fix: the daemon serializes
/// `tasks::TodoItem.active_form` as `activeForm`, and the TUI's wire
/// `TodoItem` must parse it (previously swallowed by `#[serde(default)]`).
#[test]
fn todos_changed_items_use_active_form_wire_name_parseable_by_tui() {
    let item = wgenty_code::tasks::TodoItem {
        content: "fix bug".to_string(),
        status: "in_progress".to_string(),
        active_form: "Fixing the bug".to_string(),
        subagent: None,
    };
    let wire = serde_json::to_value(&item).expect("serialize daemon item");
    assert_eq!(wire["activeForm"], "Fixing the bug");
    assert!(wire.get("active_form").is_none());

    let parsed: wgenty_code::tui::client::TodoItem =
        serde_json::from_value(wire).expect("TUI wire item parses");
    assert_eq!(parsed.active_form, "Fixing the bug");
}
