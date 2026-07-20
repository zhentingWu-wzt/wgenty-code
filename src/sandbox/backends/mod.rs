//! Backend selection and platform-specific sandbox implementations.

pub mod linux;
pub mod macos;
pub mod none;
pub mod windows;

use std::process::Stdio;

use crate::sandbox::SandboxBackend;

/// Unix prefix forced into every `sh -c` body so nested shells / `env -i`
/// wrappers still inherit non-interactive git/ssh behaviour even if process
/// env was cleared by an intermediate step.
#[cfg(not(windows))]
const NONINTERACTIVE_SHELL_PREFIX: &str = "export GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/usr/bin/false SSH_ASKPASS=/usr/bin/false SSH_ASKPASS_REQUIRE=never GCM_INTERACTIVE=never CI=true;";

/// Build a platform-native shell command that runs `command` via the system shell.
///
/// - Windows: `cmd /C <command>`
/// - Unix: `sh -c <command>`
///
/// Does **not** configure stdio or creation flags; callers that run under the
/// TUI should follow with [`configure_captured_stdio`] (or use
/// [`shell_command_captured`]).
///
/// On Unix the command body is prefixed with non-interactive `export`s so git
/// credential prompts cannot open `/dev/tty` even when an intermediate
/// `env -i` / sandbox allowlist drops process-level env vars.
pub fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("cmd");
        // cmd.exe: set vars for this command line only.
        let wrapped = format!(
            "set \"GIT_TERMINAL_PROMPT=0\"&& set \"GCM_INTERACTIVE=never\"&& set \"SSH_ASKPASS_REQUIRE=never\"&& set \"CI=true\"&& {command}"
        );
        cmd.arg("/C").arg(wrapped);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new("sh");
        let wrapped = format!("{NONINTERACTIVE_SHELL_PREFIX}{command}");
        cmd.arg("-c").arg(wrapped);
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
///
/// Always applies [`apply_noninteractive_env_std`] (same rationale as
/// [`configure_captured_stdio`]).
pub fn std_shell_command(command: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("cmd");
        let wrapped = format!(
            "set \"GIT_TERMINAL_PROMPT=0\"&& set \"GCM_INTERACTIVE=never\"&& set \"SSH_ASKPASS_REQUIRE=never\"&& set \"CI=true\"&& {command}"
        );
        cmd.arg("/C").arg(wrapped);
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        apply_noninteractive_env_std(&mut cmd);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new("sh");
        let wrapped = format!("{NONINTERACTIVE_SHELL_PREFIX}{command}");
        cmd.arg("-c").arg(wrapped);
        apply_noninteractive_env_std(&mut cmd);
        cmd
    }
}

/// Configure a child command so its stdio never touches the parent terminal.
///
/// This is critical for the TUI/REPL: without pipes, tools like `npm install`
/// write progress bars and ANSI sequences straight into the shared console and
/// corrupt ratatui's alternate-screen / raw-mode rendering (especially on
/// Windows). Captured streams are consumed by [`crate::sandbox::SandboxedChild`].
///
/// Also applies [`apply_noninteractive_env`] so tools that open `/dev/tty`
/// (notably `git push` credential prompts) cannot hijack the parent TUI input
/// line when stdin/stdout are already piped.
pub fn configure_captured_stdio(cmd: &mut tokio::process::Command) {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    apply_noninteractive_env(cmd);

    // Prevent console subsystem children (cmd.exe, node, npm) from allocating
    // or writing to a console window that shares the parent's TUI surface.
    // tokio::process::Command implements std::os::windows::process::CommandExt.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Force child processes into non-interactive mode so credential / password
/// prompts cannot write to the parent TTY and pollute the TUI input line.
///
/// Git (and some credential helpers) bypass piped stdio and open `/dev/tty`
/// directly when asking for a username/password. Setting these vars makes
/// those prompts fail closed into captured stderr instead.
///
/// Safe to call after `env_clear()` + allowlist restore — always re-apply.
pub fn apply_noninteractive_env(cmd: &mut tokio::process::Command) {
    // Git: never prompt on the terminal (uses /dev/tty even when stdio is piped).
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    // Prefer a failing askpass over opening /dev/tty if a helper still asks.
    // `/usr/bin/false` exits 1 with no output on macOS/Linux; on Windows `false`
    // may be absent — git then falls back to GIT_TERMINAL_PROMPT=0 behaviour.
    #[cfg(not(windows))]
    {
        cmd.env("GIT_ASKPASS", "/usr/bin/false");
        cmd.env("SSH_ASKPASS", "/usr/bin/false");
    }
    #[cfg(windows)]
    {
        // No portable false binary; rely on GIT_TERMINAL_PROMPT + GCM flags.
        cmd.env_remove("GIT_ASKPASS");
        cmd.env_remove("SSH_ASKPASS");
    }
    // Git Credential Manager (cross-platform helper).
    cmd.env("GCM_INTERACTIVE", "never");
    // OpenSSH 8.4+: never fall back to TTY askpass when no display/askpass.
    cmd.env("SSH_ASKPASS_REQUIRE", "never");
    // Common convention: many CLIs skip interactive prompts under CI.
    cmd.env("CI", "true");
}

/// Same as [`apply_noninteractive_env`] for `std::process::Command`.
pub fn apply_noninteractive_env_std(cmd: &mut std::process::Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(not(windows))]
    {
        cmd.env("GIT_ASKPASS", "/usr/bin/false");
        cmd.env("SSH_ASKPASS", "/usr/bin/false");
    }
    #[cfg(windows)]
    {
        cmd.env_remove("GIT_ASKPASS");
        cmd.env_remove("SSH_ASKPASS");
    }
    cmd.env("GCM_INTERACTIVE", "never");
    cmd.env("SSH_ASKPASS_REQUIRE", "never");
    cmd.env("CI", "true");
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
    fn noninteractive_env_is_visible_to_child() {
        // Prove the env vars actually reach the child (not just set on the builder).
        #[cfg(windows)]
        let check = "echo %GIT_TERMINAL_PROMPT% %GCM_INTERACTIVE% %SSH_ASKPASS_REQUIRE% %CI%";
        #[cfg(not(windows))]
        let check = "printf '%s %s %s %s' \"$GIT_TERMINAL_PROMPT\" \"$GCM_INTERACTIVE\" \"$SSH_ASKPASS_REQUIRE\" \"$CI\"";

        let output = std_shell_command(check)
            .output()
            .expect("spawn std shell with noninteractive env");
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("0") && stdout.contains("never") && stdout.contains("true"),
            "expected noninteractive env in child, got {stdout:?}"
        );
    }

    #[tokio::test]
    async fn captured_shell_inherits_noninteractive_env() {
        #[cfg(windows)]
        let check = "echo %GIT_TERMINAL_PROMPT%";
        #[cfg(not(windows))]
        let check = "printf '%s' \"$GIT_TERMINAL_PROMPT\"";

        let child = shell_command_captured(check)
            .spawn()
            .expect("spawn captured shell");
        let out = child.wait_with_output().await.expect("wait");
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.trim() == "0" || stdout.contains("0"),
            "GIT_TERMINAL_PROMPT should be 0, got {stdout:?}"
        );
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
