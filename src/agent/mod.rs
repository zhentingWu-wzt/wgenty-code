//! Agent Module — the core agent loop: while + stop_reason + tool dispatch.
//!
//! Corresponds to harness mechanism s01+s02: one loop + tools = an agent.
//! The agent loop runs in every communication channel:
//!   - CLI REPL:    cli::repl (simple line-mode loop)
//!   - TUI REPL:    cli::tui_repl (full Ratatui loop)
//!
//! In a future refinement, the pure loop logic (conversation + tool dispatch)
//! will be extracted here so both CLI and TUI frontends share the same agent
//! core. For now this module serves as documentation of the harness architecture:
//! the agent loop is the heart, everything else is harness.
