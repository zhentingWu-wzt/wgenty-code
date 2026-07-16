use crate::tools::{ToolError, ToolOutput};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, RwLock};

use crate::sandbox::{
    shell_command_captured, SandboxConfig, SandboxManager, SandboxProfile, SecurityLevel,
};

pub struct CommandSessionManager {
    sandbox: Option<Arc<SandboxManager>>,
    sandbox_profile: Option<SandboxProfile>,
    sessions: RwLock<HashMap<u64, Arc<CommandSessionHandle>>>,
    next_id: AtomicU64,
}

struct CommandSessionHandle {
    cwd: PathBuf,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
    state: Arc<SessionState>,
}

struct SessionState {
    stdout: Mutex<Vec<u8>>,
    stderr: Mutex<Vec<u8>>,
    stdout_offset: Mutex<usize>,
    stderr_offset: Mutex<usize>,
    exit_status: RwLock<Option<i32>>,
}

pub struct SessionChunk {
    pub session_id: u64,
    pub stdout: String,
    pub stderr: String,
    pub combined: String,
    pub exit_code: Option<i32>,
    pub finished: bool,
}

impl CommandSessionManager {
    pub fn new() -> Self {
        Self {
            sandbox: None,
            sandbox_profile: None,
            sessions: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Attach a sandbox manager. All future spawns will run inside the sandbox.
    pub fn with_sandbox(mut self, sandbox: Arc<SandboxManager>) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    /// Set the sandbox profile for future spawns.
    pub fn with_sandbox_profile(mut self, profile: SandboxProfile) -> Self {
        self.sandbox_profile = Some(profile);
        self
    }

    /// Build a Default sandbox profile for the given workspace.
    fn default_profile(&self, cwd: &std::path::Path) -> SandboxProfile {
        SandboxConfig::builder(cwd.to_path_buf())
            .security_level(SecurityLevel::Minimal)
            .build()
    }

    pub async fn spawn(&self, command: &str, workdir: Option<PathBuf>) -> Result<u64, ToolError> {
        let session_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cwd = workdir.unwrap_or_else(|| PathBuf::from("."));

        // Prefer sandbox spawn. On spawn failure only, fall back to direct
        // spawn that still uses platform shell + captured stdio so Windows TUI
        // cannot be corrupted. Immediate exit (including signal-killed) is
        // treated as a real session result — do not re-run outside the sandbox.
        let mut child = if let Some(ref sb) = self.sandbox {
            let profile = self
                .sandbox_profile
                .clone()
                .unwrap_or_else(|| self.default_profile(&cwd));

            match sb.spawn(command, &profile) {
                Ok(sandboxed) => sandboxed.child,
                Err(e) => {
                    tracing::warn!(
                        command = %command,
                        backend = %sb.status().backend_name,
                        error = %e,
                        "Sandbox spawn failed; falling back to direct (captured stdio)"
                    );
                    self.spawn_direct(command, &cwd)?
                }
            }
        } else {
            self.spawn_direct(command, &cwd)?
        };

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| ToolError {
            message: "Failed to capture stdout".to_string(),
            code: Some("stdout_unavailable".to_string()),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| ToolError {
            message: "Failed to capture stderr".to_string(),
            code: Some("stderr_unavailable".to_string()),
        })?;

        let state = Arc::new(SessionState {
            stdout: Mutex::new(Vec::new()),
            stderr: Mutex::new(Vec::new()),
            stdout_offset: Mutex::new(0),
            stderr_offset: Mutex::new(0),
            exit_status: RwLock::new(None),
        });

        Self::spawn_reader(stdout, state.clone(), true);
        Self::spawn_reader(stderr, state.clone(), false);

        let handle = Arc::new(CommandSessionHandle {
            cwd,
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            state,
        });

        self.sessions.write().await.insert(session_id, handle);
        Ok(session_id)
    }

    /// Direct spawn without sandbox (fallback / no-sandbox mode).
    ///
    /// Always uses the shared platform shell helper with captured stdio so
    /// Windows console apps cannot write into the parent TUI.
    fn spawn_direct(
        &self,
        command: &str,
        cwd: &PathBuf,
    ) -> Result<tokio::process::Child, ToolError> {
        let mut cmd = shell_command_captured(command);
        cmd.current_dir(cwd);

        cmd.spawn().map_err(|e| ToolError {
            message: format!("Failed to spawn command: {}", e),
            code: Some("spawn_error".to_string()),
        })
    }

    fn spawn_reader<R>(mut reader: R, state: Arc<SessionState>, is_stdout: bool)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buf[..n];
                        if is_stdout {
                            state.stdout.lock().await.extend_from_slice(chunk);
                        } else {
                            state.stderr.lock().await.extend_from_slice(chunk);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    pub async fn write_stdin(
        &self,
        session_id: u64,
        chars: &str,
        yield_time_ms: u64,
        max_output_chars: usize,
    ) -> Result<SessionChunk, ToolError> {
        let handle = self.get_session(session_id).await?;

        {
            let mut stdin = handle.stdin.lock().await;
            let Some(stdin) = stdin.as_mut() else {
                return Err(ToolError {
                    message: format!("Session {} is not writable", session_id),
                    code: Some("stdin_closed".to_string()),
                });
            };

            stdin
                .write_all(chars.as_bytes())
                .await
                .map_err(|e| ToolError {
                    message: format!("Failed to write stdin: {}", e),
                    code: Some("stdin_write_error".to_string()),
                })?;
            stdin.flush().await.map_err(|e| ToolError {
                message: format!("Failed to flush stdin: {}", e),
                code: Some("stdin_flush_error".to_string()),
            })?;
        }

        self.read_incremental(session_id, yield_time_ms, max_output_chars)
            .await
    }

    pub async fn kill_session(&self, session_id: u64) -> Result<ToolOutput, ToolError> {
        let handle = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(&session_id).ok_or_else(|| ToolError {
                message: format!("Session not found: {}", session_id),
                code: Some("session_not_found".to_string()),
            })?
        };

        let mut child = handle.child.lock().await;
        child.kill().await.map_err(|e| ToolError {
            message: format!("Failed to kill session {}: {}", session_id, e),
            code: Some("kill_error".to_string()),
        })?;

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Killed session {}", session_id),
            metadata: std::collections::HashMap::from([
                ("session_id".to_string(), serde_json::json!(session_id)),
                ("cwd".to_string(), serde_json::json!(handle.cwd)),
            ]),
        })
    }

    pub async fn read_incremental(
        &self,
        session_id: u64,
        yield_time_ms: u64,
        max_output_chars: usize,
    ) -> Result<SessionChunk, ToolError> {
        let handle = self.get_session(session_id).await?;

        if yield_time_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(yield_time_ms)).await;
        }

        self.refresh_exit_status(&handle).await;

        let stdout_bytes = handle.state.stdout.lock().await.clone();
        let stderr_bytes = handle.state.stderr.lock().await.clone();

        let stdout =
            Self::slice_incremental(&stdout_bytes, &handle.state.stdout_offset, max_output_chars)
                .await;
        let stderr =
            Self::slice_incremental(&stderr_bytes, &handle.state.stderr_offset, max_output_chars)
                .await;
        let exit_code = *handle.state.exit_status.read().await;
        let finished = exit_code.is_some();

        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }

        Ok(SessionChunk {
            session_id,
            stdout,
            stderr,
            combined,
            exit_code,
            finished,
        })
    }

    async fn slice_incremental(
        bytes: &[u8],
        offset_lock: &Mutex<usize>,
        max_output_chars: usize,
    ) -> String {
        let mut offset = offset_lock.lock().await;
        if *offset >= bytes.len() {
            return String::new();
        }

        let start = *offset;
        let end = bytes.len();
        *offset = end;

        let text = String::from_utf8_lossy(&bytes[start..end]).to_string();
        Self::truncate_chars(text, max_output_chars)
    }

    async fn refresh_exit_status(&self, handle: &Arc<CommandSessionHandle>) {
        let mut child = handle.child.lock().await;
        if handle.state.exit_status.read().await.is_some() {
            return;
        }

        if let Ok(Some(status)) = child.try_wait() {
            let code = status.code().unwrap_or(-1);
            *handle.state.exit_status.write().await = Some(code);
        }
    }

    async fn get_session(&self, session_id: u64) -> Result<Arc<CommandSessionHandle>, ToolError> {
        self.sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| ToolError {
                message: format!("Session not found: {}", session_id),
                code: Some("session_not_found".to_string()),
            })
    }

    fn truncate_chars(input: String, max_output_chars: usize) -> String {
        if max_output_chars == 0 || input.len() <= max_output_chars {
            return input;
        }
        // Find the nearest char boundary at or before max_output_chars
        // to avoid panicking on multi-byte UTF-8 characters.
        let safe_end = input
            .char_indices()
            .take_while(|(i, _)| *i < max_output_chars)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);

        if safe_end >= input.len() {
            return input;
        }

        let truncated = &input[..safe_end];
        let total_chars = input.chars().count();
        let shown_chars = truncated.chars().count();
        format!(
            "{}\n\n... (truncated: {} of {} chars shown, {} chars omitted)",
            truncated,
            shown_chars,
            total_chars,
            total_chars - shown_chars
        )
    }
}

impl Default for CommandSessionManager {
    fn default() -> Self {
        Self::new()
    }
}
