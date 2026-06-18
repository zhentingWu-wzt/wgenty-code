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
                let original = std::fs::read_to_string(path).unwrap_or_default();
                let modified = apply_hunks(&original, hunks, path).unwrap_or_default();
                diffs_json.insert(
                    path.display().to_string(),
                    serde_json::json!({
                        "old_content": original,
                        "new_content": modified,
                    }),
                );
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

        if !content.contains(&old_text) {
            return Err(ToolError {
                message: format!(
                    "Patch context not found in {}: {}",
                    path.display(),
                    old_text
                ),
                code: Some("context_not_found".to_string()),
            });
        }

        content = content.replacen(&old_text, &new_text, 1);
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
