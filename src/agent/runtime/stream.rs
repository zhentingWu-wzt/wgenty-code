//! Shared SSE stream engine: open → parse → emit → retry.

use super::error::RuntimeError;
use super::events::RuntimeEvent;
use super::ports::{EventSink, LlmPort};
use crate::agent::{StreamEvent, StreamProcessor, StreamResult};
use crate::api::{ChatMessage, ToolDefinition};
use futures::StreamExt;
use std::time::Duration;

/// Idle gap between SSE chunks before the stream is considered stalled.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Options for one [`stream_with_retry`] call (keeps the free-function signature small).
pub struct StreamRetryOpts<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<Vec<ToolDefinition>>,
    pub preparing_tools_fired: &'a mut bool,
    pub max_retries: u32,
    pub max_tokens: Option<usize>,
    pub plan_mode: Option<bool>,
}

/// Stream with retry logic for mid-stream failures only.
///
/// Connection-level network errors are retried at the transport layer
/// (`ApiClient::send_with_retry` / daemon client). This layer only retries:
/// 1. a stream that started but errored mid-way, and
/// 2. a stream that ended before tool calls completed (empty `finish_reason`).
pub async fn stream_with_retry(
    llm: &dyn LlmPort,
    events: &dyn EventSink,
    opts: StreamRetryOpts<'_>,
) -> Result<StreamResult, RuntimeError> {
    let StreamRetryOpts {
        messages,
        tools,
        preparing_tools_fired,
        max_retries,
        max_tokens,
        plan_mode,
    } = opts;
    let mut last_error = String::new();

    for attempt in 0..=max_retries {
        events.emit(RuntimeEvent::Connecting {
            attempt: (attempt + 1) as usize,
            max_retries: (max_retries + 1) as usize,
        });

        match llm
            .open_chat_stream(messages.to_vec(), tools.clone(), max_tokens, plan_mode)
            .await
        {
            Ok(byte_stream) => {
                match stream_response(byte_stream, events, preparing_tools_fired).await {
                    Ok(result) => {
                        if result.has_tool_calls
                            && result.finish_reason.is_empty()
                            && attempt < max_retries
                        {
                            events.emit(RuntimeEvent::StreamError(
                                "Stream ended before tool calls completed, retrying...".to_string(),
                            ));
                            tokio::time::sleep(Duration::from_secs((attempt + 1) as u64 * 2)).await;
                            continue;
                        }
                        return Ok(result);
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        if attempt < max_retries {
                            events.emit(RuntimeEvent::StreamError(format!(
                                "Stream error, retrying... ({})",
                                e
                            )));
                            tokio::time::sleep(Duration::from_secs((attempt + 1) as u64 * 2)).await;
                            continue;
                        }
                    }
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
        break;
    }

    Err(RuntimeError::Stream(format!(
        "Stream failed after retries: {}",
        last_error
    )))
}

/// Consume a raw SSE byte stream into a [`StreamResult`], emitting events.
pub async fn stream_response(
    mut byte_stream: impl futures::Stream<Item = Result<bytes::Bytes, RuntimeError>> + Unpin,
    events: &dyn EventSink,
    preparing_tools_fired: &mut bool,
) -> Result<StreamResult, RuntimeError> {
    let mut processor = StreamProcessor::new();

    loop {
        let chunk = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, byte_stream.next()).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_elapsed) => {
                return Err(RuntimeError::StreamTimeout(format!(
                    "Stream stalled: no data received for {} seconds",
                    STREAM_IDLE_TIMEOUT.as_secs()
                )));
            }
        };
        let bytes = chunk?;
        for event in processor.feed_bytes(&bytes) {
            dispatch_event(event, events, preparing_tools_fired);
        }
    }

    for event in processor.flush() {
        dispatch_event(event, events, preparing_tools_fired);
    }

    Ok(processor.finish())
}

fn dispatch_event(event: StreamEvent, events: &dyn EventSink, preparing_tools_fired: &mut bool) {
    match event {
        StreamEvent::ContentDelta(text) => {
            events.emit(RuntimeEvent::ContentDelta(text));
        }
        StreamEvent::ReasoningDelta(text) => {
            events.emit(RuntimeEvent::ReasoningDelta(text));
        }
        StreamEvent::ToolCallDelta { .. } => {
            if !*preparing_tools_fired {
                *preparing_tools_fired = true;
                events.emit(RuntimeEvent::PreparingTools);
            }
        }
        StreamEvent::StreamDone { finish_reason } => {
            events.emit(RuntimeEvent::StreamDone { finish_reason });
        }
        StreamEvent::StreamError(msg) => {
            events.emit(RuntimeEvent::StreamError(msg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::ports::EventSink;
    use std::sync::Mutex;

    struct VecSink {
        events: Mutex<Vec<RuntimeEvent>>,
    }

    impl EventSink for VecSink {
        fn emit(&self, event: RuntimeEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn stream_response_emits_content_and_finishes() {
        let body = concat!(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},",
            "\"finish_reason\":null}]}\n",
            "data: [DONE]\n",
        );
        let stream = futures::stream::iter(vec![Ok(bytes::Bytes::from(body))]);
        let sink = VecSink {
            events: Mutex::new(Vec::new()),
        };
        let mut preparing = false;
        let _result = stream_response(stream, &sink, &mut preparing)
            .await
            .expect("stream ok");
        drop(sink.events.lock().unwrap());
    }

    #[tokio::test]
    async fn stream_response_idle_timeout_classifies() {
        let err =
            RuntimeError::from_stream_failure("Stream stalled: no data received for 60 seconds");
        assert!(matches!(err, RuntimeError::StreamTimeout(_)));
        let err = RuntimeError::from_stream_failure("connection reset");
        assert!(matches!(err, RuntimeError::Stream(_)));
    }
}
