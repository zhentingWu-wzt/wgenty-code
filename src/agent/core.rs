//! Agent core — shared SSE stream processing and message construction.
//!
//! StreamProcessor handles the duplicated SSE parsing logic previously
//! found in both `cli::repl` and `cli::tui_repl`. Each frontend feeds
//! raw bytes into the processor and handles the resulting StreamEvents
//! in its own rendering model.

use crate::agent::events::{StreamEvent, StreamResult};
use crate::api::{ChatMessage, StreamChunk, ToolCall, Usage};

/// Processes raw SSE byte chunks into structured StreamEvents.
///
/// Usage:
/// 1. Call `feed_bytes()` with each chunk from the HTTP response stream.
/// 2. Handle each `StreamEvent` returned (render text, accumulate tool calls, etc.).
/// 3. When the stream ends, call `finish()` to get the final `StreamResult`.
pub struct StreamProcessor {
    /// Raw byte buffer to avoid UTF-8 corruption when multi-byte characters
    /// are split across SSE chunks. Only converted to String after finding
    /// a complete line (terminated by `\n`).
    buffer: Vec<u8>,
    full_content: String,
    reasoning_content: String,
    tool_calls_accum: Vec<serde_json::Value>,
    has_tool_calls: bool,
    finish_reason: String,
    /// Token usage captured from the final SSE chunk (OpenAI-compat) or
    /// the MessageDelta event (Anthropic). None if the API didn't report it.
    last_usage: Option<Usage>,
}

impl StreamProcessor {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            full_content: String::with_capacity(4096),
            reasoning_content: String::with_capacity(4096),
            tool_calls_accum: Vec::new(),
            has_tool_calls: false,
            finish_reason: String::new(),
            last_usage: None,
        }
    }

    /// Feed a raw byte chunk from the SSE stream. Returns any new events.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<StreamEvent> {
        self.buffer.extend_from_slice(bytes);
        self.drain_buffer()
    }

    /// Drain the internal line buffer, returning all pending events.
    fn drain_buffer(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            // Extract the complete line (up to but not including \n) and
            // convert to UTF-8 only after we know the line is complete.
            let line_bytes: Vec<u8> = self.buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim();

            if let Some(event) = self.process_line(line) {
                events.push(event);
            }
        }

        events
    }

    /// Process a single SSE text line, returning an event if one was produced.
    fn process_line(&mut self, line: &str) -> Option<StreamEvent> {
        // Detect daemon error events before SSE chunk parsing.
        // Daemon errors come as: data: {"error":"message"}
        let payload = line.strip_prefix("data: ").unwrap_or(line);
        if payload != "[DONE]" {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) {
                if let Some(error_msg) = parsed.get("error").and_then(|v| v.as_str()) {
                    return Some(StreamEvent::StreamError(error_msg.to_string()));
                }
            }
        }

        let chunk: StreamChunk = crate::api::parse_sse_line(line)?;

        // Capture usage from any chunk that reports it (typically the final one).
        if chunk.usage.is_some() {
            self.last_usage = chunk.usage;
        }

        let choice = chunk.choices.first()?;

        // Accumulate content/reasoning deltas FIRST, before checking
        // finish_reason — some providers send both in the same chunk,
        // and returning early on finish_reason would lose the content.
        if let Some(content) = &choice.delta.content {
            self.full_content.push_str(content);
        }
        if let Some(rc) = &choice.delta.reasoning_content {
            self.reasoning_content.push_str(rc);
        }

        // Process ALL tool call deltas (a chunk may contain multiple)
        if let Some(tc_deltas) = &choice.delta.tool_calls {
            self.has_tool_calls = true;
            for tc in tc_deltas {
                let idx = tc.index as usize;
                while self.tool_calls_accum.len() <= idx {
                    self.tool_calls_accum.push(serde_json::json!({
                        "id": null,
                        "type": "function",
                        "function": {"name": "", "arguments": ""}
                    }));
                }
                let entry = &mut self.tool_calls_accum[idx];
                if let Some(id) = &tc.id {
                    entry["id"] = serde_json::Value::String(id.clone());
                }
                if let Some(func) = &tc.function {
                    if let Some(name) = &func.name {
                        entry["function"]["name"] = serde_json::Value::String(name.clone());
                    }
                    if let Some(args) = &func.arguments {
                        if let Some(existing) = entry["function"]["arguments"].as_str() {
                            let mut combined = existing.to_string();
                            combined.push_str(args);
                            entry["function"]["arguments"] = serde_json::Value::String(combined);
                        }
                    }
                }
            }
        }

        // Check for finish reason AFTER accumulating all deltas
        if let Some(fr) = &choice.finish_reason {
            self.finish_reason = fr.clone();
            return Some(StreamEvent::StreamDone {
                finish_reason: fr.clone(),
            });
        }

        // Return the most significant delta as the event
        if let Some(content) = &choice.delta.content {
            return Some(StreamEvent::ContentDelta(content.clone()));
        }
        if let Some(rc) = &choice.delta.reasoning_content {
            return Some(StreamEvent::ReasoningDelta(rc.clone()));
        }
        if let Some(tc_deltas) = &choice.delta.tool_calls {
            if let Some(tc) = tc_deltas.first() {
                return Some(StreamEvent::ToolCallDelta {
                    index: tc.index as usize,
                    id: tc.id.clone(),
                    name: tc.function.as_ref().and_then(|f| f.name.clone()),
                    arguments: tc.function.as_ref().and_then(|f| f.arguments.clone()),
                });
            }
        }

        None
    }

    /// Flush any remaining buffered data and return events.
    pub fn flush(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            // Split on newlines in case there are multiple lines remaining
            for line_bytes in remaining.split(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(line_bytes);
                let line = line.trim();
                if !line.is_empty() {
                    if let Some(event) = self.process_line(line) {
                        events.push(event);
                    }
                }
            }
        }
        events
    }

    /// After streaming is complete, parse accumulated tool calls and return the result.
    pub fn finish(self) -> StreamResult {
        let tool_calls = self.parse_tool_calls();
        StreamResult {
            content: self.full_content,
            reasoning_content: self.reasoning_content,
            has_tool_calls: self.has_tool_calls,
            tool_calls,
            finish_reason: self.finish_reason,
            usage: self.last_usage,
        }
    }

    /// Parse accumulated tool call JSON into structured ToolCalls.
    fn parse_tool_calls(&self) -> Vec<ToolCall> {
        if !self.has_tool_calls {
            return Vec::new();
        }
        self.tool_calls_accum
            .iter()
            .filter_map(|call| {
                let id = call.get("id")?.as_str()?.to_string();
                let func = call.get("function")?;
                let name = func.get("name")?.as_str()?.to_string();
                let arguments = func.get("arguments")?.as_str()?.to_string();
                Some(ToolCall {
                    id,
                    r#type: "function".to_string(),
                    function: crate::api::ToolCallFunction { name, arguments },
                })
            })
            .collect()
    }

    /// Build an assistant ChatMessage from the accumulated content and tool calls.
    pub fn build_assistant_message(
        content: String,
        reasoning_content: String,
        tool_calls: Vec<ToolCall>,
    ) -> ChatMessage {
        let reasoning = if reasoning_content.is_empty() {
            None
        } else {
            Some(reasoning_content)
        };
        ChatMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            reasoning_content: reasoning,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
        }
    }
}

impl Default for StreamProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_delta() {
        let mut sp = StreamProcessor::new();
        let sse = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n";
        let events = sp.feed_bytes(sse.as_bytes());
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ContentDelta(text) => assert_eq!(text, "Hello"),
            _ => panic!("expected ContentDelta"),
        }
    }

    #[test]
    fn test_tool_call_delta() {
        let mut sp = StreamProcessor::new();
        let sse = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_read\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}]},\"finish_reason\":null}]}\n";
        let events = sp.feed_bytes(sse.as_bytes());
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                assert_eq!(*index, 0);
                assert_eq!(id.as_deref(), Some("call_1"));
                assert_eq!(name.as_deref(), Some("file_read"));
                assert!(arguments.is_some());
            }
            _ => panic!("expected ToolCallDelta"),
        }
    }

    #[test]
    fn test_finish_and_parse_tool_calls() {
        let mut sp = StreamProcessor::new();
        // Feed tool call delta
        sp.feed_bytes(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_read\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}]},\"finish_reason\":null}]}\n".as_bytes(),
        );
        // Feed finish
        sp.feed_bytes(
            "data: {\"id\":\"2\",\"object\":\"chat.completion.chunk\",\"created\":2,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n".as_bytes(),
        );

        let result = sp.finish();
        assert!(result.has_tool_calls);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].function.name, "file_read");
    }

    #[test]
    fn test_multiple_chunks() {
        let mut sp = StreamProcessor::new();
        let events = sp.feed_bytes(b"data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n");
        assert_eq!(events.len(), 1);

        let events =
            sp.feed_bytes(b"data: {\"id\":\"2\",\"object\":\"chat.completion.chunk\",\"created\":2,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n");
        assert_eq!(events.len(), 1);

        let result = sp.finish();
        assert_eq!(result.content, "Hello world");
    }

    #[test]
    fn test_done_line_ignored() {
        let mut sp = StreamProcessor::new();
        let events = sp.feed_bytes(b"data: [DONE]\n");
        assert!(events.is_empty());
    }
}
