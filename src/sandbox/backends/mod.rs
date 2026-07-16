//! Backend selection and platform-specific sandbox implementations.

pub mod linux;
pub mod macos;
pub mod none;
pub mod windows;

use std::process::Stdio;

use crate::sandbox::SandboxBackend;

/// Build a platform-native shell command that runs `command` via the system shell.
///
/// - Windows: `cmd /C <command>`
/// - Unix: `sh -c <command>`
///
/// Does **not** configure stdio or creation flags; callers that run under the
/// TUI should follow with [`configure_captured_stdio`] (or use
/// [`shell_command_captured`]).
pub fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// Shell command with stdio piped and (on Windows) `CREATE_NO_WINDOW`.
///
/// Preferred entry point for direct / fallback execution outside a sandbox
/// backend so child progress output cannot corrupt the parent TUI.
pub fn shell_command_captured(command: &str) -> tokio::process::Command {
    let mut cmd = shell_command(command);
    configure_captured_stdio(&mut cmd);
    cmd
}

/// Synchronous counterpart of [`shell_command`] for `std::process` callers.
///
/// On Windows also sets `CREATE_NO_WINDOW` so console apps cannot allocate a
/// window that shares the parent TUI surface. Callers that need pipes should
/// set `Stdio` themselves (or use [`std::process::Command::output`]).
pub fn std_shell_command(command: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// Configure a child command so its stdio never touches the parent terminal.
///
/// This is critical for the TUI/REPL: without pipes, tools like `npm install`
/// write progress bars and ANSI sequences straight into the shared console and
/// corrupt ratatui's alternate-screen / raw-mode rendering (especially on
/// Windows). Captured streams are consumed by [`crate::sandbox::SandboxedChild`].
pub fn configure_captured_stdio(cmd: &mut tokio::process::Command) {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Prevent console subsystem children (cmd.exe, node, npm) from allocating
    // or writing to a console window that shares the parent's TUI surface.
    // tokio::process::Command exposes creation_flags on Windows targets.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Auto-select the best available backend for the current platform.
pub fn create_backend() -> Box<dyn SandboxBackend> {
    #[cfg(target_os = "macos")]
    {
        if macos::MacOSBackend::is_available() {
            tracing::info!("Selected macOS Seatbelt sandbox backend");
            return Box::new(macos::MacOSBackend::new());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if linux::LinuxBackend::is_available() {
            tracing::info!("Selected Linux seccomp+namespace sandbox backend");
            return Box::new(linux::LinuxBackend::new());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if windows::WindowsBackend::is_available() {
            tracing::info!("Selected Windows Job Object sandbox backend");
            return Box::new(windows::WindowsBackend::new());
        }
    }

    tracing::warn!("No kernel sandbox available; using no-op fallback (policy-only enforcement)");
    Box::new(none::NoneBackend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_captured_stdio_sets_kill_on_drop() {
        // Smoke: builder accepts the helper without panicking. Full pipe
        // behaviour is covered by backend spawn tests that assert stdout content.
        let mut cmd = tokio::process::Command::new("echo");
        configure_captured_stdio(&mut cmd);
    }

    #[test]
    fn shell_command_uses_platform_shell() {
        let cmd = shell_command("echo hi");
        // Debug formatting includes the program name; keep this smoke-level.
        let debug = format!("{:?}", cmd);
        #[cfg(windows)]
        assert!(
            debug.contains("cmd") || debug.contains("CMD"),
            "expected cmd shell, got {debug}"
        );
        #[cfg(not(windows))]
        assert!(debug.contains("sh"), "expected sh shell, got {debug}");
    }

    #[test]
    fn shell_command_captured_is_buildable() {
        let _cmd = shell_command_captured("exit 0");
    }

    #[test]
    fn std_shell_command_runs_exit_zero() {
        let status = std_shell_command("exit 0")
            .status()
            .expect("spawn std shell");
        assert!(status.success());
    }
}
