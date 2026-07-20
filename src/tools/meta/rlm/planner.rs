//! RLM Planner - decomposes a task into independent sub-tasks.
//!
//! Extracted from the pipeline as a behavior-preserving refactor.

use crate::api::{ApiClient, ChatMessage};
use crate::config::Settings;
use crate::tools::meta::rlm::pipeline::extract_json;

/// A single decomposed sub-task produced by the Planner.
#[derive(Debug, Clone)]
pub struct SubTask {
    pub prompt: String,
    pub use_small_model: bool,
    pub depends_on: Vec<usize>,
}

/// Task decomposition planner.
pub struct Planner;

impl Planner {
    /// Decompose `task` (with `context`) into up to 8 sub-tasks.
    ///
    /// Behavior is identical to the previous inline planner phase in
    /// `pipeline.rs`: same prompt text, same API call, same JSON parse,
    /// same `.take(8)` truncation.
    pub async fn plan(
        settings: &Settings,
        task: &str,
        context: &str,
    ) -> Result<Vec<SubTask>, String> {
        let planner_client = ApiClient::new(settings.clone());

        let planner_prompt = format!(
            r#"You are a task decomposition planner. Analyze the following complex task and break it down into independent sub-tasks.

Rules:
- Each sub-task MUST be self-contained and independently executable
- Sub-tasks that depend on a previous sub-task's output must list their dependencies
- Return ONLY a valid JSON array. No markdown, no explanation, no additional text.
- Maximum 8 sub-tasks.

<example>
Input: "Refactor the authentication module to use JWT tokens"
Output: [
  {{"prompt": "Read and analyze the current auth module in src/auth/ to understand the existing flow, data structures, and dependencies", "use_small_model": true, "depends_on": []}},
  {{"prompt": "Research JWT library options for this project - check dependencies in Cargo.toml and identify the best JWT crate", "use_small_model": true, "depends_on": []}},
  {{"prompt": "Implement the JWT token generation and verification logic in a new src/auth/jwt.rs module. Include token creation with claims, expiry, and refresh token support", "use_small_model": false, "depends_on": [0, 1]}},
  {{"prompt": "Update the login endpoint to return JWT tokens instead of session cookies, and add middleware for token validation", "use_small_model": false, "depends_on": [2]}}
]
</example>

Task: {task}

Context: {context}
"#,
            task = task,
            context = context
        );

        let planner_messages = vec![
            ChatMessage::system(
                "You are a precise task decomposition planner. Always return valid JSON.",
            ),
            ChatMessage::user(&planner_prompt),
        ];

        let planner_response = planner_client
            .chat(planner_messages, None)
            .await
            .map_err(|e| {
                tracing::error!(target: "rlm", phase = "plan", error = %e, "RLM planner API call failed");
                format!("RLM planner API call failed: {}", e)
            })?;

        let planner_content = planner_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("");

        let json_str = extract_json(planner_content);

        let raw_sub_tasks: Vec<serde_json::Value> =
            serde_json::from_str(&json_str).unwrap_or_else(|e| {
                tracing::warn!(
                    target: "rlm",
                    phase = "plan",
                    parse_error = %e,
                    raw = %json_str,
                    "RLM: failed to parse planner output, treating as single sub-task"
                );
                vec![serde_json::json!({
                    "prompt": format!("{}. Context: {}", task, context),
                    "use_small_model": false,
                    "depends_on": []
                })]
            });

        // Truncate to at most 8 sub-tasks, then map into typed structs.
        // `n` is computed after truncation so dependency indices are validated
        // against the same bound the executor previously used.
        let raw_sub_tasks: Vec<serde_json::Value> = raw_sub_tasks.into_iter().take(8).collect();
        let n = raw_sub_tasks.len();

        let sub_tasks: Vec<SubTask> = raw_sub_tasks
            .into_iter()
            .map(|value| {
                let prompt = value
                    .get("prompt")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                let use_small_model = value
                    .get("use_small_model")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                let depends_on = value
                    .get("depends_on")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|dep| dep.as_u64())
                            .map(|idx| idx as usize)
                            .filter(|&idx| idx < n)
                            .collect()
                    })
                    .unwrap_or_default();
                SubTask {
                    prompt,
                    use_small_model,
                    depends_on,
                }
            })
            .collect();

        tracing::info!(
            target: "rlm",
            phase = "plan",
            sub_task_count = sub_tasks.len(),
            "RLM pipeline: planner decomposed task"
        );

        Ok(sub_tasks)
    }
}
