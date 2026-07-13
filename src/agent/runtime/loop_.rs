//! Shared multi-round agent loop (stream → tools → compact → repeat).

use super::compaction::{micro_compact_messages, needs_compaction};
use super::config::RuntimeConfig;
use super::error::RuntimeError;
use super::events::RuntimeEvent;
use super::ports::{
    Compactor, EventSink, HistoryStore, InteractionPort, LlmPort, PlannerPort, RoundObserver,
    SynthesisPort, ToolPort, ToolRequest,
};
use super::stream::{stream_with_retry, StreamRetryOpts};
use super::timeout::resolve_tool_timeout;
use crate::agent::{StreamProcessor, StreamResult};
use crate::api::token_counter::TokenCounter;
use crate::api::ChatMessage;
use crate::utils::lenient_json::parse_tool_args_lenient;
use crate::utils::stuck_detector::{StuckDetector, StuckStatus};
use std::time::Duration;

/// How the loop talks to the model (daemon stream vs in-process stream/non-stream).
#[derive(Debug, Clone, Copy)]
pub struct StreamStyle {
    /// When true, pass [`ToolPort::definitions`] into the model request.
    /// Daemon TUI path leaves this false (tools injected server-side).
    pub use_tool_definitions: bool,
    /// When true, pass `config.max_tokens` to the stream request.
    pub pass_max_tokens: bool,
    /// When true, pass `Some(config.plan_mode)` as the plan_mode stream flag.
    pub pass_plan_mode: bool,
    /// Prefer non-streaming `LlmPort::chat_completion` (subagent path).
    pub prefer_non_stream: bool,
    /// Allow parallel multi-`task` batches (TUI). Subagents stay sequential.
    pub allow_parallel_tasks: bool,
}

impl Default for StreamStyle {
    fn default() -> Self {
        // In-process / headless defaults.
        Self {
            use_tool_definitions: true,
            pass_max_tokens: true,
            pass_plan_mode: true,
            prefer_non_stream: false,
            allow_parallel_tasks: true,
        }
    }
}

impl StreamStyle {
    /// Match the historical TUI → daemon stream call shape.
    pub fn tui_daemon() -> Self {
        Self {
            use_tool_definitions: false,
            pass_max_tokens: false,
            pass_plan_mode: false,
            prefer_non_stream: false,
            allow_parallel_tasks: true,
        }
    }

    /// Subagent: non-stream chat, tool defs in request, sequential tools.
    pub fn subagent() -> Self {
        Self {
            use_tool_definitions: true,
            pass_max_tokens: true,
            pass_plan_mode: false,
            prefer_non_stream: true,
            allow_parallel_tasks: false,
        }
    }
}

/// Max consecutive *irrecoverable* tool-arg JSON failures before aborting.
/// Recoverable lenient-parse cases (some fields extracted) do not count.
pub const MAX_CONSECUTIVE_PARSE_ERRORS: usize = 3;

/// Mutable flags for one turn of the shared loop.
#[derive(Debug, Default)]
pub struct LoopTurnState {
    pub compact_requested: bool,
    pub compaction_failed: bool,
    pub preparing_tools_fired: bool,
    pub rounds_since_plan: usize,
    pub compacted_summary: String,
    /// Consecutive tool rounds with irrecoverable JSON arg failures.
    pub consecutive_parse_errors: usize,
}

/// Optional capabilities wired by each frontend.
#[derive(Default)]
pub struct LoopHooks<'a> {
    pub compactor: Option<&'a dyn Compactor>,
    pub interaction: Option<&'a dyn InteractionPort>,
    pub planner: Option<&'a dyn PlannerPort>,
    pub stuck_detector: Option<&'a mut StuckDetector>,
    pub token_counter: Option<&'a TokenCounter>,
    pub synthesis: Option<&'a dyn SynthesisPort>,
    pub observer: Option<&'a dyn RoundObserver>,
}

/// Bundled arguments for [`run_agent_loop`] (keeps the free-function signature small).
pub struct RunLoopArgs<'a> {
    pub llm: &'a dyn LlmPort,
    pub tools: &'a dyn ToolPort,
    pub events: &'a dyn EventSink,
    pub history: &'a dyn HistoryStore,
    pub config: &'a RuntimeConfig,
    pub state: &'a mut LoopTurnState,
    pub stream_style: StreamStyle,
    pub hooks: LoopHooks<'a>,
}

/// Run the shared agent loop until the model stops calling tools or limits hit.
///
/// Returns the final assistant text (empty string when the turn ends without
/// textual content — e.g. plan-mode confirmation already streamed).
pub async fn run_agent_loop(args: RunLoopArgs<'_>) -> Result<String, RuntimeError> {
    let RunLoopArgs {
        llm,
        tools,
        events,
        history,
        config,
        state,
        stream_style,
        mut hooks,
    } = args;
    let mut llm_rounds = 0usize;
    let max_rounds = config.max_rounds;
    let warn_rounds = max_rounds * 8 / 10;

    loop {
        if llm_rounds >= max_rounds {
            let err = RuntimeError::MaxRoundsExceeded { max_rounds };
            events.emit(RuntimeEvent::StreamError(err.to_string()));
            if let Some(obs) = hooks.observer {
                let msgs = history.get().await;
                obs.on_failed(llm_rounds, &err.to_string(), &msgs);
            }
            return Err(err);
        }
        if llm_rounds == warn_rounds {
            tracing::warn!(
                rounds = llm_rounds,
                "Approaching max LLM rounds ({})",
                max_rounds
            );
        }

        let raw = history.get().await;
        let messages = micro_compact_messages(&raw);
        if let Some(obs) = hooks.observer {
            obs.on_round_start(llm_rounds + 1, &messages);
        }

        let want_compact = state.compact_requested
            || needs_compaction(&messages, config.context_window, config.max_tokens);
        state.compact_requested = false;
        if want_compact && !state.compaction_failed {
            if let Some(compactor) = hooks.compactor {
                events.emit(RuntimeEvent::CompactionStarted);
                if compactor.compact(history).await {
                    let compacted_raw = history.get().await;
                    let compacted = micro_compact_messages(&compacted_raw);
                    if needs_compaction(&compacted, config.context_window, config.max_tokens) {
                        tracing::warn!(
                            "compaction succeeded but history still exceeds the threshold; \
                             stopping retries to avoid an infinite compaction loop"
                        );
                        events.emit(RuntimeEvent::StreamError(
                            "Compaction ran but couldn't shrink the context below the threshold \
                             (the last exchange or system prompts are too large); sending the \
                             request anyway - it may fail if still too large."
                                .to_string(),
                        ));
                        state.compaction_failed = true;
                    }
                    continue;
                }
                state.compaction_failed = true;
            } else {
                events.emit(RuntimeEvent::StreamError(
                    "Context is large enough for compaction, but auto-summary is not \
                     available on this path; continuing with micro-compacted history."
                        .to_string(),
                ));
                state.compaction_failed = true;
            }
        }

        state.preparing_tools_fired = false;
        let tool_defs = if stream_style.use_tool_definitions && !config.plan_mode {
            let defs = tools.definitions();
            if defs.is_empty() {
                None
            } else {
                Some(defs)
            }
        } else {
            None
        };
        let max_tokens = if stream_style.pass_max_tokens {
            Some(config.max_tokens)
        } else {
            None
        };
        let plan_mode = if stream_style.pass_plan_mode && config.plan_mode {
            Some(true)
        } else {
            None
        };

        // Re-fetch after possible compaction.
        let raw = history.get().await;
        let messages = micro_compact_messages(&raw);

        let result = if stream_style.prefer_non_stream {
            match complete_non_stream(llm, events, &messages, tool_defs).await {
                Ok(r) => r,
                Err(e) => {
                    events.emit(RuntimeEvent::StreamError(e.to_string()));
                    if let Some(obs) = hooks.observer {
                        obs.on_failed(llm_rounds + 1, &e.to_string(), &messages);
                    }
                    return Err(e);
                }
            }
        } else {
            match stream_with_retry(
                llm,
                events,
                StreamRetryOpts {
                    messages: &messages,
                    tools: tool_defs,
                    preparing_tools_fired: &mut state.preparing_tools_fired,
                    max_retries: config.stream_max_retries,
                    max_tokens,
                    plan_mode,
                },
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    events.emit(RuntimeEvent::StreamError(e.to_string()));
                    return Err(e);
                }
            }
        };

        llm_rounds += 1;

        if let Some(ref usage) = result.usage {
            if let Some(counter) = hooks.token_counter {
                counter.add(usage.total_tokens);
                counter.add_output(usage.completion_tokens);
                counter.set_prompt_tokens(usage.prompt_tokens);
            }
            if let Some(obs) = hooks.observer {
                obs.on_usage(usage.total_tokens);
            }
        } else if let Some(counter) = hooks.token_counter {
            let input_est: usize = messages
                .iter()
                .map(|m| m.content.as_deref().unwrap_or("").len())
                .sum::<usize>()
                / 4;
            let output_est: usize = (result.content.len()
                + result
                    .tool_calls
                    .iter()
                    .map(|tc| tc.function.arguments.len())
                    .sum::<usize>())
                / 4;
            counter.add(input_est + output_est);
            counter.add_output(output_est);
            if let Some(obs) = hooks.observer {
                obs.on_usage(input_est + output_est);
            }
        }

        if result.has_tool_calls && !result.tool_calls.is_empty() {
            let assistant_msg = StreamProcessor::build_assistant_message(
                result.content.clone(),
                result.reasoning_content.clone(),
                result.tool_calls.clone(),
            );
            history.push(assistant_msg).await;

            let mut used_plan = false;

            let all_task = stream_style.allow_parallel_tasks
                && result.tool_calls.iter().all(|tc| {
                    matches!(
                        tc.function.name.as_str(),
                        "task" | "ask_user_question" | "update_plan" | "compact"
                    )
                })
                && result
                    .tool_calls
                    .iter()
                    .any(|tc| tc.function.name == "task")
                && result
                    .tool_calls
                    .iter()
                    .filter(|tc| tc.function.name == "task")
                    .count()
                    > 1;

            if all_task {
                // Non-task meta tools first, then parallel task execution.
                for tc in &result.tool_calls {
                    match tc.function.name.as_str() {
                        "ask_user_question" => {
                            let (args, _) = parse_tool_args_lenient(
                                &tc.function.arguments,
                                &tc.function.name,
                            );
                            let content = dispatch_ask(hooks.interaction, &args).await;
                            history.push(ChatMessage::tool(&tc.id, content)).await;
                        }
                        "update_plan" => {
                            used_plan = true;
                            let (args, _) = parse_tool_args_lenient(
                                &tc.function.arguments,
                                &tc.function.name,
                            );
                            events.emit(RuntimeEvent::PlanUpdate(args));
                            history
                                .push(ChatMessage::tool(
                                    &tc.id,
                                    serde_json::json!({"success":true,"message":"Plan updated"})
                                        .to_string(),
                                ))
                                .await;
                        }
                        "compact" => {
                            schedule_compact(events, state, history, &tc.id).await;
                        }
                        _ => {}
                    }
                }

                let task_calls: Vec<_> = result
                    .tool_calls
                    .iter()
                    .filter(|tc| tc.function.name == "task")
                    .collect();

                let mut tasks: Vec<(String, String, serde_json::Value)> = Vec::new();
                for tc in &task_calls {
                    let (args, parse_err) =
                        parse_tool_args_lenient(&tc.function.arguments, &tc.function.name);
                    if let Some(ref e) = parse_err {
                        let msg = serde_json::json!({
                            "success": false,
                            "error": format!(
                                "task tool call arguments are invalid JSON (likely truncated by max_tokens): {e}. Please re-issue the tool call."
                            ),
                        })
                        .to_string();
                        events.emit(RuntimeEvent::ToolResult {
                            name: "task".to_string(),
                            args: args.clone(),
                            content: msg.clone(),
                        });
                        history.push(ChatMessage::tool(&tc.id, msg)).await;
                        continue;
                    }
                    events.emit(RuntimeEvent::ToolStart {
                        name: "task".to_string(),
                        args: args.clone(),
                    });
                    tasks.push((tc.id.clone(), "task".to_string(), args));
                }

                let tool_timeout =
                    Duration::from_secs(config.subagent_timeout_secs.saturating_add(120));
                let session_id = config.session_id.clone();
                let turn_id = config.turn_id.clone();
                let handles: Vec<_> = tasks
                    .into_iter()
                    .map(|(id, name, args)| {
                        let session_id = session_id.clone();
                        let turn_id = turn_id.clone();
                        async move {
                            let result = tokio::time::timeout(
                                tool_timeout,
                                tools.execute(ToolRequest {
                                    name: name.clone(),
                                    arguments: args.clone(),
                                    session_id,
                                    turn_id,
                                    invocation_id: Some(id.clone()),
                                    parallel: true,
                                }),
                            )
                            .await;
                            let content = match result {
                                Ok(r) => r.content,
                                Err(_) => format!(
                                    r#"{{"success":false,"error":"Tool '{}' timed out after {}s"}}"#,
                                    name,
                                    tool_timeout.as_secs()
                                ),
                            };
                            events.emit(RuntimeEvent::ToolResult {
                                name,
                                args,
                                content: content.clone(),
                            });
                            (id, content)
                        }
                    })
                    .collect();

                for (id, content) in futures::future::join_all(handles).await {
                    history.push(ChatMessage::tool(&id, content)).await;
                }
            } else {
                // Sequential path — supports recoverable lenient JSON (subagent).
                let mut had_parse_error_this_round = false;
                for tc in &result.tool_calls {
                    let raw_args = &tc.function.arguments;
                    let (args, parse_err) =
                        parse_tool_args_lenient(raw_args, &tc.function.name);

                    // Classify parse outcomes like the historical subagent loop:
                    // recoverable (some real fields) → still execute; irrecoverable
                    // → skip execute, count toward abort threshold.
                    let recovered = recovered_useful_fields(&args);

                    if let Some(ref e) = parse_err {
                        had_parse_error_this_round = true;
                        if recovered {
                            tracing::info!(
                                tool = %tc.function.name,
                                error = %e,
                                consecutive = state.consecutive_parse_errors,
                                "tool arguments recovered via lenient parser"
                            );
                        } else {
                            state.consecutive_parse_errors =
                                state.consecutive_parse_errors.saturating_add(1);
                            tracing::warn!(
                                tool = %tc.function.name,
                                error = %e,
                                consecutive = state.consecutive_parse_errors,
                                "tool call arguments irrecoverable (no fields extracted)"
                            );
                        }
                    } else {
                        state.consecutive_parse_errors = 0;
                    }

                    if state.consecutive_parse_errors >= MAX_CONSECUTIVE_PARSE_ERRORS {
                        let msg = format!(
                            "Aborted: {} consecutive tool calls had irrecoverable JSON errors. \
                             The model may be generating severely malformed tool arguments.",
                            state.consecutive_parse_errors
                        );
                        events.emit(RuntimeEvent::StreamError(msg.clone()));
                        if let Some(obs) = hooks.observer {
                            let msgs = history.get().await;
                            obs.on_failed(llm_rounds, &msg, &msgs);
                        }
                        return Err(RuntimeError::Stream(msg));
                    }

                    // Irrecoverable: don't execute with empty/garbage args.
                    if let Some(ref e) = parse_err {
                        if !recovered {
                            let mut content = format!(
                                "Error: tool call arguments are invalid JSON (likely truncated by max_tokens): {e}. Please re-issue the tool call."
                            );
                            content.push_str(&parse_error_guidance(
                                &tc.function.name,
                                e,
                                raw_args,
                            ));
                            events.emit(RuntimeEvent::ToolResult {
                                name: tc.function.name.clone(),
                                args: args.clone(),
                                content: content.clone(),
                            });
                            history.push(ChatMessage::tool(&tc.id, content)).await;
                            continue;
                        }
                    }

                    match tc.function.name.as_str() {
                        "ask_user_question" => {
                            let content = dispatch_ask(hooks.interaction, &args).await;
                            history.push(ChatMessage::tool(&tc.id, content)).await;
                            continue;
                        }
                        "update_plan" => {
                            used_plan = true;
                            events.emit(RuntimeEvent::PlanUpdate(args.clone()));
                            history
                                .push(ChatMessage::tool(
                                    &tc.id,
                                    serde_json::json!({"success":true,"message":"Plan updated"})
                                        .to_string(),
                                ))
                                .await;
                            continue;
                        }
                        "compact" => {
                            schedule_compact(events, state, history, &tc.id).await;
                            continue;
                        }
                        _ => {}
                    }

                    events.emit(RuntimeEvent::ToolStart {
                        name: tc.function.name.clone(),
                        args: args.clone(),
                    });
                    if let Some(obs) = hooks.observer {
                        let msgs = history.get().await;
                        obs.on_tool_start(llm_rounds, &tc.function.name, &msgs);
                    }

                    let tool_timeout = resolve_tool_timeout(
                        &tc.function.name,
                        &args,
                        config.subagent_timeout_secs,
                    );
                    let mut exec_result = match tokio::time::timeout(
                        tool_timeout,
                        tools.execute(ToolRequest {
                            name: tc.function.name.clone(),
                            arguments: args.clone(),
                            session_id: config.session_id.clone(),
                            turn_id: config.turn_id.clone(),
                            invocation_id: Some(tc.id.clone()),
                            parallel: false,
                        }),
                    )
                    .await
                    {
                        Ok(r) => r.content,
                        Err(_) => {
                            let msg = format!(
                                r#"{{"success":false,"error":"Tool '{}' timed out after {}s"}}"#,
                                tc.function.name,
                                tool_timeout.as_secs()
                            );
                            events.emit(RuntimeEvent::ToolResult {
                                name: tc.function.name.clone(),
                                args: args.clone(),
                                content: msg.clone(),
                            });
                            history.push(ChatMessage::tool(&tc.id, msg)).await;
                            continue;
                        }
                    };

                    // Recoverable parse: still ran the tool; inject guidance so
                    // the model can self-correct on the next attempt.
                    if let Some(ref e) = parse_err {
                        exec_result.push_str(&parse_error_guidance(
                            &tc.function.name,
                            e,
                            raw_args,
                        ));
                    }

                    events.emit(RuntimeEvent::ToolResult {
                        name: tc.function.name.clone(),
                        args,
                        content: exec_result.clone(),
                    });
                    history
                        .push(ChatMessage::tool(&tc.id, exec_result))
                        .await;
                }

                if had_parse_error_this_round {
                    history
                        .push(ChatMessage::user(
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
                             </system-reminder>",
                        ))
                        .await;
                }
            }

            state.rounds_since_plan = if used_plan {
                0
            } else {
                state.rounds_since_plan.saturating_add(1)
            };
            if state.rounds_since_plan >= 3 {
                append_to_last_tool(
                    history,
                    "\n<reminder>Update your plan with update_plan.</reminder>",
                )
                .await;
            }

            if let Some(detector) = hooks.stuck_detector.as_mut() {
                match detector.record_round(&result.tool_calls) {
                    StuckStatus::Warn(msg) => {
                        append_to_last_tool(history, &msg).await;
                    }
                    StuckStatus::Abort(msg) => {
                        events.emit(RuntimeEvent::StreamError(msg.clone()));
                        return Err(RuntimeError::Stream(msg));
                    }
                    StuckStatus::Ok => {}
                }
            }

            events.emit(RuntimeEvent::SaveSession);
            continue;
        }

        // ── No tool calls ───────────────────────────────────────────────
        if config.plan_mode {
            let confirmation = "\n\n---\nPlan generated. Reply with **execute** to proceed, or describe any changes you'd like.";
            let is_plan_reply = {
                let hist = history.get().await;
                hist.last()
                    .map(|m| {
                        m.role == "assistant"
                            && m.content
                                .as_deref()
                                .map(|c| c.contains("Reply with **execute** to proceed"))
                                .unwrap_or(false)
                    })
                    .unwrap_or(false)
            };
            if !is_plan_reply {
                if let Some(planner) = hooks.planner {
                    events.emit(RuntimeEvent::ContentDelta(
                        "(generating plan)...".to_string(),
                    ));
                    match planner.plan(&messages).await {
                        Ok(plan) => {
                            events.emit(RuntimeEvent::ContentDelta(plan.clone()));
                            events.emit(RuntimeEvent::ContentDelta(confirmation.to_string()));
                            events.emit(RuntimeEvent::StreamDone {
                                finish_reason: "stop".to_string(),
                            });
                            history
                                .push(ChatMessage {
                                    role: "assistant".to_string(),
                                    content: Some(format!("{}{}", plan, confirmation)),
                                    reasoning_content: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                })
                                .await;
                            events.emit(RuntimeEvent::SaveSession);
                            return Ok(plan);
                        }
                        Err(e) => {
                            events.emit(RuntimeEvent::StreamError(e.clone()));
                            return Err(RuntimeError::Planner(e));
                        }
                    }
                } else {
                    // Content already streamed; append confirmation.
                    events.emit(RuntimeEvent::ContentDelta(confirmation.to_string()));
                    let final_text = format!("{}{}", result.content, confirmation);
                    history
                        .push(ChatMessage {
                            role: "assistant".to_string(),
                            content: Some(final_text.clone()),
                            reasoning_content: if result.reasoning_content.is_empty() {
                                None
                            } else {
                                Some(result.reasoning_content.clone())
                            },
                            tool_calls: None,
                            tool_call_id: None,
                        })
                        .await;
                    events.emit(RuntimeEvent::StreamDone {
                        finish_reason: result.finish_reason.clone(),
                    });
                    events.emit(RuntimeEvent::SaveSession);
                    return Ok(result.content);
                }
            }
            // Plan reply — fall through to treat as normal final response.
        }

        if result.content.is_empty() && !result.has_tool_calls && result.finish_reason.is_empty() {
            events.emit(RuntimeEvent::StreamError(
                "Received empty response from API. Please check your API key, model name, and network connectivity.".to_string()
            ));
            return Err(RuntimeError::EmptyResponse);
        }

        // Push assistant turn first so synthesis can see it in history.
        if !result.content.is_empty() || !result.reasoning_content.is_empty() {
            let reasoning = if result.reasoning_content.is_empty() {
                None
            } else {
                Some(result.reasoning_content.clone())
            };
            history
                .push(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(result.content.clone()),
                    reasoning_content: reasoning,
                    tool_calls: None,
                    tool_call_id: None,
                })
                .await;
        }

        // Non-root subagent synthesis barrier: inject child results and continue.
        if let Some(synth) = hooks.synthesis {
            match synth.on_candidate_final(&result.content).await {
                Ok(Some(system_msg)) => {
                    history.push(ChatMessage::system(system_msg)).await;
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    events.emit(RuntimeEvent::StreamError(e.to_string()));
                    if let Some(obs) = hooks.observer {
                        let msgs = history.get().await;
                        obs.on_failed(llm_rounds, &e.to_string(), &msgs);
                    }
                    return Err(e);
                }
            }
        }

        events.emit(RuntimeEvent::StreamDone {
            finish_reason: result.finish_reason,
        });
        events.emit(RuntimeEvent::SaveSession);
        if let Some(obs) = hooks.observer {
            let msgs = history.get().await;
            obs.on_completed(llm_rounds, &msgs);
        }
        return Ok(result.content);
    }
}

async fn complete_non_stream(
    llm: &dyn LlmPort,
    events: &dyn EventSink,
    messages: &[ChatMessage],
    tools: Option<Vec<crate::api::ToolDefinition>>,
) -> Result<StreamResult, RuntimeError> {
    events.emit(RuntimeEvent::Connecting {
        attempt: 1,
        max_retries: 1,
    });
    let completion = llm.chat_completion(messages.to_vec(), tools).await?;
    let content = completion.message.content.clone().unwrap_or_default();
    let reasoning = completion
        .message
        .reasoning_content
        .clone()
        .unwrap_or_default();
    let tool_calls = completion.message.tool_calls.clone().unwrap_or_default();
    let has_tool_calls = !tool_calls.is_empty()
        || completion.finish_reason == "tool_calls";

    if !content.is_empty() {
        events.emit(RuntimeEvent::ContentDelta(content.clone()));
    }
    if !reasoning.is_empty() {
        events.emit(RuntimeEvent::ReasoningDelta(reasoning.clone()));
    }
    if has_tool_calls {
        events.emit(RuntimeEvent::PreparingTools);
    }
    events.emit(RuntimeEvent::StreamDone {
        finish_reason: completion.finish_reason.clone(),
    });

    Ok(StreamResult {
        content,
        reasoning_content: reasoning,
        tool_calls,
        has_tool_calls,
        finish_reason: completion.finish_reason,
        usage: completion.usage,
    })
}

async fn dispatch_ask(interaction: Option<&dyn InteractionPort>, args: &serde_json::Value) -> String {
    if let Some(port) = interaction {
        port.ask_user_question(args).await
    } else {
        serde_json::json!({
            "success": false,
            "error": "ask_user_question is not available on this path"
        })
        .to_string()
    }
}

fn parse_error_guidance(tool_name: &str, err: &str, raw_args: &str) -> String {
    let preview: String = raw_args.chars().take(500).collect();
    format!(
        "\n\n---\n## ⚠️ Tool Argument Parse Warning\n\
         Your previous tool call to `{tool_name}` had malformed JSON arguments.\n\
         **Parse error**: {err}\n\
         **Raw arguments received** (may be truncated):\n```json\n{preview}\n```\n\
         **Please retry** with properly escaped JSON. Common issues:\n\
         - Regex patterns with backslashes: use `\\\\` instead of `\\`\n\
         - Quotes inside patterns: use `\\\"` instead of `\"`\n\
         - Ensure all strings are properly closed with `\"`"
    )
}

/// Whether lenient-parse recovered at least one non-internal field.
fn recovered_useful_fields(args: &serde_json::Value) -> bool {
    args.as_object()
        .map(|obj| obj.keys().any(|k| !k.starts_with('_')))
        .unwrap_or(false)
}

async fn schedule_compact(
    events: &dyn EventSink,
    state: &mut LoopTurnState,
    history: &dyn HistoryStore,
    tool_call_id: &str,
) {
    events.emit(RuntimeEvent::ToolStart {
        name: "compact".to_string(),
        args: serde_json::json!({}),
    });
    state.compact_requested = true;
    events.emit(RuntimeEvent::ToolResult {
        name: "compact".to_string(),
        args: serde_json::json!({}),
        content: "Conversation will be compacted before the next step.".to_string(),
    });
    history
        .push(ChatMessage::tool(
            tool_call_id,
            r#"{"success":true,"content":"Compaction scheduled"}"#,
        ))
        .await;
}

async fn append_to_last_tool(history: &dyn HistoryStore, suffix: &str) {
    let mut msgs = history.get().await;
    if let Some(last) = msgs.last_mut() {
        if last.role == "tool" {
            if let Some(ref mut content) = last.content {
                content.push_str(suffix);
            }
        }
    }
    history.replace(msgs).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recovered_useful_fields_true_when_real_keys_present() {
        assert!(recovered_useful_fields(&json!({"path": "a.rs"})));
        assert!(recovered_useful_fields(&json!({"pattern": "x", "_partial": true})));
    }

    #[test]
    fn recovered_useful_fields_false_for_empty_or_internal_only() {
        assert!(!recovered_useful_fields(&json!({})));
        assert!(!recovered_useful_fields(&json!({"_error": "x"})));
        assert!(!recovered_useful_fields(&json!(null)));
    }

    #[test]
    fn parse_error_guidance_mentions_tool_and_error() {
        let g = parse_error_guidance("grep", "expected `,`", r#"{"pattern":"\d"#);
        assert!(g.contains("grep"));
        assert!(g.contains("expected `,`"));
        assert!(g.contains("Tool Argument Parse Warning"));
    }

    #[test]
    fn max_consecutive_parse_errors_is_three() {
        assert_eq!(MAX_CONSECUTIVE_PARSE_ERRORS, 3);
    }
}
