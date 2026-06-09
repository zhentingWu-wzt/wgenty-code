//! RLM Pipeline — Planner → Executor → Aggregator.
//!
//! The core pipeline used by both the `delegate` tool and auto-routing in `task` tool.

use crate::api::{ApiClient, ChatMessage};
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::tools::ToolRegistry;
use std::sync::Arc;

/// Result of the RLM pipeline including stats.
pub struct RlmResult {
    pub aggregated: String,
    pub sub_task_count: usize,
    pub completed: usize,
    pub failed: usize,
}

/// Run the full RLM pipeline: Planner → Executor → Aggregator.
/// Used by both the `delegate` tool and auto-routing in `task` tool.
pub async fn run_rlm_pipeline(
    settings: &Settings,
    tool_registry: Arc<ToolRegistry>,
    task: &str,
    context: &str,
) -> Result<RlmResult, String> {
    tracing::info!(
        target: "rlm",
        phase = "plan",
        task_len = task.len(),
        context_len = context.len(),
        "RLM pipeline: starting planner phase"
    );

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
  {{"prompt": "Research JWT library options for this project — check dependencies in Cargo.toml and identify the best JWT crate", "use_small_model": true, "depends_on": []}},
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
        ChatMessage::system("You are a precise task decomposition planner. Always return valid JSON."),
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

    let sub_tasks: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap_or_else(|e| {
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

    let sub_tasks: Vec<serde_json::Value> = sub_tasks.into_iter().take(8).collect();

    tracing::info!(
        target: "rlm",
        phase = "plan",
        sub_task_count = sub_tasks.len(),
        "RLM pipeline: planner decomposed task"
    );

    // ── Executor phase ────────────────────────────────────────────────
    let main_client = ApiClient::new(settings.clone());
    let small_client = if settings.small_model.is_some() {
        let mut small_settings = settings.clone();
        small_settings.model = settings.small_model.clone().unwrap();
        small_settings.api.max_tokens = 2048;
        if let Some(ref url) = settings.small_model_base_url {
            small_settings.api.base_url = url.clone();
        }
        Some(ApiClient::new(small_settings))
    } else {
        tracing::warn!(target: "rlm", phase = "execute", "No small model configured, using main model");
        None
    };

    let allowed_tools: Vec<String> = tool_registry
        .list()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|name| { if name == "task" { 0 < settings.max_subagent_depth } else if name == "delegate" { false } else { true } })
        .collect();

    let n = sub_tasks.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, task_item) in sub_tasks.iter().enumerate() {
        if let Some(dep_indices) = task_item.get("depends_on").and_then(|d| d.as_array()) {
            for dep in dep_indices {
                if let Some(idx) = dep.as_u64() {
                    let idx = idx as usize;
                    if idx < n {
                        deps[i].push(idx);
                    }
                }
            }
        }
    }

    let mut depth: Vec<usize> = vec![0; n];
    for i in 0..n {
        for &dep in &deps[i] {
            depth[i] = depth[i].max(depth[dep] + 1);
        }
    }

    let max_depth = depth.iter().max().copied().unwrap_or(0) + 1;
    let mut results: Vec<Option<String>> = vec![None; n];
    let mut task_errors: Vec<Option<String>> = vec![None; n];

    tracing::info!(
        target: "rlm",
        phase = "execute",
        total = n,
        levels = max_depth,
        "RLM pipeline: starting executor phase"
    );

    for level in 0..max_depth {
        let level_data: Vec<(usize, Arc<ToolRegistry>, ApiClient, String, Vec<String>)> = sub_tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| depth[*i] == level)
            .map(|(idx, task_def)| {
                let prompt = task_def.get("prompt")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                let use_small = task_def
                    .get("use_small_model")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                let client = if use_small {
                    small_client.clone().unwrap_or_else(|| main_client.clone())
                } else {
                    main_client.clone()
                };
                (idx, tool_registry.clone(), client, prompt, allowed_tools.clone())
            })
            .collect();

        if level_data.is_empty() {
            continue;
        }

        tracing::info!(
            target: "rlm",
            phase = "execute",
            level = level,
            parallel = level_data.len(),
            "RLM pipeline: executing dependency level"
        );

        let mut handles = Vec::new();

        for (idx, registry, api_client, prompt, allowed) in level_data {
            let handle = tokio::spawn(async move {
                let result = run_subagent_loop(
                    &api_client,
                    &registry,
                    "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result.",
                    &prompt,
                    &allowed,
                    20,
                )
                .await;
                (result, idx)
            });
            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok((Ok(result), idx)) => {
                    results[idx] = Some(result);
                    tracing::info!(target: "rlm", phase = "execute", sub_task = idx, status = "completed", "RLM pipeline: sub-task completed");
                }
                Ok((Err(e), idx)) => {
                    let error = format!("Sub-task {} failed: {}", idx, e);
                    task_errors[idx] = Some(error.clone());
                    results[idx] = Some(format!("[ERROR] {}", error));
                    tracing::error!(target: "rlm", phase = "execute", sub_task = idx, error = %e, "RLM pipeline: sub-task failed");
                }
                Err(e) => {
                    tracing::error!(target: "rlm", phase = "execute", error = %e, "RLM pipeline: join error");
                }
            }
        }
    }

    let completed_count = results.iter().filter(|r| r.is_some()).count();
    let failed_count = task_errors.iter().filter(|e| e.is_some()).count();

    tracing::info!(
        target: "rlm",
        phase = "execute",
        completed = completed_count,
        failed = failed_count,
        "RLM pipeline: executor phase complete"
    );

    // ── Aggregator phase ──────────────────────────────────────────────
    let mut results_section = String::new();
    for (i, result) in results.iter().enumerate() {
        if let Some(content) = result {
            results_section.push_str(&format!("## Sub-task {}\n{}\n\n", i + 1, content));
        } else if let Some(error) = &task_errors[i] {
            results_section.push_str(&format!("## Sub-task {} (FAILED)\n{}\n\n", i + 1, error));
        }
    }

    let aggregator_prompt = format!(
        r#"Merge the following sub-task results into a coherent, comprehensive response that addresses the original task.

Original Task: {task}

Context: {context}

Sub-task Results:
{results}

Provide a merged, complete response."#,
        task = task,
        context = context,
        results = results_section
    );

    let aggregator_messages = vec![
        ChatMessage::system("You are a precise result aggregator."),
        ChatMessage::user(&aggregator_prompt),
    ];

    tracing::info!(target: "rlm", phase = "aggregate", "RLM pipeline: starting aggregator phase");

    let aggregator_response = main_client
        .chat(aggregator_messages, None)
        .await
        .map_err(|e| {
            tracing::error!(target: "rlm", phase = "aggregate", error = %e, "RLM pipeline: aggregator failed");
            format!("RLM aggregator failed: {}", e)
        })?;

    let aggregated = aggregator_response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .to_string();

    tracing::info!(target: "rlm", phase = "complete", len = aggregated.len(), "RLM pipeline: complete");

    Ok(RlmResult {
        aggregated,
        sub_task_count: n,
        completed: completed_count,
        failed: failed_count,
    })
}

/// Extract a JSON array from a string, handling markdown fences and leading/trailing text.
pub fn extract_json(input: &str) -> String {
    let input = input.trim();

    // Try to extract content from markdown code fences
    if let Some(start) = input.find("```") {
        let after_fence = &input[start + 3..].trim_start();
        // Skip optional language identifier
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_fence[content_start..];
        if let Some(end) = content.find("```") {
            return content[..end].trim().to_string();
        }
    }

    // If no markdown fences, try to find JSON array directly
    if let Some(start) = input.find('[') {
        if let Some(end) = input.rfind(']') {
            return input[start..=end].to_string();
        }
    }

    input.to_string()
}
