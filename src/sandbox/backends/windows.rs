//! Windows sandbox backend.
//!
//! Uses Win32 Job Objects for:
//!   - Active process limit (`max_processes`)
//!   - Memory limit (`max_memory_bytes`)
//!   - Kill-on-job-close (process tree dies when the job handle is closed)
//!
//! Restricted Tokens (filesystem deny-only SIDs) are a planned follow-up;
//! this backend still enforces resource limits and process-tree kill when
//! Job Objects are available.
//!
//! On non-Windows targets this module still compiles as a thin stub so the
//! crate builds cross-platform; `is_available()` is always false off-Windows.

use std::path::Path;

#[cfg(windows)]
use crate::sandbox::{CleanupType, SandboxCleanup};
use crate::sandbox::{SandboxBackend, SandboxError, SandboxProfile, SandboxedChild};

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

    fn apply_env(profile: &SandboxProfile, cmd: &mut tokio::process::Command) {
        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_TIME,
    };

    /// 100-nanosecond intervals per second (FILETIME unit).
    const HUNDRED_NS_PER_SEC: i64 = 10_000_000;

    pub(super) struct JobHandle(OwnedHandle);

    impl JobHandle {
        pub fn as_raw(&self) -> HANDLE {
            self.0.as_raw_handle() as HANDLE
        }

        /// Encode the raw handle as a decimal string for `SandboxCleanup`.
        pub fn to_cleanup_token(self) -> String {
            let raw = self.0.as_raw_handle() as usize;
            // Leak into cleanup path — CloseHandle runs in perform_cleanup.
            std::mem::forget(self.0);
            raw.to_string()
        }
    }

    pub(super) fn create_job(profile: &SandboxProfile) -> Result<JobHandle, SandboxError> {
        // SAFETY: null security/name → new anonymous job, or INVALID_HANDLE_VALUE.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(SandboxError::ProfileBuild {
                reason: format!(
                    "CreateJobObjectW failed (GetLastError={})",
                    std::io::Error::last_os_error()
                ),
            });
        }
        let owned = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
        let job = JobHandle(owned);

        // SAFETY: zeroed extended limit info is valid for SetInformationJobObject.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        let basic: &mut JOBOBJECT_BASIC_LIMIT_INFORMATION = &mut info.BasicLimitInformation;

        basic.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_JOB_MEMORY;

        basic.ActiveProcessLimit = profile.resources.max_processes.max(1);

        if profile.resources.max_cpu_seconds > 0 {
            basic.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
            basic.PerProcessUserTimeLimit =
                (profile.resources.max_cpu_seconds as i64).saturating_mul(HUNDRED_NS_PER_SEC);
        }

        info.JobMemoryLimit = profile.resources.max_memory_bytes as usize;

        let ok = unsafe {
            SetInformationJobObject(
                job.as_raw(),
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(SandboxError::ProfileBuild {
                reason: format!(
                    "SetInformationJobObject failed (GetLastError={})",
                    std::io::Error::last_os_error()
                ),
            });
        }

        Ok(job)
    }

    pub(super) fn assign_pid_to_job(job: HANDLE, pid: u32) -> Result<(), SandboxError> {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() || process == INVALID_HANDLE_VALUE {
            return Err(SandboxError::ProfileBuild {
                reason: format!(
                    "OpenProcess({pid}) failed (GetLastError={})",
                    std::io::Error::last_os_error()
                ),
            });
        }

        let ok = unsafe { AssignProcessToJobObject(job, process) };
        unsafe {
            CloseHandle(process);
        }
        if ok == 0 {
            return Err(SandboxError::ProfileBuild {
                reason: format!(
                    "AssignProcessToJobObject failed (GetLastError={})",
                    std::io::Error::last_os_error()
                ),
            });
        }
        Ok(())
    }

    pub(super) fn close_job_token(token: &str) {
        if let Ok(raw) = token.parse::<usize>() {
            if raw != 0 {
                unsafe {
                    CloseHandle(raw as HANDLE);
                }
            }
        }
    }
}

impl SandboxBackend for WindowsBackend {
    fn name(&self) -> &str {
        if cfg!(windows) {
            "job-object"
        } else {
            "windows-stub"
        }
    }

    fn is_available() -> bool {
        cfg!(target_os = "windows")
    }

    fn is_hardware_enforced(&self) -> bool {
        // Job Objects provide kernel-enforced resource limits + kill-on-close.
        // Restricted Token FS isolation is not yet wired.
        cfg!(windows)
    }

    fn capabilities(&self) -> Vec<&str> {
        if cfg!(windows) {
            vec![
                "memory-limit",
                "cpu-limit",
                "process-limit",
                "kill-on-close",
                "env-filter",
            ]
        } else {
            vec!["env-filter"]
        }
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        #[cfg(windows)]
        {
            self.spawn_windows(profile, command, workdir)
        }
        #[cfg(not(windows))]
        {
            let mut cmd = tokio::process::Command::new("cmd");
            cmd.arg("/C").arg(command);
            Self::apply_env(profile, &mut cmd);
            if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
                cmd.current_dir(dir);
            }
            let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
                io_error: format!("{}", e),
            })?;
            Ok(SandboxedChild {
                child,
                backend_name: "windows-stub".into(),
                cleanup: None,
            })
        }
    }
}

#[cfg(windows)]
impl WindowsBackend {
    fn spawn_windows(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        let job = win::create_job(profile)?;

        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd.kill_on_drop(true);

        Self::apply_env(profile, &mut cmd);

        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        if let Some(pid) = child.id() {
            if let Err(e) = win::assign_pid_to_job(job.as_raw(), pid) {
                tracing::error!(
                    error = %e,
                    pid,
                    "failed to assign process to Job Object; terminating child"
                );
                drop(child);
                return Err(e);
            }
        } else {
            tracing::warn!("child has no pid; Job Object limits will not apply");
        }

        let token = job.to_cleanup_token();
        Ok(SandboxedChild {
            child,
            backend_name: "job-object".into(),
            cleanup: Some(SandboxCleanup {
                cleanup_type: CleanupType::CloseJobObject,
                resource_handle: Some(token),
            }),
        })
    }
}

/// Close a job handle from a cleanup token (called by `SandboxedChild::perform_cleanup`).
#[cfg(windows)]
pub fn close_job_handle_token(token: &str) {
    win::close_job_token(token);
}

#[cfg(not(windows))]
pub fn close_job_handle_token(_token: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_name_is_stable() {
        let b = WindowsBackend::new();
        assert!(!b.name().is_empty());
    }

    #[test]
    fn is_available_matches_target_os() {
        assert_eq!(WindowsBackend::is_available(), cfg!(target_os = "windows"));
    }

    #[test]
    fn capabilities_include_env_filter() {
        let b = WindowsBackend::new();
        assert!(b.capabilities().contains(&"env-filter"));
    }

    #[cfg(windows)]
    #[test]
    fn create_job_succeeds_with_default_profile() {
        let profile = SandboxProfile::default_for_workspace(Path::new("."));
        let job = win::create_job(&profile).expect("CreateJobObject should succeed");
        let token = job.to_cleanup_token();
        win::close_job_token(&token);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn spawn_echo_under_job_object() {
        let backend = WindowsBackend::new();
        let profile = SandboxProfile::default_for_workspace(Path::new("."));
        let child = backend
            .spawn(&profile, "echo hello-sandbox", None)
            .expect("spawn");
        assert_eq!(child.backend_name, "job-object");
        let out = child.wait_with_output().await.expect("wait");
        assert!(
            out.stdout.contains("hello-sandbox") || out.exit_code == 0,
            "stdout={} stderr={} code={}",
            out.stdout,
            out.stderr,
            out.exit_code
        );
    }
}
