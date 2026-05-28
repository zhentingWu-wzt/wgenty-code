//! TUI frontend — ratatui-based terminal UI replacing the TypeScript frontend.
//!
//! Architecture:
//!   app.rs       — main event loop + layout
//!   agent.rs     — AgentLoop (SSE streaming + tool execution loop)
//!   client.rs    — HTTP client for the daemon API
//!   theme.rs     — color/styling constants
//!   components/  — ratatui widget components

pub mod agent;
pub mod app;
pub mod client;
pub mod components;
pub mod theme;
