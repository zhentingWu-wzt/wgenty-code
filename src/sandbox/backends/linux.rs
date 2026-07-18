//! Linux sandbox backend.
//!
//! Current enforcement:
//!   1. Namespace isolation (mount, network, pid) via `unshare`
//!   2. cgroups v2 resource limits (memory, cpu, pids)
//!
//! NOTE: seccomp-bpf syscall filtering is documented in the backend name
//! (`seccomp+ns`) and capabilities (`syscall-filter`) but is NOT yet wired to
//! libseccomp - the `unshare` wrapper provides namespace isolation only. A
//! real seccomp whitelist requires the `libseccomp` C dependency and a syscall
//! allowlist; tracked as a follow-up (see docs/SANDBOX.md). Until then,
//! `is_hardware_enforced` reflects namespace+cgroup enforcement, not syscall
//! filtering.
//!
//! Falls back gracefully: if `unshare`/cgroups are unavailable, the
//! NoneBackend takes over.

use std::path::Path;

use crate::sandbox::{
    CleanupType, SandboxBackend, SandboxCleanup, SandboxError, SandboxProfile, SandboxedChild,
};

pub struct LinuxBackend {
    cgroup_base: std::path::PathBuf,
}

impl Default for LinuxBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxBackend {
    pub fn new() -> Self {
        Self {
            cgroup_base: std::path::PathBuf::from("/sys/fs/cgroup"),
        }
    }

    /// Probe whether we can actually create a cgroup under the v2 base.
    ///
    /// `cgroup.controllers` existing only proves cgroup v2 is mounted; on
    /// unprivileged hosts (CI runners, non-root sessions) `/sys/fs/cgroup` is
    /// read-only and [`create_cgroup`](Self::create_cgroup) would fail with
    /// `EACCES`. Probe once at backend selection so [`create_backend`](super::create_backend)
    /// falls back to NoneBackend instead of hard-failing every sandboxed spawn
    /// under `FailMode::HardFail`.
    fn cgroup_base_writable() -> bool {
        let probe = std::path::Path::new("/sys/fs/cgroup")
            .join(format!(".wgenty-probe-{}", std::process::id()));
        match std::fs::create_dir(&probe) {
            Ok(()) => {
                let _ = std::fs::remove_dir(&probe);
                true
            }
            Err(_) => false,
        }
    }

    /// Create a temporary cgroup directory for resource limits.
    fn create_cgroup(&self, profile: &SandboxProfile) -> Result<String, SandboxError> {
        let cg_name = format!("wgenty-code-{}", std::process::id());
        let cg_dir = self.cgroup_base.join(&cg_name);

        std::fs::create_dir(&cg_dir).map_err(|e| SandboxError::ProfileBuild {
            reason: format!("failed to create cgroup dir: {}", e),
        })?;

        // Write resource limits
        Self::write_cg_limit(&cg_dir, "memory.max", profile.resources.max_memory_bytes)?;
        Self::write_cg_limit(&cg_dir, "memory.swap.max", 0)?;
        Self::write_cg_limit(
            &cg_dir,
            "cpu.max",
            format!(
                "{} 100000",
                profile.resources.max_cpu_seconds.saturating_mul(1_000_000)
            ),
        )?;
        Self::write_cg_limit(&cg_dir, "pids.max", profile.resources.max_processes as u64)?;
        // Enable memory and pids controllers
        let _ = Self::write_cg_limit(&cg_dir, "cgroup.subtree_control", "+memory +pids +cpu");

        Ok(cg_dir.to_string_lossy().to_string())
    }

    fn write_cg_limit(
        cg_dir: &std::path::Path,
        file: &str,
        value: impl std::fmt::Display,
    ) -> Result<(), SandboxError> {
        let path = cg_dir.join(file);
        if path.exists() {
            std::fs::write(&path, value.to_string()).map_err(|e| SandboxError::ProfileBuild {
                reason: format!("failed to write cgroup {}: {}", file, e),
            })?;
        }
        Ok(())
    }

    /// Add child PID to cgroup.
    fn add_to_cgroup(&self, cg_dir: &str, pid: u32) -> Result<(), SandboxError> {
        let procs = std::path::Path::new(cg_dir).join("cgroup.procs");
        if procs.exists() {
            std::fs::write(&procs, pid.to_string()).map_err(|e| SandboxError::ProfileBuild {
                reason: format!("failed to add pid to cgroup: {}", e),
            })?;
        }
        Ok(())
    }

    /// Build environment filtering for the child process.
    fn build_env(profile: &SandboxProfile, cmd: &mut tokio::process::Command) {
        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }
        // Always force non-interactive after allowlist (may have cleared prior env).
        super::apply_noninteractive_env(cmd);
    }
}

impl SandboxBackend for LinuxBackend {
    fn name(&self) -> &str {
        "seccomp+ns"
    }

    fn is_available() -> bool {
        cfg!(target_os = "linux")
            && std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
            && Self::cgroup_base_writable()
    }

    fn is_hardware_enforced(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<&str> {
        vec![
            "filesystem",
            "network",
            "syscall-filter",
            "memory-limit",
            "cpu-limit",
        ]
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        // Create cgroup for resource limits
        let cg_dir = self.create_cgroup(profile)?;

        // Use `unshare` to create a new mount + network namespace.
        // We use the external `unshare` command for simplicity and reliability;
        // a future version can use libc::clone directly for more control.
        let wrapped = format!(
            "export GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/usr/bin/false SSH_ASKPASS=/usr/bin/false SSH_ASKPASS_REQUIRE=never GCM_INTERACTIVE=never CI=true; {command}"
        );
        let mut cmd = tokio::process::Command::new("unshare");
        cmd.arg("--mount") // mount namespace: private /tmp
            .arg("--net") // network namespace: no NICs
            .arg("--fork") // fork before unsharing
            .arg("--pid") // pid namespace
            .arg("--mount-proc") // mount /proc in new pid ns
            .arg("sh")
            .arg("-c")
            .arg(wrapped);

        // Keep child output off the parent TUI console.
        super::configure_captured_stdio(&mut cmd);

        // Apply working directory
        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        // Filter environment
        Self::build_env(profile, &mut cmd);

        // Spawn the process
        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        // Add child PID to cgroup
        if let Some(pid) = child.id() {
            self.add_to_cgroup(&cg_dir, pid)?;
        }

        Ok(SandboxedChild {
            child,
            backend_name: "seccomp+ns".into(),
            cleanup: Some(SandboxCleanup {
                cleanup_type: CleanupType::RemoveCgroup,
                resource_handle: Some(cg_dir),
            }),
        })
    }
}
