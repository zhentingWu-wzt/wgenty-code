//! Workspace-scoped file browsing + preview for the web file tree.
//!
//! Unlike `fs.rs` (which lists directories anywhere on disk for the directory
//! picker), these endpoints are **scoped to registered workspaces**: the main
//! project root, every registered project root, and each project's linked git
//! worktrees. Anything that canonicalizes outside those roots is refused with
//! a uniform 403 so the daemon never becomes a general-purpose file server.
//!
//! - `GET /api/v1/fs/entries?path=<dir>` — one-level directory listing
//!   (dirs first, case-insensitive, 2000-entry cap with `truncated` flag).
//! - `GET /api/v1/fs/file?path=<file>` — preview a single file: raw bytes for
//!   whitelisted image/pdf extensions, JSON `{lines, version}` for UTF-8 text,
//!   or a JSON `{is_binary: true, version}` variant when decoding fails.
//!
//! Size ceilings are checked against metadata *before* the body is read, so
//! oversized files never enter daemon memory (413 with `{size, limit}`).
//!
//! Both endpoints live behind the daemon bearer token (see `routes.rs`), same
//! as every other protected route.

use crate::daemon::state::DaemonState;
use crate::daemon::worktrees;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Cap on entries collected per `/fs/entries` request. Reaching the cap stops
/// collection and sets `truncated: true` so the frontend can badge the list.
const MAX_ENTRIES: usize = 2000;

/// Preview ceiling for text files (UTF-8 JSON response), checked before read.
const TEXT_MAX_BYTES: u64 = 1_572_864; // 1.5 MiB

/// Preview ceiling for whitelisted binary previews (raw byte stream).
const BIN_MAX_BYTES: u64 = 5_242_880; // 5 MiB

/// Bytes sampled from the head of a file for NUL-byte binary detection.
const PROBE_BYTES: usize = 8192;

/// Query params shared by both endpoints (`path` is the target to resolve).
#[derive(Debug, Deserialize)]
pub struct FsPathQuery {
    /// Absolute path of the directory (`/fs/entries`) or file (`/fs/file`).
    pub path: Option<String>,
}

/// One child entry in a directory listing. `size` falls back to 0 when the
/// child's metadata cannot be read (raced deletion, permissions, …).
#[derive(Debug, Clone, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Response DTO for `GET /api/v1/fs/entries`.
#[derive(Debug, Serialize)]
pub struct FsEntries {
    /// Canonicalized absolute path of the listed directory.
    pub current: String,
    /// Directories first, then files; each group case-insensitive by name.
    /// Hidden (`.`-prefixed) entries and non-file/non-dir filesystem objects
    /// (symlinks, sockets, fifos) are skipped.
    pub entries: Vec<FsEntry>,
    /// True when `MAX_ENTRIES` stopped collection early.
    pub truncated: bool,
}

// ── GET /api/v1/fs/git-status ───────────────────────────────────────────────

/// Simplified change kind for one file, derived from porcelain XY codes:
/// `A`/`?` → added, `D` (either side) → deleted, everything else that shows
/// up in status (`M`, `R`, `C`, `T`, …) → modified. Keeps the wire format
/// tiny; the tree only colors, it does not stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitChangeKind {
    Added,
    Modified,
    Deleted,
}

/// One changed file under a workspace root. `path` is relative to that root
/// (forward slashes, as `git status --porcelain` prints when run there).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GitFileStatus {
    pub path: String,
    pub status: GitChangeKind,
}

/// Content version for a previewed file — the frontend uses it to detect
/// stale previews (file changed on disk since the last fetch).
#[derive(Debug, Clone, Serialize)]
pub struct FileVersion {
    /// `metadata.modified()` in milliseconds since the Unix epoch (0 if the
    /// platform reports no mtime).
    pub mtime_ms: u64,
    pub size: u64,
}

/// Response DTO for `GET /api/v1/fs/file` (text branch).
///
/// `lines` is absent on the binary variant (`is_binary: true`) so the
/// frontend can branch on a single field without sentinel values.
#[derive(Debug, Serialize)]
pub struct FileContent {
    /// One string per `\n`-split line, trailing `\r` stripped. Present only
    /// when the file decoded as UTF-8 text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// True when the file was detected as (or degraded to) binary.
    pub is_binary: bool,
    pub version: FileVersion,
}

/// JSON error body shared by both endpoints. `size`/`limit` are only present
/// on 413 responses so the UI can show the real numbers.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Errors surfaced by the workspace fs endpoints, mapped to
/// `(StatusCode, Json<ErrorBody>)` by the `From` impl below.
#[derive(Debug)]
pub enum WorkspaceFsError {
    /// Path does not exist or cannot be canonicalized.
    NotFound,
    /// Path resolves outside every registered workspace root. The message is
    /// uniform and does not leak whether the target exists.
    OutsideWorkspaces,
    /// A directory was required but the path is a file.
    NotADirectory,
    /// A file was required but the path is a directory.
    NotAFile,
    /// The directory itself could not be listed (permissions, …).
    ReadDenied,
    /// An already-validated file could not be opened/read.
    ReadFailed,
    /// Size exceeds the preview ceiling; carries the real numbers for the UI.
    TooLarge { size: u64, limit: u64 },
}

impl From<WorkspaceFsError> for (StatusCode, Json<ErrorBody>) {
    fn from(err: WorkspaceFsError) -> Self {
        let (status, message, extra) = match err {
            WorkspaceFsError::NotFound => (StatusCode::NOT_FOUND, "not found", (None, None)),
            WorkspaceFsError::OutsideWorkspaces => (
                StatusCode::FORBIDDEN,
                "outside registered workspaces",
                (None, None),
            ),
            WorkspaceFsError::NotADirectory => (
                StatusCode::BAD_REQUEST,
                "path is not a directory",
                (None, None),
            ),
            WorkspaceFsError::NotAFile => {
                (StatusCode::BAD_REQUEST, "path is not a file", (None, None))
            }
            WorkspaceFsError::ReadDenied => (
                StatusCode::FORBIDDEN,
                "directory not readable",
                (None, None),
            ),
            WorkspaceFsError::ReadFailed => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read file",
                (None, None),
            ),
            WorkspaceFsError::TooLarge { size, limit } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "file too large to preview",
                (Some(size), Some(limit)),
            ),
        };
        (
            status,
            Json(ErrorBody {
                error: message.to_string(),
                size: extra.0,
                limit: extra.1,
            }),
        )
    }
}

// ── Workspace root resolution ───────────────────────────────────────────────

/// Every filesystem root the endpoints may serve: the main project root, all
/// registered project roots, and each project's linked git worktrees. Roots
/// are canonicalized (unresolvable ones are silently skipped — a registered
/// path may have been deleted since) and deduplicated, since a worktree path
/// may alias a registered root.
pub(crate) async fn resolve_workspace_roots(state: &DaemonState) -> Vec<PathBuf> {
    let repos = {
        let mut repos = vec![state.projects.main_root()];
        repos.extend(state.projects.registered_roots());
        repos
    };

    let mut roots = repos.clone();
    for repo in &repos {
        // Best-effort: a non-git root or a failed `git` spawn simply
        // contributes no extra worktrees rather than failing the request.
        if let Ok(out) = worktrees::git(&["worktree", "list", "--porcelain"], repo).await {
            roots.extend(
                worktrees::parse_worktree_list(&out)
                    .into_iter()
                    .map(|w| PathBuf::from(w.path)),
            );
        }
    }

    let mut seen: Vec<PathBuf> = Vec::new();
    for root in roots {
        if let Ok(canon) = root.canonicalize() {
            if !seen.contains(&canon) {
                seen.push(canon);
            }
        }
    }
    seen
}

/// Canonicalize `raw` and require it to sit inside one of `roots`.
///
/// A canonicalization failure is a 404 (the path does not exist — that is
/// not secret). A path that resolves outside every root is a 403 with a
/// uniform message so callers cannot probe for the existence of files
/// elsewhere on disk.
fn resolve_within_roots(raw: &str, roots: &[PathBuf]) -> Result<PathBuf, WorkspaceFsError> {
    let canon = Path::new(raw)
        .canonicalize()
        .map_err(|_| WorkspaceFsError::NotFound)?;
    if roots.iter().any(|root| canon.starts_with(root)) {
        Ok(canon)
    } else {
        Err(WorkspaceFsError::OutsideWorkspaces)
    }
}

/// `resolve_within_roots` against the daemon's live workspace roots.
pub(crate) async fn ensure_within_roots(
    raw: &str,
    state: &DaemonState,
) -> Result<PathBuf, WorkspaceFsError> {
    let roots = resolve_workspace_roots(state).await;
    resolve_within_roots(raw, &roots)
}

// ── GET /api/v1/fs/entries ──────────────────────────────────────────────────

/// Single-pass collection for a directory listing. `dir` must already be a
/// canonicalized directory path (see `ensure_within_roots`).
fn collect_entries(dir: &Path) -> Result<FsEntries, WorkspaceFsError> {
    let rd = std::fs::read_dir(dir).map_err(|_| WorkspaceFsError::ReadDenied)?;

    let mut entries: Vec<FsEntry> = Vec::new();
    let mut truncated = false;
    for entry in rd {
        // Stop as soon as the cap is reached; anything left sets the flag so
        // the UI knows the list is partial.
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        // `file_type()` comes from the directory entry itself (no symlink
        // follow), so symlinks — even to allowed targets — are skipped along
        // with sockets, fifos and other special files.
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() && !ft.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        entries.push(FsEntry {
            name,
            is_dir: ft.is_dir(),
            size,
        });
    }

    // Directories first; each group case-insensitive by name.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(FsEntries {
        current: dir.to_string_lossy().to_string(),
        entries,
        truncated,
    })
}

/// `GET /api/v1/fs/entries?path=<dir>` — list one level of a workspace
/// directory. 404 when the path does not resolve, 403 when it sits outside
/// every registered workspace, 400 when it is a file rather than a directory.
pub async fn list_entries(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<FsPathQuery>,
) -> Result<Json<FsEntries>, (StatusCode, Json<ErrorBody>)> {
    let dir = ensure_within_roots(q.path.as_deref().unwrap_or(""), &state).await?;
    if !dir.is_dir() {
        return Err(WorkspaceFsError::NotADirectory.into());
    }
    collect_entries(&dir).map(Json).map_err(Into::into)
}

// ── GET /api/v1/fs/file ─────────────────────────────────────────────────────

/// Content-Type for whitelisted binary previews; `None` means the file goes
/// down the text branch. Whitelisted extensions skip NUL/UTF-8 probing —
/// the extension is the contract (a `.png` is served as bytes even if a
/// NUL byte never appears in the first probe window).
fn binary_mime(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

/// Check-before-read: the applicable ceiling depends on the branch (binary
/// whitelist gets 5 MiB, everything else 1.5 MiB). Oversized files are
/// refused without reading a byte of the body.
fn check_preview_limit(len: u64, mime: Option<&str>) -> Result<u64, WorkspaceFsError> {
    let limit = if mime.is_some() {
        BIN_MAX_BYTES
    } else {
        TEXT_MAX_BYTES
    };
    if len > limit {
        Err(WorkspaceFsError::TooLarge { size: len, limit })
    } else {
        Ok(limit)
    }
}

/// `metadata` → preview version stamp.
fn file_version(md: &Metadata) -> FileVersion {
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    FileVersion {
        mtime_ms,
        size: md.len(),
    }
}

/// Split decoded text into lines: split on `\n`, strip a trailing `\r` per
/// line (CRLF files). An empty file yields no lines.
fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

/// Read up to `buf.len()` bytes from `file`, looping over short reads.
/// Returns the number of bytes read (0 at EOF).
fn read_up_to(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Build the JSON preview for a non-whitelisted file: probe the first
/// `PROBE_BYTES` for a NUL byte; otherwise pull the (already size-capped)
/// remainder and attempt strict UTF-8. Decode failures degrade to the
/// `{is_binary: true, version}` variant instead of erroring.
fn read_text_or_binary(path: &Path, md: &Metadata) -> Result<FileContent, WorkspaceFsError> {
    let mut file = std::fs::File::open(path).map_err(|_| WorkspaceFsError::ReadFailed)?;

    // Cheap binary sniff before reading the body: a NUL byte in the head of
    // the file means it is not human-readable text with near-certainty.
    let mut probe = vec![0u8; PROBE_BYTES];
    let probed = read_up_to(&mut file, &mut probe).map_err(|_| WorkspaceFsError::ReadFailed)?;
    if probe[..probed].contains(&0) {
        return Ok(FileContent {
            lines: None,
            is_binary: true,
            version: file_version(md),
        });
    }

    // Text candidate — read the remainder (size already capped by the
    // caller) and decode. The probe bytes are part of the content, so they
    // are prepended back before decoding.
    let mut rest = Vec::new();
    file.read_to_end(&mut rest)
        .map_err(|_| WorkspaceFsError::ReadFailed)?;
    let mut bytes = probe[..probed].to_vec();
    bytes.append(&mut rest);

    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileContent {
            lines: Some(split_lines(&text)),
            is_binary: false,
            version: file_version(md),
        }),
        // No NUL in the head but still not valid UTF-8 (multi-byte sequence
        // cut or mangled deeper in the file) — degrade to the binary variant.
        Err(_) => Ok(FileContent {
            lines: None,
            is_binary: true,
            version: file_version(md),
        }),
    }
}

/// `GET /api/v1/fs/file?path=<file>` — preview a single workspace file.
///
/// Whitelisted extensions (png/jpg/jpeg/gif/webp/svg/pdf) stream raw bytes
/// with the mapped `Content-Type`; everything else returns the JSON text
/// preview or its binary variant. 400 for directories, 413 (with
/// `{size, limit}`) when the applicable ceiling is exceeded.
pub async fn get_file(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<FsPathQuery>,
) -> Result<Response, (StatusCode, Json<ErrorBody>)> {
    let path = ensure_within_roots(q.path.as_deref().unwrap_or(""), &state).await?;
    let md = std::fs::metadata(&path).map_err(|_| WorkspaceFsError::NotFound)?;
    if md.is_dir() {
        return Err(WorkspaceFsError::NotAFile.into());
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = binary_mime(&ext);
    check_preview_limit(md.len(), mime)?;

    if let Some(mime) = mime {
        let bytes = std::fs::read(&path).map_err(|_| WorkspaceFsError::ReadFailed)?;
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .body(Body::from(bytes))
            .map_err(|_| WorkspaceFsError::ReadFailed)?;
        return Ok(response);
    }

    let content = read_text_or_binary(&path, &md)?;
    Ok(Json(content).into_response())
}

// ── GET /api/v1/fs/git-status ───────────────────────────────────────────────

/// Map one porcelain XY pair to the simplified kind. Order matters: a
/// deletion anywhere wins, then adds, then everything else that appears in
/// status (`M`, `R`, `C`, `T`, …) counts as modified; renames are reported
/// at their new path.
fn classify_xy(xy: &str) -> GitChangeKind {
    let split = xy.len().min(2);
    let (x, y) = xy.split_at(split);
    if x.contains('D') || y.contains('D') {
        GitChangeKind::Deleted
    } else if x.contains('A') || y.contains('A') || x.contains('?') || y.contains('?') {
        GitChangeKind::Added
    } else {
        GitChangeKind::Modified
    }
}

/// Parse `git status --porcelain=v1 -z --untracked-files=all` output into
/// simplified per-file statuses.
///
/// `-z` separates records with NUL and never quotes paths. Rename/copy
/// records carry the old path as an extra NUL-separated field after the
/// record (`R  new\0old\0`) — the follower is consumed so it is not mistaken
/// for a record of its own.
pub(crate) fn parse_git_status_z(input: &str) -> Vec<GitFileStatus> {
    let mut out = Vec::new();
    let mut records = input.split('\0');
    while let Some(record) = records.next() {
        if record.len() < 3 || !record.is_char_boundary(2) {
            continue; // defensive: not a porcelain record
        }
        let (xy, path) = record.split_at(2);
        // XY columns are one of the porcelain codes or a space (" M", "D ", …).
        if !xy.bytes().all(|b| {
            matches!(
                b,
                b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'T' | b'?' | b'!'
            )
        }) {
            continue;
        }
        let path = &path[1..]; // exactly one space after XY
        if path.is_empty() {
            continue;
        }
        let kind = classify_xy(xy);
        if xy.starts_with('R') || xy.starts_with('C') || xy.ends_with('R') || xy.ends_with('C') {
            records.next(); // old path of the rename/copy pair
        }
        out.push(GitFileStatus {
            path: path.to_string(),
            status: kind,
        });
    }
    out
}

/// `GET /api/v1/fs/git-status?path=<root>` — changed files under a registered
/// workspace root (tracked changes plus untracked files). Non-git roots
/// degrade to an empty list: the file tree simply shows no colors.
pub async fn git_status(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<FsPathQuery>,
) -> Result<Json<Vec<GitFileStatus>>, (StatusCode, Json<ErrorBody>)> {
    let root = ensure_within_roots(q.path.as_deref().unwrap_or(""), &state).await?;
    let stdout = match worktrees::git(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        &root,
    )
    .await
    {
        Ok(stdout) => stdout,
        // Non-git project (the registry allows them) — no colors, not an error.
        Err((_, stderr)) if stderr.contains("not a git repository") => return Ok(Json(Vec::new())),
        Err((code, msg)) => {
            return Err((
                code,
                Json(ErrorBody {
                    error: msg,
                    size: None,
                    limit: None,
                }),
            ))
        }
    };
    Ok(Json(parse_git_status_z(&stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_within_roots ────────────────────────────────────────────────

    #[test]
    fn resolve_allows_paths_inside_roots() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let roots = vec![tmp.path().canonicalize().unwrap()];

        let inside = resolve_within_roots(tmp.path().join("sub").to_str().unwrap(), &roots)
            .expect("inside root resolves");
        assert_eq!(inside, tmp.path().join("sub").canonicalize().unwrap());

        // The root itself is within the root.
        assert!(resolve_within_roots(tmp.path().to_str().unwrap(), &roots).is_ok());

        // Component-wise prefix: a sibling sharing the string prefix must
        // NOT count as inside (`/tmp/x` vs `/tmp/x-evil`).
        let sibling = tmp.path().parent().unwrap().join(format!(
            "{}-evil",
            tmp.path().file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir(&sibling).unwrap();
        assert!(matches!(
            resolve_within_roots(sibling.to_str().unwrap(), &roots),
            Err(WorkspaceFsError::OutsideWorkspaces)
        ));
        std::fs::remove_dir(&sibling).unwrap();
    }

    #[test]
    fn resolve_rejects_missing_and_outside() {
        let tmp = tempfile::tempdir().expect("tmp");
        let roots = vec![tmp.path().canonicalize().unwrap()];

        // Missing path anywhere → 404 (not 403: existence is not secret).
        assert!(matches!(
            resolve_within_roots("/definitely/not/here", &roots),
            Err(WorkspaceFsError::NotFound)
        ));

        // Exists, but outside every root → 403 with the uniform message.
        let outside = tempfile::tempdir().expect("outside");
        assert!(matches!(
            resolve_within_roots(outside.path().to_str().unwrap(), &roots),
            Err(WorkspaceFsError::OutsideWorkspaces)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_follows_symlink_out_of_root() {
        // A symlink inside the root pointing outside resolves (canonicalize)
        // to the target — outside — and must be refused, not served.
        let inside = tempfile::tempdir().expect("inside");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
        let link = inside.path().join("link.txt");
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), &link).unwrap();

        assert!(matches!(
            resolve_within_roots(
                link.to_str().unwrap(),
                &[inside.path().canonicalize().unwrap()]
            ),
            Err(WorkspaceFsError::OutsideWorkspaces)
        ));
    }

    // ── collect_entries ─────────────────────────────────────────────────────

    #[test]
    fn collect_sorts_dirs_first_case_insensitive() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::create_dir(tmp.path().join("zdir")).unwrap();
        std::fs::create_dir(tmp.path().join("Adir")).unwrap();
        std::fs::write(tmp.path().join("Zfile"), b"1").unwrap();
        std::fs::write(tmp.path().join("afile"), b"22").unwrap();

        let listing = collect_entries(tmp.path()).expect("ok");
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Adir", "zdir", "afile", "Zfile"]);
        // Sizes carried through; directories report their metadata len.
        let zfile = listing.entries.iter().find(|e| e.name == "Zfile").unwrap();
        assert_eq!(zfile.size, 1);
        assert!(!zfile.is_dir);
    }

    #[test]
    fn collect_skips_hidden_and_special_entries() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join(".hidden-file"), b"x").unwrap();
        std::fs::create_dir(tmp.path().join(".hidden-dir")).unwrap();
        std::fs::write(tmp.path().join("visible.txt"), b"x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("visible.txt", tmp.path().join("link")).unwrap();
        }

        let listing = collect_entries(tmp.path()).expect("ok");
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["visible.txt"]);
        assert!(!listing.truncated);
    }

    #[test]
    fn collect_marks_truncation_at_cap() {
        let tmp = tempfile::tempdir().expect("tmp");
        // One more than the cap → collection stops at MAX_ENTRIES and flags.
        for i in 0..=MAX_ENTRIES {
            std::fs::write(tmp.path().join(format!("f{i:05}")), b"x").unwrap();
        }
        let listing = collect_entries(tmp.path()).expect("ok");
        assert_eq!(listing.entries.len(), MAX_ENTRIES);
        assert!(listing.truncated);

        // Under the cap → no flag.
        let small = tempfile::tempdir().expect("small");
        std::fs::write(small.path().join("only.txt"), b"x").unwrap();
        let listing = collect_entries(small.path()).expect("ok");
        assert_eq!(listing.entries.len(), 1);
        assert!(!listing.truncated);
    }

    #[test]
    fn collect_unreadable_dir_maps_to_error() {
        let bogus = PathBuf::from("/this/does/not/exist");
        assert!(matches!(
            collect_entries(&bogus),
            Err(WorkspaceFsError::ReadDenied)
        ));
    }

    // ── text/binary preview ─────────────────────────────────────────────────

    #[test]
    fn text_preview_splits_lines_and_strips_cr() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("note.txt");
        std::fs::write(&file, b"alpha\r\nbeta\ngamma\n").unwrap();
        let md = std::fs::metadata(&file).unwrap();

        let content = read_text_or_binary(&file, &md).expect("ok");
        assert!(!content.is_binary);
        assert_eq!(content.lines.unwrap(), vec!["alpha", "beta", "gamma", ""]);
        assert_eq!(content.version.size, md.len());
    }

    #[test]
    fn text_preview_empty_file_yields_empty_lines() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("empty.txt");
        std::fs::write(&file, b"").unwrap();
        let md = std::fs::metadata(&file).unwrap();

        let content = read_text_or_binary(&file, &md).expect("ok");
        assert!(!content.is_binary);
        assert_eq!(content.lines.unwrap(), Vec::<String>::new());
        assert_eq!(content.version.size, 0);
    }

    #[test]
    fn null_byte_probe_short_circuits_to_binary() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("blob.bin");
        // NUL inside the probe window (first 8192 bytes) → binary, and the
        // body is never decoded.
        std::fs::write(&file, b"ok\x00not-text").unwrap();
        let md = std::fs::metadata(&file).unwrap();

        let content = read_text_or_binary(&file, &md).expect("ok");
        assert!(content.is_binary);
        assert!(content.lines.is_none());
        assert_eq!(content.version.size, md.len());
    }

    #[test]
    fn invalid_utf8_without_nul_degrades_to_binary_variant() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("bad.txt");
        // 0xFF is invalid UTF-8 but not a NUL — decode fails → is_binary.
        std::fs::write(&file, [0x68, 0x65, 0xFF, 0x79]).unwrap();
        let md = std::fs::metadata(&file).unwrap();

        let content = read_text_or_binary(&file, &md).expect("ok");
        assert!(content.is_binary);
        assert!(content.lines.is_none());
    }

    #[test]
    fn text_larger_than_probe_window_still_decodes() {
        // A text file crossing the 8192-byte probe boundary must reassemble
        // probe + remainder into the full content.
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("big.txt");
        let body = "x".repeat(PROBE_BYTES + 16);
        std::fs::write(&file, body.as_bytes()).unwrap();
        let md = std::fs::metadata(&file).unwrap();

        let content = read_text_or_binary(&file, &md).expect("ok");
        assert!(!content.is_binary);
        assert_eq!(content.lines.unwrap(), vec![body]);
    }

    // ── limits / mime / error mapping ───────────────────────────────────────

    #[test]
    fn limit_check_uses_branch_ceiling() {
        // Non-whitelisted: text ceiling.
        assert_eq!(
            check_preview_limit(TEXT_MAX_BYTES, None).unwrap(),
            TEXT_MAX_BYTES
        );
        assert!(matches!(
            check_preview_limit(TEXT_MAX_BYTES + 1, None),
            Err(WorkspaceFsError::TooLarge { size, limit })
                if size == TEXT_MAX_BYTES + 1 && limit == TEXT_MAX_BYTES
        ));
        // Whitelisted binary: bigger ceiling, and a text-oversized file is
        // still fine as a png.
        assert_eq!(
            check_preview_limit(BIN_MAX_BYTES, Some("image/png")).unwrap(),
            BIN_MAX_BYTES
        );
        assert!(check_preview_limit(TEXT_MAX_BYTES + 1, Some("image/png")).is_ok());
        assert!(matches!(
            check_preview_limit(BIN_MAX_BYTES + 1, Some("image/png")),
            Err(WorkspaceFsError::TooLarge { limit, .. }) if limit == BIN_MAX_BYTES
        ));
    }

    #[test]
    fn binary_mime_covers_whitelist() {
        assert_eq!(binary_mime("png"), Some("image/png"));
        assert_eq!(binary_mime("jpg"), Some("image/jpeg"));
        assert_eq!(binary_mime("jpeg"), Some("image/jpeg"));
        assert_eq!(binary_mime("gif"), Some("image/gif"));
        assert_eq!(binary_mime("webp"), Some("image/webp"));
        assert_eq!(binary_mime("svg"), Some("image/svg+xml"));
        assert_eq!(binary_mime("pdf"), Some("application/pdf"));
        assert_eq!(binary_mime("txt"), None);
        assert_eq!(binary_mime(""), None);
    }

    #[test]
    fn error_mapping_statuses_and_bodies() {
        let (status, Json(body)) = WorkspaceFsError::NotFound.into();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error, "not found");
        assert!(body.size.is_none() && body.limit.is_none());
        // Non-413 variants serialize without size/limit.
        assert!(serde_json::to_value(&body).unwrap().get("size").is_none());

        let (status, Json(body)) = WorkspaceFsError::OutsideWorkspaces.into();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.error, "outside registered workspaces");

        let (status, Json(body)) = WorkspaceFsError::NotADirectory.into();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "path is not a directory");

        let (status, Json(body)) = WorkspaceFsError::NotAFile.into();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "path is not a file");

        // 413 carries the numbers for the UI.
        let (status, Json(body)) = WorkspaceFsError::TooLarge {
            size: 12345,
            limit: 100,
        }
        .into();
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body.size, Some(12345));
        assert_eq!(body.limit, Some(100));
        assert!(serde_json::to_value(&body).unwrap().get("size").is_some());
    }

    #[test]
    fn split_lines_semantics() {
        assert_eq!(split_lines(""), Vec::<String>::new());
        assert_eq!(split_lines("one"), vec!["one"]);
        assert_eq!(split_lines("a\r\nb"), vec!["a", "b"]);
        // Lone \r mid-line is preserved; only the line-end \r is stripped.
        assert_eq!(split_lines("a\rb\r\n"), vec!["a\rb", ""]);
    }
}

#[cfg(test)]
mod git_status_tests {
    use super::*;

    /// `git status --porcelain=v1 -z --untracked-files=all` records: two XY
    /// chars, one space, path, NUL. Renames carry the old path as a second
    /// NUL-separated field right after the record.
    #[test]
    fn parses_modified_added_deleted_untracked() {
        let input = " M web/src/a.ts\0?? new.txt\0 D gone.rs\0M  staged.ts\0";
        let out = parse_git_status_z(input);
        assert_eq!(
            out,
            vec![
                GitFileStatus {
                    path: "web/src/a.ts".into(),
                    status: GitChangeKind::Modified
                },
                GitFileStatus {
                    path: "new.txt".into(),
                    status: GitChangeKind::Added
                },
                GitFileStatus {
                    path: "gone.rs".into(),
                    status: GitChangeKind::Deleted
                },
                GitFileStatus {
                    path: "staged.ts".into(),
                    status: GitChangeKind::Modified
                },
            ]
        );
    }

    #[test]
    fn rename_consumes_old_path_record_and_reports_new_path() {
        let input = "R  b.txt\0a.txt\0?? other\0";
        let out = parse_git_status_z(input);
        assert_eq!(
            out,
            vec![
                GitFileStatus {
                    path: "b.txt".into(),
                    status: GitChangeKind::Modified
                },
                GitFileStatus {
                    path: "other".into(),
                    status: GitChangeKind::Added
                },
            ]
        );
    }

    #[test]
    fn delete_beats_other_codes() {
        // `D ` / ` D` / `DM` all mean the file is gone from at least one tree.
        for xy in ["D ", " D", "DM"] {
            let out = parse_git_status_z(&format!("{xy} wip.rs\0"));
            assert_eq!(out[0].status, GitChangeKind::Deleted, "xy={xy}");
        }
    }

    #[test]
    fn empty_and_garbage_input_yield_nothing_or_skip() {
        assert!(parse_git_status_z("").is_empty());
        assert!(parse_git_status_z("\0\0\0").is_empty());
        // Too-short / non-graphic XY prefixes are skipped, not fatal.
        assert_eq!(parse_git_status_z("x\0?? ok.txt\0").len(), 1);
    }

    #[test]
    fn classify_xy_priority_order() {
        assert_eq!(classify_xy("D "), GitChangeKind::Deleted);
        assert_eq!(classify_xy(" D"), GitChangeKind::Deleted);
        assert_eq!(classify_xy("??"), GitChangeKind::Added);
        assert_eq!(classify_xy("A "), GitChangeKind::Added);
        assert_eq!(classify_xy(" M"), GitChangeKind::Modified);
        assert_eq!(classify_xy("MM"), GitChangeKind::Modified);
        assert_eq!(classify_xy("R "), GitChangeKind::Modified);
    }
}

// ── GET /api/v1/fs/git-diff ─────────────────────────────────────────────────

/// One inline-diff row kind: context (unchanged), added, or deleted line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

/// One row of a file's inline diff. With the daemon's huge `-U` context every
/// unchanged line appears too, so `lines` IS the full new-file content with
/// deleted lines interleaved at their positions.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// Line number in HEAD (context + deleted rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_no: Option<u32>,
    /// Line number in the worktree (context + added rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_no: Option<u32>,
    pub text: String,
}

/// Response DTO for `GET /api/v1/fs/git-diff`.
#[derive(Debug, Serialize)]
pub struct FileDiff {
    pub status: GitChangeKind,
    /// True when MAX_DIFF_LINES stopped collection early.
    pub truncated: bool,
    pub lines: Vec<DiffLine>,
}

/// Row cap per diff response — keeps pathological files bounded.
const MAX_DIFF_LINES: usize = 10_000;

/// Parse the body of a unified diff (output of `git diff … -- <path>`) into
/// flat inline rows. Lines before the first `@@` header are file metadata and
/// skipped; `\ No newline at end of file` markers are skipped; a second `@@`
/// resets the counters (single-file diffs rarely have one at `-U1000000`).
pub(crate) fn parse_unified_diff(body: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    let mut in_hunk = false;

    for line in body.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        let text = text.strip_suffix('\r').unwrap_or(text);

        if let Some(hunk) = text.strip_prefix("@@") {
            // "@@ -a[,b] +c[,d] @@ ctx" — restart both counters.
            let hdr = hunk.split("@@").next().unwrap_or("").trim();
            let mut parts = hdr.split_whitespace();
            let old = parts.next().unwrap_or("-0").trim_start_matches('-');
            let new = parts.next().unwrap_or("+0").trim_start_matches('+');
            old_no = old
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            new_no = new
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue; // "diff --git"/"index"/"---"/"+++" metadata
        }

        let (kind, content) = match text.as_bytes().first() {
            Some(b' ') => (DiffLineKind::Context, &text[1..]),
            Some(b'-') => (DiffLineKind::Delete, &text[1..]),
            Some(b'+') => (DiffLineKind::Add, &text[1..]),
            Some(b'\\') => continue, // "\ No newline at end of file"
            _ => continue,
        };
        let mut row = DiffLine {
            kind,
            old_no: None,
            new_no: None,
            text: content.to_string(),
        };
        match kind {
            DiffLineKind::Context => {
                row.old_no = Some(old_no);
                row.new_no = Some(new_no);
                old_no += 1;
                new_no += 1;
            }
            DiffLineKind::Delete => {
                row.old_no = Some(old_no);
                old_no += 1;
            }
            DiffLineKind::Add => {
                row.new_no = Some(new_no);
                new_no += 1;
            }
        }
        out.push(row);
    }
    out
}

/// Apply the row cap: `(rows, truncated)`.
fn truncate_diff_lines(mut lines: Vec<DiffLine>) -> (Vec<DiffLine>, bool) {
    if lines.len() <= MAX_DIFF_LINES {
        return (lines, false);
    }
    lines.truncate(MAX_DIFF_LINES);
    (lines, true)
}

/// Like `ensure_within_roots`, but for a file that may have been deleted from
/// disk: canonicalize the PARENT, verify it is inside a workspace root, and
/// re-join the file name. A deleted file still has a diff against HEAD.
async fn ensure_file_within_roots(
    raw: &str,
    state: &DaemonState,
) -> Result<PathBuf, WorkspaceFsError> {
    let path = Path::new(raw);
    let parent = path.parent().ok_or(WorkspaceFsError::NotFound)?;
    let canon_parent = parent
        .canonicalize()
        .map_err(|_| WorkspaceFsError::NotFound)?;
    let roots = resolve_workspace_roots(state).await;
    if !roots.iter().any(|r| canon_parent.starts_with(r)) {
        return Err(WorkspaceFsError::OutsideWorkspaces);
    }
    Ok(canon_parent.join(path.file_name().ok_or(WorkspaceFsError::NotFound)?))
}

/// `GET /api/v1/fs/git-diff?path=<file>` — the file's full inline diff vs
/// HEAD (staged + unstaged), as flat rows with line numbers. Untracked files
/// diff against /dev/null (whole file added); deleted files still diff
/// (whole file removed). Runs git in the file's parent dir, so both main
/// checkouts and worktrees work without knowing the repo root.
pub async fn git_diff(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<FsPathQuery>,
) -> Result<Json<FileDiff>, (StatusCode, Json<ErrorBody>)> {
    let file = ensure_file_within_roots(q.path.as_deref().unwrap_or(""), &state).await?;
    let parent = file
        .parent()
        .ok_or(WorkspaceFsError::NotFound)?
        .to_path_buf();
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(WorkspaceFsError::NotFound)?
        .to_string();

    // One porcelain record for this pathspec: XY + tracked/untracked state.
    let status_out = worktrees::git(
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            &name,
        ],
        &parent,
    )
    .await
    .map_err(|(code, msg)| {
        (
            code,
            Json(ErrorBody {
                error: msg,
                size: None,
                limit: None,
            }),
        )
    })?;
    let record = status_out.split('\0').find(|r| r.len() >= 3).unwrap_or("");
    if record.is_empty() {
        // Unchanged (or already-matched index) — no rows to show.
        return Ok(Json(FileDiff {
            status: GitChangeKind::Modified,
            truncated: false,
            lines: Vec::new(),
        }));
    }
    let xy = &record[..2];
    let status = classify_xy(xy);
    let untracked = xy == "??";

    // Tracked (incl. staged adds): full diff vs HEAD. Untracked: diff against
    // the empty file; --no-index exits 1 when files differ, so spawn directly
    // instead of going through `worktrees::git` (which maps non-zero to Err).
    let body = if untracked {
        let out = tokio::process::Command::new("git")
            .args(["diff", "--no-index", "-U1000000", "--", "/dev/null", &name])
            .current_dir(&parent)
            .output()
            .await
            .map_err(|_| WorkspaceFsError::ReadFailed)?;
        if !out.status.success() && out.status.code() != Some(1) {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: msg,
                    size: None,
                    limit: None,
                }),
            ));
        }
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        worktrees::git(&["diff", "HEAD", "-U1000000", "--", &name], &parent)
            .await
            .map_err(|(code, msg)| {
                (
                    code,
                    Json(ErrorBody {
                        error: msg,
                        size: None,
                        limit: None,
                    }),
                )
            })?
    };

    let (lines, truncated) = truncate_diff_lines(parse_unified_diff(&body));
    Ok(Json(FileDiff {
        status,
        truncated,
        lines,
    }))
}

#[cfg(test)]
mod git_diff_tests {
    use super::*;

    #[test]
    fn parses_hunk_with_context_add_delete_and_line_numbers() {
        let body = "diff --git a/a.rs b/a.rs\nindex 111..222 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,4 @@ fn main\n old1\n-old2\n+new2\n+extra\n old3\n";
        let out = parse_unified_diff(body);
        assert_eq!(out.len(), 5);
        assert_eq!(
            out[0],
            DiffLine {
                kind: DiffLineKind::Context,
                old_no: Some(1),
                new_no: Some(1),
                text: "old1".into()
            }
        );
        assert_eq!(
            out[1],
            DiffLine {
                kind: DiffLineKind::Delete,
                old_no: Some(2),
                new_no: None,
                text: "old2".into()
            }
        );
        assert_eq!(
            out[2],
            DiffLine {
                kind: DiffLineKind::Add,
                old_no: None,
                new_no: Some(2),
                text: "new2".into()
            }
        );
        assert_eq!(
            out[3],
            DiffLine {
                kind: DiffLineKind::Add,
                old_no: None,
                new_no: Some(3),
                text: "extra".into()
            }
        );
        assert_eq!(
            out[4],
            DiffLine {
                kind: DiffLineKind::Context,
                old_no: Some(3),
                new_no: Some(4),
                text: "old3".into()
            }
        );
    }

    #[test]
    fn second_hunk_resets_counters_and_no_newline_marker_skipped() {
        let body = "@@ -1,1 +1,1 @@\n a\n\\ No newline at end of file\n@@ -5,2 +5,1 @@\n b\n-c\n";
        let out = parse_unified_diff(body);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].old_no, Some(1));
        assert_eq!(out[1].old_no, Some(5));
        assert_eq!(
            out[2],
            DiffLine {
                kind: DiffLineKind::Delete,
                old_no: Some(6),
                new_no: None,
                text: "c".into()
            }
        );
    }

    #[test]
    fn empty_and_metadata_only_bodies_yield_nothing() {
        assert!(parse_unified_diff("").is_empty());
        assert!(parse_unified_diff("diff --git a/x b/x\nindex 1..2\n").is_empty());
    }

    #[test]
    fn crlf_lines_are_stripped() {
        let out = parse_unified_diff("@@ -1,1 +1,1 @@\n+a\r\n");
        assert_eq!(out[0].text, "a");
    }

    #[test]
    fn truncation_caps_rows() {
        let lines: Vec<DiffLine> = (0..MAX_DIFF_LINES + 5)
            .map(|i| DiffLine {
                kind: DiffLineKind::Context,
                old_no: Some(i as u32),
                new_no: Some(i as u32),
                text: String::new(),
            })
            .collect();
        let (kept, truncated) = truncate_diff_lines(lines);
        assert!(truncated);
        assert_eq!(kept.len(), MAX_DIFF_LINES);
    }
}
