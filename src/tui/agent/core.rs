use super::AgentLoop;
use crate::agent::StreamProcessor;
use crate::api::ChatMessage;
use crate::tui::app::AppEvent;
use crate::utils::stuck_detector::StuckStatus;
use std::time::Duration;

impl AgentLoop {
    /// Core LLM loop: stream, handle tools, compaction, etc.
    pub(super) async fn run_agent_loop(&mut self) -> Result<(), String> {
        let mut llm_rounds = 0;
        let max_rounds = self.max_rounds;
        let warn_rounds = max_rounds * 8 / 10;
        loop {
            if llm_rounds >= max_rounds {
                let _ = self.event_tx.send(AppEvent::StreamError(
                    format!("Agent exceeded {} LLM rounds", max_rounds)
                ));
                return Err(format!("Exceeded {} LLM rounds", max_rounds));
            }
            if llm_rounds == warn_rounds {
                tracing::warn!(
                    rounds = llm_rounds,
                    "Approaching max LLM rounds ({})", max_rounds
                );
            }

            let messages = self.micro_compact().await;

            if self.needs_compaction(&messages) {
                self.do_auto_compact().await;
                continue;
            }

            // Check token budget before each LLM call
            if self.token_counter.is_exhausted() {
                let msg = format!(
                    "Token budget exhausted ({}k). Type /continue to resume or increase token_budget_k in settings.json.",
                    self.token_counter.budget_tokens() / 1000
                );
                let _ = self.event_tx.send(AppEvent::StreamError(msg.clone()));
                return Err(msg);
            }

            self.preparing_tools_fired = false;
            let result = match self.stream_with_retry(&messages).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = self.event_tx.send(AppEvent::StreamError(e.to_string()));
                    return Err(e.to_string());
                }
            };

            llm_rounds += 1;

            // Estimate tokens consumed this round (prompt + completion)
            let input_est: usize = messages.iter()
                .map(|m| m.content.as_deref().unwrap_or("").len())
                .sum::<usize>() * 2 / 3; // ~1.5 chars per token
            let output_est: usize = (result.content.len()
                + result.tool_calls.iter()
                    .map(|tc| tc.function.arguments.len())
                    .sum::<usize>()) * 2 / 3;
            self.token_counter.add(input_est + output_est);

            // Check budget after accounting
            if self.token_counter.is_exhausted() {
                let msg = format!(
                    "Token budget exhausted ({}k). Type /continue to resume or increase token_budget_k in settings.json.",
                    self.token_counter.budget_tokens() / 1000
                );
                let _ = self.event_tx.send(AppEvent::StreamError(msg.clone()));
                return Err(msg);
            }

            if result.has_tool_calls && !result.tool_calls.is_empty() {
                let assistant_msg = StreamProcessor::build_assistant_message(
                    result.content,
                    result.reasoning_content,
                    result.tool_calls.clone(),
                );
                {
                    let mut history = self.conversation_history.lock().await;
                    history.push(assistant_msg);
                }

                let mut used_todo = false;

                // Fire all task tools in parallel when every executable tool is a task.
                let all_task = result.tool_calls.iter().all(|tc| {
                    matches!(tc.function.name.as_str(), "task" | "TodoWrite" | "ask_user_question" | "update_plan" | "compact")
                }) && result.tool_calls.iter().any(|tc| tc.function.name == "task")
                  && result.tool_calls.iter().filter(|tc| tc.function.name == "task").count() > 1;

                if all_task {
                    // ── Parallel task execution ──────────────────────────
                    let client = self.client.clone();
                    let session_id = self.session_id.clone();
                    let event_tx = self.event_tx.clone();
                    let history = self.conversation_history.clone();

                    // Handle non-task tools first (ask, plan, compact, todo)
                    for tc in &result.tool_calls {
                        match tc.function.name.as_str() {
                            "TodoWrite" => used_todo = true,
                            "ask_user_question" => {
                                let (args, _) = crate::utils::lenient_json::parse_tool_args_lenient(
                                    &tc.function.arguments, &tc.function.name,
                                );
                                let result = self.handle_ask_user_question(&args).await;
                                history.lock().await.push(ChatMessage::tool(&tc.id, result));
                            }
                            "update_plan" => {
                                let (args, _) = crate::utils::lenient_json::parse_tool_args_lenient(
                                    &tc.function.arguments, &tc.function.name,
                                );
                                let _ = event_tx.send(AppEvent::PlanUpdate(args.clone()));
                                history.lock().await.push(ChatMessage::tool(
                                    &tc.id,
                                    serde_json::json!({"success":true,"message":"Plan updated"}).to_string(),
                                ));
                            }
                            "compact" => {
                                let _ = event_tx.send(AppEvent::ToolStart {
                                    name: "compact".to_string(),
                                    args: serde_json::json!({}),
                                });
                                self.do_auto_compact().await;
                                let _ = event_tx.send(AppEvent::ToolResult {
                                    name: "compact".to_string(),
                                    args: serde_json::json!({}),
                                    content: "Conversation history compressed.".to_string(),
                                });
                                history.lock().await.push(ChatMessage::tool(
                                    &tc.id,
                                    r#"{"success":true,"content":"Conversation compressed"}"#,
                                ));
                            }
                            _ => {} // task tools handled below
                        }
                    }

                    // Collect and fire all task tools in parallel
                    let task_calls: Vec<_> = result.tool_calls.iter()
                        .filter(|tc| tc.function.name == "task")
                        .collect();

                    let mut tasks: Vec<(String, String, serde_json::Value)> = Vec::new();
                    for tc in &task_calls {
                        let (args, _) = crate::utils::lenient_json::parse_tool_args_lenient(
                            &tc.function.arguments,
                            &tc.function.name,
                        );
                        let _ = event_tx.send(AppEvent::ToolStart {
                            name: "task".to_string(),
                            args: args.clone(),
                        });
                        tasks.push((tc.id.clone(), "task".to_string(), args));
                    }

                    let handles: Vec<_> = tasks.into_iter().map(|(id, name, args)| {
                        let client = client.clone();
                        let session_id = session_id.clone();
                        let event_tx = event_tx.clone();
                        tokio::spawn(async move {
                            let result = tokio::time::timeout(
                                Duration::from_secs(300),
                                Self::execute_tool_static(&client, &name, args.clone(), &session_id, Some(event_tx.clone())),
                            ).await;
                            let content = match result {
                                Ok(r) => r,
                                Err(_) => format!(
                                    r#"{{"success":false,"error":"Tool '{}' timed out after 300s"}}"#,
                                    name
                                ),
                            };
                            let _ = event_tx.send(AppEvent::ToolResult {
                                name,
                                args,
                                content: content.clone(),
                            });
                            (id, content)
                        })
                    }).collect();

                    let results = futures::future::join_all(handles).await;
                    let mut history = history.lock().await;
                    for (id, content) in results.into_iter().flatten() {
                        history.push(ChatMessage::tool(&id, content));
                    }
                } else {
                    // ── Sequential execution (original path) ─────────────
                    for tc in &result.tool_calls {
                        let (args, parse_err) = crate::utils::lenient_json::parse_tool_args_lenient(
                            &tc.function.arguments,
                            &tc.function.name,
                        );
                        if let Some(ref e) = parse_err {
                            tracing::warn!(
                                tool = %tc.function.name,
                                error = %e,
                                args_len = tc.function.arguments.len(),
                                "Tool call arguments parse issue (lenient recovery attempted)"
                            );
                        }

                        if tc.function.name == "ask_user_question" {
                            let tool_result = self.handle_ask_user_question(&args).await;
                            {
                                let mut history = self.conversation_history.lock().await;
                                history.push(ChatMessage::tool(&tc.id, tool_result));
                            }
                            continue;
                        }

                        if tc.function.name == "update_plan" {
                            let _ = self.event_tx.send(AppEvent::PlanUpdate(args.clone()));
                            {
                                let mut history = self.conversation_history.lock().await;
                                history.push(ChatMessage::tool(
                                    &tc.id,
                                    serde_json::json!({"success":true,"message":"Plan updated"}).to_string(),
                                ));
                            }
                            continue;
                        }

                        if tc.function.name == "compact" {
                            let _ = self.event_tx.send(AppEvent::ToolStart {
                                name: "compact".to_string(),
                                args: serde_json::json!({}),
                            });
                            self.do_auto_compact().await;
                            let _ = self.event_tx.send(AppEvent::ToolResult {
                                name: "compact".to_string(),
                                args: serde_json::json!({}),
                                content: "Conversation history compressed.".to_string(),
                            });
                            let mut history = self.conversation_history.lock().await;
                            history.push(ChatMessage::tool(
                                &tc.id,
                                r#"{"success":true,"content":"Conversation compressed"}"#,
                            ));
                            continue;
                        }

                        if tc.function.name == "TodoWrite" {
                            used_todo = true;
                        }

                        let _ = self.event_tx.send(AppEvent::ToolStart {
                            name: tc.function.name.clone(),
                            args: args.clone(),
                        });

                        // Spawn progress poller for task/delegate tools in sequential path
                        let poll_handle = if (tc.function.name == "task" || tc.function.name == "delegate")
                            && self.event_tx.is_closed() == false
                        {
                            let tx = self.event_tx.clone();
                            let client = self.client.clone();
                            Some(tokio::spawn(async move {
                                let start = tokio::time::Instant::now();
                                let max_duration = Duration::from_secs(120);
                                loop {
                                    if start.elapsed() > max_duration {
                                        break;
                                    }
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    match client.poll_subagent_progress().await {
                                        Ok(map) => {
                                            for (_id, progress) in map {
                                                let _ = tx.send(AppEvent::SubagentUpdate(progress));
                                            }
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }))
                        } else {
                            None
                        };

                        let tool_timeout = if tc.function.name == "task" {
                            Duration::from_secs(300)
                        } else {
                            Duration::from_secs(120)
                        };
                        let exec_result = match tokio::time::timeout(
                            tool_timeout,
                            self.execute_tool_with_permission(&tc.function.name, args.clone()),
                        ).await {
                            Ok(result) => result,
                            Err(_elapsed) => {
                                let msg = format!(
                                    r#"{{"success":false,"error":"Tool '{}' timed out after {}s"}}"#,
                                    tc.function.name,
                                    tool_timeout.as_secs()
                                );
                                let _ = self.event_tx.send(AppEvent::ToolResult {
                                    name: tc.function.name.clone(),
                                    args: args.clone(),
                                    content: msg.clone(),
                                });
                                {
                                    let mut history = self.conversation_history.lock().await;
                                    history.push(ChatMessage::tool(&tc.id, msg));
                                }
                                if let Some(h) = poll_handle {
                                    h.abort();
                                }
                                continue;
                            }
                        };

                        // Drop poll handle (poller self-terminates after 120s max)
                        drop(poll_handle);

                        let _ = self.event_tx.send(AppEvent::ToolResult {
                            name: tc.function.name.clone(),
                            args: args.clone(),
                            content: exec_result.clone(),
                        });

                        {
                            let mut history = self.conversation_history.lock().await;
                            history.push(ChatMessage::tool(&tc.id, exec_result));
                        }
                    }
                }

                self.rounds_since_todo = if used_todo {
                    0
                } else {
                    self.rounds_since_todo + 1
                };
                if self.rounds_since_todo >= 3 {
                    let mut history = self.conversation_history.lock().await;
                    if let Some(last) = history.last_mut() {
                        if last.role == "tool" {
                            if let Some(ref mut content) = last.content {
                                content.push_str(
                                    "\n<reminder>Update your todos with TodoWrite.</reminder>",
                                );
                            }
                        }
                    }
                }

                // ── Stuck detection ─────────────────────────────────
                match self.stuck_detector.record_round(&result.tool_calls) {
                    StuckStatus::Warn(msg) => {
                        let mut history = self.conversation_history.lock().await;
                        if let Some(last) = history.last_mut() {
                            if last.role == "tool" {
                                if let Some(ref mut c) = last.content {
                                    c.push_str(&msg);
                                }
                            }
                        }
                    }
                    StuckStatus::Abort(msg) => {
                        let _ = self.event_tx.send(AppEvent::StreamError(msg.clone()));
                        return Err(msg);
                    }
                    StuckStatus::Ok => {}
                }

                let _ = self.event_tx.send(AppEvent::SaveSession);
                continue;
            }

            if self.plan_mode {
                let confirmation = "\n\n---\nPlan generated. Reply with **execute** to proceed, or describe any changes you'd like.";
                if let Some(ref planner) = self.planner_client {
                    let _ = self.event_tx.send(AppEvent::ContentDelta("(generating plan)...".to_string()));
                    match self.plan_with_model(planner, &messages).await {
                        Ok(plan) => {
                            let _ = self.event_tx.send(AppEvent::ContentDelta(plan.clone()));
                            let _ = self.event_tx.send(AppEvent::ContentDelta(confirmation.to_string()));
                            let _ = self.event_tx.send(AppEvent::StreamDone { finish_reason: "stop".to_string() });
                            {
                                let mut history = self.conversation_history.lock().await;
                                history.push(ChatMessage {
                                    role: "assistant".to_string(),
                                    content: Some(format!("{}{}", plan, confirmation)),
                                    reasoning_content: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                });
                            }
                        }
                        Err(e) => {
                            let _ = self.event_tx.send(AppEvent::StreamError(e.clone()));
                            return Err(e);
                        }
                    }
                } else {
                    // Content already streamed via ContentDelta during stream_with_retry.
                    // Just append confirmation prompt and finish.
                    let _ = self.event_tx.send(AppEvent::ContentDelta(confirmation.to_string()));
                    {
                        let mut history = self.conversation_history.lock().await;
                        history.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: Some(format!("{}{}", result.content, confirmation)),
                            reasoning_content: Some(result.reasoning_content.clone()),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                    let _ = self.event_tx.send(AppEvent::StreamDone { finish_reason: result.finish_reason.clone() });
                }
                let _ = self.event_tx.send(AppEvent::SaveSession);
                return Ok(());
            }

            if !result.content.is_empty() {
                let reasoning = if result.reasoning_content.is_empty() {
                    None
                } else {
                    Some(result.reasoning_content)
                };
                {
                    let mut history = self.conversation_history.lock().await;
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: Some(result.content),
                        reasoning_content: reasoning,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }

            let _ = self.event_tx.send(AppEvent::StreamDone {
                finish_reason: result.finish_reason,
            });
            let _ = self.event_tx.send(AppEvent::SaveSession);
            return Ok(());
        }
    }
}
