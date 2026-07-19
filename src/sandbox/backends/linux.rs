//! Linux sandbox backend.
//!
//! Current enforcement (two paths, selected at construction time):
//!
//! - **`bwrap` path** (preferred, when `bwrap` binary is available):
//!   1. `bubblewrap` builds a confined FS view: `--ro-bind / /` (entire root
//!      read-only) + `--bind <writable_paths>` (workspace/tmp writable) +
//!      `--tmpfs <secret>` (credential dirs hidden). This is the only path
//!      that honors `writable_paths`/`readable_paths` - enforcement fidelity
//!      is `full`, on par with macOS seatbelt.
//!   2. `--unshare-net` when `NetworkPolicy::None`/`LoopbackOnly`.
//!   3. cgroups v2 resource limits (memory, cpu, pids).
//!
//! - **`seccomp+ns` path** (fallback, when `bwrap` is absent):
//!   1. `unshare --user --map-root-user --mount --pid --mount-proc` (namespace
//!      isolation; unprivileged: user ns grants the needed caps inside).
//!   2. tmpfs secret-deny inside the new mount namespace (best-effort).
//!   3. `NetworkPolicy` honored via `--net` toggle.
//!   4. cgroups v2 resource limits.
//!   - Does NOT enforce `writable_paths`/`readable_paths` - fidelity is
//!     `partial`.
//!
//! NOTE: the backend name `seccomp+ns` is historical - seccomp-bpf syscall
//! filtering is NOT yet wired to libseccomp (tracked as a follow-up, see
//! docs/SANDBOX.md). `is_hardware_enforced` reflects kernel-level enforcement
//! (namespace+cgroup+bwrap), not syscall filtering.
//!
//! Falls back gracefully: if `unshare`/cgroups are unavailable, the
//! NoneBackend takes over.

use std::path::{Path, PathBuf};

use crate::sandbox::{
    CleanupType, NetworkPolicy, SandboxBackend, SandboxCleanup, SandboxError, SandboxProfile,
    SandboxedChild,
};

pub struct LinuxBackend {
    cgroup_base: PathBuf,
    /// Whether `bwrap` (bubblewrap) is available. When true, spawn uses the
    /// bwrap path (full FS confinement); when false, falls back to the
    /// unshare path (namespace + tmpfs secret-deny, no writable_paths fence).
    has_bwrap: bool,
}

impl Default for LinuxBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxBackend {
    pub fn new() -> Self {
        Self {
            cgroup_base: PathBuf::from("/sys/fs/cgroup"),
            has_bwrap: Self::bwrap_available(),
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
        let probe =
            Path::new("/sys/fs/cgroup").join(format!(".wgenty-probe-{}", std::process::id()));
        match std::fs::create_dir(&probe) {
            Ok(()) => {
                let _ = std::fs::remove_dir(&probe);
                true
            }
            Err(_) => false,
        }
    }

    /// Probe whether `unshare` can create the namespace set we need.
    ///
    /// `--mount --net --pid` require `CAP_SYS_ADMIN` on the host unless paired
    /// with `--user --map-root-user`: the user namespace grants the needed
    /// caps (CAP_SYS_ADMIN for mount/pid, CAP_NET_ADMIN for net) *inside* the
    /// new namespace, so unprivileged hosts can still isolate. On kernels or
    /// container runtimes that disable unprivileged user namespaces, this probe
    /// fails and [`create_backend`](super::create_backend) falls back to
    /// NoneBackend instead of failing every sandboxed spawn under
    /// `FailMode::HardFail`.
    ///
    /// The probe mirrors the production spawn argument set (including
    /// `--mount-proc`) so a success here guarantees the real spawn can create
    /// the same namespaces.
    fn unshare_available() -> bool {
        use std::process::Stdio;
        let result = std::process::Command::new("unshare")
            .arg("--user")
            .arg("--map-root-user")
            .arg("--mount")
            .arg("--net")
            .arg("--pid")
            .arg("--fork")
            .arg("--mount-proc")
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        matches!(result, Ok(s) if s.success())
    }

    /// Probe whether `bwrap` (bubblewrap) is installed and callable.
    ///
    /// `bwrap --version` verifies the binary exists and runs; it does not
    /// create namespaces. The unshare probe ([`unshare_available`]) already
    /// covers user-namespace capability, which bwrap relies on internally.
    fn bwrap_available() -> bool {
        use std::process::Stdio;
        let result = std::process::Command::new("bwrap")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        matches!(result, Ok(s) if s.success())
    }

    /// Credential directories to hide from the sandboxed child.
    ///
    /// Mirrors the macOS seatbelt secret-deny list (Linux equivalents). When
    /// `NetworkPolicy::Full`, `.ssh` is left readable so `git push` / `gh` can
    /// use the user's SSH keys - matching the macOS Keychain/`.ssh` carve-out.
    fn secret_paths(profile: &SandboxProfile) -> Vec<PathBuf> {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => return Vec::new(),
        };
        let allow_remote_auth = matches!(profile.network, NetworkPolicy::Full);
        let mut secret_dirs: Vec<&str> =
            vec![".aws", ".gnupg", ".config/gcloud", ".kube", ".docker"];
        // Remote-auth material only denied when egress is off (Plan/High).
        if !allow_remote_auth {
            secret_dirs.push(".ssh");
        }
        secret_dirs.iter().map(|s| home.join(s)).collect()
    }

    /// Build a shell snippet that mounts a read-only tmpfs over each credential
    /// directory inside the unshare path's mount namespace.
    ///
    /// Best-effort: a failed `mount` (missing util-linux, readonly ns, ...) is
    /// swallowed by `2>/dev/null` so the command still runs - the secret stays
    /// readable in that edge case. Only directory-type secrets are covered;
    /// file secrets like `.netrc` are TODO (would need bind-mount `/dev/null`).
    fn build_secret_guard(profile: &SandboxProfile) -> String {
        let mut guard = String::new();
        for p in Self::secret_paths(profile) {
            let p_str = p.to_string_lossy();
            // `[ -d ]` gates directory-only secrets. `ro` tmpfs hides the
            // original contents behind an empty read-only mount.
            guard.push_str(&format!(
                "[ -d '{p_str}' ] && mount -t tmpfs -o ro tmpfs '{p_str}' 2>/dev/null; "
            ));
        }
        guard
    }

    /// Build the `bwrap` argument list for a sandboxed spawn.
    ///
    /// FS view construction (order matters - later mounts override earlier):
    ///   1. `--ro-bind / /`         entire root read-only (writes denied)
    ///   2. `--proc /proc`           procfs (overrides ro-bind's /proc)
    ///   3. `--dev /dev`             minimal devtmpfs (no /dev/tty - matches
    ///      macOS seatbelt /dev/tty deny)
    ///   4. `--bind <writable>`      writable paths override ro-bind -> writable
    ///   5. `--tmpfs <secret>`       credential dirs -> empty read-only dir
    ///
    /// Network: `--unshare-net` for `None`/`LoopbackOnly`; inherit host net
    /// for `Full`. `LoopbackOnly` best-effort brings up `lo` inside the ns.
    fn build_bwrap_args(profile: &SandboxProfile, command: &str) -> Vec<String> {
        // 1-3. Read-only root + /proc + /dev. /proc and /dev must be real mounts
        //       (bwrap refuses to bind them from the ro-bind). --dev provides
        //       /dev/null /dev/zero /dev/random etc. but NOT /dev/tty, matching
        //       macOS seatbelt behavior.
        let mut args: Vec<String> = vec![
            "--ro-bind".into(),
            "/".into(),
            "/".into(),
            "--proc".into(),
            "/proc".into(),
            "--dev".into(),
            "/dev".into(),
        ];

        // 4. Writable paths override the ro-bind -> become writable.
        //    Typically workspace + /tmp.
        for path in &profile.writable_paths {
            args.push("--bind".into());
            args.push(path.to_string_lossy().into_owned());
            args.push(path.to_string_lossy().into_owned());
        }

        // 5. Secret-deny: tmpfs overlay hides credential dirs (empty ro dir).
        for path in Self::secret_paths(profile) {
            args.push("--tmpfs".into());
            args.push(path.to_string_lossy().into_owned());
        }

        // Network policy.
        let mut net_up_prefix = "";
        match profile.network {
            NetworkPolicy::None => {
                args.push("--unshare-net".into());
            }
            NetworkPolicy::LoopbackOnly => {
                args.push("--unshare-net".into());
                // bwrap --unshare-net creates a ns with lo DOWN. Best-effort
                // bring-up; degrades to None (no loopback) if `ip` is absent.
                net_up_prefix = "ip link set lo up 2>/dev/null || true; ";
            }
            NetworkPolicy::Full => {
                // Host network inherited (no --unshare-net).
            }
        }

        // Run via sh -c with non-interactive prefix + loopback up + command.
        let wrapped = format!(
            "export GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/usr/bin/false \
             SSH_ASKPASS=/usr/bin/false SSH_ASKPASS_REQUIRE=never \
             GCM_INTERACTIVE=never CI=true; {net_up_prefix}{command}"
        );
        args.push("sh".into());
        args.push("-c".into());
        args.push(wrapped);

        args
    }

    /// Build the `unshare` argument list (fallback when bwrap is absent).
    ///
    /// Namespace isolation + tmpfs secret-deny inside the new mount namespace.
    /// Does NOT enforce writable_paths (mount namespace alone cannot remount
    /// host mounts read-only without pivot_root).
    fn build_unshare_args(profile: &SandboxProfile, command: &str) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--user".into(),
            "--map-root-user".into(),
            "--mount".into(),
            "--fork".into(),
            "--pid".into(),
            "--mount-proc".into(),
        ];

        // Network policy.
        let mut net_up_prefix = "";
        match profile.network {
            NetworkPolicy::None => {
                args.push("--net".into());
            }
            NetworkPolicy::LoopbackOnly => {
                args.push("--net".into());
                net_up_prefix = "ip link set lo up 2>/dev/null || true; ";
            }
            NetworkPolicy::Full => {
                // No --net: child shares the host network stack.
            }
        }

        // Secret-deny: mount ro tmpfs over credential dirs inside the new
        // mount namespace. See `build_secret_guard` for rationale.
        let secret_guard = Self::build_secret_guard(profile);
        let wrapped = format!(
            "export GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/usr/bin/false \
             SSH_ASKPASS=/usr/bin/false SSH_ASKPASS_REQUIRE=never \
             GCM_INTERACTIVE=never CI=true; {net_up_prefix}{secret_guard}{command}"
        );
        args.push("sh".into());
        args.push("-c".into());
        args.push(wrapped);

        args
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
        cg_dir: &Path,
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
        let procs = Path::new(cg_dir).join("cgroup.procs");
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
        // Runtime backend name reflects the actual spawn path; this is also
        // what `enforcement_fidelity()` keys on.
        if self.has_bwrap {
            "bwrap"
        } else {
            "seccomp+ns"
        }
    }

    fn is_available() -> bool {
        cfg!(target_os = "linux")
            && Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
            && Self::cgroup_base_writable()
            && Self::unshare_available()
    }

    fn is_hardware_enforced(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<&str> {
        // bwrap path adds writable_paths confinement ("filesystem-write");
        // the unshare fallback cannot enforce writable_paths.
        if self.has_bwrap {
            vec![
                "filesystem",
                "filesystem-write",
                "network",
                "secret-deny",
                "memory-limit",
                "cpu-limit",
                "process-limit",
            ]
        } else {
            vec![
                "filesystem",
                "network",
                "secret-deny",
                "memory-limit",
                "cpu-limit",
                "process-limit",
            ]
        }
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        // Create cgroup for resource limits (shared by both paths).
        let cg_dir = self.create_cgroup(profile)?;

        // Select spawn path: bwrap (full FS confinement) or unshare (fallback).
        let (backend_name, args): (&str, Vec<String>) = if self.has_bwrap {
            ("bwrap", Self::build_bwrap_args(profile, command))
        } else {
            ("seccomp+ns", Self::build_unshare_args(profile, command))
        };

        let binary = if self.has_bwrap { "bwrap" } else { "unshare" };
        let mut cmd = tokio::process::Command::new(binary);
        cmd.args(&args);

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
            backend_name: backend_name.into(),
            cleanup: Some(SandboxCleanup {
                cleanup_type: CleanupType::RemoveCgroup,
                resource_handle: Some(cg_dir),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::profile::ResourceLimits;

    fn profile_with_network(network: NetworkPolicy) -> SandboxProfile {
        SandboxProfile {
            readable_paths: vec![],
            writable_paths: vec![PathBuf::from("/tmp/ws"), std::env::temp_dir()],
            full_disk_read: true,
            network,
            resources: ResourceLimits::default(),
            env_allowlist: vec!["*".into()],
            allow_subprocess: true,
            workdir: Some(PathBuf::from("/tmp/ws")),
        }
    }

    #[test]
    fn secret_paths_denies_ssh_when_network_off() {
        let paths = LinuxBackend::secret_paths(&profile_with_network(NetworkPolicy::None));
        // .ssh must be denied when network is off (Plan/High).
        assert!(
            paths.iter().any(|p| p.ends_with(".ssh")),
            ".ssh should be in secret paths when network is None: {paths:?}"
        );
    }

    #[test]
    fn secret_paths_allows_ssh_when_network_full() {
        let paths = LinuxBackend::secret_paths(&profile_with_network(NetworkPolicy::Full));
        // .ssh must NOT be denied when network is Full (git/gh need keys).
        assert!(
            !paths.iter().any(|p| p.ends_with(".ssh")),
            ".ssh should NOT be in secret paths when network is Full: {paths:?}"
        );
    }

    #[test]
    fn secret_paths_always_denies_core_secrets() {
        let paths = LinuxBackend::secret_paths(&profile_with_network(NetworkPolicy::Full));
        // These are denied regardless of network policy.
        for secret in [".aws", ".gnupg", ".kube", ".docker", ".config/gcloud"] {
            assert!(
                paths.iter().any(|p| p.ends_with(secret)),
                "{secret} should always be in secret paths: {paths:?}"
            );
        }
    }

    #[test]
    fn capabilities_do_not_claim_seccomp() {
        // P1#5: capabilities must not advertise syscall-filter (seccomp not wired).
        let backend = LinuxBackend::new();
        assert!(
            !backend.capabilities().contains(&"syscall-filter"),
            "capabilities must not claim syscall-filter: {:?}",
            backend.capabilities()
        );
    }

    #[test]
    fn bwrap_args_include_ro_bind_root() {
        // The foundation of writable_paths confinement: entire root read-only.
        let args =
            LinuxBackend::build_bwrap_args(&profile_with_network(NetworkPolicy::None), "echo test");
        let ro_bind_idx = args.iter().position(|a| a == "--ro-bind");
        assert!(ro_bind_idx.is_some(), "must have --ro-bind: {args:?}");
        // After --ro-bind should be "/" "/" (src dest).
        let idx = ro_bind_idx.unwrap();
        assert_eq!(args.get(idx + 1), Some(&"/".to_string()));
        assert_eq!(args.get(idx + 2), Some(&"/".to_string()));
    }

    #[test]
    fn bwrap_args_bind_writable_paths() {
        // writable_paths must be --bind (writable overlay on ro root).
        let profile = profile_with_network(NetworkPolicy::None);
        let args = LinuxBackend::build_bwrap_args(&profile, "echo test");
        for wp in &profile.writable_paths {
            let wp_str = wp.to_string_lossy().into_owned();
            let mut found = false;
            for (i, a) in args.iter().enumerate() {
                if a == "--bind" && args.get(i + 1) == Some(&wp_str) {
                    found = true;
                    break;
                }
            }
            assert!(found, "writable path {wp_str} must be --bind: {args:?}");
        }
    }

    #[test]
    fn bwrap_args_tmpfs_secret_dirs() {
        // Secret dirs must be --tmpfs (empty overlay hiding originals).
        let profile = profile_with_network(NetworkPolicy::None);
        let args = LinuxBackend::build_bwrap_args(&profile, "echo test");
        let secrets = LinuxBackend::secret_paths(&profile);
        for secret in &secrets {
            let s_str = secret.to_string_lossy().into_owned();
            let mut found = false;
            for (i, a) in args.iter().enumerate() {
                if a == "--tmpfs" && args.get(i + 1) == Some(&s_str) {
                    found = true;
                    break;
                }
            }
            assert!(found, "secret {s_str} must be --tmpfs: {args:?}");
        }
    }

    #[test]
    fn bwrap_args_unshare_net_when_network_off() {
        let args =
            LinuxBackend::build_bwrap_args(&profile_with_network(NetworkPolicy::None), "echo test");
        assert!(
            args.iter().any(|a| a == "--unshare-net"),
            "NetworkPolicy::None must --unshare-net: {args:?}"
        );
    }

    #[test]
    fn bwrap_args_share_net_when_network_full() {
        let args =
            LinuxBackend::build_bwrap_args(&profile_with_network(NetworkPolicy::Full), "echo test");
        assert!(
            !args.iter().any(|a| a == "--unshare-net"),
            "NetworkPolicy::Full must NOT --unshare-net: {args:?}"
        );
    }

    #[test]
    fn bwrap_args_proc_and_dev_present() {
        // /proc and /dev must be real mounts (bwrap refuses to bind them).
        let args =
            LinuxBackend::build_bwrap_args(&profile_with_network(NetworkPolicy::None), "echo test");
        assert!(
            args.iter().any(|a| a == "--proc"),
            "must have --proc: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--dev"),
            "must have --dev: {args:?}"
        );
    }

    #[test]
    fn unshare_args_include_user_map_root() {
        // Fallback path must use --user --map-root-user (unprivileged support).
        let args = LinuxBackend::build_unshare_args(
            &profile_with_network(NetworkPolicy::None),
            "echo test",
        );
        assert!(
            args.iter().any(|a| a == "--user"),
            "unshare args must have --user: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--map-root-user"),
            "unshare args must have --map-root-user: {args:?}"
        );
    }

    #[test]
    fn unshare_args_include_secret_guard() {
        // Fallback path injects tmpfs secret-deny into the sh -c body.
        let args = LinuxBackend::build_unshare_args(
            &profile_with_network(NetworkPolicy::None),
            "echo test",
        );
        let last = args.last().expect("must have sh -c body");
        assert!(
            last.contains("mount -t tmpfs"),
            "unshare body must include tmpfs secret-deny: {last}"
        );
    }
}
