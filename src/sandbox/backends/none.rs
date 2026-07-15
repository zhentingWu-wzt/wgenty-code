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
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);

        // Filter environment to allowlist
        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }

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
        let profile = SandboxConfig::builder("/tmp").build();
        let mut child = backend
            .spawn(&profile, "echo hello", None)
            .expect("spawn should succeed");
        assert_eq!(child.backend_name, "none");
        // Wait for the child to finish to avoid zombie processes
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_spawn_with_restricted_env() {
        let backend = NoneBackend;
        let profile = SandboxConfig::builder("/tmp")
            .env_allowlist(vec!["PATH".into()])
            .build();
        // Should not panic even with a restricted env allowlist
        let mut child = backend
            .spawn(&profile, "true", None)
            .expect("spawn with env filter should succeed");
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_spawn_with_workdir() {
        let backend = NoneBackend;
        let profile = SandboxConfig::builder("/tmp").build();
        let workdir = std::path::Path::new("/tmp");
        let mut child = backend
            .spawn(&profile, "pwd", Some(workdir))
            .expect("spawn with workdir should succeed");
        let _ = child.child.wait().await;
    }

    #[tokio::test]
    async fn none_backend_spawn_invalid_command_returns_error() {
        // sh -c with a command that doesn't exist still spawns sh successfully;
        // the error is in the exit code, not spawn. So this tests that spawn
        // itself doesn't error for syntactically valid shell commands.
        let backend = NoneBackend;
        let profile = SandboxConfig::builder("/tmp").build();
        let mut child = backend
            .spawn(&profile, "exit 1", None)
            .expect("spawn should succeed even for failing commands");
        let status = child.child.wait().await.expect("wait should succeed");
        assert!(!status.success());
    }
}
