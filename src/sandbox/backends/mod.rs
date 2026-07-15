//! Backend selection and platform-specific sandbox implementations.

pub mod linux;
pub mod macos;
pub mod none;
pub mod windows;

use std::process::Stdio;

use crate::sandbox::SandboxBackend;

/// Configure a child command so its stdio never touches the parent terminal.
///
/// This is critical for the TUI/REPL: without pipes, tools like `npm install`
/// write progress bars and ANSI sequences straight into the shared console and
/// corrupt ratatui's alternate-screen / raw-mode rendering (especially on
/// Windows). Captured streams are consumed by [`crate::sandbox::SandboxedChild`].
pub(crate) fn configure_captured_stdio(cmd: &mut tokio::process::Command) {
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
}
