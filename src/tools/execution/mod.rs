pub mod background;
pub mod exec_command;
pub mod execute_command;
pub mod git_operations;
pub mod kill_session;
pub mod run_test;
pub mod sandbox_exec;
pub mod session_manager;
pub mod test_output;
pub mod write_stdin;

/// Default yield window before an interactive command tool returns partial
/// output (milliseconds). Shared by `exec_command` and `write_stdin`.
pub(crate) const DEFAULT_YIELD_TIME_MS: u64 = 1000;
/// Default output cap (characters) for interactive command tools.
pub(crate) const DEFAULT_MAX_OUTPUT_CHARS: usize = 4000;
/// Default wall-clock timeout (seconds) for `exec_background`.
pub(crate) const DEFAULT_BACKGROUND_TIMEOUT_SECS: u64 = 300;

pub use background::{BackgroundManager, BackgroundResult, BackgroundTool};
pub use exec_command::ExecCommandTool;
pub use execute_command::ExecuteCommandTool;
pub use git_operations::GitOperationsTool;
pub use kill_session::KillSessionTool;
pub use session_manager::CommandSessionManager;
pub use write_stdin::WriteStdinTool;

pub use run_test::RunTestTool;
