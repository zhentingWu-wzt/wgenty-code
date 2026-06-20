//! Subagent Loop — isolated agent loop for subagent execution.
//!
//! Each subagent gets its own `messages=[]` context (no shared conversation history
//! with the parent), runs a complete multi-round tool-use loop, and returns the
//! final assistant response back to the caller.

use crate::agent::progress::{
    ErrorInfo, ErrorType, ProgressCallback, SubagentEvent, SubagentEventType, SubagentMetadata,
    SubagentProgress, SubagentStatus,
};
use crate::api::{ApiClient, ChatMessage, ToolCall, ToolDefinition};
use crate::tools::ToolRegistry;
use crate::utils::stuck_detector::{StuckDetector, StuckStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Maximum consecutive JSON parse errors before aborting the subagent.
/// When the LLM repeatedly generates malformed tool-call arguments, we
/// inject a correction prompt instead of letting it loop forever.
const MAX_CONSECUTIVE_PARSE_ERRORS: usize = 3;
/// Per-round API call timeout. Defense-in-depth on top of the HTTP client
/// timeout. Individual rounds should complete well within this bound;
/// the overall loop timeout (`timeout_secs`) acts as the hard ceiling.
const PER_ROUND_API_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum length of a tool parameter summary string.
const MAX_PARAMS_SUMMARY_LEN: usize = 80;

/// Extract a human-readable summary of the most meaningful tool parameters.
///
/// For common tools, picks the 1-2 most informative parameter values.
/// Truncates long values at MAX_PARAMS_SUMMARY_LEN chars.
fn extract_params_summary(tool_name: &str, args: &serde_json::Value) -> String {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    // Per-tool: pick the most meaningful parameter(s).
    let keys: Vec<&str> = match tool_name {
        "file_read" | "read_file" | "file_write" | "write_file" => {
            vec!["file_path"]
        }
        "grep" | "search" => {
            if obj.contains_key("path") {
                vec!["pattern", "path"]
            } else {
                vec!["pattern"]
            }
        }
        "glob" | "file_glob" => {
            vec!["pattern"]
        }
        "execute_command" | "exec_command" | "shell" => {
            vec!["command"]
        }
        "web_fetch" | "web_search" => {
            vec!["url", "query"]
        }
        "task" | "delegate" => {
            vec!["description"]
        }
        "edit" | "file_edit" | "write" => {
            vec!["file_path"]
        }
        _ => {
            // For unknown tools, pick the first non-empty string param.
            obj.keys().map(|s| s.as_str()).take(2).collect()
        }
    };

    let parts: Vec<String> = keys
        .iter()
        .filter_map(|&k| {
            obj.get(k).map(|v| {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if s.len() > MAX_PARAMS_SUMMARY_LEN {
                    format!("{}…", &s[..MAX_PARAMS_SUMMARY_LEN])
                } else {
                    s
                }
            })
        })
        .collect();

    parts.join(", ")
}

/// Run a subagent with an isolated agent loop.
///
/// The subagent starts with a clean `[system, user]` message list and iterates
/// until the model stops requesting tool calls (i.e. `finish_reason != "tool_calls"`)
/// or the maximum number of rounds is exceeded.
///
/// # Arguments
/// * `api_client`      — API client for chat completions.
/// * `tool_registry`   — Registry of all available tools (filtered at call site).
/// * `system_prompt`   — System prompt that sets the subagent's role/behavior.
/// * `user_prompt`     — The task description and instructions to execute.
/// * `allowed_tools`   — Names of tools the subagent is permitted to call.
/// * `max_rounds`      — Maximum tool-use iterations (typically 100; caller decides).
/// * `timeout_secs`    — Wall-clock timeout in seconds for the entire loop.
/// * `on_progress`     — Optional callback for real-time execution progress updates.
///
/// # Returns
/// * `Ok(String)` — The final assistant content (text response).
/// * `Err(String)` — An error description if the loop fails or times out.
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent_loop(
    api_client: &ApiClient,
    tool_registry: &ToolRegistry,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
    timeout_secs: u64,
    on_progress: Option<ProgressCallback>,
    token_budget_k: Option<u64>,
) -> Result<String, String> {
    let timeout_duration = Duration::from_secs(timeout_secs);
    tracing::info!(
        prompt_len = user_prompt.len(),
        tool_count = allowed_tools.len(),
        max_rounds = max_rounds,
        timeout_secs = timeout_secs,
        "Subagent: starting agent loop"
    );
    static SUBAGENT_COUNTER: AtomicU64 = AtomicU64::new(0);
    let trace_id = SUBAGENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    tracing::info!(trace_id = trace_id, "Subagent: trace context");

    let start = Instant::now();
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    let on_progress_inner = on_progress.clone();

    let loop_future = async {
        let mut messages: Vec<ChatMessage> = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_prompt),
        ];

        // Pre-compute tool definitions for the allowed tool set.
        let all_tools = tool_registry.list();
        let tool_defs: Vec<ToolDefinition> = all_tools
            .iter()
            .filter(|t| allowed_tools.iter().any(|name| name == t.name()))
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect();

        let has_tools = !tool_defs.is_empty();
        let mut stuck_detector = StuckDetector::new();
        let mut consecutive_parse_errors: usize = 0;

        // Stateful progress fields — mutated across rounds via Mutex (must be Send+Sync for tokio::spawn).
        let action_log: Mutex<Vec<SubagentEvent>> = Mutex::new(Vec::new());
        let text_snapshot: Mutex<Option<String>> = Mutex::new(None);
        let current_params_val: Mutex<Option<String>> = Mutex::new(None);
        let cumulative_tokens: Mutex<usize> = Mutex::new(0);

        let emit = |status: SubagentStatus,
                    round: Option<usize>,
                    current_tool: Option<String>,
                    error_msg: Option<String>,
                    progress_delta: Option<f32>| {
            if let Some(ref cb) = on_progress_inner {
                let elapsed = start.elapsed();
                let is_terminal = matches!(
                    status,
                    SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
                );
                let snapshot = text_snapshot.lock().unwrap().clone();
                let metadata = if is_terminal || error_msg.is_some() {
                    Some(SubagentMetadata {
                        token_count: Some(*cumulative_tokens.lock().unwrap()),
                        error: error_msg.clone(),
                        depends_on: vec![],
                    })
                } else {
                    None
                };
                let error_details = error_msg.as_ref().map(|msg| ErrorInfo {
                    error_type: ErrorType::Unknown,
                    message: msg.clone(),
                    last_tool: current_tool.clone(),
                    last_params: current_params_val.lock().unwrap().clone(),
                    round: round.unwrap_or(0) as u32,
                    retryable: true,
                });
                cb(SubagentProgress {
                    node_id: trace_id.to_string(),
                    parent_id: None,
                    label: String::new(),
                    status,
                    round,
                    max_rounds: Some(max_rounds),
                    current_tool,
                    current_params: current_params_val.lock().unwrap().clone(),
                    action_log: action_log.lock().unwrap().clone(),
                    text_snapshot: if is_terminal { None } else { snapshot },
                    started_at: started_at_ms,
                    elapsed_ms: elapsed.as_millis() as u64,
                    metadata,
                    progress_delta,
                    token_budget_k,
                    cumulative_tokens: *cumulative_tokens.lock().unwrap() as u64,
                    error_details,
                    events: Vec::new(),
                });
            }
        };

        emit(SubagentStatus::Running, Some(0), None, None, None);

        for round in 0..max_rounds {
            let elapsed = start.elapsed().as_secs();
            // Info-level progress log every round (not just debug)
            tracing::info!(
                round = round,
                max_rounds = max_rounds,
                messages = messages.len(),
                elapsed_secs = elapsed,
                "Subagent: round {}/{}",
                round + 1,
                max_rounds
            );

            emit(SubagentStatus::Running, Some(round + 1), None, None, None);

            let response = tokio::time::timeout(
                PER_ROUND_API_TIMEOUT,
                api_client.chat(
                    messages.clone(),
                    if has_tools {
                        Some(tool_defs.clone())
                    } else {
                        None
                    },
                ),
            )
            .await
            .map_err(|_| {
                format!(
                    "Subagent API call timed out after {}s",
                    PER_ROUND_API_TIMEOUT.as_secs()
                )
            })?
            .map_err(|e| format!("Subagent API call failed: {}", e))?;

            // Log what the model returned (text or tool calls) so users can
            // distinguish "waiting for LLM" from "dead".
            let tool_call_count = response
                .choices
                .first()
                .and_then(|c| c.message.tool_calls.as_ref())
                .map(|tcs| tcs.len())
                .unwrap_or(0);
            let has_content = response
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .map(|c| !c.is_empty())
                .unwrap_or(false);

            // ── Accumulate token usage & budget enforcement ───────────────
            if let Some(ref usage) = response.usage {
                *cumulative_tokens.lock().unwrap() += usage.total_tokens;
            }
            // ── Token budget enforcement ─────────────────────────────────────
            if let Some(budget_k) = token_budget_k {
                let used = *cumulative_tokens.lock().unwrap();
                if used > (budget_k as usize) * 1000 {
                    let last_tool = current_params_val
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "none".to_string());
                    let msg = format!(
                        "Token budget exceeded: limit {}k, used {}k tokens after {} rounds (last tool: {})",
                        budget_k,
                        used / 1000,
                        round + 1,
                        last_tool,
                    );
                    emit(
                        SubagentStatus::Failed,
                        Some(round + 1),
                        None,
                        Some(msg.clone()),
                        None,
                    );
                    return Err(msg);
                }
            }

            // ── Capture text snapshot (full text, no truncation) ────────────
            if let Some(content) = response
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
            {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    *text_snapshot.lock().unwrap() = Some(trimmed.to_string());
                    // Append Thought event to action_log (full text)
                    {
                        let mut log = action_log.lock().unwrap();
                        let elapsed = start.elapsed().as_millis() as u64;
                        log.push(SubagentEvent {
                            event_type: SubagentEventType::Thought {
                                text: trimmed.to_string(),
                            },
                            elapsed_ms: elapsed,
                        });
                    }
                }
            }

            tracing::info!(
                round = round,
                tool_calls = tool_call_count,
                has_content = has_content,
                elapsed_secs = start.elapsed().as_secs(),
                "Subagent: API response received ({} tool calls, content={})",
                tool_call_count,
                has_content
            );

            let choice = response.choices.into_iter().next().ok_or_else(|| {
                "Subagent received empty response from API (no choices)".to_string()
            })?;

            let finish_reason = choice.finish_reason.unwrap_or_default();
            let tool_calls = choice.message.tool_calls.clone();
            let is_tool_call =
                finish_reason == "tool_calls" && tool_calls.as_ref().is_some_and(|c| !c.is_empty());

            // Push the assistant message into history.
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: choice.message.content.clone(),
                reasoning_content: choice.message.reasoning_content.clone(),
                tool_calls: tool_calls.clone(),
                tool_call_id: None,
            });

            if !is_tool_call {
                let elapsed = start.elapsed();
                tracing::info!(
                    trace_id = trace_id,
                    round = round,
                    elapsed_secs = elapsed.as_secs(),
                    "Subagent: completed successfully"
                );
                emit(SubagentStatus::Completed, Some(round + 1), None, None, None);
                return Ok(choice.message.content.unwrap_or_default());
            }

            // ── Stuck detection ──────────────────────────────────────────
            let active_tool_calls: Vec<ToolCall> = tool_calls.clone().unwrap_or_default();
            match stuck_detector.record_round(&active_tool_calls) {
                StuckStatus::Warn(msg) => {
                    tracing::warn!(
                        trace_id = trace_id,
                        round = round,
                        "Subagent: stuck warning"
                    );
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            if let Some(ref mut content) = last.content {
                                content.push_str(&msg);
                            }
                        }
                    }
                }
                StuckStatus::Abort(msg) => {
                    tracing::error!(
                        trace_id = trace_id,
                        round = round,
                        error = %msg,
                        "Subagent: stuck abort"
                    );
                    emit(
                        SubagentStatus::Failed,
                        Some(round + 1),
                        None,
                        Some(msg.clone()),
                        None,
                    );
                    return Err(msg);
                }
                StuckStatus::Ok => {}
            }

            // Execute each tool call and push results back as tool-result messages.
            let tool_results: Vec<ToolCall> = tool_calls.unwrap();
            let mut had_parse_error_this_round = false;

            for tool_call in tool_results {
                let tool_name = &tool_call.function.name;
                let raw_args = &tool_call.function.arguments;
                let (args, parse_err) =
                    crate::utils::lenient_json::parse_tool_args_lenient(raw_args, tool_name);

                if let Some(ref e) = parse_err {
                    // Check whether the lenient parser recovered useful fields.
                    // If it did, the error is "recoverable" — the tool can still
                    // execute with the recovered args. These don't count toward
                    // the abort threshold.
                    let recovered_useful_fields = args
                        .as_object()
                        .map(|obj| obj.keys().any(|k| !k.starts_with('_')))
                        .unwrap_or(false);

                    had_parse_error_this_round = true;

                    if recovered_useful_fields {
                        // Recoverable: tool will execute with extracted args.
                        // Don't reset the counter, but don't increment either.
                        tracing::info!(
                            tool = %tool_name,
                            error = %e,
                            consecutive = consecutive_parse_errors,
                            raw_len = raw_args.len(),
                            "Subagent: tool arguments recovered via lenient parser"
                        );
                    } else {
                        // Unrecoverable: completely garbled JSON, no fields extracted.
                        consecutive_parse_errors += 1;
                        tracing::warn!(
                            tool = %tool_name,
                            error = %e,
                            consecutive = consecutive_parse_errors,
                            raw_len = raw_args.len(),
                            "Subagent: tool call arguments irrecoverable (no fields extracted)"
                        );
                    }
                } else {
                    // Successful parse (possibly via pre-processing fix) — reset counter
                    consecutive_parse_errors = 0;
                }

                // ── Abort on excessive consecutive IRRECOVERABLE parse errors ──
                if consecutive_parse_errors >= MAX_CONSECUTIVE_PARSE_ERRORS {
                    let msg = format!(
                        "Subagent aborted: {} consecutive tool calls had irrecoverable JSON errors. \
                         The model may be generating severely malformed tool arguments.",
                        consecutive_parse_errors
                    );
                    tracing::error!( trace_id = trace_id, error = %msg);
                    return Err(msg);
                }

                // ── Extract params summary & update action log ──────────────
                let params_summary = extract_params_summary(tool_name, &args);
                *current_params_val.lock().unwrap() = Some(params_summary.clone());
                {
                    let mut log = action_log.lock().unwrap();
                    let elapsed = start.elapsed().as_millis() as u64;
                    log.push(SubagentEvent {
                        event_type: SubagentEventType::Action {
                            tool_name: tool_name.clone(),
                            params_summary: params_summary.clone(),
                        },
                        elapsed_ms: elapsed,
                    });
                }

                emit(
                    SubagentStatus::Running,
                    Some(round + 1),
                    Some(tool_name.clone()),
                    None,
                    None,
                );

                tracing::debug!(
                    tool = %tool_name,
                    round = round,
                    "Subagent: executing tool"
                );
                // Per-tool timeout: 90s for most tools, 120s for exec_command/grep
                let tool_timeout = if tool_name == "exec_command" || tool_name == "execute_command"
                {
                    Duration::from_secs(120)
                } else {
                    Duration::from_secs(90)
                };
                let tool_result =
                    tokio::time::timeout(tool_timeout, tool_registry.execute(tool_name, args))
                        .await;

                let mut content = match tool_result {
                    Ok(Ok(output)) => output.content,
                    Ok(Err(e)) => format!("Error: {}", e.message),
                    Err(_) => {
                        let msg = format!(
                            "Tool '{}' timed out after {}s",
                            tool_name,
                            tool_timeout.as_secs()
                        );
                        tracing::warn!(
                            tool = %tool_name,
                            timeout_secs = tool_timeout.as_secs(),
                            "Subagent: tool timed out"
                        );
                        msg
                    }
                };

                // ── Append ToolResult event to action log ─────────────────────
                {
                    let mut log = action_log.lock().unwrap();
                    let success = !content.starts_with("Error:") && !content.starts_with("Tool '");
                    let summary: String = content.chars().take(200).collect();
                    log.push(SubagentEvent {
                        event_type: SubagentEventType::ToolResult {
                            tool_name: tool_name.clone(),
                            success,
                            summary,
                        },
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    });
                }

                // ── Inject parse error into tool result so the LLM can self-correct ──
                if let Some(ref err_msg) = parse_err {
                    content.push_str(&format!(
                        "\n\n---\n## ⚠️ Tool Argument Parse Warning\n\
                         Your previous tool call to `{}` had malformed JSON arguments.\n\
                         **Parse error**: {}\n\
                         **Raw arguments received** (may be truncated):\n```json\n{}\n```\n\
                         **Please retry** with properly escaped JSON. Common issues:\n\
                         - Regex patterns with backslashes: use `\\\\` instead of `\\`\n\
                         - Quotes inside patterns: use `\\\"` instead of `\"`\n\
                         - Ensure all strings are properly closed with `\"`",
                        tool_name, err_msg, raw_args
                    ));
                }

                messages.push(ChatMessage::tool(&tool_call.id, content));
            }

            // ── If any tool calls this round had parse errors, inject correction guidance ──
            if had_parse_error_this_round {
                messages.push(ChatMessage::user(
                    "<system-reminder>\n\
                     Your previous tool call(s) had malformed JSON arguments. \
                     This usually happens when special characters (backslashes, quotes) \
                     in grep patterns, file paths, or code snippets are not properly JSON-escaped.\n\
                     \n\
                     **JSON escaping rules for regex patterns:**\n\
                     - `\\d` → write as `\\\\d` (double the backslash)\n\
                     - `\\w` → write as `\\\\w`\n\
                     - `\\s` → write as `\\\\s`\n\
                     - Backslash `\\` → write as `\\\\`\n\
                     - Quote `\"` inside a string → write as `\\\"`\n\
                     - Newline → write as `\\n`\n\
                     \n\
                     The system will attempt to auto-fix common escaping issues, \
                     but please ensure your tool arguments are valid JSON.\n\
                     </system-reminder>"
                ));
            }
        }

        tracing::info!(
            trace_id = trace_id,
            max_rounds = max_rounds,
            elapsed_secs = start.elapsed().as_secs(),
            "Subagent: exceeded max rounds"
        );
        emit(
            SubagentStatus::Failed,
            Some(max_rounds),
            None,
            Some("Subagent exceeded maximum number of rounds".to_string()),
            None,
        );
        Err("Subagent exceeded maximum number of rounds".to_string())
    };

    match tokio::time::timeout(timeout_duration, loop_future).await {
        Ok(result) => result,
        Err(_elapsed) => {
            if let Some(ref cb) = on_progress {
                cb(SubagentProgress {
                    node_id: trace_id.to_string(),
                    parent_id: None,
                    label: String::new(),
                    status: SubagentStatus::Failed,
                    round: None,
                    max_rounds: Some(max_rounds),
                    current_tool: None,
                    current_params: None,
                    action_log: Vec::new(),
                    text_snapshot: None,
                    started_at: started_at_ms,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    metadata: Some(SubagentMetadata {
                        token_count: None,
                        error: Some(format!(
                            "Timed out after {} seconds",
                            timeout_duration.as_secs()
                        )),
                        depends_on: vec![],
                    }),
                    progress_delta: None,
                    token_budget_k: None,
                    cumulative_tokens: 0,
                    error_details: Some(ErrorInfo {
                        error_type: ErrorType::Timeout,
                        message: format!("Timed out after {} seconds", timeout_duration.as_secs()),
                        last_tool: None,
                        last_params: None,
                        round: 0,
                        retryable: true,
                    }),
                    events: Vec::new(),
                });
            }
            tracing::error!(
                trace_id = trace_id,
                timeout_secs = timeout_duration.as_secs(),
                "Subagent: timed out"
            );
            Err(format!(
                "Subagent timed out after {} seconds",
                timeout_duration.as_secs()
            ))
        }
    }
}
