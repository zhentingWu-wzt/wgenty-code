//! [`LlmPort`] backed by in-process [`ApiClient`].

use crate::agent::runtime::error::RuntimeError;
use crate::agent::runtime::ports::{ChatCompletion, LlmPort};
use crate::api::{ApiClient, ChatMessage, ToolDefinition};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};

/// Direct provider access (CLI headless, tests, optional in-process daemon).
pub struct ApiLlmPort {
    client: ApiClient,
}

impl ApiLlmPort {
    pub fn new(client: ApiClient) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &ApiClient {
        &self.client
    }
}

#[async_trait]
impl LlmPort for ApiLlmPort {
    async fn open_chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
        _max_tokens: Option<usize>,
        _plan_mode: Option<bool>,
    ) -> Result<BoxStream<'static, Result<Bytes, RuntimeError>>, RuntimeError> {
        let response = self
            .client
            .chat_stream(messages, tools)
            .await
            .map_err(|e| RuntimeError::from_stream_failure(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RuntimeError::Stream(format!(
                "API error ({}): {}",
                status, body
            )));
        }

        let stream = response
            .bytes_stream()
            .map(|item| item.map_err(|e| RuntimeError::from_stream_failure(e.to_string())));
        Ok(Box::pin(stream))
    }

    async fn chat_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatCompletion, RuntimeError> {
        let response = self
            .client
            .chat(messages, tools)
            .await
            .map_err(|e| RuntimeError::Stream(e.to_string()))?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(RuntimeError::EmptyResponse)?;
        Ok(ChatCompletion {
            message: choice.message,
            finish_reason: choice.finish_reason.unwrap_or_default(),
            usage: response.usage,
        })
    }
}
