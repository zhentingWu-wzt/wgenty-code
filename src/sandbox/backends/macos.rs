//! macOS Seatbelt sandbox backend.
//!
//! Uses `/usr/bin/sandbox-exec` with a dynamically generated `.sb` profile.
//! The profile is written to a temp file and cleaned up after use.
//! Seatbelt provides filesystem, network, and syscall-level restrictions.

use std::path::Path;
use std::process::Stdio;

use super::super::profile::NetworkPolicy;
use crate::sandbox::{
    CleanupType, SandboxBackend, SandboxCleanup, SandboxError, SandboxProfile, SandboxedChild,
};

pub struct MacOSBackend;

impl Default for MacOSBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MacOSBackend {
    pub fn new() -> Self {
        Self
    }

    /// Generate a Seatbelt profile using an **allow-default + precise deny**
    /// (blacklist) model.
    ///
    /// # Why blacklist, not whitelist
    ///
    /// On macOS 15.7+ (Sequoia) the Seatbelt profile compiler inside
    /// `/usr/bin/sandbox-exec` aborts (SIGABRT) the moment a profile references
    /// the `process-exec` operation in *any* form — bare, subpath, literal,
    /// wildcard, or regex. The previous `(deny default) ... (allow
    /// process-exec (subpath ...))` whitelist model therefore cannot launch a
    /// single command on those systems: `sandbox-exec` dies before the child
    /// runs. The public `sandbox_init` C API is deprecated since 10.8 and
    /// accepts only named presets, so there is no in-process escape hatch.
    ///
    /// The blacklist model sidesteps the bug entirely: `(allow default)` grants
    /// exec by default (no `process-exec` token is ever emitted), and we layer
    /// precise `(deny ...)` rules for the dangerous surfaces — network, secret
    /// files, and writes outside the workspace. Enforcement is still
    /// kernel-level and inherited by all descendants.
    ///
    /// Trade-off vs. the ideal whitelist: reads are allowed by default except
    /// for an explicit secret-deny list, and any binary on `PATH` may exec.
    /// This is a deliberate, verified downgrade to keep a *real* sandbox on
    /// modern macOS rather than silently falling back to no-op.
    fn generate_profile(profile: &SandboxProfile) -> String {
        let mut sb = String::new();

        sb.push_str("(version 1)\n");
        // Blacklist base: everything allowed unless explicitly denied. This is
        // the only model that can exec a command on macOS 15.7+ without
        // tripping the `process-exec` SIGABRT.
        sb.push_str("(allow default)\n\n");

        // ---- Network: deny by policy (default None -> fully denied). ----
        match profile.network {
            NetworkPolicy::None => {
                sb.push_str(";; Deny all network access\n");
                sb.push_str("(deny network*)\n\n");
            }
            NetworkPolicy::LoopbackOnly => {
                sb.push_str(";; Allow loopback only\n");
                sb.push_str("(allow network* (local ip))\n");
                sb.push_str("(deny network*)\n\n");
            }
            NetworkPolicy::Full => {
                sb.push_str(";; Allow full network access\n");
                sb.push_str("(allow network*)\n\n");
            }
        }

        // ---- Secret files: hard-deny reads of credential material. ----
        // These are denied regardless of the workspace, so an escaped or
        // hostile command cannot exfiltrate keys even though reads are
        // otherwise allowed under `(allow default)`.
        let home = dirs::home_dir();
        let secret_subpaths: Vec<String> = home
            .as_ref()
            .map(|h| {
                [
                    ".ssh",
                    ".aws",
                    ".gnupg",
                    ".config/gcloud",
                    ".kube",
                    ".docker",
                    ".netrc",
                    "Library/Keychains",
                    "Library/Application Support/Google/Chrome",
                    "Library/Cookies",
                ]
                .iter()
                .map(|s| h.join(s).to_string_lossy().into_owned())
                .collect()
            })
            .unwrap_or_default();
        sb.push_str(";; Hard-deny reads of secret / credential material\n");
        sb.push_str("(deny file-read*\n");
        for path in &secret_subpaths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path));
        }
        sb.push_str(")\n\n");

        // ---- Writes: deny everything under $HOME, then re-allow the
        // workspace and /tmp. Under `(allow default)` file writes would be
        // unrestricted; this fence confines writes to approved dirs while
        // keeping reads (minus secrets) open for dev tooling. ----
        if let Some(ref home) = home {
            sb.push_str(";; Deny writes under the user home, then carve out allows\n");
            sb.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n\n",
                home.display()
            ));
        }

        sb.push_str(";; Allow writes to approved paths\n");
        sb.push_str("(allow file-write*\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
        }
        sb.push_str(")\n\n");

        sb.push_str(";; Allow file deletion (unlink) within writable paths\n");
        sb.push_str("(allow file-write-unlink\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")", path.display()));
            sb.push('\n');
        }
        sb.push_str(")\n\n");

        // Subprocess creation. Under `(allow default)` fork is already
        // permitted; we still honor the profile flag for parity with the
        // other backends by leaving an explicit allow (a no-op here) when
        // requested. There is no kernel-level way to *forbid* fork without
        // `process-exec`-style filtering on this macOS version, so
        // `allow_subprocess=false` is advisory only here.
        if profile.allow_subprocess {
            sb.push_str(";; Allow subprocess creation\n");
            sb.push_str("(allow process-fork)\n\n");
        }

        sb.push_str(";; System necessities\n");
        sb.push_str("(allow file-read* file-write*\n");
        sb.push_str("    (subpath \"/dev/null\")\n");
        sb.push_str("    (subpath \"/dev/zero\")\n");
        sb.push_str("    (subpath \"/dev/random\")\n");
        sb.push_str("    (subpath \"/dev/urandom\")\n");
        sb.push_str("    (subpath \"/dev/stdin\")\n");
        sb.push_str("    (subpath \"/dev/stdout\")\n");
        sb.push_str("    (subpath \"/dev/stderr\")\n");
        sb.push_str("    (subpath \"/dev/tty\")\n");
        sb.push_str("    (subpath \"/dev/dtracehelper\")\n");
        sb.push_str(")\n");
        sb.push_str("(allow sysctl-read)\n");
        sb.push_str("(allow signal)\n");
        sb.push_str("(allow process-info*)\n");

        sb
    }
}

impl SandboxBackend for MacOSBackend {
    fn name(&self) -> &str {
        "seatbelt"
    }

    fn is_available() -> bool {
        if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").exists() {
            return false;
        }
        // Probe: run a real command through the *same* blacklist profile
        // grammar that `generate_profile` emits. This catches the macOS 15.7+
        // Seatbelt compiler bug where profiles referencing `process-exec`
        // SIGABRT before the child runs - and, by mirroring the production
        // grammar, guarantees the probe verdict reflects real execution rather
        // than just "parsed without crashing". We require the child's stdout to
        // match a sentinel so a silently-aborted `sandbox-exec` (signal death,
        // empty stdout) is correctly treated as unavailable.
        let probe_profile = "(version 1)\n(allow default)\n(deny network*)\n(allow process-fork)\n";
        let tmp =
            std::env::temp_dir().join(format!("wgenty-sandbox-probe-{}.sb", std::process::id()));
        if std::fs::write(&tmp, probe_profile.as_bytes()).is_err() {
            return false;
        }
        let result = std::process::Command::new("/usr/bin/sandbox-exec")
            .arg("-f")
            .arg(&tmp)
            .arg("/bin/sh")
            .arg("-c")
            .arg("printf probe-ok")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let _ = std::fs::remove_file(&tmp);
        match result {
            Ok(out) => {
                // Signal death (SIGABRT) -> code None; a parsed-but-denied
                // child still exits normally. Accept only a clean exit whose
                // stdout carries the sentinel.
                let clean = out.status.code().is_some()
                    && String::from_utf8_lossy(&out.stdout).contains("probe-ok");
                if !clean {
                    tracing::warn!(
                        exit_code = ?out.status.code(),
                        stdout = %String::from_utf8_lossy(&out.stdout),
                        stderr = %String::from_utf8_lossy(&out.stderr),
                        "sandbox-exec probe did not run the child cleanly; \
                         falling back to no-op sandbox backend"
                    );
                    return false;
                }
                true
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "sandbox-exec probe failed to spawn; falling back to no-op backend"
                );
                false
            }
        }
    }

    fn is_hardware_enforced(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<&str> {
        vec!["filesystem", "network", "syscall-filter"]
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        let sb_profile = Self::generate_profile(profile);

        let tmp_dir = std::env::temp_dir();
        // Include a UUID so concurrent sandboxed commands in the same process
        // don't clobber or delete each other's profile files during cleanup.
        let profile_path = tmp_dir.join(format!(
            "claude-sandbox-{}-{}.sb",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&profile_path, sb_profile.as_bytes()).map_err(|e| {
            SandboxError::ProfileBuild {
                reason: format!("failed to write seatbelt profile: {}", e),
            }
        })?;

        // Absolute paths only: after env_clear, bare `sh` can ENOENT if PATH is
        // missing/wrong. Codex also pins sandbox-exec to /usr/bin.
        let mut cmd = tokio::process::Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-f")
            .arg(&profile_path)
            .arg("/bin/sh")
            .arg("-c")
            .arg(command);

        // Keep child output off the parent TUI console.
        super::configure_captured_stdio(&mut cmd);

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

        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        let profile_path_owned = profile_path.clone();
        let cleanup = SandboxCleanup {
            cleanup_type: CleanupType::DeleteTempFile,
            resource_handle: Some(profile_path_owned.to_string_lossy().to_string()),
        };

        Ok(SandboxedChild {
            child,
            backend_name: "seatbelt".into(),
            cleanup: Some(cleanup),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_profile() -> SandboxProfile {
        SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"))
    }

    #[test]
    fn profile_uses_blacklist_model() {
        // The profile must NOT emit `process-exec` - on macOS 15.7+ that
        // operation makes sandbox-exec SIGABRT before the child runs. The
        // blacklist `(allow default) ... (deny ...)` model is the only form
        // that can launch a command on those systems.
        let sb = MacOSBackend::generate_profile(&ws_profile());
        assert!(
            !sb.contains("process-exec"),
            "seatbelt profile must not reference process-exec (SIGABRT on 15.7+): {sb}"
        );
        assert!(
            sb.contains("(allow default)"),
            "seatbelt profile must be built on (allow default): {sb}"
        );
        assert!(
            !sb.contains("(deny default)"),
            "seatbelt profile must not use (deny default) anymore: {sb}"
        );
    }

    #[test]
    fn profile_denies_secrets_and_home_writes() {
        let sb = MacOSBackend::generate_profile(&ws_profile());
        let home = dirs::home_dir().expect("home dir");

        // Secret material is hard-deny-read regardless of workspace.
        for secret in [".ssh", ".aws", "Library/Keychains"] {
            let p = home.join(secret).to_string_lossy().into_owned();
            assert!(
                sb.contains(&format!("(subpath \"{}\")", p)),
                "seatbelt profile should hard-deny read of secret {}: {sb}",
                p
            );
        }
        // Writes under $HOME are denied, then the workspace re-allowed.
        assert!(
            sb.contains(&format!(
                "(deny file-write* (subpath \"{}\"))",
                home.display()
            )),
            "seatbelt profile should deny writes under $HOME: {sb}"
        );
        assert!(
            sb.contains("(subpath \"/tmp/test-ws\")"),
            "seatbelt profile should re-allow writes to the workspace: {sb}"
        );
    }

    #[test]
    fn profile_respects_network_policy() {
        let mut p = ws_profile();
        p.network = NetworkPolicy::None;
        assert!(MacOSBackend::generate_profile(&p).contains("(deny network*)"));

        p.network = NetworkPolicy::Full;
        assert!(MacOSBackend::generate_profile(&p).contains("(allow network*)"));

        p.network = NetworkPolicy::LoopbackOnly;
        let sb = MacOSBackend::generate_profile(&p);
        assert!(sb.contains("(allow network* (local ip))"));
        assert!(sb.contains("(deny network*)"));
    }

    /// End-to-end: spawn a real command under the generated profile and verify
    /// the sandbox actually confines writes. macOS-only; skipped elsewhere
    /// because `sandbox-exec` only exists there.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn seatbelt_profile_confines_writes_end_to_end() {
        if !MacOSBackend::is_available() {
            eprintln!("skipping: macOS seatbelt backend not available");
            return;
        }

        let ws = std::env::temp_dir().join("wgenty-sb-e2e-ws");
        std::fs::create_dir_all(&ws).unwrap();
        // A scratch file *outside* the workspace but inside $HOME to probe the
        // write fence. Use a uniquely-named file under the home dir.
        let home = dirs::home_dir().expect("home dir");
        let escape_target = home.join(format!(
            "wgenty-sb-e2e-escape-{}.txt",
            uuid::Uuid::new_v4().simple()
        ));
        let ws_target = ws.join("inside.txt");

        let profile = SandboxProfile::default_for_workspace(&ws);
        let backend = MacOSBackend::new();
        // Command writes to both targets; the outside one must be denied.
        let cmd = format!(
            "echo inside > {ws} && echo escape > {esc} 2>/dev/null; \
             [ -f {esc} ] && echo ESCAPED || echo CONFINED",
            ws = ws_target.display(),
            esc = escape_target.display()
        );
        let child = backend
            .spawn(&profile, &cmd, Some(&ws))
            .expect("spawn under seatbelt");
        let out = child.wait_with_output().await.expect("wait");

        assert_eq!(
            out.exit_code, 0,
            "command should exit 0; stderr={}",
            out.stderr
        );
        assert!(
            out.stdout.contains("CONFINED") && !out.stdout.contains("ESCAPED"),
            "sandbox must confine writes outside the workspace; stdout={:?}",
            out.stdout
        );
        // Belt-and-suspenders: the escape file must not exist on disk.
        assert!(
            !escape_target.exists(),
            "escape target was written despite the sandbox: {}",
            escape_target.display()
        );
        assert!(ws_target.exists(), "workspace write should be allowed");

        // Cleanup.
        let _ = std::fs::remove_file(&ws_target);
        let _ = std::fs::remove_file(&escape_target);
        let _ = std::fs::remove_dir_all(&ws);
    }
}
