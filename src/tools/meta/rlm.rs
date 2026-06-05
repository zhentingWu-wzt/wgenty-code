//! RLM Delegate Tool — Recursive Language Model delegation.
//!
//! The `delegate` tool implements a Planner → Executor → Aggregator pipeline
//! for complex tasks. The parent agent delegates a complex task, the RLM
//! system automatically:
//!   1. Decomposes the task into independent sub-tasks (Planner)
//!   2. Executes sub-tasks in parallel via subagent loops (Executor)
//!   3. Merges results into a coherent response (Aggregator)
//!
//! This coexists with the existing `task` tool: simple tasks use `task` directly,
//! complex tasks use `delegate` to trigger the full RLM pipeline.

use crate::api::{ApiClient, ChatMessage};
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Run the full RLM pipeline: Planner → Executor → Aggregator.
/// Used by both the `delegate` tool and auto-routing in `task` tool.
pub async fn run_rlm_pipeline(
    settings: &Settings,
    tool_registry: Arc<ToolRegistry>,
    task: &str,
    context: &str,
) -> Result<String, String> {
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
        .filter(|name| name != "task" && name != "delegate")
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

    Ok(aggregated)
}

pub struct RlmDelegateTool {
    settings: Settings,
    tool_registry: std::sync::Weak<ToolRegistry>,
}

impl RlmDelegateTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<ToolRegistry>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
        }
    }
}

#[async_trait]
impl Tool for RlmDelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Delegate a complex task to the RLM system for automatic decomposition, parallel execution, and result aggregation. Use for complex, multi-step tasks that benefit from structured planning and parallel sub-agent execution. Simple tasks should use the `task` tool instead."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The complex task to delegate — describe what needs to be done, including context and constraints"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context or reference information to help the planner decompose the task effectively"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task = input["task"].as_str().unwrap_or("");
        let context = input["context"].as_str().unwrap_or("");

        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        tracing::info!(
            target: "rlm",
            phase = "plan",
            task_len = task.len(),
            context_len = context.len(),
            "RLM: starting planner phase"
        );

        // ── Phase 1: Planner — decomposes the task into sub-tasks ──────
        let planner_client = ApiClient::new(self.settings.clone());

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
                ToolError {
                    message: format!("RLM planner API call failed: {}", e),
                    code: Some("planner_api_error".to_string()),
                }
            })?;

        let planner_content = planner_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("");

        tracing::info!(
            target: "rlm",
            phase = "plan",
            raw_response_len = planner_content.len(),
            "RLM: planner received response"
        );

        // Parse the planner response: extract JSON from potential markdown fences
        let json_str = extract_json(&planner_content);

        let sub_tasks: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap_or_else(|e| {
            tracing::warn!(
                target: "rlm",
                phase = "plan",
                parse_error = %e,
                raw = %json_str,
                "RLM: failed to parse planner output as JSON array, treating as single sub-task"
            );
            // Fallback: treat the entire task as a single sub-task
            vec![serde_json::json!({
                "prompt": format!("{}. Context: {}", task, context),
                "use_small_model": false,
                "depends_on": []
            })]
        });

        // Enforce max sub-tasks limit
        let sub_tasks: Vec<serde_json::Value> = sub_tasks.into_iter().take(8).collect();

        tracing::info!(
            target: "rlm",
            phase = "plan",
            sub_task_count = sub_tasks.len(),
            "RLM: planner decomposed task into sub-tasks"
        );

        // ── Phase 2: Executor — run sub-tasks with dependency ordering ──
        let main_client = ApiClient::new(self.settings.clone());
        let small_client = if self.settings.small_model.is_some() {
            let mut small_settings = self.settings.clone();
            small_settings.model = self.settings.small_model.clone().unwrap();
            small_settings.api.max_tokens = 2048;
            if let Some(ref url) = self.settings.small_model_base_url {
                small_settings.api.base_url = url.clone();
            }
            Some(ApiClient::new(small_settings))
        } else {
            // Fall back to main model if no small model configured
            tracing::warn!(target: "rlm", phase = "execute", "No small model configured, using main model for all sub-tasks");
            None
        };

        // Allowed tools for sub-agents: exclude "task" and "delegate" to prevent recursive explosion
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| name != "task" && name != "delegate")
            .collect();

        // Build dependency graph: compute execution order
        let n = sub_tasks.len();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];

        for (i, task) in sub_tasks.iter().enumerate() {
            if let Some(dep_indices) = task.get("depends_on").and_then(|d| d.as_array()) {
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

        // Topological sort by dependency depth
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
            total_sub_tasks = n,
            dependency_levels = max_depth,
            max_concurrent = %self.settings.max_concurrent_subagents,
            "RLM: starting executor phase"
        );

        for level in 0..max_depth {
            // Gather sub-tasks at this depth level
            let level_tasks: Vec<(usize, &serde_json::Value)> = sub_tasks
                .iter()
                .enumerate()
                .filter(|(i, _)| depth[*i] == level)
                .collect();

            if level_tasks.is_empty() {
                continue;
            }

            tracing::info!(
                target: "rlm",
                phase = "execute",
                dependency_level = level,
                parallel_count = level_tasks.len(),
                "RLM: executing dependency level"
            );

            let mut handles = Vec::new();

            // Extract owned data to break the borrow on sub_tasks
            let level_data: Vec<(usize, Arc<crate::tools::ToolRegistry>, ApiClient, String, Vec<String>)> = level_tasks.iter().map(|(idx, task_def)| {
                let registry = tool_registry.clone();
                let prompt = (*task_def).get("prompt")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                let use_small = (*task_def)
                    .get("use_small_model")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);

                let client = if use_small {
                    small_client.as_ref().unwrap_or(&main_client)
                } else {
                    &main_client
                };
                let api_client = client.clone();

                let allowed = allowed_tools.clone();
                let client = if use_small {
                    small_client.clone().unwrap_or_else(|| main_client.clone())
                } else {
                    main_client.clone()
                };
                (*idx, registry, client, prompt, allowed)
            }).collect();

            for (idx, registry, api_client, prompt, allowed) in level_data {
                tracing::debug!(
                    target: "rlm",
                    phase = "execute",
                    sub_task = idx,
                    "RLM: dispatching sub-task"
                );

                let handle = tokio::spawn(async move {
                    let result = run_subagent_loop(
                        &api_client,
                        &registry,
                        "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result. The parent coordinator will merge all sub-task results into the final output.",
                        &prompt,
                        &allowed,
                        20,
                    )
                    .await;
                    (result, idx)
                });
                handles.push(handle);
            }

            // Collect results for this level
            for handle in handles {
                match handle.await {
                    Ok((Ok(result), idx)) => {
                        results[idx] = Some(result);
                        tracing::info!(
                            target: "rlm",
                            phase = "execute",
                            sub_task = idx,
                            status = "completed",
                            "RLM: sub-task completed"
                        );
                    }
                    Ok((Err(e), idx)) => {
                        let error = format!("Sub-task {} failed: {}", idx, e);
                        task_errors[idx] = Some(error.clone());
                        results[idx] = Some(format!("[ERROR] {}", error));
                        tracing::error!(
                            target: "rlm",
                            phase = "execute",
                            sub_task = idx,
                            error = %e,
                            "RLM: sub-task failed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "rlm",
                            phase = "execute",
                            error = %e,
                            "RLM: sub-task join error"
                        );
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
            total = n,
            "RLM: executor phase complete"
        );

        // ── Phase 3: Aggregator — merge sub-task results ────────────────
        let mut results_section = String::new();
        for (i, result) in results.iter().enumerate() {
            if let Some(content) = result {
                results_section.push_str(&format!(
                    "## Sub-task {}\n{}\n\n",
                    i + 1,
                    content
                ));
            } else if let Some(error) = &task_errors[i] {
                results_section.push_str(&format!(
                    "## Sub-task {} (FAILED)\n{}\n\n",
                    i + 1,
                    error
                ));
            }
        }

        let aggregator_prompt = format!(
            r#"You are a result aggregator. Merge the following sub-task results into a coherent, comprehensive response that directly addresses the original task.

Original Task: {task}

Context: {context}

Sub-task Results:
{results}

Provide a merged, complete response. Synthesize the sub-task results into a unified narrative. If any sub-task failed, acknowledge it and work with the available results.
"#,
            task = task,
            context = context,
            results = results_section
        );

        let aggregator_messages = vec![
            ChatMessage::system("You are a precise result aggregator. Synthesize sub-task results into a coherent response."),
            ChatMessage::user(&aggregator_prompt),
        ];

        tracing::info!(
            target: "rlm",
            phase = "aggregate",
            results_len = results_section.len(),
            "RLM: starting aggregator phase"
        );

        let aggregator_response = main_client
            .chat(aggregator_messages, None)
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "rlm",
                    phase = "aggregate",
                    error = %e,
                    "RLM: aggregator API call failed"
                );
                ToolError {
                    message: format!("RLM aggregator API call failed: {}", e),
                    code: Some("aggregator_api_error".to_string()),
                }
            })?;

        let aggregated = aggregator_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .to_string();

        tracing::info!(
            target: "rlm",
            phase = "complete",
            aggregated_len = aggregated.len(),
            "RLM: task delegation complete"
        );

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: aggregated,
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "sub_task_count".to_string(),
                    serde_json::json!(n),
                );
                m.insert(
                    "completed".to_string(),
                    serde_json::json!(completed_count),
                );
                m.insert(
                    "failed".to_string(),
                    serde_json::json!(failed_count),
                );
                m
            },
        })
    }
}

/// Extract a JSON array from a string, handling markdown fences and leading/trailing text.
fn extract_json(input: &str) -> String {
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
