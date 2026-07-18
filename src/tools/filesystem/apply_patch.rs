use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ApplyPatchTool;

impl ApplyPatchTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
enum PatchOperation {
    Add {
        path: PathBuf,
        content: String,
    },
    Update {
        path: PathBuf,
        hunks: Vec<UpdateHunk>,
    },
    Delete {
        path: PathBuf,
    },
}

#[derive(Debug)]
struct UpdateHunk {
    lines: Vec<HunkLine>,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply structured file patches with add, update, and delete operations"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch content in the Begin/End Patch format"
                },
                "workdir": {
                    "type": "string",
                    "description": "Base directory used to resolve relative paths in the patch"
                }
            },
            "required": ["patch"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let patch = input["patch"].as_str().ok_or_else(|| ToolError {
            message: "patch is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let workdir = input["workdir"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let operations = parse_patch(patch, &workdir)?;
        // Collect diff data before applying (read originals while they still exist)
        let mut diffs_json = serde_json::Map::new();
        for op in &operations {
            if let PatchOperation::Update { path, hunks } = op {
                // H5: 不用 unwrap_or_default 吞错误；读取/匹配失败时跳过 diff 收集，
                // apply_operations 会再次执行并返回真实错误。
                if let Ok(original) = std::fs::read_to_string(path) {
                    if let Ok(modified) = apply_hunks(&original, hunks, path) {
                        diffs_json.insert(
                            path.display().to_string(),
                            serde_json::json!({
                                "old_content": original,
                                "new_content": modified,
                            }),
                        );
                    }
                }
            }
        }

        apply_operations(&operations)?;

        let mut metadata = HashMap::new();
        metadata.insert(
            "files_changed".to_string(),
            serde_json::json!(operations
                .iter()
                .map(|op| match op {
                    PatchOperation::Add { path, .. } => path.display().to_string(),
                    PatchOperation::Update { path, .. } => path.display().to_string(),
                    PatchOperation::Delete { path } => path.display().to_string(),
                })
                .collect::<Vec<_>>()),
        );
        if !diffs_json.is_empty() {
            metadata.insert("diffs".to_string(), serde_json::Value::Object(diffs_json));
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Applied patch with {} operation(s)", operations.len()),
            metadata,
        })
    }
}

/// Extract affected file paths from an apply_patch document without fully
/// validating hunks. Best-effort: returns whatever file headers can be parsed.
/// Paths are returned as they appear in the patch (relative or absolute).
pub fn extract_patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            paths.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            paths.push(path.trim().to_string());
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            paths.push(path.trim().to_string());
        }
    }
    paths
}

fn parse_patch(patch: &str, workdir: &Path) -> Result<Vec<PatchOperation>, ToolError> {
    let lines: Vec<&str> = patch.lines().collect();
    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(ToolError {
            message: "Patch must start with *** Begin Patch".to_string(),
            code: Some("invalid_patch".to_string()),
        });
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(ToolError {
            message: "Patch must end with *** End Patch".to_string(),
            code: Some("invalid_patch".to_string()),
        });
    }

    let mut i = 1usize;
    let mut operations = Vec::new();

    while i < lines.len() - 1 {
        let line = lines[i];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = Vec::new();
            while i < lines.len() - 1 && !lines[i].starts_with("*** ") {
                if let Some(added) = lines[i].strip_prefix('+') {
                    content.push(added.to_string());
                } else {
                    return Err(ToolError {
                        message: format!("Add file line must start with '+': {}", lines[i]),
                        code: Some("invalid_patch".to_string()),
                    });
                }
                i += 1;
            }
            operations.push(PatchOperation::Add {
                path: resolve_patch_path(workdir, path),
                content: join_lines(content),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(PatchOperation::Delete {
                path: resolve_patch_path(workdir, path),
            });
            i += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut hunks = Vec::new();
            while i < lines.len() - 1 && !lines[i].starts_with("*** ") {
                if lines[i].starts_with("@@") {
                    i += 1;
                    let mut hunk_lines = Vec::new();
                    while i < lines.len() - 1
                        && !lines[i].starts_with("@@")
                        && !lines[i].starts_with("*** ")
                    {
                        let current = lines[i];
                        if let Some(rest) = current.strip_prefix(' ') {
                            hunk_lines.push(HunkLine::Context(rest.to_string()));
                        } else if let Some(rest) = current.strip_prefix('-') {
                            hunk_lines.push(HunkLine::Remove(rest.to_string()));
                        } else if let Some(rest) = current.strip_prefix('+') {
                            hunk_lines.push(HunkLine::Add(rest.to_string()));
                        } else if current.is_empty() {
                            // H4: 空行视作空 context 行（模型常把空行 context
                            // 写成零字符，否则会被当作非法行报错）。
                            hunk_lines.push(HunkLine::Context(String::new()));
                        } else {
                            return Err(ToolError {
                                message: format!("Invalid hunk line: {}", current),
                                code: Some("invalid_patch".to_string()),
                            });
                        }
                        i += 1;
                    }
                    hunks.push(UpdateHunk { lines: hunk_lines });
                } else {
                    return Err(ToolError {
                        message: format!("Expected hunk header, found: {}", lines[i]),
                        code: Some("invalid_patch".to_string()),
                    });
                }
            }

            operations.push(PatchOperation::Update {
                path: resolve_patch_path(workdir, path),
                hunks,
            });
            continue;
        }

        return Err(ToolError {
            message: format!("Unknown patch directive: {}", line),
            code: Some("invalid_patch".to_string()),
        });
    }

    Ok(operations)
}

fn apply_operations(operations: &[PatchOperation]) -> Result<(), ToolError> {
    let mut writes: Vec<(PathBuf, Option<String>)> = Vec::new();

    for operation in operations {
        match operation {
            PatchOperation::Add { path, content } => {
                if path.exists() {
                    return Err(ToolError {
                        message: format!("File already exists: {}", path.display()),
                        code: Some("file_exists".to_string()),
                    });
                }
                writes.push((path.clone(), Some(content.clone())));
            }
            PatchOperation::Delete { path } => {
                if !path.exists() {
                    return Err(ToolError {
                        message: format!("File does not exist: {}", path.display()),
                        code: Some("file_not_found".to_string()),
                    });
                }
                writes.push((path.clone(), None));
            }
            PatchOperation::Update { path, hunks } => {
                let content = std::fs::read_to_string(path).map_err(|e| ToolError {
                    message: format!("Failed to read file {}: {}", path.display(), e),
                    code: Some("read_error".to_string()),
                })?;
                let updated = apply_hunks(&content, hunks, path)?;
                writes.push((path.clone(), Some(updated)));
            }
        }
    }

    for (path, content) in writes {
        match content {
            Some(content) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| ToolError {
                        message: format!(
                            "Failed to create parent directory for {}: {}",
                            path.display(),
                            e
                        ),
                        code: Some("write_error".to_string()),
                    })?;
                }
                std::fs::write(&path, content).map_err(|e| ToolError {
                    message: format!("Failed to write file {}: {}", path.display(), e),
                    code: Some("write_error".to_string()),
                })?;
            }
            None => {
                std::fs::remove_file(&path).map_err(|e| ToolError {
                    message: format!("Failed to delete file {}: {}", path.display(), e),
                    code: Some("write_error".to_string()),
                })?;
            }
        }
    }

    Ok(())
}

fn apply_hunks(original: &str, hunks: &[UpdateHunk], path: &Path) -> Result<String, ToolError> {
    let mut content = original.to_string();

    for hunk in hunks {
        let old_block = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                HunkLine::Context(text) | HunkLine::Remove(text) => Some(text.clone()),
                HunkLine::Add(_) => None,
            })
            .collect::<Vec<_>>();
        let new_block = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                HunkLine::Context(text) | HunkLine::Add(text) => Some(text.clone()),
                HunkLine::Remove(_) => None,
            })
            .collect::<Vec<_>>();

        let old_text = join_lines(old_block);
        let new_text = join_lines(new_block);

        // H3: 优先精确匹配；失败时若文件为 CRLF，将 hunk 行尾转为 CRLF 重试，
        // 写入保留原文件行尾风格（join_lines 用 "\n" 拼接，CRLF 文件需适配）。
        let (match_old, match_new) = match resolve_match(&content, &old_text, &new_text) {
            Some(pair) => pair,
            None => {
                // 诊断日志: 记录 hunk 匹配失败的关键上下文，便于定位 CRLF/行尾/缩进差异。
                // 运行时通过 RUST_LOG=wgenty_code::tools::filesystem::apply_patch=debug 可见。
                tracing::debug!(
                    target: "apply_patch",
                    path = %path.display(),
                    file_lines = content.lines().count(),
                    file_has_crlf = content.contains('\r'),
                    old_text_len = old_text.len(),
                    old_text_has_crlf = old_text.contains('\r'),
                    "patch context not found (exact substring match failed)"
                );
                return Err(ToolError {
                    message: format!(
                        "Patch context not found in {}: {}",
                        path.display(),
                        old_text
                    ),
                    code: Some("context_not_found".to_string()),
                });
            }
        };

        content = content.replacen(&match_old, &match_new, 1);
    }

    Ok(content)
}

fn join_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n")
    }
}

/// 尝试在 content 中匹配 old_text。精确匹配失败时，若文件含 CRLF，
/// 则将 old_text/new_text 的行尾转为 CRLF 重试，保留原文件行尾风格。
/// old_text 由 join_lines 用 "\n" 拼接，对 CRLF 文件需做行尾适配。
fn resolve_match(content: &str, old_text: &str, new_text: &str) -> Option<(String, String)> {
    if content.contains(old_text) {
        return Some((old_text.to_string(), new_text.to_string()));
    }
    if content.contains("\r\n") {
        let crlf_old = old_text.replace('\n', "\r\n");
        if content.contains(&crlf_old) {
            return Some((crlf_old, new_text.replace('\n', "\r\n")));
        }
    }
    None
}

/// Resolve a patch file path relative to the workspace root.
///
/// **Security:** Rejects absolute paths and validates that the resolved
/// path stays within the workspace to prevent path traversal attacks
/// (e.g. `../../etc/passwd`).
fn resolve_patch_path(workdir: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);

    // Reject absolute paths — patches must be relative to workdir
    let relative = if path.is_absolute() {
        // Best-effort: strip common prefixes, but if it's truly absolute
        // and outside workdir, fall back to filename only
        path.file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("unknown"))
    } else {
        path
    };

    let resolved = workdir.join(&relative);

    // Canonicalize both paths and verify containment
    if let (Ok(resolved_canon), Ok(workdir_canon)) = (
        std::fs::canonicalize(&resolved),
        std::fs::canonicalize(workdir),
    ) {
        if resolved_canon.starts_with(&workdir_canon) {
            return resolved_canon;
        }
        // Path escapes workspace — fall back to a safe name under workdir
        tracing::warn!(
            path = %raw_path,
            "patch path escaped workspace root, using filename only"
        );
    }

    // Fallback: use just the filename under workdir
    let safe_name = relative
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("patched_file"));
    workdir.join(safe_name)
}

impl Default for ApplyPatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 基线: 正常 LF 文件 update 应成功（确认 patch 格式与路径解析正确）。
    #[tokio::test]
    async fn apply_patch_lf_baseline() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("base.txt"), "alpha\nbeta\n").unwrap();
        let tool = ApplyPatchTool::new();
        let patch =
            "*** Begin Patch\n*** Update File: base.txt\n@@\n alpha\n-beta\n+beta2\n*** End Patch";
        tool.execute(serde_json::json!({
            "patch": patch,
            "workdir": tmp.path().to_string_lossy(),
        }))
        .await
        .expect("LF 基线应成功");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("base.txt")).unwrap(),
            "alpha\nbeta2\n"
        );
    }

    /// H3 修复: CRLF 行尾文件用 LF hunk，归一化重试后成功，保留 CRLF 行尾。
    #[tokio::test]
    async fn h3_apply_patch_handles_crlf_line_endings() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("h3.txt"), "alpha\r\nbeta\r\n").unwrap();
        let tool = ApplyPatchTool::new();
        let patch =
            "*** Begin Patch\n*** Update File: h3.txt\n@@\n alpha\n-beta\n+beta2\n*** End Patch";
        tool.execute(serde_json::json!({
            "patch": patch,
            "workdir": tmp.path().to_string_lossy(),
        }))
        .await
        .expect("CRLF 文件应容错成功");
        // 保留原 CRLF 行尾。
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("h3.txt")).unwrap(),
            "alpha\r\nbeta2\r\n"
        );
    }

    /// H4 修复: 空行 context 写成零字符时，视作空 context 行，patch 成功。
    #[tokio::test]
    async fn h4_apply_patch_accepts_blank_context_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("h4.txt"), "a\n\nb\n").unwrap();
        let tool = ApplyPatchTool::new();
        // hunk 中第二行为零字符空行，模拟模型常见写法。
        let patch = "*** Begin Patch\n*** Update File: h4.txt\n@@\n a\n\n-b\n+c\n*** End Patch";
        tool.execute(serde_json::json!({
            "patch": patch,
            "workdir": tmp.path().to_string_lossy(),
        }))
        .await
        .expect("空行 context 应容错成功");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("h4.txt")).unwrap(),
            "a\n\nc\n"
        );
    }

    /// H5: 验证 `execute` 内 `unwrap_or_default()` 不会导致匹配失败时静默成功。
    /// 预期: 整体返回 Err（apply_operations 会再次调用 apply_hunks 并报错）。
    #[tokio::test]
    async fn h5_apply_patch_mismatch_returns_error_not_silent() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("h5.txt"), "only line\n").unwrap();
        let tool = ApplyPatchTool::new();
        let patch = "*** Begin Patch\n*** Update File: h5.txt\n@@\n context-not-in-file\n+new\n*** End Patch";
        let result = tool
            .execute(serde_json::json!({
                "patch": patch,
                "workdir": tmp.path().to_string_lossy(),
            }))
            .await;
        assert!(result.is_err(), "应返回错误而非静默成功");
    }

    #[test]
    fn extract_patch_paths_finds_add_update_delete() {
        let patch = "*** Begin Patch\n*** Add File: a.rs\n+fn a() {}\n*** Update File: b.rs\n@@\n-old\n+new\n*** Delete File: c.rs\n*** End Patch";
        let paths = extract_patch_paths(patch);
        assert_eq!(paths, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn extract_patch_paths_empty_on_garbage() {
        assert!(extract_patch_paths("not a patch").is_empty());
    }
}
