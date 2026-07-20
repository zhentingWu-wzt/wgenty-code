use crate::config::Settings;
use crate::tools::{Tool, ToolError};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    workspace_root: PathBuf,
}

#[derive(Debug)]
pub enum PolicyDecision {
    Allow,
    Ask(PermissionRequest),
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub reason: String,
    pub session_rule: String,
}

impl ToolPermissionPolicy {
    /// Create a new policy rooted at the given workspace directory.
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root: canonical_or_original(&workspace_root),
        }
    }

    pub fn from_settings(settings: &Settings) -> Self {
        let workspace_root = canonical_or_original(&settings.storage.working_dir);
        Self { workspace_root }
    }

    pub fn validate_tool_call(
        &self,
        tool: &dyn Tool,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        if tool.is_read_only() {
            return self.validate_read_paths(tool_name, args, session_rules);
        }

        if tool.requires_confirmation() {
            let session_rule = format!("tool:{tool_name}");
            if session_rules.contains(&session_rule) {
                return Ok(PolicyDecision::Allow);
            }
            return Ok(PolicyDecision::Ask(PermissionRequest {
                tool_name: tool_name.to_string(),
                reason: format!(
                    "external tool `{tool_name}` may modify state; explicit approval is required"
                ),
                session_rule,
            }));
        }

        match tool_name {
            "file_write" | "file_edit" | "apply_patch" => {
                self.validate_write_paths(tool_name, args, session_rules)
            }
            "execute_command" | "exec_command" => {
                self.validate_command(tool_name, args, session_rules)
            }
            _ => Ok(PolicyDecision::Allow),
        }
    }

    /// Validate read-only tools that access filesystem paths.
    /// Read-only tools that reference paths outside the workspace require
    /// approval (Ask), matching the write-path behaviour.
    fn validate_read_paths(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        match tool_name {
            "file_read" | "list_files" | "view" | "grep" | "glob" | "search" | "lsp" => {
                if let Some(path_str) = args["path"].as_str() {
                    return self.check_path_boundary(tool_name, path_str, "read", session_rules);
                }
            }
            _ => {}
        }
        Ok(PolicyDecision::Allow)
    }

    /// Shared helper: check whether `path_str` lies inside the workspace.
    /// Returns `Ask` with a `path:` session-rule when the path is outside.
    fn check_path_boundary(
        &self,
        tool_name: &str,
        path_str: &str,
        operation: &str,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        if let Some(rule_key) = self.path_rule_key(path_str)? {
            if session_rules.contains(&rule_key) {
                return Ok(PolicyDecision::Allow);
            }
            return Ok(PolicyDecision::Ask(PermissionRequest {
                tool_name: tool_name.to_string(),
                reason: format!("{operation} path is outside the workspace: {path_str}"),
                session_rule: rule_key,
            }));
        }
        Ok(PolicyDecision::Allow)
    }

    fn validate_write_paths(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        match tool_name {
            "file_write" | "file_edit" => {
                if let Some(path) = args["path"].as_str() {
                    if let Some(rule_key) = self.path_rule_key(path)? {
                        if session_rules.contains(&rule_key) {
                            return Ok(PolicyDecision::Allow);
                        }
                        return Ok(PolicyDecision::Ask(PermissionRequest {
                            tool_name: tool_name.to_string(),
                            reason: format!("write path is outside the workspace: {}", path),
                            session_rule: rule_key,
                        }));
                    }
                }
            }
            "apply_patch" => {
                let workdir = args["workdir"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.workspace_root.clone());
                let resolved = canonical_or_normalized(&workdir);
                if !resolved.starts_with(&self.workspace_root) {
                    let rule_key = format!("workdir:{}", resolved.display());
                    if session_rules.contains(&rule_key) {
                        return Ok(PolicyDecision::Allow);
                    }
                    return Ok(PolicyDecision::Ask(PermissionRequest {
                        tool_name: tool_name.to_string(),
                        reason: format!(
                            "apply_patch workdir is outside the workspace: {}",
                            resolved.display()
                        ),
                        session_rule: rule_key,
                    }));
                }
            }
            _ => {}
        }
        Ok(PolicyDecision::Allow)
    }

    fn validate_command(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        let command = args["command"].as_str().ok_or_else(|| ToolError {
            message: format!("command is required for {}", tool_name),
            code: Some("missing_parameter".to_string()),
        })?;

        // Fork-bomb detection (no proper command name, must be fragment-matched)
        if command.contains(":(){:|:&};:") {
            let rule_key = "command:forkbomb".to_string();
            if session_rules.contains(&rule_key) {
                return Ok(PolicyDecision::Allow);
            }
            return Ok(PolicyDecision::Ask(PermissionRequest {
                tool_name: tool_name.to_string(),
                reason: format!("fork bomb detected: {}", command),
                session_rule: rule_key,
            }));
        }

        // Classify risk by analysing each sub-command's base name
        if let Some((base_cmd, reason)) = classify_command_risk(command) {
            let rule_key = format!("command:{}", base_cmd);
            if session_rules.contains(&rule_key) {
                return Ok(PolicyDecision::Allow);
            }
            return Ok(PolicyDecision::Ask(PermissionRequest {
                tool_name: tool_name.to_string(),
                reason,
                session_rule: rule_key,
            }));
        }

        // Check workdir bounds
        if let Some(workdir) = args["workdir"].as_str() {
            if let Some(rule_key) = self.path_rule_key(workdir)? {
                if session_rules.contains(&rule_key) {
                    return Ok(PolicyDecision::Allow);
                }
                return Ok(PolicyDecision::Ask(PermissionRequest {
                    tool_name: tool_name.to_string(),
                    reason: format!("workdir is outside the workspace: {}", workdir),
                    session_rule: rule_key,
                }));
            }
        }

        Ok(PolicyDecision::Allow)
    }

    /// Public for shared validation tests (root + subagent paths).
    pub(crate) fn path_rule_key(&self, raw_path: &str) -> Result<Option<String>, ToolError> {
        let resolved = self.resolve_path(raw_path);

        if !resolved.starts_with(&self.workspace_root) {
            return Ok(Some(format!("path:{}", resolved.display())));
        }

        Ok(None)
    }

    fn resolve_path(&self, raw_path: &str) -> PathBuf {
        let path = PathBuf::from(raw_path);
        let absolute = if path.is_absolute() {
            path
        } else {
            self.workspace_root.join(path)
        };
        canonical_or_normalized(&absolute)
    }
}

fn canonical_or_normalized(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }

    let normalized = normalize_path(path);
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();

    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            return normalized;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return normalized;
        };
        ancestor = parent;
    }

    let Ok(mut resolved) = ancestor.canonicalize() else {
        return normalized;
    };
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    resolved
}

fn canonical_or_original(path: &Path) -> PathBuf {
    canonical_or_normalized(path)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Split a shell command by control operators into individual sub-commands.
fn split_shell_commands(command: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0u8;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = command.as_bytes();
    let mut start = 0;

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'$' if !in_single && !in_double && i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                depth += 1;
            }
            b')' if !in_single && !in_double && depth > 0 => {
                depth -= 1;
            }
            // `2>&1` / `>&2` are fd-duplication redirects, not control operators.
            // Do not split on the `&` inside `>&`.
            b'&' if !in_single && !in_double && depth == 0 => {
                if i > 0 && bytes[i - 1] == b'>' {
                    continue;
                }
                // `&>file` is a redirect form, not `cmd & cmd` backgrounding when
                // immediately followed by `>` — leave it inside the sub-command.
                if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                    continue;
                }
                if start < i {
                    let sub = command[start..i].trim();
                    if !sub.is_empty() {
                        result.push(sub);
                    }
                }
                // Skip the second char of &&
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    start = i + 2;
                    continue;
                }
                // background `&`
                start = i + 1;
            }
            b';' | b'|' if !in_single && !in_double && depth == 0 => {
                if start < i {
                    let sub = command[start..i].trim();
                    if !sub.is_empty() {
                        result.push(sub);
                    }
                }
                // Skip the second char of ||
                if b == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    start = i + 2;
                    continue;
                }
                start = i + 1;
            }
            _ => {}
        }
    }

    let tail = command[start..].trim();
    if !tail.is_empty() {
        result.push(tail);
    }

    result
}

/// Shell keywords that are not executable command names.
///
/// When splitting `for x in a; do rm y; done`, the second segment starts with
/// `do` — treat it as a keyword and look at the next token for risk basing.
const SHELL_KEYWORDS: &[&str] = &[
    "do", "done", "then", "else", "elif", "fi", "if", "for", "while", "until", "case", "esac",
    "in", "time", "coproc", "select", "function", "!", "{", "}",
];

/// Return the first non-keyword token of a sub-command (basename-ish).
fn command_base_name(sub: &str) -> Option<&str> {
    for token in sub.split_whitespace() {
        // Strip simple env assignments: `FOO=bar cmd`
        if token.contains('=') && !token.starts_with('-') && !token.starts_with('/') {
            let key = token.split('=').next().unwrap_or("");
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
        }
        if SHELL_KEYWORDS.contains(&token) {
            continue;
        }
        // Basename of path-qualified commands: `/usr/bin/rm` → `rm`
        let base = token.rsplit('/').next().unwrap_or(token);
        return Some(base);
    }
    None
}

/// True when `target` is a null sink (`/dev/null` or Windows `NUL`).
fn is_null_sink(target: &str) -> bool {
    let t = target.trim().trim_matches(|c| c == '\'' || c == '"');
    t.eq_ignore_ascii_case("/dev/null") || t.eq_ignore_ascii_case("nul")
}

/// Detect a *file-writing* redirect outside quotes, ignoring:
/// - quoted `>` (e.g. Python `{m:>6}`, `echo "a > b"`)
/// - fd duplication (`2>&1`, `>&2`)
/// - redirects to `/dev/null` / `NUL` (`2>/dev/null`, `>/dev/null`)
///
/// Still flags real sinks: `> file`, `>> log`, `2> err.log`, `&> out`.
fn has_file_redirect(sub: &str) -> bool {
    let bytes = sub.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' if in_double && i + 1 < bytes.len() => {
                // Skip escaped char inside double quotes.
                i += 2;
                continue;
            }
            b'\'' if !in_double => {
                in_single = !in_single;
                i += 1;
                continue;
            }
            b'"' if !in_single => {
                in_double = !in_double;
                i += 1;
                continue;
            }
            b'>' if !in_single && !in_double => {
                // Optional leading fd digits: `2>`, `10>`
                let mut j = i;
                while j > 0 && bytes[j - 1].is_ascii_digit() {
                    j -= 1;
                }
                // Ensure digits (if any) are a token boundary, not part of a word.
                if j > 0 && !bytes[j - 1].is_ascii_whitespace() && bytes[j - 1] != b'&' {
                    // e.g. `foo2>bar` is unusual; still treat `>` as redirect op
                    // only when the run of digits is at a boundary OR starts the
                    // redirect form after whitespace. Fall through.
                }

                let mut k = i + 1;
                // `>>`
                if k < bytes.len() && bytes[k] == b'>' {
                    k += 1;
                }
                // `>&` fd dup: `2>&1`, `>&2`
                if k < bytes.len() && bytes[k] == b'&' {
                    // fd-to-fd duplication — not a file write
                    i = k + 1;
                    continue;
                }
                // skip spaces before target
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                // extract redirect target token (until whitespace / operator)
                let start = k;
                while k < bytes.len()
                    && !bytes[k].is_ascii_whitespace()
                    && bytes[k] != b'|'
                    && bytes[k] != b';'
                    && bytes[k] != b'&'
                    && bytes[k] != b'<'
                    && bytes[k] != b'>'
                {
                    k += 1;
                }
                let target = if start < k { &sub[start..k] } else { "" };
                if is_null_sink(target) {
                    i = if k > i { k } else { i + 1 };
                    continue;
                }
                // Empty target or real path → treat as file redirect
                return true;
            }
            // `&>file` / `>&file` already handled; bare `&>` without earlier `>`
            b'&' if !in_single && !in_double && i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                let mut k = i + 2;
                if k < bytes.len() && bytes[k] == b'>' {
                    k += 1; // `&>>`
                }
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                let start = k;
                while k < bytes.len()
                    && !bytes[k].is_ascii_whitespace()
                    && bytes[k] != b'|'
                    && bytes[k] != b';'
                {
                    k += 1;
                }
                let target = if start < k { &sub[start..k] } else { "" };
                if is_null_sink(target) {
                    i = if k > i { k } else { i + 1 };
                    continue;
                }
                return true;
            }
            _ => {
                i += 1;
                continue;
            }
        }
    }
    false
}

/// Classify the risk of a shell command by examining each sub-command's base name.
///
/// Returns `Some((base_command_name, reason))` if the command needs approval,
/// or `None` if it is safe to execute.
fn classify_command_risk(command: &str) -> Option<(String, String)> {
    let sub_commands = split_shell_commands(command);

    for sub in &sub_commands {
        let Some(base) = command_base_name(sub) else {
            continue;
        };

        // ── Filesystem-modifying commands ────────────────────────────
        // Checked before redirects so `rm a > b` still keys as `rm`.
        const FS_MODIFIERS: &[&str] = &[
            "mv", "cp", "rm", "dd", "touch", "mkdir", "tee", "install", "ln", "chmod", "chown",
            "truncate", "rmdir", "chattr", "setfacl", "setfattr",
        ];
        if FS_MODIFIERS.contains(&base) {
            return Some((
                base.to_string(),
                format!(
                    "filesystem-modifying command requires approval: {}",
                    command
                ),
            ));
        }

        // ── Script interpreters (arbitrary code execution) ───────────
        // Before redirect scan so `python3 -c '...{m:>6}...'` reports
        // interpreter risk (not a misleading "file redirect").
        const INTERPRETERS: &[&str] = &[
            "python3",
            "python",
            "python2",
            "ruby",
            "perl",
            "node",
            "php",
            "sh",
            "bash",
            "zsh",
            "fish",
            "dash",
            "pwsh",
            "powershell",
            "lua",
            "tclsh",
            "awk",
            "sed",
            "groovy",
        ];
        if INTERPRETERS.contains(&base) {
            return Some((
                base.to_string(),
                format!("interpreter command requires approval: {}", command),
            ));
        }

        // ── System / privilege commands ──────────────────────────────
        const SYSTEM: &[&str] = &[
            "sudo",
            "su",
            "doas",
            "shutdown",
            "reboot",
            "halt",
            "poweroff",
            "mkfs",
            "mount",
            "umount",
            "systemctl",
            "service",
            "kill",
            "pkill",
            "killall",
            "crontab",
            "at",
            "batch",
        ];
        if SYSTEM.contains(&base) {
            return Some((
                base.to_string(),
                format!("system command requires approval: {}", command),
            ));
        }

        // ── Network / remote commands ────────────────────────────────
        const NETWORK: &[&str] = &[
            "curl", "wget", "nc", "ncat", "scp", "rsync", "ftp", "sftp", "ssh", "telnet", "nmap",
            "tcpdump", "tshark", "socat",
        ];
        if NETWORK.contains(&base) {
            return Some((
                base.to_string(),
                format!("network command requires approval: {}", command),
            ));
        }

        // ── File redirect operators (quote-/null-aware) ──────────────
        if has_file_redirect(sub) {
            return Some((
                base.to_string(),
                format!("file redirect requires approval: {}", command),
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::filesystem::file_read::FileReadTool;
    use crate::tools::filesystem::file_write::FileWriteTool;
    use crate::tools::filesystem::list_files::ListFilesTool;
    use crate::tools::search::grep::GrepTool;
    use crate::tools::ToolOutput;
    use std::collections::HashSet;

    // ── validate_read_paths tests ─────────────────────────────────

    fn policy_for_test() -> ToolPermissionPolicy {
        // Use the current working directory as the "workspace"
        ToolPermissionPolicy {
            workspace_root: std::env::current_dir().unwrap().canonicalize().unwrap(),
        }
    }

    fn empty_rules() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn test_read_inside_workspace_allowed() {
        let policy = policy_for_test();
        let tool = FileReadTool::new();
        let args = serde_json::json!({"path": "Cargo.toml"});
        let result = policy.validate_tool_call(&tool, "file_read", &args, &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Allow)));
    }

    #[test]
    fn test_read_outside_workspace_asks() {
        let policy = policy_for_test();
        let tool = FileReadTool::new();
        let args = serde_json::json!({"path": "/etc/passwd"});
        let result = policy.validate_tool_call(&tool, "file_read", &args, &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_read_outside_with_session_rule_allowed() {
        let policy = policy_for_test();
        let tool = FileReadTool::new();
        let path = "/etc/passwd";
        let args = serde_json::json!({"path": path});

        // Compute expected rule key to pre-approve
        let rule_key = policy.path_rule_key(path).unwrap().unwrap();
        let mut rules = HashSet::new();
        rules.insert(rule_key);

        let result = policy.validate_tool_call(&tool, "file_read", &args, &rules);
        assert!(matches!(result, Ok(PolicyDecision::Allow)));
    }

    #[test]
    fn test_read_nonexistent_path_traversal_asks() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace should be created");
        let policy = ToolPermissionPolicy::new(workspace);
        let tool = FileReadTool::new();
        let args = serde_json::json!({"path": "missing/../../outside/secret.txt"});

        let result = policy.validate_tool_call(&tool, "file_read", &args, &empty_rules());

        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_write_nonexistent_path_traversal_asks() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace should be created");
        let policy = ToolPermissionPolicy::new(workspace);
        let tool = FileWriteTool::new();
        let args = serde_json::json!({
            "path": "missing/../../outside/secret.txt",
            "content": "secret"
        });

        let result = policy.validate_tool_call(&tool, "file_write", &args, &empty_rules());

        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[cfg(unix)]
    #[test]
    fn test_write_through_symlink_to_nonexistent_outside_file_asks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temp directory should be created");
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&workspace).expect("workspace should be created");
        std::fs::create_dir(&outside).expect("outside directory should be created");
        symlink(&outside, workspace.join("external"))
            .expect("symlink to outside directory should be created");
        let policy = ToolPermissionPolicy::new(workspace);
        let tool = FileWriteTool::new();
        let args = serde_json::json!({
            "path": "external/new-file.txt",
            "content": "secret"
        });

        let result = policy.validate_tool_call(&tool, "file_write", &args, &empty_rules());

        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_apply_patch_nonexistent_workdir_traversal_asks() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace should be created");
        let policy = ToolPermissionPolicy::new(workspace.clone());
        let tool = crate::tools::filesystem::apply_patch::ApplyPatchTool::new();
        let workdir = workspace.join("missing/../../outside");
        let args = serde_json::json!({
            "workdir": workdir,
            "patch": "*** Begin Patch\n*** End Patch"
        });

        let result = policy.validate_tool_call(&tool, "apply_patch", &args, &empty_rules());

        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_list_files_outside_asks() {
        let policy = policy_for_test();
        let tool = ListFilesTool::new();
        let args = serde_json::json!({"path": "/Users"});
        let result = policy.validate_tool_call(&tool, "list_files", &args, &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_grep_outside_asks() {
        let policy = policy_for_test();
        let tool = GrepTool::new();
        let args = serde_json::json!({"path": "/home", "pattern": ".*"});
        let result = policy.validate_tool_call(&tool, "grep", &args, &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_non_filesystem_read_tool_allowed() {
        let policy = policy_for_test();
        // web_search has no path param — should always be allowed
        struct WebSearchTool;
        #[async_trait::async_trait]
        impl Tool for WebSearchTool {
            fn name(&self) -> &str {
                "web_search"
            }
            fn is_read_only(&self) -> bool {
                true
            }
            fn description(&self) -> &str {
                ""
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: String::new(),
                    metadata: std::collections::HashMap::new(),
                })
            }
        }
        let tool = WebSearchTool;
        let args = serde_json::json!({"query": "anything"});
        let result = policy.validate_tool_call(&tool, "web_search", &args, &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Allow)));
    }

    #[test]
    fn test_external_mutating_tool_requires_confirmation() {
        let policy = policy_for_test();
        struct ExternalMutationTool;
        #[async_trait::async_trait]
        impl Tool for ExternalMutationTool {
            fn name(&self) -> &str {
                "remote_mutation"
            }
            fn description(&self) -> &str {
                ""
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn requires_confirmation(&self) -> bool {
                true
            }
            async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
                unreachable!("permission test does not execute the tool")
            }
        }

        let tool = ExternalMutationTool;
        let result =
            policy.validate_tool_call(&tool, tool.name(), &serde_json::json!({}), &empty_rules());
        assert!(matches!(result, Ok(PolicyDecision::Ask(_))));
    }

    #[test]
    fn test_external_mutating_tool_respects_session_approval() {
        let policy = policy_for_test();
        struct ExternalMutationTool;
        #[async_trait::async_trait]
        impl Tool for ExternalMutationTool {
            fn name(&self) -> &str {
                "remote_mutation"
            }
            fn description(&self) -> &str {
                ""
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn requires_confirmation(&self) -> bool {
                true
            }
            async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
                unreachable!("permission test does not execute the tool")
            }
        }

        let tool = ExternalMutationTool;
        let rules = HashSet::from(["tool:remote_mutation".to_string()]);
        let result = policy.validate_tool_call(&tool, tool.name(), &serde_json::json!({}), &rules);
        assert!(matches!(result, Ok(PolicyDecision::Allow)));
    }

    // ── Existing classify/split tests ─────────────────────────────

    #[test]
    fn test_split_simple() {
        let parts = split_shell_commands("ls -la");
        assert_eq!(parts, vec!["ls -la"]);
    }

    #[test]
    fn test_split_and_and() {
        let parts = split_shell_commands("cp A B && rm A");
        assert_eq!(parts, vec!["cp A B", "rm A"]);
    }

    #[test]
    fn test_split_semicolon() {
        let parts = split_shell_commands("echo hello; mv a b");
        assert_eq!(parts, vec!["echo hello", "mv a b"]);
    }

    #[test]
    fn test_split_respects_quotes() {
        let parts = split_shell_commands(r#"echo "hello && world"; ls"#);
        assert_eq!(parts, vec![r#"echo "hello && world""#, "ls"]);
    }

    #[test]
    fn test_split_keeps_fd_dup_redirect_intact() {
        // Must not treat `&` inside `2>&1` as a control operator.
        let parts = split_shell_commands("cargo test 2>&1 | tail -20");
        assert_eq!(parts, vec!["cargo test 2>&1", "tail -20"]);
        let parts = split_shell_commands("ls >/dev/null 2>&1");
        assert_eq!(parts, vec!["ls >/dev/null 2>&1"]);
    }

    #[test]
    fn test_classify_cp_rm_bypass() {
        let result = classify_command_risk("cp A B && rm A");
        assert!(result.is_some());
        let (base, _) = result.unwrap();
        assert_eq!(base, "cp");
    }

    #[test]
    fn test_classify_mv() {
        assert!(classify_command_risk("mv a b").is_some());
    }

    #[test]
    fn test_classify_safe() {
        assert!(classify_command_risk("ls -la").is_none());
        assert!(classify_command_risk("cargo build").is_none());
        assert!(classify_command_risk("echo hello").is_none());
    }

    #[test]
    fn test_classify_python3() {
        assert!(classify_command_risk("python3 -c 'print(1)'").is_some());
    }

    #[test]
    fn test_classify_redirect() {
        assert!(classify_command_risk("cat a > b").is_some());
        assert!(classify_command_risk("echo hello >> log.txt").is_some());
    }

    #[test]
    fn test_classify_sudo() {
        assert!(classify_command_risk("sudo ls").is_some());
    }

    #[test]
    fn test_classify_curl() {
        assert!(classify_command_risk("curl https://example.com").is_some());
    }

    #[test]
    fn test_classify_mv_chinese_filename() {
        assert!(classify_command_risk(r#"mv "我爱你宝宝哦.txt" "我爱宝宝.txt""#).is_some());
    }

    #[test]
    fn test_classify_rm_chinese_filename() {
        assert!(classify_command_risk(r#"rm "我爱宝宝.txt""#).is_some());
    }

    // ── Redirect false-positive regressions (log-driven) ───────────

    #[test]
    fn test_classify_stderr_to_dev_null_is_safe() {
        assert!(classify_command_risk("ls -la 2>/dev/null").is_none());
        assert!(classify_command_risk("du -sh target 2>/dev/null").is_none());
        assert!(classify_command_risk("git status 2>/dev/null").is_none());
    }

    #[test]
    fn test_classify_stdout_to_dev_null_is_safe() {
        assert!(classify_command_risk("ls >/dev/null").is_none());
        assert!(classify_command_risk("cargo fmt --check >/dev/null 2>&1").is_none());
    }

    #[test]
    fn test_classify_fd_dup_redirect_is_safe() {
        // Common pipeline pattern from agent tooling; not a file write.
        assert!(classify_command_risk("cargo test 2>&1 | tail -20").is_none());
    }

    #[test]
    fn test_classify_gt_inside_quotes_is_not_redirect() {
        assert!(classify_command_risk(r#"echo "a > b""#).is_none());
        assert!(classify_command_risk(r#"echo 'n:>6'"#).is_none());
    }

    #[test]
    fn test_classify_python_with_format_align_reports_interpreter() {
        // Regression: `{m:>6}` used to trip the naive `contains('>')` redirect
        // check *before* the interpreter rule, producing a misleading reason.
        let (base, reason) =
            classify_command_risk(r#"python3 -c 'print(f"{m:>6}")'"#).expect("should ask");
        assert_eq!(base, "python3");
        assert!(
            reason.starts_with("interpreter command requires approval:"),
            "reason should be interpreter, got: {reason}"
        );
    }

    #[test]
    fn test_classify_python_heredoc_with_format_align_reports_interpreter() {
        let cmd = "python3 - <<'PY'\nprint(f\"{m:>6}\")\nPY";
        let (base, reason) = classify_command_risk(cmd).expect("should ask");
        assert_eq!(base, "python3");
        assert!(
            reason.starts_with("interpreter command requires approval:"),
            "reason should be interpreter, got: {reason}"
        );
    }

    #[test]
    fn test_classify_real_redirect_still_asks() {
        let (base, reason) = classify_command_risk("echo hello > /tmp/out.txt").expect("ask");
        assert_eq!(base, "echo");
        assert!(
            reason.starts_with("file redirect requires approval:"),
            "got: {reason}"
        );
        assert!(classify_command_risk("echo hello >> log.txt").is_some());
        assert!(classify_command_risk("cmd 2> err.log").is_some());
    }

    #[test]
    fn test_classify_for_loop_body_not_keyed_as_do() {
        // `for x in a; do rg y; done` must not surface session_rule command:do.
        assert!(classify_command_risk("for m in agent api; do rg foo; done").is_none());
        let (base, _) =
            classify_command_risk("for m in agent; do rm -rf /tmp/x; done").expect("rm asks");
        assert_eq!(base, "rm");
    }
}
