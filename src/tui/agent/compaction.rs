//! TUI-only background-result injection (command tasks).
//!
//! Micro/auto compaction policy lives in `agent::runtime`; auto-summary I/O is
//! `adapters::TuiCompactor`.

use super::AgentLoop;
use crate::api::ChatMessage;
use crate::tools::execution::BackgroundResult;

impl AgentLoop {
    pub(super) async fn inject_background_results(&mut self) {
        match self.client.get_background_results().await {
            Ok(results) if !results.is_empty() => {
                // Subagent results arrive through task-group continuation turns.
                // Only command background results are injected here.
                let mut delivered = self.delivered_background_task_ids.lock().await;
                let recovered = results
                    .into_iter()
                    .filter_map(|r| {
                        // Retained results are global; only this loop's session
                        // may recover them. Missing ids are legacy/unowned data.
                        if r["session_id"].as_str() != Some(self.session_id.as_str()) {
                            return None;
                        }
                        let result_type = r["result_type"].as_str().unwrap_or("command");
                        if result_type == "subagent" {
                            return None;
                        }
                        let result: BackgroundResult = serde_json::from_value(r).ok()?;
                        let model_payload = serde_json::to_string(&result).ok()?;
                        if !delivered.insert(result.task_id.clone()) {
                            return None;
                        }
                        Some((result, model_payload))
                    })
                    .collect::<Vec<_>>();
                drop(delivered);
                if recovered.is_empty() {
                    return;
                }
                {
                    let mut history = self.conversation_history.lock().await;
                    history.extend(
                        recovered
                            .iter()
                            .map(|(_, model_payload)| ChatMessage::user(model_payload)),
                    );
                }
                for (result, _) in recovered {
                    let _ =
                        self.event_tx
                            .send(crate::tui::app::types::AppEvent::BackgroundTaskResult(
                                result.format_completion_notification(),
                            ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AgentLoop;
    // Prompt/parse checks that used to live next to do_auto_compact remain useful
    // as documentation of the summary JSON contract used by TuiCompactor.
    use crate::api::ChatMessage;
    use crate::context::MemoryManager;
    use crate::runtime::hooks::HookManager;
    use crate::tui::app::AppEvent;
    use crate::tui::client::DaemonClient;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::collections::HashSet;
    use std::sync::Arc;

    async fn retained_background_results() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "results": [
                {"task_id": "bg_a", "session_id": "session-a", "result_type": "command", "command": "printf recovered", "stdout": "recovered output", "stderr": "warning detail", "exit_code": 7, "success": false, "sandbox_bypassed": false, "permission_mode": "normal", "sandbox_level": "standard"},
                {"task_id": "bg_seen", "session_id": "session-a", "result_type": "command", "command": "true", "stdout": "", "stderr": "", "exit_code": 0, "success": true},
                {"task_id": "bg_b", "session_id": "session-b", "result_type": "command", "command": "true", "stdout": "", "stderr": "", "exit_code": 0, "success": true},
                {"task_id": "bg_legacy", "result_type": "command", "command": "true", "stdout": "", "stderr": "", "exit_code": 0, "success": true}
            ]
        }))
    }

    #[tokio::test]
    async fn recovery_injects_only_unseen_results_for_the_active_session() {
        let app = Router::new().route(
            "/api/v1/background/results",
            get(retained_background_results),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind retained-results test server");
        let address = listener
            .local_addr()
            .expect("read retained-results test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve retained-results test server");
        });

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let history = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let seen = Arc::new(tokio::sync::Mutex::new(HashSet::from([
            "bg_seen".to_string()
        ])));
        let tmp = tempfile::TempDir::new_in(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
        )
        .expect("create memory tempdir");
        let mut agent = AgentLoop::new(
            DaemonClient::new(format!("http://{address}")),
            event_tx,
            "session-a".to_string(),
            seen.clone(),
            None,
            history.clone(),
            vec![],
            false,
            None,
            100,
            crate::api::token_counter::TokenCounter::new(),
            Arc::new(HookManager::default()),
            Arc::new(crate::prompts::PromptContext::new()),
            1800,
            200_000,
            65_536,
            Arc::new(MemoryManager::new(tmp.path().to_path_buf())),
            0,
            false,
        );

        agent.inject_background_results().await;
        agent.inject_background_results().await;

        let notifications = match event_rx.recv().await {
            Some(AppEvent::BackgroundTaskResult(notification)) => vec![notification],
            event => panic!("expected background notification, got {event:?}"),
        };
        assert_eq!(
            notifications,
            vec!["[Background task bg_a completed: FAILED]\ncommand: printf recovered\nexit code: 7\nstdout:\nrecovered output\nstderr:\nwarning detail"]
        );
        assert!(
            event_rx.try_recv().is_err(),
            "recovery displays each result once"
        );
        assert!(seen.lock().await.contains("bg_a"));
        let injected = history.lock().await;
        assert_eq!(injected.len(), 1);
        assert_eq!(
            injected[0].content.as_deref(),
            Some(
                r#"{"task_id":"bg_a","session_id":"session-a","result_type":"command","command":"printf recovered","stdout":"recovered output","stderr":"warning detail","exit_code":7,"success":false,"sandbox_bypassed":false,"permission_mode":"normal","sandbox_level":"standard"}"#
            )
        );

        server.abort();
    }

    #[test]
    fn test_compaction_prompt_includes_json_format() {
        let messages = [ChatMessage::system(
            "You are a conversation summary assistant for an AI coding agent. \
             Your task is to:\n\
             1. Summarize the conversation history, preserving key details: \
             project context, files modified, decisions made, bugs found, \
             commands executed, and any pending tasks.\n\
             2. Extract key memories from the conversation as structured JSON.\n\n\
             Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
             {\n\
               \"summary\": \"<concise summary string>\",\n\
               \"memories\": [\n\
                 {\n\
                   \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
                   \"content\": \"<what to remember>\",\n\
                   \"importance\": <0.0 to 1.0>\n\
                 }\n\
               ]\n\
             }\n\n\
             If there is nothing worth remembering, return an empty memories array.\n\
             Do NOT use any tools — just return the JSON as plain text.",
        )];
        let sys_content = messages[0].content.as_deref().unwrap();
        assert!(sys_content.contains("\"summary\""));
        assert!(sys_content.contains("\"memories\""));
        assert!(sys_content.contains("decision"));
        assert!(sys_content.contains("importance"));
    }

    #[test]
    fn test_parse_compaction_json_success() {
        let json_response = r#"{
            "summary": "The user asked about memory systems.",
            "memories": [
                {"type": "decision", "content": "Use Jaccard for dedup", "importance": 0.8},
                {"type": "knowledge", "content": "Project uses Rust", "importance": 0.6}
            ]
        }"#;
        let json: serde_json::Value = serde_json::from_str(json_response).unwrap();
        let summary = json.get("summary").and_then(|v| v.as_str()).unwrap();
        let memories = json.get("memories").and_then(|v| v.as_array()).unwrap();
        assert_eq!(summary, "The user asked about memory systems.");
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0]["type"].as_str().unwrap(), "decision");
    }

    #[test]
    fn test_parse_compaction_json_failure_graceful() {
        let bad_response = "This is just a plain text summary, not JSON at all.";
        let result = serde_json::from_str::<serde_json::Value>(bad_response);
        assert!(result.is_err());
    }
}
