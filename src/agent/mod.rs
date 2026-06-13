//! Agent Module — shared agent loop and SSE stream processing.
//!
//! The agent loop runs in every communication channel:
//!   - CLI REPL:    cli::repl (simple line-mode loop)
//!   - TUI REPL:    cli::tui_repl (full Ratatui loop)
//!   - Daemon API:  daemon::handlers (HTTP SSE proxy)
//!
//! `StreamProcessor` handles the duplicated SSE parsing logic previously
//! found in both frontends, producing structured `StreamEvent`s.

pub mod core;
pub mod events;
pub mod progress;

pub use core::StreamProcessor;
pub use events::{StreamEvent, StreamResult};
pub use progress::{ProgressCallback, SubagentMetadata, SubagentProgress, SubagentStatus};
