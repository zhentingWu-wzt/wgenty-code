//! Sandbox module — cross-platform OS-level process isolation.
//!
//! Provides a unified `SandboxBackend` trait with platform-specific
//! implementations backed by kernel sandbox mechanisms:
//!   - macOS:    Seatbelt (sandbox-exec + .sb profiles)
//!   - Linux:    seccomp-bpf + namespaces + cgroups v2
//!   - Windows:  Job Objects + Restricted Tokens
//!
//! Gracefully degrades to a no-op fallback when kernel sandbox is unavailable.

pub mod backends;
pub mod config;
pub mod error;
pub mod platform;
pub mod profile;

use std::path::Path;

pub use backends::create_backend;
pub use config::{SandboxConfig, SecurityLevel};
pub use error::SandboxError;
pub use platform::Platform;
pub use profile::SandboxProfile;

/// Result of a sandboxed command execution.
#[derive(Debug, Clone)]
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// True if the process was killed by a sandbox violation or resource limit.
    pub killed_by_sandbox: bool,
}

/// Handle to a running sandboxed child process.
pub struct SandboxedChild {
    pub child: tokio::process::Child,
    pub backend_name: String,
    /// Optional cleanup handle (e.g. temp profile file, cgroup directory).
    pub cleanup: Option<SandboxCleanup>,
}

/// Cleanup actions to run after the child process exits.
pub struct SandboxCleanup {
    pub cleanup_type: CleanupType,
    pub resource_handle: Option<String>,
}

/// The kind of cleanup to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupType {
    /// Delete a temp file (macOS seatbelt profile).
    DeleteTempFile,
    /// Remove a cgroup directory recursively (Linux).
    RemoveCgroup,
    /// Close a Windows Job Object handle (handled by OS on process exit).
    CloseJobObject,
}

impl SandboxedChild {
    /// Wait for the child to exit, run cleanup, return output.
    pub async fn wait_with_output(self) -> Result<SandboxOutput, SandboxError> {
        // Destructure to avoid partial-move issues
        let SandboxedChild { child, cleanup, .. } = self;
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| SandboxError::Spawn {
                io_error: format!("wait failed: {}", e),
            })?;

        // Run cleanup if needed
        Self::perform_cleanup(&cleanup);

        Ok(SandboxOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            killed_by_sandbox: output.status.code().is_none(),
        })
    }

    /// Execute cleanup actions after the child exits.
    fn perform_cleanup(cleanup: &Option<SandboxCleanup>) {
        if let Some(ref cleanup) = *cleanup {
            if let Some(ref handle) = cleanup.resource_handle {
                match cleanup.cleanup_type {
                    CleanupType::DeleteTempFile => {
                        let _ = std::fs::remove_file(handle);
                    }
                    CleanupType::RemoveCgroup => {
                        let _ = std::fs::remove_dir_all(handle);
                    }
                    CleanupType::CloseJobObject => {
                        // Windows Job Object handles are closed automatically
                        // when the process exits. No explicit cleanup needed.
                    }
                }
            }
        }
    }
}

/// The core sandbox backend trait. Each platform provides an implementation.
pub trait SandboxBackend: Send + Sync {
    /// Unique name for this backend (e.g. "seatbelt", "seccomp", "none").
    fn name(&self) -> &str;

    /// Probe whether this backend is available on the current system.
    fn is_available() -> bool
    where
        Self: Sized;

    /// Whether this backend provides kernel-level enforcement.
    fn is_hardware_enforced(&self) -> bool;

    /// Capabilities this backend supports (e.g. "filesystem", "network", "memory-limit").
    fn capabilities(&self) -> Vec<&str>;

    /// Spawn a command inside the sandbox.
    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError>;
}

/// Top-level manager: selects the best available backend and routes calls to it.
pub struct SandboxManager {
    backend: Box<dyn SandboxBackend>,
}

impl SandboxManager {
    /// Create a new manager, auto-selecting the best available backend.
    pub fn new() -> Self {
        let backend = backends::create_backend();
        tracing::info!(
            "Sandbox backend: {} (enforced={})",
            backend.name(),
            backend.is_hardware_enforced()
        );
        Self { backend }
    }

    /// Execute a command with the given profile. Blocks until completion.
    pub async fn execute(
        &self,
        command: &str,
        profile: &SandboxProfile,
    ) -> Result<SandboxOutput, SandboxError> {
        let child = self.spawn(command, profile)?;
        child.wait_with_output().await
    }

    /// Spawn a command without waiting (for interactive sessions).
    pub fn spawn(
        &self,
        command: &str,
        profile: &SandboxProfile,
    ) -> Result<SandboxedChild, SandboxError> {
        let workdir = profile.workdir.as_deref();
        self.backend.spawn(profile, command, workdir)
    }

    /// Inspect the active backend.
    pub fn status(&self) -> SandboxStatus {
        SandboxStatus {
            backend_name: self.backend.name().to_string(),
            is_hardware_enforced: self.backend.is_hardware_enforced(),
            capabilities: self
                .backend
                .capabilities()
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Diagnostic information about the active sandbox backend.
#[derive(Debug, Clone)]
pub struct SandboxStatus {
    pub backend_name: String,
    pub is_hardware_enforced: bool,
    pub capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn manager_status_has_backend_name() {
        let status = SandboxManager::new().status();

        assert!(!status.backend_name.trim().is_empty());
    }

    #[test]
    fn manager_capabilities_are_unique() {
        let status = SandboxManager::new().status();
        let unique: HashSet<&str> = status.capabilities.iter().map(String::as_str).collect();

        assert_eq!(unique.len(), status.capabilities.len());
    }

    #[test]
    fn cleanup_missing_resource_is_idempotent() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let missing = temp.path().join("missing-profile.sb");
        let cleanup = Some(SandboxCleanup {
            cleanup_type: CleanupType::DeleteTempFile,
            resource_handle: Some(missing.display().to_string()),
        });

        SandboxedChild::perform_cleanup(&cleanup);
        SandboxedChild::perform_cleanup(&cleanup);
    }
}
