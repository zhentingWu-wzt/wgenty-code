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
