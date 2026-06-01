//! GUI Module - Desktop GUI using egui/eframe
//!
//! This module provides a native desktop GUI for Wgenty Code
//! with a modern, responsive interface.

pub mod app;
pub mod brand_icon;
pub mod chat;
pub mod settings;
pub mod sidebar;
pub mod syntax_highlight;
pub mod theme;
pub mod tool_calls;

pub use app::WgentyCodeApp;
pub use theme::Theme;

/// Async message type for GUI communication
#[derive(Debug, Clone)]
pub enum GuiMessage {
    /// Send a chat message to the API
    SendMessage {
        messages: Vec<crate::api::ChatMessage>,
    },
    /// Received a chunk of streaming response
    StreamChunk { content: String, done: bool },
    /// API error occurred
    ApiError { error: String },
    /// Test connection result
    TestConnectionResult { success: bool, message: String },
    /// Settings updated
    SettingsUpdated,
}
