//! RLM Pipeline — Planner → Executor → Aggregator.
//!
//! The core pipeline used by the `delegate` tool.

use crate::agent::progress::{ProgressCallback, SubagentProgress, SubagentStatus};
use crate::api::{ApiClient, ChatMessage};
use crate::config::Settings;
use crate::teams::guarding_tool_port::SubagentPermissionContext;
use crate::teams::subagent_loop::run_subagent_loop_with_permissions;
use crate::tools::meta::task::transcript::{new_transcript_id, save_minimal_transcript};
use crate::tools::ToolRegistry;
use crate::transcript::SubagentTranscriptStore;
use std::collections::{HashMap, HashSet, VecDeque};

use super::formats::jaccard_similarity;
use super::planner::{Planner, ReplacementSubTask, SubTask};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Result of the RLM pipeline including stats.
pub struct RlmResult {
    pub aggregated: String,
    pub sub_task_count: usize,
    pub completed: usize,
    pub failed: usize,
}

/// Tuple for a single sub-task execution entry in a dependency level.
type SubTaskExecItem = (usize, Arc<ToolRegistry>, ApiClient, String, Vec<String>);
type ProgressStore = Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>;
type ProgressContext = (ProgressStore, String);

/// Build a per-sub-task progress callback that registers a `Pending` node and
/// writes updates to the shared progress store. Shared by the normal spawn path
/// and the structural-fallback path so fallback subtasks appear in the tree.
async fn build_sub_progress(
    progress_store: Option<&ProgressContext>,
    root_node_id: &Option<String>,
    node_id: &str,
    label: &str,
) -> Option<ProgressCallback> {
    let (store, session_id) = progress_store?;
    let store = store.clone();
    let sid = session_id.clone();
    let nid = node_id.to_string();
    let pid = root_node_id.clone();
    let lbl = label.to_string();
    {
        let mut s = store.write().await;
        s.entry(sid.clone()).or_default().insert(
            nid.clone(),
            SubagentProgress {
                node_id: nid.clone(),
                parent_id: pid.clone(),
                label: lbl.clone(),
                status: SubagentStatus::Pending,
                round: None,
                max_rounds: Some(100),
                current_tool: None,
                current_params: None,
                action_log: Vec::new(),
                text_snapshot: None,
                started_at: chrono::Utc::now().timestamp_millis(),
                elapsed_ms: 0,
                metadata: None,
                progress_delta: None,
                token_budget_k: None,
                cumulative_tokens: 0,
                error_details: None,
                events: Vec::new(),
                messages: Vec::new(),
            },
        );
    }
    Some(Arc::new(move |mut progress: SubagentProgress| {
        progress.node_id = nid.clone();
        progress.parent_id = pid.clone();
        progress.label = lbl.clone();
        let store = store.clone();
        let sid = sid.clone();
        let nid = nid.clone();
        tokio::spawn(async move {
            let mut s = store.write().await;
            s.entry(sid).or_default().insert(nid, progress);
        });
    }))
}

/// Run the full RLM pipeline: Planner → Executor → Aggregator.
/// Used by the `delegate` tool.
///
/// `progress_store` and `session_id` are used to create per-sub-task progress
/// nodes so each sub-agent appears as a distinct entry in the subagent tree.
/// `root_node_id` is the parent for all sub-task nodes.
#[allow(clippy::too_many_arguments)]
pub async fn run_rlm_pipeline(
    settings: &Settings,
    tool_registry: Arc<ToolRegistry>,
    coordinator: Arc<crate::agent::AgentCoordinator>,
    caller: &crate::agent::AgentExecutionContext,
    task: &str,
    context: &str,
    progress_store: Option<ProgressContext>, // (store, session_id)
    root_node_id: Option<String>,
    token_budget_k: Option<u64>,
    workdir: Option<std::path::PathBuf>,
    transcript_store: Option<Arc<SubagentTranscriptStore>>,
) -> Result<RlmResult, String> {
    tracing::info!(
        target: "rlm",
        phase = "plan",
        task_len = task.len(),
        context_len = context.len(),
        "RLM pipeline: starting planner phase"
    );

    let sub_tasks: Vec<SubTask> = Planner::plan(settings, task, context).await?;

    // ── Budget allocation ──────────────────────────────────────────
    let budget_used = token_budget_k.unwrap_or(0);
    let mut allocation = if budget_used > 0 {
        Some(crate::tools::meta::rlm::budget::BudgetAllocation::new(
            budget_used,
        ))
    } else {
        None
    };
    let per_task_budget = allocation
        .as_ref()
        .map(|a| a.distribute_to_tasks(sub_tasks.len()));

    // ── Executor phase ────────────────────────────────────────────────
    let main_client = ApiClient::new(settings.clone());
    let small_client = if settings.models.small.is_some() {
        Some(ApiClient::new(settings.small_model_settings()))
    } else {
        tracing::warn!(target: "rlm", phase = "execute", "No small model configured, using main model");
        None
    };

    let allowed_tools: Vec<String> = tool_registry
        .list()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|name| {
            if name == "task" {
                0 < settings.agent.subagent.max_depth
            } else if name == "delegate" {
                false
            } else {
                name != "delegate"
            }
        })
        .collect();

    let n = sub_tasks.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, task_item) in sub_tasks.iter().enumerate() {
        deps[i] = task_item
            .depends_on
            .iter()
            .copied()
            .filter(|&idx| idx < n)
            .collect();
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
        let level_data: Vec<SubTaskExecItem> = sub_tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| depth[*i] == level)
            .map(|(idx, task_def)| {
                let prompt = task_def.prompt.clone();
                let use_small = task_def.use_small_model;
                let client = if use_small {
                    small_client.clone().unwrap_or_else(|| main_client.clone())
                } else {
                    main_client.clone()
                };
                (
                    idx,
                    tool_registry.clone(),
                    client,
                    prompt,
                    allowed_tools.clone(),
                )
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
        let timeout_secs = settings.agent.subagent.timeout_secs;

        for (idx, registry, api_client, prompt, allowed) in level_data {
            // ── Create a per-sub-task progress callback with unique node_id ──
            let sub_label = {
                // Truncate prompt to ~50 chars for a readable label.
                let p = prompt.trim();
                if p.len() > 50 {
                    let truncate_at = {
                        let mut end = 47;
                        while end > 0 && !p.is_char_boundary(end) {
                            end -= 1;
                        }
                        end
                    };
                    format!("sub: {}…", &p[..truncate_at])
                } else {
                    format!("sub: {}", p)
                }
            };
            let task_budget = per_task_budget
                .as_ref()
                .and_then(|budgets| budgets.get(idx).copied());
            // Reserve a coordinator-owned child for this subtask so it runs as
            // a direct child of the RLM caller (trusted parentage/depth/session,
            // never derived from model JSON). Depth hides `task` at the limit;
            // the coordinator remains the enforcement boundary.
            let reservation = match coordinator
                .reserve_child(caller, crate::agent::SpawnChildRequest::new(&prompt))
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    // Structural failure (depth / concurrency / task-group):
                    // self-execute the subtask inline in the calling agent via
                    // the shared fallback path, so depth-limit does not drop
                    // work. Mirrors `task`'s interception-point-1 fallback.
                    use crate::agent::fallback::{
                        fallback_eligible_from_coordinator_error, prepare_structural_fallback,
                        FallbackBlocked,
                    };
                    if fallback_eligible_from_coordinator_error(&e).is_none() {
                        task_errors[idx] = Some(format!("coordinator reserve failed: {}", e));
                        continue;
                    }
                    let fallback_key = format!("pending:{}", prompt);
                    let prepared = match prepare_structural_fallback(
                        &coordinator,
                        caller,
                        &fallback_key,
                    )
                    .await
                    {
                        Ok(p) => p,
                        Err(FallbackBlocked::RootCaller) => {
                            task_errors[idx] = Some(format!(
                                "coordinator reserve failed (root cannot self-execute): {}",
                                e
                            ));
                            continue;
                        }
                        Err(FallbackBlocked::AlreadyUsed) => {
                            task_errors[idx] = Some(format!(
                                "coordinator reserve failed (fallback already used): {}",
                                e
                            ));
                            continue;
                        }
                    };
                    let ghost_context = prepared.ghost;
                    let ghost_node_id = prepared.agent_id;
                    let sub_progress = build_sub_progress(
                        progress_store.as_ref(),
                        &root_node_id,
                        &ghost_node_id,
                        &sub_label,
                    )
                    .await;
                    let sub_coordinator = coordinator.clone();
                    let sub_workdir = workdir.clone();
                    tracing::info!(
                        target: "rlm",
                        phase = "execute",
                        fallback = "structural",
                        idx = idx,
                        "RLM pipeline: depth-limit fallback, self-executing subtask inline"
                    );
                    let sub_settings = Arc::new(settings.clone());
                    let sub_transcript_store = transcript_store.clone();
                    let handle = tokio::spawn(async move {
                        let mut sub_system_prompt =
                            "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result.".to_string();
                        inject_format_instruction("analysis", &mut sub_system_prompt);
                        let ghost_agent_id = ghost_context.agent_id.as_str().to_string();
                        let permission = SubagentPermissionContext::headless(
                            sub_workdir.clone().unwrap_or_else(|| {
                                std::env::current_dir()
                                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            }),
                            ghost_agent_id,
                        );
                        let started_at = chrono::Utc::now().timestamp_millis();
                        let result = run_subagent_loop_with_permissions(
                            &api_client,
                            registry.clone(),
                            &ghost_context,
                            sub_coordinator.clone(),
                            &sub_system_prompt,
                            &prompt,
                            &allowed,
                            100,
                            timeout_secs,
                            sub_progress,
                            task_budget,
                            sub_workdir,
                            permission,
                            sub_settings.clone(),
                            sub_transcript_store.clone(),
                            None,
                        )
                        .await;
                        if let Some(ref store) = sub_transcript_store {
                            let retention = if sub_settings.storage.transcript.max_age_days > 0 {
                                Some(sub_settings.storage.transcript.max_age_days)
                            } else {
                                None
                            };
                            save_minimal_transcript(
                                store,
                                &new_transcript_id(),
                                ghost_context.session_id.as_str(),
                                &prompt,
                                Some(sub_system_prompt.clone()),
                                prompt.clone(),
                                started_at,
                                &result,
                                sub_settings.agent.subagent.trace.context_char_limit,
                                retention,
                            );
                        }
                        // Ghost is not registered in coordinator scopes; skip
                        // finish_child (no permit to release, no scope to retire).
                        (result, idx)
                    });
                    handles.push(handle);
                    continue;
                }
            };
            let sub_context = reservation.context.clone();
            // Use the coordinator-owned child's identity as the progress-store
            // key so build_local_view() can cross-fill messages/snapshots/tokens.
            let sub_node_id = sub_context.agent_id.as_str().to_string();
            let sub_progress = build_sub_progress(
                progress_store.as_ref(),
                &root_node_id,
                &sub_node_id,
                &sub_label,
            )
            .await;
            let sub_coordinator = coordinator.clone();
            let sub_workdir = workdir.clone();
            let sub_settings = Arc::new(settings.clone());
            let sub_transcript_store = transcript_store.clone();
            let handle = tokio::spawn(async move {
                let mut sub_system_prompt = "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result.".to_string();
                inject_format_instruction("analysis", &mut sub_system_prompt);
                let sub_agent_id = sub_context.agent_id.as_str().to_string();
                let permission = SubagentPermissionContext::headless(
                    sub_workdir.clone().unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                    }),
                    sub_agent_id,
                );
                let started_at = chrono::Utc::now().timestamp_millis();
                let result = run_subagent_loop_with_permissions(
                    &api_client,
                    registry.clone(),
                    &sub_context,
                    sub_coordinator.clone(),
                    &sub_system_prompt,
                    &prompt,
                    &allowed,
                    100,
                    timeout_secs,
                    sub_progress,
                    task_budget,
                    sub_workdir,
                    permission,
                    sub_settings.clone(),
                    sub_transcript_store.clone(),
                    None,
                )
                .await;
                if let Some(ref store) = sub_transcript_store {
                    let retention = if sub_settings.storage.transcript.max_age_days > 0 {
                        Some(sub_settings.storage.transcript.max_age_days)
                    } else {
                        None
                    };
                    save_minimal_transcript(
                        store,
                        &new_transcript_id(),
                        sub_context.session_id.as_str(),
                        &prompt,
                        Some(sub_system_prompt.clone()),
                        prompt.clone(),
                        started_at,
                        &result,
                        sub_settings.agent.subagent.trace.context_char_limit,
                        retention,
                    );
                }
                // Persist the child's terminal through the coordinator so its
                // permit is released and it appears in the hierarchy.
                let terminal = match &result {
                    Ok(r) => crate::agent::ChildTerminal::Completed {
                        summary: r.chars().take(500).collect(),
                    },
                    Err(_) => crate::agent::ChildTerminal::Failed {
                        code: "subagent_failed".to_string(),
                        partial_result: None,
                    },
                };
                let _ = sub_coordinator.finish_child(&sub_context, terminal).await;
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

    // ── Replan phase (P0-2): incrementally re-decompose failed sub-tasks ──
    let rlm_cfg = resolve_rlm_settings(settings, caller);
    if rlm_cfg.retry_enabled && rlm_cfg.max_replan_cycles > 0 {
        let max_cycles = rlm_cfg.max_replan_cycles;
        let jaccard_threshold = rlm_cfg.jaccard_threshold;

        // Replan budget unit (Q4): replanner calls + replacement execution
        // draw from the executor pool. Estimate each replanner call and each
        // replacement at one per-task share; when the pool cannot cover a
        // call, replan is skipped (see per-cycle check below).
        let repl_per_task = per_task_budget
            .as_ref()
            .and_then(|b| b.first().copied())
            .unwrap_or(0);

        for cycle in 0..max_cycles {
            let (replace_ids, replace_set, failure_reasons) =
                compute_replan_scope(&deps, &task_errors, n);
            if replace_ids.is_empty() {
                break;
            }

            // Q4: skip replan when the executor pool cannot cover a replanner call.
            if let Some(ref alloc) = allocation {
                if !alloc.executor_has(repl_per_task) {
                    tracing::info!(
                        target: "rlm",
                        phase = "replan",
                        remaining = alloc.executor_pool,
                        "RLM pipeline: replan budget exhausted, skipping replan"
                    );
                    break;
                }
            }

            // Partial results: only preserved (non-replaced) tasks.
            let partial_results: HashMap<usize, String> = (0..n)
                .filter_map(|i| {
                    if replace_set.contains(&i) {
                        None
                    } else {
                        results[i]
                            .clone()
                            .filter(|s| !s.starts_with("[ERROR]"))
                            .map(|s| (i, s))
                    }
                })
                .collect();

            tracing::info!(
                target: "rlm",
                phase = "replan",
                cycle = cycle + 1,
                max_cycles = max_cycles,
                failed = task_errors.iter().filter(|e| e.is_some()).count(),
                replace_total = replace_ids.len(),
                "RLM pipeline: incremental replan"
            );

            let replacements = match Planner::replan_incremental(
                settings,
                &sub_tasks,
                &replace_ids,
                &failure_reasons,
                &partial_results,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        target: "rlm",
                        phase = "replan",
                        error = %e,
                        "RLM pipeline: replan planner failed, aborting replan"
                    );
                    break;
                }
            };

            // Q4: charge the replanner call against the executor pool.
            if let Some(ref mut alloc) = allocation {
                alloc.charge_executor(repl_per_task);
            }

            let accepted = jaccard_dedup_replacements(replacements, &sub_tasks, jaccard_threshold);

            if accepted.is_empty() {
                tracing::info!(
                    target: "rlm",
                    phase = "replan",
                    "RLM pipeline: no viable replacements after dedup, stopping replan"
                );
                break;
            }

            tracing::info!(
                target: "rlm",
                phase = "replan",
                cycle = cycle + 1,
                replacements = accepted.len(),
                "RLM pipeline: executing replacement sub-tasks"
            );

            // Execute all accepted replacements in parallel (single tier —
            // they only depend on preserved tasks which are already done).
            let mut replan_handles: Vec<
                tokio::task::JoinHandle<(
                    Result<String, crate::teams::subagent_loop::SubagentError>,
                    usize,
                )>,
            > = Vec::new();
            let timeout_secs = settings.agent.subagent.timeout_secs;

            for repl in accepted {
                let replaces_id = repl.replaces_id;
                let prompt = repl.prompt.clone();
                let client = if repl.use_small_model {
                    small_client.clone().unwrap_or_else(|| main_client.clone())
                } else {
                    main_client.clone()
                };
                let registry = tool_registry.clone();
                let allowed = allowed_tools.clone();

                // Q4: charge one per-task share for replacement execution.
                let repl_task_budget = if let Some(ref mut alloc) = allocation {
                    let charged = alloc.charge_executor(repl_per_task);
                    if charged > 0 {
                        Some(charged)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let sub_label = {
                    let p = prompt.trim();
                    if p.len() > 46 {
                        let truncate_at = {
                            let mut end = 43;
                            while end > 0 && !p.is_char_boundary(end) {
                                end -= 1;
                            }
                            end
                        };
                        format!("replan[{}]: {}…", replaces_id, &p[..truncate_at])
                    } else {
                        format!("replan[{}]: {}", replaces_id, p)
                    }
                };

                let reservation = match coordinator
                    .reserve_child(caller, crate::agent::SpawnChildRequest::new(&prompt))
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            target: "rlm",
                            phase = "replan",
                            replaces_id = replaces_id,
                            error = %e,
                            "RLM pipeline: replan reserve_child failed, skipping replacement"
                        );
                        continue;
                    }
                };
                let sub_context = reservation.context.clone();
                let sub_node_id = sub_context.agent_id.as_str().to_string();
                let sub_progress = build_sub_progress(
                    progress_store.as_ref(),
                    &root_node_id,
                    &sub_node_id,
                    &sub_label,
                )
                .await;
                let sub_coordinator = coordinator.clone();
                let sub_workdir = workdir.clone();
                let sub_settings = Arc::new(settings.clone());
                let sub_transcript_store = transcript_store.clone();
                let handle = tokio::spawn(async move {
                    let mut sub_system_prompt = "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result.".to_string();
                    inject_format_instruction("analysis", &mut sub_system_prompt);
                    let sub_agent_id = sub_context.agent_id.as_str().to_string();
                    let permission = SubagentPermissionContext::headless(
                        sub_workdir.clone().unwrap_or_else(|| {
                            std::env::current_dir()
                                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                        }),
                        sub_agent_id,
                    );
                    let started_at = chrono::Utc::now().timestamp_millis();
                    let result = run_subagent_loop_with_permissions(
                        &client,
                        registry.clone(),
                        &sub_context,
                        sub_coordinator.clone(),
                        &sub_system_prompt,
                        &prompt,
                        &allowed,
                        100,
                        timeout_secs,
                        sub_progress,
                        repl_task_budget,
                        sub_workdir,
                        permission,
                        sub_settings.clone(),
                        sub_transcript_store.clone(),
                        None,
                    )
                    .await;
                    if let Some(ref store) = sub_transcript_store {
                        let retention = if sub_settings.storage.transcript.max_age_days > 0 {
                            Some(sub_settings.storage.transcript.max_age_days)
                        } else {
                            None
                        };
                        save_minimal_transcript(
                            store,
                            &new_transcript_id(),
                            sub_context.session_id.as_str(),
                            &prompt,
                            Some(sub_system_prompt.clone()),
                            prompt.clone(),
                            started_at,
                            &result,
                            sub_settings.agent.subagent.trace.context_char_limit,
                            retention,
                        );
                    }
                    let terminal = match &result {
                        Ok(r) => crate::agent::ChildTerminal::Completed {
                            summary: r.chars().take(500).collect(),
                        },
                        Err(_) => crate::agent::ChildTerminal::Failed {
                            code: "subagent_failed".to_string(),
                            partial_result: None,
                        },
                    };
                    let _ = sub_coordinator.finish_child(&sub_context, terminal).await;
                    (result, replaces_id)
                });
                replan_handles.push(handle);
            }

            for handle in replan_handles {
                match handle.await {
                    Ok((Ok(result), replaces_id)) => {
                        results[replaces_id] = Some(result);
                        task_errors[replaces_id] = None;
                        tracing::info!(
                            target: "rlm",
                            phase = "replan",
                            replaces_id = replaces_id,
                            status = "completed",
                            "RLM pipeline: replacement sub-task completed"
                        );
                    }
                    Ok((Err(e), replaces_id)) => {
                        let error = format!("Replan sub-task {} failed: {}", replaces_id, e);
                        task_errors[replaces_id] = Some(error.clone());
                        results[replaces_id] = Some(format!("[ERROR] {}", error));
                        tracing::error!(
                            target: "rlm",
                            phase = "replan",
                            replaces_id = replaces_id,
                            error = %e,
                            "RLM pipeline: replacement sub-task failed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "rlm",
                            phase = "replan",
                            error = %e,
                            "RLM pipeline: replan join error"
                        );
                    }
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

    // ── Roll over unused executor budget to aggregator ────────────
    if let Some(ref mut alloc) = allocation {
        let failed_count = task_errors.iter().filter(|e| e.is_some()).count() as u64;
        let per_task = per_task_budget
            .as_ref()
            .and_then(|b| b.first().copied())
            .unwrap_or(0);
        let unused = per_task * failed_count;
        if unused > 0 {
            alloc.rollover_unused("executor", unused);
        }
    }

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

/// Resolved RLM replan settings after applying subagent overrides.
///
/// Only carries the three fields the replan phase consumes. The other
/// `RlmSettings` fields (enabled / delegate_tool / auto_routing) gate the
/// pipeline entry point upstream and are not overridden at the pipeline level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EffectiveRlmSettings {
    pub retry_enabled: bool,
    pub max_replan_cycles: usize,
    pub jaccard_threshold: f64,
}

/// Resolve RLM replan settings for `caller`.
///
/// Root callers (depth == 0) use `agent.rlm` verbatim. Subagent callers
/// (depth > 0) start from `agent.rlm` and let each `Some(_)` field of
/// `agent.subagent.rlm` override the corresponding value; `None` fields
/// inherit the top-level setting. This mirrors `SubagentRlmOverride`'s
/// per-field resolution contract (see `config::agent`).
pub(crate) fn resolve_rlm_settings(
    settings: &Settings,
    caller: &crate::agent::AgentExecutionContext,
) -> EffectiveRlmSettings {
    let base = &settings.agent.rlm;
    if caller.depth == 0 {
        return EffectiveRlmSettings {
            retry_enabled: base.retry_enabled,
            max_replan_cycles: base.max_replan_cycles,
            jaccard_threshold: base.jaccard_threshold,
        };
    }
    let ov = &settings.agent.subagent.rlm;
    EffectiveRlmSettings {
        retry_enabled: ov.retry_enabled.unwrap_or(base.retry_enabled),
        max_replan_cycles: ov.max_replan_cycles.unwrap_or(base.max_replan_cycles),
        jaccard_threshold: ov.jaccard_threshold.unwrap_or(base.jaccard_threshold),
    }
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

/// Inject a format instruction string into a prompt based on task type.
fn inject_format_instruction(task_type: &str, prompt: &mut String) {
    match task_type {
        "analysis" => {
            prompt.push_str("\n\nOUTPUT FORMAT: structured-claims/1 JSON.\n");
            prompt.push_str(
                "Your output MUST be valid JSON matching the structured-claims schema.\n",
            );
            prompt.push_str("{\n  \"format\": \"structured-claims/1\",\n  \"claims\": [\n    {\n      \"id\": \"c1\",\n      \"claim\": \"...\",\n      \"evidence\": \"...\",\n      \"confidence\": 0.9,\n      \"conflicts_with\": [],\n      \"actionable\": false\n    }\n  ]\n}\n");
        }
        "modification" => {
            prompt.push_str("\n\nOUTPUT FORMAT: unified-diff/1 JSON.\n");
            prompt.push_str("Your output MUST be valid JSON matching the unified-diff schema.\n");
            prompt.push_str("{\n  \"format\": \"unified-diff/1\",\n  \"changes\": [\n    {\n      \"file\": \"path/to/file.rs\",\n      \"intent\": \"description of change\",\n      \"diff\": \"@@ -1,3 +1,4 @@\\n...\",\n      \"confidence\": 0.9,\n      \"depends_on\": []\n    }\n  ]\n}\n");
        }
        _ => {} // mixed or unknown — no format injection, LLM decides
    }
}

/// Compute the set of sub-task ids to re-decompose during replan: directly
/// failed tasks plus their transitive downstream dependents.
///
/// Returns `(replace_ids sorted, replace_set, failure_reasons)`.
/// `failure_reasons` maps each replaced id to either its own error message
/// (directly failed) or an "upstream dependencies failed" notice (tainted
/// downstream dependent).
fn compute_replan_scope(
    deps: &[Vec<usize>],
    task_errors: &[Option<String>],
    n: usize,
) -> (Vec<usize>, HashSet<usize>, HashMap<usize, String>) {
    let failed_ids: Vec<usize> = (0..n).filter(|&i| task_errors[i].is_some()).collect();
    let failed_set: HashSet<usize> = failed_ids.iter().copied().collect();

    let mut replace_set = failed_set.clone();
    let mut queue: VecDeque<usize> = failed_ids.iter().copied().collect();
    while let Some(failed_id) = queue.pop_front() {
        for (i, i_deps) in deps.iter().enumerate() {
            if i_deps.contains(&failed_id) && !replace_set.contains(&i) {
                replace_set.insert(i);
                queue.push_back(i);
            }
        }
    }

    let mut replace_ids: Vec<usize> = replace_set.iter().copied().collect();
    replace_ids.sort_unstable();

    let mut failure_reasons: HashMap<usize, String> = HashMap::new();
    for &id in &replace_ids {
        if let Some(err) = &task_errors[id] {
            failure_reasons.insert(id, err.clone());
        } else {
            let tainted_deps: Vec<usize> = deps[id]
                .iter()
                .filter(|&&d| failed_set.contains(&d))
                .copied()
                .collect();
            failure_reasons.insert(
                id,
                format!("Upstream dependencies failed: {:?}", tainted_deps),
            );
        }
    }

    (replace_ids, replace_set, failure_reasons)
}

/// Filter replacement sub-tasks via Jaccard similarity dedup.
///
/// Drops replacements whose prompt is too similar to the failed original
/// prompt (would repeat the same approach) or to an already-accepted
/// replacement in this cycle. Returns the accepted replacements in order.
fn jaccard_dedup_replacements(
    replacements: Vec<ReplacementSubTask>,
    sub_tasks: &[SubTask],
    threshold: f64,
) -> Vec<ReplacementSubTask> {
    let mut accepted: Vec<ReplacementSubTask> = Vec::new();
    let mut seen_prompts: Vec<String> = Vec::new();
    for repl in replacements {
        let failed_prompt = sub_tasks
            .get(repl.replaces_id)
            .map(|t| t.prompt.as_str())
            .unwrap_or("");
        let sim_to_failed = jaccard_similarity(&repl.prompt, failed_prompt);
        if sim_to_failed >= threshold {
            tracing::info!(
                target: "rlm",
                phase = "replan",
                replaces_id = repl.replaces_id,
                jaccard = sim_to_failed,
                "RLM pipeline: dropping replacement too similar to failed prompt"
            );
            continue;
        }
        let dup_in_cycle = seen_prompts
            .iter()
            .any(|p| jaccard_similarity(&repl.prompt, p) >= threshold);
        if dup_in_cycle {
            continue;
        }
        seen_prompts.push(repl.prompt.clone());
        accepted.push(repl);
    }
    accepted
}

#[cfg(test)]
mod replan_tests {
    use super::*;

    #[test]
    fn test_compute_replan_scope_direct_failures_only() {
        let deps = vec![vec![], vec![], vec![]];
        let task_errors = vec![None, Some("timeout".to_string()), None];
        let (replace_ids, replace_set, failure_reasons) =
            compute_replan_scope(&deps, &task_errors, 3);
        assert_eq!(replace_ids, vec![1]);
        assert!(replace_set.contains(&1));
        assert_eq!(failure_reasons.get(&1), Some(&"timeout".to_string()));
    }

    #[test]
    fn test_compute_replan_scope_downstream_propagation() {
        // 0 failed; 1 and 2 depend on 0; 3 depends on 2.
        let deps = vec![vec![], vec![0], vec![0], vec![2]];
        let task_errors = vec![Some("err0".to_string()), None, None, None];
        let (replace_ids, _, failure_reasons) = compute_replan_scope(&deps, &task_errors, 4);
        assert_eq!(replace_ids, vec![0, 1, 2, 3]);
        assert_eq!(failure_reasons.get(&0), Some(&"err0".to_string()));
        assert!(failure_reasons.get(&1).unwrap().contains("Upstream"));
        assert!(failure_reasons.get(&2).unwrap().contains("Upstream"));
        assert!(failure_reasons.get(&3).unwrap().contains("Upstream"));
    }

    #[test]
    fn test_compute_replan_scope_no_failures() {
        let deps = vec![vec![], vec![]];
        let task_errors = vec![None, None];
        let (replace_ids, replace_set, _) = compute_replan_scope(&deps, &task_errors, 2);
        assert!(replace_ids.is_empty());
        assert!(replace_set.is_empty());
    }

    #[test]
    fn test_compute_replan_scope_partial_downstream() {
        // 0 failed; 1 depends on 0; 2 is independent.
        let deps = vec![vec![], vec![0], vec![]];
        let task_errors = vec![Some("err0".to_string()), None, None];
        let (replace_ids, _, _) = compute_replan_scope(&deps, &task_errors, 3);
        // Only 0 and 1 should be replaced; 2 is independent.
        assert_eq!(replace_ids, vec![0, 1]);
    }

    #[test]
    fn test_jaccard_dedup_drops_similar_to_failed() {
        let sub_tasks = vec![SubTask {
            prompt: "Read the auth module and analyze the login flow".to_string(),
            use_small_model: true,
            depends_on: vec![],
        }];
        let replacements = vec![
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Read the auth module and analyze the login flow".to_string(),
                use_small_model: true,
                depends_on: vec![],
            },
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Inspect the authentication service for session token handling".to_string(),
                use_small_model: true,
                depends_on: vec![],
            },
        ];
        let accepted = jaccard_dedup_replacements(replacements, &sub_tasks, 0.8);
        assert_eq!(accepted.len(), 1);
        assert!(accepted[0].prompt.contains("authentication service"));
    }

    #[test]
    fn test_jaccard_dedup_drops_duplicates_within_cycle() {
        let sub_tasks = vec![SubTask {
            prompt: "original prompt".to_string(),
            use_small_model: false,
            depends_on: vec![],
        }];
        let replacements = vec![
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Use a hash map to cache the results".to_string(),
                use_small_model: false,
                depends_on: vec![],
            },
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Use a hash map to cache the results".to_string(),
                use_small_model: false,
                depends_on: vec![],
            },
        ];
        let accepted = jaccard_dedup_replacements(replacements, &sub_tasks, 0.8);
        assert_eq!(accepted.len(), 1);
    }

    #[test]
    fn test_jaccard_dedup_keeps_different_approaches() {
        let sub_tasks = vec![SubTask {
            prompt: "Read the auth module and analyze the login flow".to_string(),
            use_small_model: true,
            depends_on: vec![],
        }];
        let replacements = vec![
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Search for JWT library options in Cargo.toml".to_string(),
                use_small_model: true,
                depends_on: vec![],
            },
            ReplacementSubTask {
                replaces_id: 0,
                prompt: "Write integration tests for the token refresh endpoint".to_string(),
                use_small_model: false,
                depends_on: vec![],
            },
        ];
        let accepted = jaccard_dedup_replacements(replacements, &sub_tasks, 0.8);
        assert_eq!(accepted.len(), 2);
    }

    #[test]
    fn test_jaccard_dedup_empty_input() {
        let sub_tasks: Vec<SubTask> = vec![];
        let replacements: Vec<ReplacementSubTask> = vec![];
        let accepted = jaccard_dedup_replacements(replacements, &sub_tasks, 0.8);
        assert!(accepted.is_empty());
    }

    #[test]
    fn test_resolve_rlm_settings_root_uses_top_level() {
        let mut settings = Settings::default();
        settings.agent.rlm.retry_enabled = true;
        settings.agent.rlm.max_replan_cycles = 3;
        settings.agent.rlm.jaccard_threshold = 0.7;
        let caller = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let eff = resolve_rlm_settings(&settings, &caller);
        assert!(eff.retry_enabled);
        assert_eq!(eff.max_replan_cycles, 3);
        assert!((eff.jaccard_threshold - 0.7).abs() < 1e-9);
    }

    #[test]
    fn test_resolve_rlm_settings_subagent_override_applied() {
        let mut settings = Settings::default();
        settings.agent.rlm.retry_enabled = true;
        settings.agent.rlm.max_replan_cycles = 3;
        settings.agent.rlm.jaccard_threshold = 0.7;
        settings.agent.subagent.rlm.retry_enabled = Some(false);
        settings.agent.subagent.rlm.max_replan_cycles = Some(1);
        settings.agent.subagent.rlm.jaccard_threshold = Some(0.9);
        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let caller = root.child(crate::agent::AgentId::new("child"));
        let eff = resolve_rlm_settings(&settings, &caller);
        assert!(!eff.retry_enabled);
        assert_eq!(eff.max_replan_cycles, 1);
        assert!((eff.jaccard_threshold - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_resolve_rlm_settings_subagent_without_override_falls_back() {
        let settings = Settings::default();
        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let caller = root.child(crate::agent::AgentId::new("child"));
        let eff = resolve_rlm_settings(&settings, &caller);
        assert_eq!(eff.retry_enabled, settings.agent.rlm.retry_enabled);
        assert_eq!(eff.max_replan_cycles, settings.agent.rlm.max_replan_cycles);
        assert!((eff.jaccard_threshold - settings.agent.rlm.jaccard_threshold).abs() < 1e-9);
    }
}
