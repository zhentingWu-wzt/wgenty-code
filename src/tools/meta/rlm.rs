//! RLM Delegate Tool — Recursive Language Model delegation.
//!
//! The `delegate` tool implements an Explorer→Planner→Executor→Aggregator pipeline
//! for complex tasks. The parent agent delegates a complex task, the RLM
//! system automatically:
//!   1. Explores the codebase to gather context (P0)
//!   2. Decomposes the task into independent sub-tasks (Planner)
//!   3. Executes sub-tasks in parallel via subagent loops (Executor)
//!      — with automatic retry on failure (P0)
//!      — with tool tiering by depth (P1)
//!   4. Re-plans if failure rate > 50% (P1)
//!   5. Merges results + self-checks completeness (Aggregator, P2)
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
use std::time::Duration;

/// Timeout for individual (non-looping) API calls inside the RLM pipeline.
/// Defense-in-depth on top of the HTTP client timeout.
const RLM_API_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Shorter timeout for self-check / review calls.
const RLM_REVIEW_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the full RLM pipeline: Explorer → Planner → Executor → Aggregator.
/// Used by both the `delegate` tool and auto-routing in `task` tool.
///
/// P0: Explorer gathers codebase context before planning; sub-tasks retry on failure.
/// P1: If >50% sub-tasks fail, re-plan with failure feedback (max N cycles).
/// P2: Aggregator self-checks coverage completeness.
pub async fn run_rlm_pipeline(
    settings: &Settings,
    tool_registry: Arc<ToolRegistry>,
    task: &str,
    context: &str,
    depth: usize,
) -> Result<String, String> {
    // ── P0-1: Explorer — gather codebase context before planning ──────
    tracing::info!(
        phase = "explore",
        task_len = task.len(),
        depth = depth,
        "RLM pipeline: starting explorer phase"
    );

    let explorer_client = ApiClient::new(settings.clone());
    let explore_tools: Vec<String> = tool_registry
        .list().iter().map(|t| t.name().to_string())
        .filter(|n| matches!(n.as_str(), "grep" | "glob" | "file_read" | "list_files" | "search"))
        .collect();

    let explore_prompt = format!(
        "Explore the codebase to understand the scope for this task: \"{}\"\n\n\
         Use grep/glob to find relevant files. Read key files briefly. \
         Return ONLY a concise context summary (file paths + key structures, max 300 words). \
         Do NOT implement anything — just gather context.",
        task
    );

    let context_summary = run_subagent_loop(
        &explorer_client, &tool_registry,
        "You are a codebase explorer. Return ONLY a concise summary of relevant files and structures.",
        &explore_prompt, &explore_tools, 8, settings.subagent_timeout_secs,
    ).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "RLM: explorer failed, proceeding without context");
        format!("[exploration unavailable: {}]", e)
    });

    tracing::info!(
        phase = "explore→plan",
        summary_len = context_summary.len(),
        "RLM pipeline: explorer complete, transitioning to planner phase"
    );

    let enriched_context = if context.is_empty() {
        context_summary
    } else {
        format!("{}\n\nCodebase context:\n{}", context, context_summary)
    };

    tracing::info!(
        phase = "plan",
        context_len = enriched_context.len(),
        "RLM pipeline: starting planner phase (context enriched)"
    );

    // ── Planner — decompose task with enriched context ─────────────────
    let planner_client = ApiClient::new(settings.clone());

    let planner_prompt = format!(
        r#"You are a task decomposition planner. Analyze the following complex task and break it down into independent sub-tasks.

Rules:
- Each sub-task MUST be self-contained and independently executable
- Sub-tasks that depend on a previous sub-task's output must list their dependencies
- Use the provided codebase context to create accurate, file-specific sub-tasks
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

Context: {enriched_context}
"#,
        task = task,
        enriched_context = enriched_context
    );

    let planner_messages = vec![
        ChatMessage::system("You are a precise task decomposition planner. Always return valid JSON."),
        ChatMessage::user(&planner_prompt),
    ];

    let planner_response = tokio::time::timeout(RLM_API_CALL_TIMEOUT, planner_client.chat(planner_messages, None))
        .await
        .map_err(|_| {
            tracing::error!(phase = "plan", "RLM planner API call timed out after {:?}", RLM_API_CALL_TIMEOUT);
            format!("RLM planner API call timed out after {:?}", RLM_API_CALL_TIMEOUT)
        })?
        .map_err(|e| {
            tracing::error!(phase = "plan", error = %e, "RLM planner API call failed");
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
            phase = "plan",
            parse_error = %e,
            raw = %json_str,
            "RLM: failed to parse planner output, treating as single sub-task"
        );
        vec![serde_json::json!({
            "prompt": format!("{}. Context: {}", task, enriched_context),
            "use_small_model": false,
            "depends_on": []
        })]
    });

    let mut current_sub_tasks: Vec<serde_json::Value> = sub_tasks.into_iter().take(8).collect();

    tracing::info!(
        phase = "plan→execute",
        sub_task_count = current_sub_tasks.len(),
        "RLM pipeline: planner complete, transitioning to executor phase"
    );

    // ── P1-1: Re-plan loop (max configured cycles) ─────────────────────
    let max_replan = settings.rlm_max_replan_cycles;
    let mut replan_cycle = 0;
    let mut final_results: Vec<Option<String>> = vec![None; current_sub_tasks.len()];
    let mut final_errors: Vec<Option<String>> = vec![None; current_sub_tasks.len()];
    let mut current_n;

    loop {
        current_n = current_sub_tasks.len();
        let (results, task_errors, _completed, failed_count) = execute_sub_tasks(
            settings, &tool_registry, &current_sub_tasks, depth,
        ).await;

        final_results = results;
        final_errors = task_errors;
        let failure_rate = if current_n > 0 { failed_count as f64 / current_n as f64 } else { 0.0 };

        // P1-1: If failure rate > 50% and replan cycles remain, re-plan
        if failure_rate > 0.5 && replan_cycle < max_replan {
            replan_cycle += 1;
            tracing::warn!(
                phase = "replan",
                cycle = replan_cycle,
                max = max_replan,
                failure_rate = format!("{:.0}%", failure_rate * 100.0),
                "RLM: high failure rate, re-planning"
            );

            let failure_summary: String = final_errors.iter().enumerate()
                .filter(|(_, e)| e.is_some())
                .map(|(i, e)| format!("Sub-task {}: {}", i + 1, e.as_ref().unwrap()))
                .collect::<Vec<_>>().join("\n");

            let replan_prompt = format!(
                r#"Previous decomposition had {} failures out of {} sub-tasks.

Failures:
{}

Original task: {}

Re-decompose considering the failure modes. Simplify or split the failed sub-tasks. Return ONLY a valid JSON array."#,
                failed_count, current_n, failure_summary, task
            );

            match tokio::time::timeout(
                RLM_API_CALL_TIMEOUT,
                planner_client.chat(vec![
                    ChatMessage::system("You are a task decomposition planner. Return ONLY valid JSON array."),
                    ChatMessage::user(&replan_prompt),
                ], None),
            ).await {
                Ok(Ok(resp)) => {
                    let replan_content = resp.choices.first()
                        .and_then(|c| c.message.content.as_deref()).unwrap_or("");
                    let replan_json = extract_json(replan_content);
                    if let Ok(new_tasks) = serde_json::from_str::<Vec<serde_json::Value>>(&replan_json) {
                        current_sub_tasks = new_tasks.into_iter().take(8).collect();
                        tracing::info!(phase = "replan", new_count = current_sub_tasks.len(), "RLM: replan succeeded");
                    } else {
                        tracing::warn!(phase = "replan", "RLM: replan parse failed, using original results");
                        break;
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!(phase = "replan", error = %e, "RLM: replan API failed, using original results");
                    break;
                }
                Err(_elapsed) => {
                    tracing::error!(phase = "replan", "RLM: replan API timed out after {:?}, using original results", RLM_API_CALL_TIMEOUT);
                    break;
                }
            }
        } else {
            break;
        }
    }

    let completed_count = final_results.iter().filter(|r| r.is_some()).count();
    let failed_count = final_errors.iter().filter(|e| e.is_some()).count();

    tracing::info!(
        phase = "execute→aggregate",
        completed = completed_count,
        failed = failed_count,
        replan_cycles = replan_cycle,
        "RLM pipeline: executor complete, transitioning to aggregator phase"
    );

    // ── Aggregator phase ──────────────────────────────────────────────
    let mut results_section = String::new();
    for (i, result) in final_results.iter().enumerate() {
        if let Some(content) = result {
            results_section.push_str(&format!("## Sub-task {}\n{}\n\n", i + 1, content));
        } else if let Some(error) = &final_errors[i] {
            results_section.push_str(&format!("## Sub-task {} (FAILED)\n{}\n\n", i + 1, error));
        }
    }

    let main_client = ApiClient::new(settings.clone());
    let aggregator_prompt = format!(
        r#"Merge the following sub-task results into a coherent, comprehensive response that addresses the original task.

Original Task: {task}

Context: {enriched_context}

Sub-task Results:
{results}

Provide a merged, complete response."#,
        task = task,
        enriched_context = enriched_context,
        results = results_section
    );

    let aggregator_messages = vec![
        ChatMessage::system("You are a precise result aggregator."),
        ChatMessage::user(&aggregator_prompt),
    ];

    tracing::info!(phase = "aggregate", "RLM pipeline: starting aggregator phase");

    let aggregator_response = tokio::time::timeout(RLM_API_CALL_TIMEOUT, main_client.chat(aggregator_messages, None))
        .await
        .map_err(|_| {
            tracing::error!(phase = "aggregate", "RLM pipeline: aggregator timed out after {:?}", RLM_API_CALL_TIMEOUT);
            format!("RLM aggregator timed out after {:?}", RLM_API_CALL_TIMEOUT)
        })?
        .map_err(|e| {
            tracing::error!(phase = "aggregate", error = %e, "RLM pipeline: aggregator failed");
            format!("RLM aggregator failed: {}", e)
        })?;

    let mut aggregated = aggregator_response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .to_string();

    // ── P2-1: Aggregator self-check ────────────────────────────────────
    let check_prompt = format!(
        "Review this merged response against the original task.\n\n\
         Original task: {task}\n\n\
         Merged response:\n{aggregated}\n\n\
         If anything important is missing or inaccurate, append a [SUPPLEMENT] section \
         with the missing information. If the response fully covers the task, \
         reply with just 'COVERAGE_COMPLETE'.",
        task = task,
        aggregated = aggregated
    );

    let check_response = tokio::time::timeout(
        RLM_REVIEW_TIMEOUT,
        main_client.chat(vec![
            ChatMessage::system("You are a quality reviewer. Be concise."),
            ChatMessage::user(&check_prompt),
        ], None),
    ).await;

    match check_response {
        Ok(Ok(resp)) => {
            let check_text = resp.choices.first()
                .and_then(|c| c.message.content.as_deref())
                .unwrap_or("");
            if !check_text.contains("COVERAGE_COMPLETE") {
                tracing::info!(phase = "aggregate", check_len = check_text.len(), "RLM: aggregator self-check found gaps");
                aggregated = format!("{}\n\n---\n## Quality Review\n{}", aggregated, check_text);
            } else {
                tracing::info!(phase = "aggregate", "RLM: aggregator self-check passed");
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(phase = "aggregate", error = %e, "RLM: aggregator self-check API failed, using unverified result");
        }
        Err(_elapsed) => {
            tracing::warn!(phase = "aggregate", "RLM: aggregator self-check timed out after {:?}, using unverified result", RLM_REVIEW_TIMEOUT);
        }
    }

    tracing::info!(phase = "complete", len = aggregated.len(), "RLM pipeline: complete");

    Ok(aggregated)
}

/// Execute a set of sub-tasks respecting dependency ordering.
/// P0-2: retries each sub-task once on failure.
/// P1-2: applies tool tiering by depth.
async fn execute_sub_tasks(
    settings: &Settings,
    tool_registry: &Arc<ToolRegistry>,
    sub_tasks: &[serde_json::Value],
    depth: usize,
) -> (Vec<Option<String>>, Vec<Option<String>>, usize, usize) {
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
        None
    };

    // P1-2: Tool tiering by depth
    let allowed_tools = allowed_tools_by_depth(depth, tool_registry, settings);

    let n = sub_tasks.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, task_item) in sub_tasks.iter().enumerate() {
        if let Some(dep_indices) = task_item.get("depends_on").and_then(|d| d.as_array()) {
            for dep in dep_indices {
                if let Some(idx) = dep.as_u64() {
                    let idx = idx as usize;
                    if idx < n { deps[i].push(idx); }
                }
            }
        }
    }

    let mut depth_level: Vec<usize> = vec![0; n];
    for i in 0..n {
        for &dep in &deps[i] {
            depth_level[i] = depth_level[i].max(depth_level[dep] + 1);
        }
    }

    let max_depth = depth_level.iter().max().copied().unwrap_or(0) + 1;
    let mut results: Vec<Option<String>> = vec![None; n];
    let mut task_errors: Vec<Option<String>> = vec![None; n];

    tracing::info!(
        phase = "execute",
        total = n,
        levels = max_depth,
        subagent_depth = depth,
        "RLM pipeline: starting executor phase"
    );

    let retry = settings.rlm_retry_enabled;
    let timeout = settings.subagent_timeout_secs;

    for level in 0..max_depth {
        let level_data: Vec<(usize, Arc<ToolRegistry>, ApiClient, String, Vec<String>)> = sub_tasks
            .iter().enumerate()
            .filter(|(i, _)| depth_level[*i] == level)
            .map(|(idx, task_def)| {
                let prompt = task_def.get("prompt").and_then(|p| p.as_str()).unwrap_or("").to_string();
                let use_small = task_def.get("use_small_model").and_then(|s| s.as_bool()).unwrap_or(false);
                let client = if use_small {
                    small_client.clone().unwrap_or_else(|| main_client.clone())
                } else {
                    main_client.clone()
                };
                (idx, tool_registry.clone(), client, prompt, allowed_tools.clone())
            })
            .collect();

        if level_data.is_empty() { continue; }

        tracing::info!(
            phase = "execute",
            level = level,
            parallel = level_data.len(),
            "RLM pipeline: executing dependency level"
        );

        let mut handles: Vec<(usize, tokio::task::JoinHandle<Result<String, String>>)> = Vec::new();

        for (idx, registry, api_client, prompt, allowed) in level_data {
            let handle = tokio::spawn(async move {
                let result = run_subagent_loop(
                    &api_client, &registry,
                    "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result. The parent coordinator will merge all sub-task results into the final output.",
                    &prompt, &allowed, 20, timeout,
                ).await;

                // P0-2: Retry once on failure with a different angle
                if retry && result.is_err() {
                    let retry_prompt = format!(
                        "Your previous attempt failed with: {}\n\n\
                         Try a different approach to complete this task: {}",
                        result.as_ref().unwrap_err(), prompt
                    );
                    run_subagent_loop(
                        &api_client, &registry,
                        "You are a sub-agent. Your previous attempt failed. Try a different approach.",
                        &retry_prompt, &allowed, 15, timeout / 2,
                    ).await
                } else {
                    result
                }
            });
            handles.push((idx, handle));
        }

        // JoinHandle timeout = subagent timeout + 30s grace period
        let join_timeout = Duration::from_secs(timeout + 30);

        for (idx, handle) in handles {
            match tokio::time::timeout(join_timeout, handle).await {
                Ok(Ok(Ok(result))) => {
                    results[idx] = Some(result);
                    tracing::info!(phase = "execute", sub_task = idx, status = "completed");
                }
                Ok(Ok(Err(e))) => {
                    let error = format!("Sub-task {} failed: {}", idx, e);
                    task_errors[idx] = Some(error.clone());
                    results[idx] = Some(format!("[ERROR] {}", error));
                    tracing::error!(phase = "execute", sub_task = idx, error = %e);
                }
                Ok(Err(e)) => {
                    let error = format!("Sub-task {} join error (panic): {}", idx, e);
                    task_errors[idx] = Some(error.clone());
                    results[idx] = Some(format!("[ERROR] {}", error));
                    tracing::error!(phase = "execute", sub_task = idx, error = %e);
                }
                Err(_elapsed) => {
                    let error = format!(
                        "Sub-task {} timed out waiting for JoinHandle after {:?}",
                        idx, join_timeout
                    );
                    task_errors[idx] = Some(error.clone());
                    results[idx] = Some(format!("[ERROR] {}", error));
                    tracing::error!(phase = "execute", sub_task = idx, error = %error);
                }
            }
        }
    }

    let completed = results.iter().filter(|r| r.is_some()).count();
    let failed = task_errors.iter().filter(|e| e.is_some()).count();
    (results, task_errors, completed, failed)
}

/// P1-2: Filter tool availability by subagent depth.
/// Deeper subagents get progressively fewer tools to prevent runaway.
fn allowed_tools_by_depth(
    depth: usize,
    registry: &ToolRegistry,
    settings: &Settings,
) -> Vec<String> {
    registry.list().iter()
        .map(|t| t.name().to_string())
        .filter(|name| match name.as_str() {
            // Always block explicit recursive delegate
            "delegate" => false,
            // task only within configured depth
            "task" => depth < settings.max_subagent_depth,
            // Depth 2+: read-only, no file writes or command execution
            "file_write" | "file_edit" | "apply_patch" | "exec_command" | "execute_command"
                if depth >= 2 => false,
            // Depth 3+: only local search/read, no web
            "web_search" | "web_fetch" if depth >= 3 => false,
            _ => true,
        })
        .collect()
}

// ── RlmDelegateTool (the `delegate` tool exposed to LLM) ───────────────

pub struct RlmDelegateTool {
    settings: Settings,
    tool_registry: std::sync::Weak<ToolRegistry>,
}

impl RlmDelegateTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<ToolRegistry>,
    ) -> Self {
        Self { settings, tool_registry }
    }
}

#[async_trait]
impl Tool for RlmDelegateTool {
    fn name(&self) -> &str { "delegate" }

    fn is_read_only(&self) -> bool { false }

    fn description(&self) -> &str {
        "Delegate a complex task to the RLM system for automatic exploration, decomposition, parallel execution, result aggregation, and quality self-check. Use for complex, multi-step tasks. Simple tasks should use the `task` tool instead."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The complex task to delegate"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context to help the planner"
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
            task_len = task.len(),
            context_len = context.len(),
            "RLM delegate: starting pipeline"
        );

        // Delegate to the shared pipeline at depth 0
        match run_rlm_pipeline(&self.settings, tool_registry, task, context, 0).await {
            Ok(aggregated) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: aggregated,
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("execution_mode".to_string(), serde_json::json!("rlm"));
                    m
                },
            }),
            Err(e) => Err(ToolError {
                message: format!("RLM pipeline failed: {}", e),
                code: Some("rlm_error".to_string()),
            }),
        }
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
