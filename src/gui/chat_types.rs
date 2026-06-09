//! Chat message data types for the GUI.

use super::tool_calls::ToolCall;
use chrono::{DateTime, Utc};

/// A chat message - matches Claude.ai structure
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub is_streaming: bool,
    pub tool_calls: Vec<ToolCall>,
    pub attachments: Vec<Attachment>,
    pub thinking: Option<String>,
    pub thinking_expanded: bool,
    /// Whether the body text is collapsed (long content)
    pub content_collapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub name: String,
    pub content_type: String,
    pub size: usize,
}

impl ChatMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        let content_str: String = content.into();
        let line_count = content_str.lines().count();
        let content_collapsed = matches!(role, MessageRole::Assistant) && line_count > 50;
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content_str,
            timestamp: Utc::now(),
            is_streaming: false,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            thinking: None,
            thinking_expanded: false,
            content_collapsed,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content)
    }

    pub fn with_thinking(mut self, thinking: impl Into<String>) -> Self {
        self.thinking = Some(thinking.into());
        self
    }

    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }
}
