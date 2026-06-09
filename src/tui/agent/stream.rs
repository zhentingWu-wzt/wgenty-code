use super::{AgentLoop, MAX_RETRIES};
use crate::agent::{StreamEvent, StreamProcessor, StreamResult};
use crate::api::ChatMessage;
use crate::tui::app::AppEvent;
use std::time::Duration;

impl AgentLoop {
    /// Stream with retry logic. Retries up to MAX_RETRIES on network/stream errors.
    pub(super) async fn stream_with_retry(
        &mut self,
        messages: &[ChatMessage],
    ) -> anyhow::Result<StreamResult> {
        let mut last_error = String::new();

        for attempt in 0..=MAX_RETRIES {
            match self.client.chat_stream(messages.to_vec(), None).await {
                Ok(response) => match self.stream_response(response).await {
                    Ok(result) => {
                        // Detect incomplete stream: has tool calls without finish_reason
                        if result.has_tool_calls
                            && result.finish_reason.is_empty()
                            && attempt < MAX_RETRIES
                        {
                            let _ = self.event_tx.send(AppEvent::StreamError(
                                "Stream ended before tool calls completed, retrying...".to_string(),
                            ));
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                (attempt + 1) as u64 * 2,
                            ))
                            .await;
                            continue;
                        }
                        return Ok(result);
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        if attempt < MAX_RETRIES {
                            let _ = self.event_tx.send(AppEvent::StreamError(format!(
                                "Stream error, retrying... ({})",
                                e
                            )));
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                (attempt + 1) as u64 * 2,
                            ))
                            .await;
                            continue;
                        }
                    }
                },
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(tokio::time::Duration::from_secs(
                            (attempt + 1) as u64 * 2,
                        ))
                        .await;
                        continue;
                    }
                }
            }
            break;
        }

        Err(anyhow::anyhow!(
            "Stream failed after retries: {}",
            last_error
        ))
    }

    pub(super) async fn stream_response(
        &mut self,
        response: reqwest::Response,
    ) -> anyhow::Result<StreamResult> {
        const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
        let mut processor = StreamProcessor::new();
        let mut stream = response.bytes_stream();

        use futures::StreamExt;
        loop {
            let chunk = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_elapsed) => {
                    return Err(anyhow::anyhow!(
                        "Stream stalled: no data received for {} seconds",
                        STREAM_IDLE_TIMEOUT.as_secs()
                    ));
                }
            };
            let bytes = chunk?;
            for event in processor.feed_bytes(&bytes) {
                self.dispatch_event(event);
            }
        }

        // Flush remaining buffered data
        for event in processor.flush() {
            self.dispatch_event(event);
        }

        Ok(processor.finish())
    }

    fn dispatch_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::ContentDelta(text) => {
                let _ = self.event_tx.send(AppEvent::ContentDelta(text));
            }
            StreamEvent::ReasoningDelta(text) => {
                let _ = self.event_tx.send(AppEvent::ReasoningDelta(text));
            }
            StreamEvent::ToolCallDelta { .. } => {
                // Fire once to show "preparing tools..." before execution starts
                if !self.preparing_tools_fired {
                    self.preparing_tools_fired = true;
                    let _ = self.event_tx.send(AppEvent::PreparingTools);
                }
            }
            StreamEvent::StreamDone { finish_reason } => {
                let _ = self.event_tx.send(AppEvent::StreamDone { finish_reason });
            }
        }
    }
}
