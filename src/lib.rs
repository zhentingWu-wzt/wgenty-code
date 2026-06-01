//! Wgenty Code Rust — harness-centric agent infrastructure.
//!
//! Module organization mirrors the harness component model:
//!   agent/       — core agent loop (s01+s02)
//!   tools/       — agent hands: filesystem, search, execution, meta
//!   knowledge/   — on-demand skill loading + magic docs (s05)
//!   context/     — memory, sessions, compression (s06+s07)
//!   tasks/       — task CRUD + dependency graph (s07+s08)
//!   teams/       — subagents, mailboxes, worktree isolation (s04,s09-s12)
//!   permissions/ — tool governance and sandboxing
//!   api/         — API client + types (thin transport layer)
//!   mcp/         — MCP protocol extensions
//!   cli/         — frontend: args, REPL, TUI, commands
//!   config/      — settings
//!   services/    — background daemons
//!   plugins/     — plugin system
//!   sandbox/     — cross-platform OS-level process isolation
//!   state/       — shared application state

pub mod agent;
pub mod api;
pub mod cli;
pub mod config;
pub mod context;
pub mod hooks;
pub mod knowledge;
pub mod mcp;
pub mod permissions;
pub mod plugins;
pub mod guardian;
pub mod prompts;
pub mod sandbox;
pub mod services;
pub mod state;
pub mod tasks;
pub mod teams;
pub mod tools;
pub mod tui;
pub mod utils;
pub mod voice;

// Feature-gated modules
#[cfg(feature = "daemon")]
pub mod daemon;
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
pub use context::MemoryManager;
pub use knowledge::{
    Skill, SkillCategory, SkillContext, SkillError, SkillExecutor, SkillParams, SkillRegistry,
    SkillResult,
};
pub use mcp::McpManager;
pub use guardian::{Guardian, GuardianConfig, GuardianDecision, RiskLevel};
pub use plugins::PluginManager;
pub use state::AppState;
pub use tools::ToolRegistry;
pub use voice::VoiceService;

// Feature-gated re-exports
#[cfg(feature = "gui-egui")]
pub use gui::WgentyCodeApp;
#[cfg(feature = "i18n")]
pub use i18n::Translator;
#[cfg(feature = "wasm")]
pub use wasm::WgentyCodeWasm;
#[cfg(feature = "web")]
pub use web::WebServer;
