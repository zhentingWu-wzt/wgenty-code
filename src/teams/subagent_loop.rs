//! Subagent Loop — isolated agent loop for subagent execution.
//!
//! Each subagent gets its own `messages=[]` context (no shared conversation history
//! with the parent), runs a complete multi-round tool-use loop, and returns the
//! final assistant response back to the caller.

use crate::api::{ApiClient, ChatMessage, ToolDefinition};
use crate::tools::ToolRegistry;
use std::sync::atomic::{AtomicU64, Ordering};

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
/// * `max_rounds`      — Maximum tool-use iterations (cap at 10 to prevent runaway).
///
/// # Returns
/// * `Ok(String)` — The final assistant content (text response).
/// * `Err(String)` — An error description if the loop fails or times out.
pub async fn run_subagent_loop(
    api_client: &ApiClient,
    tool_registry: &ToolRegistry,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
) -> Result<String, String> {
    tracing::info!(
        target: "subagent",
        prompt_len = user_prompt.len(),
        tool_count = allowed_tools.len(),
        max_rounds = max_rounds,
        "Subagent: starting agent loop"
    );
    static SUBAGENT_COUNTER: AtomicU64 = AtomicU64::new(0);
    let trace_id = SUBAGENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    tracing::info!(target: "subagent", trace_id = trace_id, "Subagent: trace context");

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

    for round in 0..max_rounds {
        // Call the API with current message history and filtered tools.
        tracing::debug!(
            target: "subagent",
            round = round,
            message_count = messages.len(),
            "Subagent: calling API"
        );

        let response = api_client
            .chat(
                messages.clone(),
                if has_tools {
                    Some(tool_defs.clone())
                } else {
                    None
                },
            )
            .await
            .map_err(|e| format!("Subagent API call failed: {}", e))?;

        let choice =
            response.choices.into_iter().next().ok_or_else(|| {
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
            // The model responded with a final text answer — we are done.
            return Ok(choice.message.content.unwrap_or_default());
        }

        // Execute each tool call and push results back as tool-result messages.
        for tool_call in tool_calls.unwrap() {
            let tool_name = &tool_call.function.name;
            let (args, parse_err) = crate::utils::lenient_json::parse_tool_args_lenient(
                &tool_call.function.arguments,
                tool_name,
            );
            if let Some(ref e) = parse_err {
                tracing::warn!(
                    tool = %tool_name,
                    error = %e,
                    "Subagent: tool call arguments parse issue (lenient recovery attempted)"
                );
            }

            tracing::debug!(
                    target: "subagent",
                    tool = %tool_name,
                    round = round,
                    "Subagent: executing tool"
                );
            let result = tool_registry.execute(tool_name, args).await;

            let content = match result {
                Ok(output) => output.content,
                Err(e) => format!("Error: {}", e.message),
            };

            messages.push(ChatMessage::tool(&tool_call.id, content));
        }
    }

    tracing::info!(
        target: "subagent",
        max_rounds = max_rounds,
        "Subagent: exceeded max rounds"
    );
    Err("Subagent exceeded maximum number of rounds".to_string())
}
