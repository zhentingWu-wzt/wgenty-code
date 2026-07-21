//! RLM Planner - decomposes a task into independent sub-tasks.
//!
//! Extracted from the pipeline as a behavior-preserving refactor.

use crate::api::{ApiClient, ChatMessage};
use crate::config::Settings;
use crate::tools::meta::rlm::pipeline::extract_json;
use std::collections::{HashMap, HashSet};

/// A single decomposed sub-task produced by the Planner.
#[derive(Debug, Clone)]
pub struct SubTask {
    pub prompt: String,
    pub use_small_model: bool,
    pub depends_on: Vec<usize>,
}

/// A replacement sub-task produced by incremental replan (P0-2).
///
/// `replaces_id` points to the original sub-task id being replaced.
/// `depends_on` may only reference preserved (non-replaced) sub-task ids;
/// invalid references are filtered out during parsing.
#[derive(Debug, Clone)]
pub struct ReplacementSubTask {
    pub replaces_id: usize,
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

    /// Incrementally re-decompose the sub-tasks identified by `replace_ids`
    /// (P0-2: RLM task-level replan ability).
    ///
    /// Unlike `plan`, this is a targeted re-decomposition: only the specified
    /// sub-task ids (failed tasks + their downstream dependents) are replaced.
    /// Completed (preserved) sub-tasks are NOT re-decomposed. Replacement
    /// sub-tasks' `depends_on` may only reference preserved ids.
    ///
    /// Inputs:
    /// - `original_plan`: the full original plan (for context).
    /// - `replace_ids`: original sub-task ids to re-decompose.
    /// - `failure_reasons`: reason for each failed id (subset of `replace_ids`).
    /// - `partial_results`: completed sub-task results (for context).
    ///
    /// Output replacements are validated: `replaces_id` must be in
    /// `replace_ids`, and `depends_on` must reference only preserved ids.
    /// Invalid entries are dropped (not errored) so a partial replan still
    /// yields usable replacements. Jaccard dedup against the failed prompts is
    /// performed by the executor (design Q5), not here.
    pub async fn replan_incremental(
        settings: &Settings,
        original_plan: &[SubTask],
        replace_ids: &[usize],
        failure_reasons: &HashMap<usize, String>,
        partial_results: &HashMap<usize, String>,
    ) -> Result<Vec<ReplacementSubTask>, String> {
        let replanner_client = ApiClient::new(settings.clone());

        let n = original_plan.len();
        let replace_set: HashSet<usize> = replace_ids.iter().copied().collect();

        // Render the original plan as indexed JSON for context.
        let plan_json: Vec<serde_json::Value> = original_plan
            .iter()
            .enumerate()
            .map(|(i, t)| {
                serde_json::json!({
                    "id": i,
                    "prompt": t.prompt,
                    "depends_on": t.depends_on,
                })
            })
            .collect();

        // Failure reasons for the failed subset of replace_ids.
        let failures_json: Vec<serde_json::Value> = replace_ids
            .iter()
            .filter_map(|&id| {
                failure_reasons.get(&id).map(|reason| {
                    serde_json::json!({
                        "id": id,
                        "original_prompt": original_plan.get(id).map(|t| t.prompt.as_str()).unwrap_or(""),
                        "failure_reason": reason,
                    })
                })
            })
            .collect();

        // Partial results of completed tasks (truncated for prompt size).
        let partial_json: Vec<serde_json::Value> = partial_results
            .iter()
            .map(|(id, result)| {
                let snippet: String = result.chars().take(500).collect();
                serde_json::json!({
                    "id": id,
                    "result_snippet": snippet,
                })
            })
            .collect();

        let preserved_ids: Vec<usize> = (0..n).filter(|i| !replace_set.contains(i)).collect();

        let replan_prompt = format!(
            r#"You are an incremental task re-decomposition planner. Some sub-tasks from a previous plan failed and must be re-decomposed. You are given the original plan, the ids to re-decompose, their failure reasons, and the partial results of completed sub-tasks for context.

Rules:
- ONLY re-decompose the sub-tasks whose ids are listed in `replace_ids`. Do NOT re-decompose completed (preserved) sub-tasks.
- Each replacement MUST declare which original sub-task id it replaces via `replaces_id` (must be one of `replace_ids`).
- A replacement's `depends_on` may ONLY reference preserved (non-replaced) sub-task ids. It must NOT reference any id in `replace_ids` (those are being replaced).
- Use the failure reasons to produce a DIFFERENT, more viable decomposition (different approach, smaller scope, or clarified prompt). Do not repeat the failed approach.
- A single failed id may be replaced by one or more replacements (split into smaller steps) if helpful.
- Return ONLY a valid JSON array of replacement objects. No markdown, no explanation.

Output schema:
[{{"replaces_id": <id>, "prompt": "...", "use_small_model": false, "depends_on": [<preserved_id>, ...]}}]

Original plan:
{plan_json}

replace_ids (re-decompose these): {replace_ids}

Preserved ids (may be referenced in depends_on, must NOT be re-decomposed): {preserved_ids}

Failed sub-tasks and reasons:
{failures_json}

Completed sub-task partial results (for context):
{partial_json}
"#,
            plan_json = serde_json::to_string(&plan_json).unwrap_or_default(),
            replace_ids = serde_json::to_string(replace_ids).unwrap_or_default(),
            preserved_ids = serde_json::to_string(&preserved_ids).unwrap_or_default(),
            failures_json = serde_json::to_string(&failures_json).unwrap_or_default(),
            partial_json = serde_json::to_string(&partial_json).unwrap_or_default(),
        );

        let replan_messages = vec![
            ChatMessage::system(
                "You are a precise incremental task re-decomposition planner. Always return valid JSON.",
            ),
            ChatMessage::user(&replan_prompt),
        ];

        let replan_response = replanner_client
            .chat(replan_messages, None)
            .await
            .map_err(|e| {
                tracing::error!(target: "rlm", phase = "replan", error = %e, "RLM replanner API call failed");
                format!("RLM replanner API call failed: {}", e)
            })?;

        let replan_content = replan_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("");

        let json_str = extract_json(replan_content);

        let raw_replacements: Vec<serde_json::Value> = serde_json::from_str(&json_str)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    target: "rlm",
                    phase = "replan",
                    parse_error = %e,
                    raw = %json_str,
                    "RLM: failed to parse replanner output, no replacements"
                );
                Vec::new()
            });

        // Validate + filter: replaces_id must be in replace_ids; depends_on
        // must reference only preserved ids. Invalid entries are dropped.
        let replacements: Vec<ReplacementSubTask> = raw_replacements
            .into_iter()
            .filter_map(|value| {
                let replaces_id = value.get("replaces_id").and_then(|v| v.as_u64())? as usize;
                if !replace_set.contains(&replaces_id) {
                    tracing::warn!(
                        target: "rlm",
                        phase = "replan",
                        replaces_id = replaces_id,
                        "RLM replan: dropping replacement with invalid replaces_id"
                    );
                    return None;
                }
                let prompt = value
                    .get("prompt")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                let use_small_model = value
                    .get("use_small_model")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                let depends_on: Vec<usize> = value
                    .get("depends_on")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|dep| dep.as_u64().map(|x| x as usize))
                            .filter(|&idx| idx < n && !replace_set.contains(&idx))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(ReplacementSubTask {
                    replaces_id,
                    prompt,
                    use_small_model,
                    depends_on,
                })
            })
            .collect();

        tracing::info!(
            target: "rlm",
            phase = "replan",
            replace_count = replace_ids.len(),
            replacement_count = replacements.len(),
            "RLM pipeline: replanner produced replacements"
        );

        Ok(replacements)
    }
}
