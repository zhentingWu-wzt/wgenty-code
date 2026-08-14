//! CLI Module — command-line argument parsing, branding, and subcommand dispatch.
//!
//! The interactive REPL uses a ratatui-based TUI frontend. Running `cargo run`
//! (or `cargo run -- repl`) starts the daemon in the background and launches
//! the ratatui terminal UI. No Node.js/npm dependency required.

pub mod args;
pub mod branding;
pub mod commands;
pub mod daemon_admin;
pub mod headless_runtime;
pub mod org_graph;
pub mod subagent;

pub use args::Cli;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Wgenty Code - AI-powered coding assistant
#[derive(Parser, Debug)]
#[command(name = "wgenty-code")]
#[command(author = "Anthropic")]
#[command(version = "0.1.0")]
#[command(about = "High-performance Rust implementation of Wgenty Code CLI")]
#[command(disable_version_flag = true)]
#[command(disable_help_subcommand = true)]
pub struct CliArgs {
    /// Path to the project directory
    #[arg(short, long, value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Model to use (sonnet, opus, haiku)
    #[arg(short, long, default_value = "sonnet")]
    pub model: String,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Run in non-interactive mode
    #[arg(short, long)]
    pub no_interactive: bool,

    /// Print version information
    #[arg(long)]
    pub version: bool,

    /// Print system information
    #[arg(long)]
    pub info: bool,

    /// Subcommands
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start an interactive REPL session (ratatui TUI)
    Repl {
        /// Initial prompt to send
        #[arg(short, long)]
        prompt: Option<String>,
    },

    /// Execute a single query
    Query {
        /// The query to execute (use --prompt-file for long prompts)
        #[arg(short, long)]
        prompt: Option<String>,

        /// Read the prompt from a file (useful for long task instructions
        /// such as SWE eval prompts; avoids shell ARG_MAX limits)
        #[arg(long, value_name = "PATH")]
        prompt_file: Option<String>,

        /// Autonomous mode: full filesystem/network access and no approval
        /// gating. Bypasses the Critical-risk guardian block so the agent
        /// can run arbitrary commands (builds, tests, installs). Intended for
        /// disposable sandboxed eval runs (e.g. DeepSWE).
        #[arg(long)]
        yolo: bool,

        /// Override the agent loop max rounds
        /// (default: settings.agent.max_rounds or 100)
        #[arg(long, value_name = "N")]
        max_rounds: Option<usize>,
    },

    /// Manage configuration settings
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },

    /// Manage MCP servers
    Mcp {
        #[command(subcommand)]
        action: McpCommands,
    },

    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginCommands,
    },

    /// Manage memory and sessions
    Memory {
        #[command(subcommand)]
        action: MemoryCommands,
    },

    /// Voice input mode
    Voice {
        /// Enable push-to-talk mode
        #[arg(short, long)]
        push_to_talk: bool,
    },

    /// Initialize a new project
    Init {
        /// Project name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Update to latest version
    Update,

    /// Show help and usage information
    Help {
        /// Topic to show help for
        #[arg(short, long)]
        topic: Option<String>,
    },

    /// Manage background services
    Services {
        #[command(subcommand)]
        action: ServiceCommands,
    },

    /// Run an agent
    Agent {
        /// Agent type (guide, explore, plan, verify, general)
        #[arg(short, long)]
        agent_type: String,
        /// Prompt for the agent
        #[arg(short, long)]
        prompt: String,
    },

    /// Manage Magic Docs
    MagicDocs {
        #[command(subcommand)]
        action: MagicDocsCommands,
    },

    /// Team memory sync
    TeamSync {
        #[command(subcommand)]
        action: TeamSyncCommands,
    },

    /// Manage skills
    Skills {
        #[command(subcommand)]
        action: SkillsCommands,
    },

    /// Manage sandbox settings
    Sandbox {
        #[command(subcommand)]
        action: SandboxCommands,
    },

    /// Run stress tests
    StressTest {
        /// Number of concurrent requests
        #[arg(short, long, default_value = "5")]
        concurrency: usize,
        /// Number of iterations per request
        #[arg(short, long, default_value = "10")]
        iterations: usize,
    },

    /// Start the daemon HTTP API server
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonCommands>,

        /// Port to listen on
        #[arg(long, default_value = "8371")]
        port: u16,
    },

    /// Inspect subagent transcripts offline (list / trace / health)
    Subagent {
        #[command(subcommand)]
        action: SubagentCommands,
    },

    /// Inspect the Org-Graph node-contract registry
    OrgGraph {
        #[command(subcommand)]
        action: OrgGraphCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommands {
    /// Show the running daemon's status (discovery file + health probe)
    Status,
    /// Gracefully stop the running daemon (API-triggered shutdown)
    Stop,
}

#[derive(Subcommand, Debug)]
pub enum SubagentCommands {
    /// List subagent runs, most recent first
    List {
        /// Filter by session id
        #[arg(long)]
        session: Option<String>,
        /// Filter by status (completed | failed | cancelled | running)
        #[arg(long)]
        status: Option<String>,
        /// Max rows to print
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Filter by project root path (the working dir the subagent ran in)
        #[arg(long)]
        project: Option<String>,
    },

    /// Render a single subagent trace by id
    Trace {
        /// Transcript id
        id: String,
        /// Render format
        #[arg(long, value_enum, default_value_t = crate::cli::subagent::TraceFormat::CallTree)]
        format: crate::cli::subagent::TraceFormat,
        /// Print raw stored diagnostics as pretty JSON (ignores --format)
        #[arg(long)]
        raw: bool,
    },

    /// Show aggregate subagent health (success rate + failure modes)
    Health {
        /// Time window
        #[arg(long, value_enum, default_value_t = crate::cli::subagent::HealthPeriodArg::All)]
        period: crate::cli::subagent::HealthPeriodArg,
        /// Filter by session id
        #[arg(long)]
        session: Option<String>,
        /// Filter by project root path
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum OrgGraphCommands {
    /// Render the built-in node contracts.
    ///
    /// Visual formats (table / dot / mermaid) omit the long `system_prompt`
    /// field for readability; use `--format json` for the lossless source.
    Contracts {
        /// Output format (table | dot | mermaid | json)
        #[arg(
            long,
            value_enum,
            default_value_t = crate::cli::org_graph::OrgGraphFormatArg::Table
        )]
        format: crate::cli::org_graph::OrgGraphFormatArg,
    },
    /// Summarize durable work-graph audit records for one checkpointed session.
    Audit {
        /// Trusted session id whose persisted audit should be read.
        #[arg(long)]
        session: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,

    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,
        /// Configuration value
        value: String,
    },

    /// Reset configuration to defaults
    Reset,
}

#[derive(Subcommand, Debug)]
pub enum McpCommands {
    /// List configured MCP servers
    List,

    /// Add a new MCP server
    Add {
        /// Server name (e.g. filesystem)
        name: String,
        /// Server command (可选，filesystem 可只用 --path)
        command: Option<String>,
        /// Filesystem 专用路径
        #[arg(long, short = 'p', value_name = "PATH")]
        path: Option<String>,
    },

    /// Remove an MCP server
    Remove {
        /// Server name
        name: String,
    },

    /// Restart an MCP server
    Restart {
        /// Server name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// List installed plugins
    List,

    /// Install a plugin
    Install {
        /// Plugin name or URL
        plugin: String,
    },

    /// Remove a plugin
    Remove {
        /// Plugin name
        name: String,
    },

    /// Update all plugins
    Update,

    /// Search for plugins
    Search {
        /// Search query
        query: String,
    },

    /// Enable a plugin
    Enable {
        /// Plugin name
        name: String,
    },

    /// Disable a plugin
    Disable {
        /// Plugin name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommands {
    /// Show memory status
    Status,

    /// Clear all memories
    Clear,

    /// Export memories
    Export {
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Import memories
    Import {
        /// Input file path
        input: PathBuf,
    },

    /// Run memory consolidation (dream)
    Dream,

    /// Force AutoDream consolidation
    AutoDream,

    /// Prune low-value / expired memories (project + global)
    Prune,

    /// List memories (sorted by importance desc)
    List {
        /// Only show memories at or above this importance (0.0–1.0)
        #[arg(long)]
        min_importance: Option<f32>,
        /// Max entries to print (default 50)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// Audit tombstoned memories (superseded entries retained for audit)
    Audit,
}

#[derive(Subcommand, Debug)]
pub enum ServiceCommands {
    /// Show status of all services
    Status,

    /// Start all services
    Start,

    /// Stop all services
    Stop,

    /// Check AutoDream status
    AutoDream,

    /// Check Voice status
    Voice,

    /// Check Magic Docs status
    MagicDocs,

    /// Check Team Sync status
    TeamSync,

    /// Check Plugins status
    Plugins,

    /// Check Agents status
    Agents,
}

#[derive(Subcommand, Debug)]
pub enum MagicDocsCommands {
    /// List tracked Magic Docs
    List,

    /// Check a file for Magic Doc header
    Check {
        /// File path to check
        file: String,
    },

    /// Update a Magic Doc
    Update {
        /// File path to update
        file: String,
        /// Context for update
        #[arg(short, long)]
        context: Option<String>,
    },

    /// Clear all tracked Magic Docs
    Clear,
}

#[derive(Subcommand, Debug)]
pub enum TeamSyncCommands {
    /// Show sync status
    Status,

    /// Authenticate with team
    Auth {
        /// Team ID
        team_id: String,
    },

    /// Sync memories
    Sync,

    /// List team memories
    List,

    /// Create a team memory
    Create {
        /// Memory title
        title: String,
        /// Memory content
        #[arg(short, long)]
        content: String,
        /// Tags (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,
    },

    /// Delete a team memory
    Delete {
        /// Memory ID
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SkillsCommands {
    /// List all available skills
    List,

    /// Execute a skill
    Execute {
        /// Skill name
        skill: String,
        /// Arguments for the skill
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Get help for a skill
    Help {
        /// Skill name
        skill: String,
    },

    /// Search for skills
    Search {
        /// Search query
        query: String,
    },

    /// Install bundled skills to the user's skills directory
    Install,
}

#[derive(Subcommand, Debug)]
pub enum SandboxCommands {
    /// Show sandbox status
    Status,

    /// Disable sandbox for the session
    Disable,

    /// Enable sandbox
    Enable,
}
