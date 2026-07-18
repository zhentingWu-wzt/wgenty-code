//! Windows sandbox backend.
//!
//! Uses Win32 Job Objects for:
//!   - Active process limit (`max_processes`)
//!   - Memory limit (`max_memory_bytes`)
//!   - Kill-on-job-close (process tree dies when the job handle is closed)
//!
//! Processes are created with `CREATE_SUSPENDED`, assigned to the job, then
//! resumed — closing the spawn→assign race where grandchildren could escape
//! the job before assignment completed.
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

    /// Resume every thread in `pid` after a `CREATE_SUSPENDED` spawn.
    ///
    /// tokio does not expose the primary thread handle from `Command::spawn`,
    /// so we snapshot threads and call `ResumeThread` on each owned by `pid`.
    pub(super) fn resume_process_threads(pid: u32) -> Result<(), SandboxError> {
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        };
        use windows_sys::Win32::System::Threading::{
            OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        };

        // SAFETY: TH32CS_SNAPTHREAD with process id 0 snapshots all threads.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return Err(SandboxError::Spawn {
                io_error: format!(
                    "CreateToolhelp32Snapshot failed (GetLastError={})",
                    std::io::Error::last_os_error()
                ),
            });
        }

        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;

        let mut resumed = 0u32;
        let mut ok = unsafe { Thread32First(snap, &mut entry) };
        while ok != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() && thread != INVALID_HANDLE_VALUE {
                    // ResumeThread returns previous suspend count, or u32::MAX on error.
                    let prev = unsafe { ResumeThread(thread) };
                    unsafe {
                        CloseHandle(thread);
                    }
                    if prev == u32::MAX {
                        unsafe {
                            CloseHandle(snap);
                        }
                        return Err(SandboxError::Spawn {
                            io_error: format!(
                                "ResumeThread({}) failed (GetLastError={})",
                                entry.th32ThreadID,
                                std::io::Error::last_os_error()
                            ),
                        });
                    }
                    resumed += 1;
                }
            }
            ok = unsafe { Thread32Next(snap, &mut entry) };
        }

        unsafe {
            CloseHandle(snap);
        }

        if resumed == 0 {
            return Err(SandboxError::Spawn {
                io_error: format!(
                    "no threads found to resume for pid {pid} after CREATE_SUSPENDED spawn"
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
            let mut cmd = super::shell_command_captured(command);
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

        // Platform shell + piped stdio / CREATE_NO_WINDOW.
        use std::os::windows::process::CommandExt;
        let mut cmd = super::shell_command_captured(command);
        // creation_flags replaces the whole mask (not OR), so re-set every flag
        // the helper already applied plus the Job Object spawn requirements:
        // - CREATE_SUSPENDED: assign to job before any user code runs
        // - CREATE_NO_WINDOW: keep console apps off the parent TUI surface
        // - CREATE_BREAKAWAY_FROM_JOB: GHA/CI (and some shells) already run the
        //   parent inside a Job Object that forbids nesting; without breakaway,
        //   AssignProcessToJobObject fails with ERROR_ACCESS_DENIED and every
        //   Windows sandbox spawn/test dies. No-op when the parent is not in a
        //   job (or the parent job allows breakaway).
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);

        Self::apply_env(profile, &mut cmd);

        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        // ERROR_ACCESS_DENIED on CreateProcess typically means the parent is
        // already inside a Job Object whose limits forbid breakaway (common on
        // GHA Windows runners). CREATE_BREAKAWAY_FROM_JOB then fails the spawn
        // outright; retry as a plain captured spawn so the tool still runs
        // instead of hard-failing every sandboxed spawn under HardFail.
        const ERROR_ACCESS_DENIED: i32 = 5;
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                tracing::warn!(
                    error = %e,
                    "sandboxed spawn denied (parent job forbids breakaway); \
                     degrading to direct spawn without job limits"
                );
                let token = job.to_cleanup_token();
                win::close_job_token(&token);
                return Self::spawn_degraded(profile, command, workdir);
            }
            Err(e) => {
                let token = job.to_cleanup_token();
                win::close_job_token(&token);
                return Err(SandboxError::Spawn {
                    io_error: format!("{}", e),
                });
            }
        };

        let Some(pid) = child.id() else {
            // Suspended child with no pid cannot be assigned or resumed safely.
            drop(child); // kill_on_drop
            return Err(SandboxError::Spawn {
                io_error: "child has no pid after CREATE_SUSPENDED spawn".into(),
            });
        };

        // Prefer full Job Object isolation. If assignment fails (common when the
        // parent is already inside a non-breakaway job, e.g. some CI runners),
        // resume the child and continue without job limits rather than failing
        // every sandboxed spawn.
        let cleanup = match win::assign_pid_to_job(job.as_raw(), pid) {
            Ok(()) => {
                let token = job.to_cleanup_token();
                Some(SandboxCleanup {
                    cleanup_type: CleanupType::CloseJobObject,
                    resource_handle: Some(token),
                })
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    pid,
                    "AssignProcessToJobObject failed; resuming child without job limits"
                );
                // Drop the unused job handle (OwnedHandle closes on drop via token path;
                // here we still hold `job` — convert to token and close immediately).
                let token = job.to_cleanup_token();
                win::close_job_token(&token);
                None
            }
        };

        if let Err(e) = win::resume_process_threads(pid) {
            tracing::error!(
                error = %e,
                pid,
                "failed to resume process after CREATE_SUSPENDED spawn; terminating child"
            );
            drop(child); // kill_on_drop
            return Err(e);
        }

        Ok(SandboxedChild {
            child,
            backend_name: if cleanup.is_some() {
                "job-object".into()
            } else {
                "job-object-degraded".into()
            },
            cleanup,
        })
    }

    /// Fallback spawn without Job Object isolation or `CREATE_SUSPENDED`.
    ///
    /// Used when the full sandboxed spawn is denied (`ERROR_ACCESS_DENIED`,
    /// typically because the parent Job Object forbids breakaway on CI
    /// runners). Keeps `CREATE_NO_WINDOW` + captured stdio so child output does
    /// not corrupt the parent TUI, but runs with no resource limits and reports
    /// `job-object-degraded` so callers/UI can mark the lack of enforcement.
    fn spawn_degraded(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        use std::os::windows::process::CommandExt;
        let mut cmd = super::shell_command_captured(command);
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        Self::apply_env(profile, &mut cmd);
        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }
        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;
        Ok(SandboxedChild {
            child,
            backend_name: "job-object-degraded".into(),
            cleanup: None,
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
        // Full isolation ("job-object") or CI degraded mode ("job-object-degraded")
        // when the parent process is already inside a non-breakaway Job Object.
        assert!(
            child.backend_name == "job-object" || child.backend_name == "job-object-degraded",
            "unexpected backend_name {}",
            child.backend_name
        );
        // stdout must be piped/captured — empty stdout previously passed because
        // the assertion allowed exit_code == 0 while output leaked to the console.
        let out = child.wait_with_output().await.expect("wait");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(
            out.stdout.contains("hello-sandbox"),
            "stdout must be captured (not inherited by parent console): stdout={:?} stderr={:?}",
            out.stdout,
            out.stderr
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn spawn_captures_stdout_not_empty_for_console_app() {
        let backend = WindowsBackend::new();
        let profile = SandboxProfile::default_for_workspace(Path::new("."));
        // cmd's built-in echo is a classic console writer; CREATE_NO_WINDOW +
        // pipes must still deliver the bytes to wait_with_output.
        let child = backend
            .spawn(&profile, "echo capture-check", None)
            .expect("spawn");
        let out = child.wait_with_output().await.expect("wait");
        assert!(
            out.stdout.to_ascii_lowercase().contains("capture-check"),
            "expected captured stdout, got {:?}",
            out.stdout
        );
    }
}
