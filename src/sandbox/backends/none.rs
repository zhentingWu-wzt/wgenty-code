//! NoneBackend — no-op sandbox fallback.
//!
//! Used when no kernel-level sandbox mechanism is available.
//! Commands run with normal process privileges, relying solely on the
//! policy layer (permissions/policy.rs) for enforcement.

use std::path::Path;

use crate::sandbox::{SandboxBackend, SandboxError, SandboxProfile, SandboxedChild};

pub struct NoneBackend;

impl SandboxBackend for NoneBackend {
    fn name(&self) -> &str {
        "none"
    }

    fn is_available() -> bool {
        true // Always available as a fallback
    }

    fn is_hardware_enforced(&self) -> bool {
        false
    }

    fn capabilities(&self) -> Vec<&str> {
        vec!["none"]
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        // Platform shell + piped stdio / CREATE_NO_WINDOW via shared helper.
        let mut cmd = super::shell_command_captured(command);

        // Filter environment to allowlist
        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }
        // Re-apply after possible env_clear (configure_captured_stdio already set these).
        super::apply_noninteractive_env(&mut cmd);

        // Apply working directory
        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        Ok(SandboxedChild {
            child,
            backend_name: "none".into(),
            cleanup: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::config::SandboxConfig;

    #[test]
    fn none_backend_properties() {
        let backend = NoneBackend;
        assert_eq!(backend.name(), "none");
        assert!(NoneBackend::is_available());
        assert!(!backend.is_hardware_enforced());
        assert_eq!(backend.capabilities(), vec!["none"]);
    }

    #[tokio::test]
    async fn none_backend_spawn_succeeds() {
        let backend = NoneBackend;
        let profile = SandboxConfig::builder(std::env::temp_dir()).build();
        let mut child = backend
            .spawn(&profile, "echo hello", None)
            .expect("spawn should succeed");
        assert_eq!(child.backend_name, "none");
        // Wait for the child to finish to avoid zombie processes
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_captures_stdout() {
        let backend = NoneBackend;
        let profile = SandboxConfig::builder(std::env::temp_dir()).build();
        let child = backend
            .spawn(&profile, "echo hello-capture", None)
            .expect("spawn should succeed");
        let out = child.wait_with_output().await.expect("wait");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(
            out.stdout.contains("hello-capture"),
            "stdout must be piped/captured, got {:?}",
            out.stdout
        );
    }

    #[tokio::test]
    async fn none_backend_spawn_with_restricted_env() {
        let backend = NoneBackend;
        let profile = SandboxConfig::builder(std::env::temp_dir())
            .env_allowlist(vec!["PATH".into(), "SystemRoot".into(), "COMSPEC".into()])
            .build();
        // Portable no-op: `exit 0` works under both `sh -c` and `cmd /C`.
        let mut child = backend
            .spawn(&profile, "exit 0", None)
            .expect("spawn with env filter should succeed");
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_spawn_with_workdir() {
        let backend = NoneBackend;
        let workdir = std::env::temp_dir();
        let profile = SandboxConfig::builder(&workdir).build();
        // Portable command that succeeds in the workdir under sh and cmd.
        let mut child = backend
            .spawn(&profile, "exit 0", Some(workdir.as_path()))
            .expect("spawn with workdir should succeed");
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_spawn_invalid_command_returns_error() {
        // Shell spawn succeeds; the non-zero exit is reported via wait status.
        let backend = NoneBackend;
        let profile = SandboxConfig::builder(std::env::temp_dir()).build();
        let mut child = backend
            .spawn(&profile, "exit 1", None)
            .expect("spawn should succeed even for failing commands");
        let status = child.child.wait().await.expect("wait should succeed");
        assert!(!status.success());
    }
}
