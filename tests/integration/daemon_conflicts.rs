//! Acceptance: approval double-resolve 409s, `expected_version` conflicts,
//! and per-session isolation (brief Step 3).

use crate::daemon_harness::{create_session, spawn_daemon, TestDaemon};
use wgenty_code::daemon::interaction_bridge::QuestionPayload;
use wgenty_code::teams::permission_bridge::StructuredApproval;

/// Brief Step 3.1: two clients resolving the same interaction — the first
/// gets 200, the second gets 409 carrying the standing (first) resolution;
/// unknown ids get 404.
#[tokio::test]
async fn interaction_double_resolve_second_gets_409_with_standing_answer() {
    let d = spawn_daemon().await;
    let answer = r#"{"selected":["a"],"text":""}"#;

    // Register a pending question the way the server-side loop does.
    let payload = QuestionPayload {
        request_id: "req-1".to_string(),
        session_id: "s".to_string(),
        question: "pick one".to_string(),
        options: vec![],
        multi_select: false,
    };
    let bridge = d.state.interaction_bridge.clone();
    let waiter = tokio::spawn(async move { bridge.request(payload).await });
    // Wait until the question is actually pending before resolving.
    for _ in 0..100 {
        if !d.state.interaction_bridge.pending().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let url = format!("{}/interactions/req-1/resolve", d.base);
    let first = d
        .client
        .post(&url)
        .json(&serde_json::json!({ "answer": answer }))
        .send()
        .await
        .expect("first resolve");
    assert_eq!(first.status(), reqwest::StatusCode::OK);
    assert_eq!(
        first.json::<serde_json::Value>().await.unwrap()["resolved"],
        true
    );

    // The waiter unblocks with the delivered answer.
    assert_eq!(waiter.await.expect("waiter joins"), answer);

    // Duplicate resolve → 409 with the standing (first) answer.
    let second = d
        .client
        .post(&url)
        .json(&serde_json::json!({ "answer": r#"{"selected":["b"]}"# }))
        .send()
        .await
        .expect("duplicate resolve");
    assert_eq!(second.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = second.json().await.unwrap();
    assert_eq!(body["resolved"], false);
    assert_eq!(
        body["answer"], answer,
        "409 carries the standing resolution"
    );

    // Unknown id → 404.
    let unknown = d
        .client
        .post(format!("{}/interactions/nope/resolve", d.base))
        .json(&serde_json::json!({ "answer": "{}" }))
        .send()
        .await
        .expect("unknown resolve");
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
}

/// Brief Step 3.2: subagent `resolve-permission` has the same 409 semantics.
#[tokio::test]
async fn subagent_permission_double_resolve_second_gets_409() {
    let d = spawn_daemon().await;

    let approval = StructuredApproval::policy_ask(
        "perm-1",
        "subagent-explore",
        "execute_command",
        "wants to run ls",
        "command:ls",
    );
    let bridge = d.state.permission_bridge.clone();
    let waiter = tokio::spawn(async move { bridge.request_indefinite(approval).await });
    for _ in 0..100 {
        if !d.state.permission_bridge.pending().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let url = format!("{}/tools/resolve-permission", d.base);
    let first = d
        .client
        .post(&url)
        .json(&serde_json::json!({
            "request_id": "perm-1",
            "approved": true,
            "always": true,
            "session_rule": "command:ls",
        }))
        .send()
        .await
        .expect("first resolve");
    assert_eq!(first.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = first.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["resolved"], true);
    assert!(waiter.await.expect("waiter joins"), "approved");

    // Duplicate (contradictory) answer → 409 with the standing decision.
    let second = d
        .client
        .post(&url)
        .json(&serde_json::json!({ "request_id": "perm-1", "approved": false }))
        .send()
        .await
        .expect("duplicate resolve");
    assert_eq!(second.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = second.json().await.unwrap();
    assert_eq!(body["resolved"], true);
    assert_eq!(body["approved"], true, "standing decision is the first one");

    // Unknown id → 404.
    let unknown = d
        .client
        .post(&url)
        .json(&serde_json::json!({ "request_id": "nope", "approved": true }))
        .send()
        .await
        .expect("unknown resolve");
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
}

async fn put_session(
    d: &TestDaemon,
    id: &str,
    expected_version: Option<u64>,
    name: &str,
) -> reqwest::Response {
    d.client
        .put(format!("{}/sessions/{id}", d.base))
        .json(&serde_json::json!({
            "name": name,
            "expected_version": expected_version,
        }))
        .send()
        .await
        .expect("PUT session")
}

/// Brief Step 3.3 (sequential contract): a stale `expected_version` is
/// rejected with 409 + `current_version`; re-reading and retrying succeeds.
#[tokio::test]
async fn stale_expected_version_409_then_retry_succeeds() {
    let d = spawn_daemon().await;
    let id = "ver-conflict".to_string();

    // Upsert (legacy, no expected_version) → version 1.
    let resp = put_session(&d, &id, None, "v1").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.json::<serde_json::Value>().await.unwrap()["version"],
        1
    );

    // Stale writer (saw version 0) → 409 + current_version.
    let resp = put_session(&d, &id, Some(0), "stale").await;
    assert_eq!(resp.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["current_version"], 1);

    // Matching writer → 200, version advances.
    let resp = put_session(&d, &id, Some(1), "v2").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.json::<serde_json::Value>().await.unwrap()["version"],
        2
    );

    // Re-read and retry: the loser's recovery path works.
    let got: serde_json::Value = d
        .client
        .get(format!("{}/sessions/{id}", d.base))
        .send()
        .await
        .expect("GET session")
        .json()
        .await
        .unwrap();
    assert_eq!(got["version"], 2);
    assert_eq!(got["name"], "v2", "stale write was rejected");
    let resp = put_session(&d, &id, Some(got["version"].as_u64().unwrap()), "v3").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.json::<serde_json::Value>().await.unwrap()["version"],
        3
    );
}

/// Brief Step 3.3 (concurrent pair): two writers PUTting with the same
/// `expected_version` — exactly one succeeds, the other gets 409.
#[tokio::test]
async fn concurrent_put_same_expected_version_exactly_one_wins() {
    let d = spawn_daemon().await;
    let id = create_session(&d, "race").await;
    // Seed to version 1.
    let resp = put_session(&d, &id, Some(0), "seed").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Two writers race on the same expected_version.
    let (r1, r2) = futures::future::join(
        put_session(&d, &id, Some(1), "writer-a"),
        put_session(&d, &id, Some(1), "writer-b"),
    )
    .await;
    let mut statuses = [r1.status(), r2.status()];
    statuses.sort();
    assert_eq!(
        statuses,
        [reqwest::StatusCode::OK, reqwest::StatusCode::CONFLICT],
        "exactly one writer wins the race"
    );

    // The loser re-reads and retries successfully.
    let got: serde_json::Value = d
        .client
        .get(format!("{}/sessions/{id}", d.base))
        .send()
        .await
        .expect("GET session")
        .json()
        .await
        .unwrap();
    assert_eq!(got["version"], 2);
    let resp = put_session(&d, &id, Some(2), "loser-retry").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

/// Brief Step 3.4: permission modes and approved rules belong to their own
/// session — two sessions (in two projects) do not see each other's entries.
#[tokio::test]
async fn permission_modes_and_rules_are_isolated_per_session() {
    let d = spawn_daemon().await;
    let sid_a = create_session(&d, "session-a").await;

    // Second session in a second (registered) project → different root.
    let proj_b = tempfile::tempdir().expect("project B tempdir");
    let b_canon = proj_b.path().canonicalize().expect("canonical");
    d.state
        .projects
        .add(proj_b.path().to_str().expect("utf8 path"))
        .expect("register project B");
    let resp = d
        .client
        .post(format!("{}/sessions", d.base))
        .json(&serde_json::json!({
            "name": "session-b",
            "project_path": b_canon.to_string_lossy(),
        }))
        .send()
        .await
        .expect("create session in project B");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let sid_b = resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .expect("id")
        .to_string();

    // Session A switches to yolo; session B's mode is untouched.
    let resp = d
        .client
        .post(format!("{}/permission-mode", d.base))
        .json(&serde_json::json!({ "mode": "yolo", "session_id": sid_a }))
        .send()
        .await
        .expect("set mode for A");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    async fn mode_of(d: &TestDaemon, sid: &str) -> String {
        let body: serde_json::Value = d
            .client
            .get(format!("{}/permission-mode?session_id={sid}", d.base))
            .send()
            .await
            .expect("get mode")
            .json()
            .await
            .unwrap();
        body["mode"].as_str().expect("mode").to_string()
    }
    assert_eq!(mode_of(&d, &sid_a).await, "yolo");
    assert_ne!(
        mode_of(&d, &sid_b).await,
        "yolo",
        "session B must not see session A's mode"
    );

    // Missing session_id is a client bug on the server-side path → 400.
    let resp = d
        .client
        .get(format!("{}/permission-mode", d.base))
        .send()
        .await
        .expect("get mode without session_id");
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Approved rules are keyed by session: visible for A, invisible for B.
    d.state
        .approve_rule(&sid_a, "command:git status".to_string())
        .await;
    assert!(d.state.is_rule_approved(&sid_a, "command:git status").await);
    assert!(
        !d.state.is_rule_approved(&sid_b, "command:git status").await,
        "session B must not see session A's approved rule"
    );
}
