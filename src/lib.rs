//! Claude Code Rust - High-performance CLI for Claude AI
//!
//! A complete Rust implementation of Claude Code, featuring:
//! - Async-first architecture with Tokio
//! - Native terminal UI with Ratatui
//! - MCP protocol support
//! - Voice input support
//! - Memory management and team sync
//! - Plugin system
//! - SSH connection support
//! - Remote execution
//! - Project initialization
//! - WebAssembly support for browser environments
//! - Native GUI with egui/eframe
//! - Plugin marketplace web interface
//! - Multi-language i18n support

pub mod advanced;
pub mod api;
pub mod branding;
pub mod cli;
pub mod config;
pub mod mcp;
pub mod memory;
pub mod plugins;
pub mod services;
pub mod session;
pub mod skills;
pub mod state;
pub mod terminal;
pub mod tools;
pub mod utils;
pub mod voice;

// Feature-gated modules
#[cfg(feature = "gui-egui")]
pub mod gui;
#[cfg(feature = "i18n")]
pub mod i18n;
#[cfg(feature = "wasm")]
pub mod wasm;
#[cfg(feature = "web")]
pub mod web;

pub use api::{AnthropicClient, ApiClient, ChatMessage};
pub use cli::Cli;
pub use config::Settings;
pub use mcp::McpManager;
pub use memory::MemoryManager;
pub use plugins::PluginManager;
pub use skills::{
    Skill, SkillCategory, SkillContext, SkillError, SkillExecutor, SkillParams, SkillRegistry,
    SkillResult,
};
pub use state::AppState;
pub use tools::ToolRegistry;
pub use voice::VoiceInput;

// Feature-gated re-exports
#[cfg(feature = "gui-egui")]
pub use gui::ClaudeCodeApp;
#[cfg(feature = "i18n")]
pub use i18n::Translator;
#[cfg(feature = "wasm")]
pub use wasm::ClaudeCodeWasm;
#[cfg(feature = "web")]
pub use web::WebServer;
