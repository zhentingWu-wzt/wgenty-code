//! Daemon-owned background continuation scheduling acceptance tests.

use crate::daemon_harness::{create_session, spawn_daemon_custom};
use axum::extract::State;
use axum::http::header;
use axum::routing::post;
use axum::{Json, Router};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};
use wgenty_code::tools::execution::background::BackgroundResult;

#[derive(Clone)]
struct MockModelState {
    calls: Arc<AtomicUsize>,
    first_call_gate: Arc<Notify>,
    requests: mpsc::UnboundedSender<serde_json::Value>,
}

async fn mock_chat_stream(
    State(state): State<MockModelState>,
    Json(request): Json<serde_json::Value>,
) -> ([(header::HeaderName, &'static str); 1], String) {
    let call = state.calls.fetch_add(1, Ordering::SeqCst);
    let _ = state.requests.send(request);
    if call == 0 {
        state.first_call_gate.notified().await;
    }
    let body = format!(
        "data: {{\"id\":\"mock-{call}\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"mock\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"ack-{call}\"}},\"finish_reason\":null}}]}}\n\n\
         data: {{\"id\":\"mock-{call}\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"mock\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    );
    ([(header::CONTENT_TYPE, "text/event-stream")], body)
}

fn result_for(session_id: &str) -> BackgroundResult {
    BackgroundResult {
        task_id: "bg_1".to_string(),
        session_id: Some(session_id.to_string()),
        result_type: "command".to_string(),
        command: "printf done".to_string(),
        stdout: "done".to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
        sandbox_bypassed: false,
        permission_mode: None,
        sandbox_level: None,
    }
}

#[tokio::test]
async fn busy_run_final_save_precedes_background_continuation() {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel();
    let model_state = MockModelState {
        calls: Arc::new(AtomicUsize::new(0)),
        first_call_gate: Arc::new(Notify::new()),
        requests: request_tx,
    };
    let model_gate = Arc::clone(&model_state.first_call_gate);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock model");
    let model_addr = listener.local_addr().expect("mock model address");
    let model_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(mock_chat_stream))
                .with_state(model_state),
        )
        .await
        .expect("serve mock model");
    });

    let daemon = spawn_daemon_custom(
        |settings| {
            settings.models.main.name = "mock".to_string();
            settings.models.main.base_url = Some(format!("http://{model_addr}/v1"));
            settings.models.main.api_key = Some("test-key".to_string());
            settings.models.main.provider = Some("openai".to_string());
        },
        |_| {},
    )
    .await;
    let session_id = create_session(&daemon, "background-handoff").await;

    let response = daemon
        .client
        .post(format!("{}/sessions/{session_id}/run", daemon.base))
        .json(&serde_json::json!({"message": "foreground"}))
        .send()
        .await
        .expect("start foreground run");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let first_request = tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
        .await
        .expect("foreground reaches model")
        .expect("model request channel");
    assert_eq!(
        first_request["messages"]
            .as_array()
            .expect("messages")
            .last()
            .and_then(|message| message["content"].as_str()),
        Some("foreground")
    );

    daemon
        .state
        .record_background_result(result_for(&session_id))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        daemon
            .state
            .background_results_snapshot_for_session(&session_id)
            .await
            .len(),
        1,
        "busy run leaves the inbox untouched"
    );
    assert!(request_rx.try_recv().is_err(), "no hidden run while busy");

    model_gate.notify_one();
    let continuation_request =
        tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
            .await
            .expect("continuation reaches model")
            .expect("continuation request channel");
    let messages = continuation_request["messages"]
        .as_array()
        .expect("continuation messages");
    assert!(messages
        .iter()
        .any(|message| { message["role"] == "assistant" && message["content"] == "ack-0" }));
    let continuation: serde_json::Value = serde_json::from_str(
        messages
            .last()
            .and_then(|message| message["content"].as_str())
            .expect("structured continuation user message"),
    )
    .expect("continuation JSON");
    assert_eq!(continuation["type"], "background_task_results");
    assert_eq!(continuation["results"][0]["task_id"], "bg_1");

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while daemon.state.session_runs.is_active(&session_id) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("continuation finishes");
    assert!(daemon
        .state
        .background_results_snapshot_for_session(&session_id)
        .await
        .is_empty());

    model_server.abort();
}
