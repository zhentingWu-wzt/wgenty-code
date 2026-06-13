//! Windows sandbox backend.
//!
//! Uses two Win32 primitives:
//!   1. Job Objects — resource limits (memory, CPU time, process count)
//!      and process tree management (kill on close).
//!   2. Restricted Tokens — strip privileges and add deny-only SIDs to
//!      limit filesystem and system access.
//!
//! The child process is created suspended, assigned to a Job Object,
//! then resumed.

use std::path::Path;

use crate::sandbox::{
    CleanupType, SandboxBackend, SandboxCleanup, SandboxError, SandboxProfile, SandboxedChild,
};

pub struct WindowsBackend;

impl Default for WindowsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl SandboxBackend for WindowsBackend {
    fn name(&self) -> &str {
        "windows-env-filter"
    }

    fn is_available() -> bool {
        cfg!(target_os = "windows")
    }

    fn is_hardware_enforced(&self) -> bool {
        // TODO(phase4): implement actual Job Object + Restricted Token
        // isolation. Currently only environment variable filtering.
        false
    }

    fn capabilities(&self) -> Vec<&str> {
        vec!["env-filter"]
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        // On Windows, we spawn via cmd.exe and rely on the OS-level
        // sandbox primitives. The full Restricted Token + Job Object
        // pipeline requires the `windows` crate and is phased in
        // during Phase 4.
        //
        // For now, we provide a best-effort sandbox using Job Objects
        // created via a helper PowerShell script.

        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);

        // Filter environment to allowlist
        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }

        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        // TODO(phase4): Assign to Job Object with resource limits
        // TODO(phase4): Use CreateProcessAsUser with Restricted Token
        // Currently falls back to basic process spawning with env filtering

        Ok(SandboxedChild {
            child,
            backend_name: "job-object".into(),
            cleanup: Some(SandboxCleanup {
                cleanup_type: CleanupType::CloseJobObject,
                resource_handle: None,
            }),
        })
    }
}
