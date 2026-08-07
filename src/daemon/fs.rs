//! Read-only filesystem browsing for the web directory picker.
//!
//! `GET /api/v1/fs/dirs?path=<dir>` lists the sub-directories of `<dir>` so the
//! browser can render a folder tree without a real native picker (browsers
//! cannot expose true local paths). Only directories are returned — never
//! files — and path traversal via `..` is neutralized by canonicalization.
//!
//! The endpoint lives behind the daemon bearer token (see `routes.rs`), same as
//! every other protected route.

use axum::{extract::Query, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Query params for `GET /api/v1/fs/dirs`.
#[derive(Debug, Deserialize)]
pub struct DirQuery {
    /// Directory to list. Omit (or empty) to default to the user's home dir.
    pub path: Option<String>,
}

/// A single child directory entry. `is_hidden` lets the frontend de-emphasize
/// dot-directories (`.git`, `.config`, …) without re-deriving the rule.
#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_hidden: bool,
}

/// Response DTO.
#[derive(Debug, Clone, Serialize)]
pub struct DirListing {
    /// Canonicalized absolute path of the listed directory.
    pub current: String,
    /// Parent directory (canonicalized), or `None` at a filesystem root.
    pub parent: Option<String>,
    /// Sorted child directories (hidden ones interleaved; the frontend shades
    /// them via `is_hidden`). Entries that cannot be read (permission denied)
    /// are silently skipped rather than failing the whole request.
    pub entries: Vec<DirEntry>,
}

impl DirListing {
    /// Build a listing for `dir`. Canonicalizes `dir` first so symlinks and
    /// `..` segments resolve to a single absolute form; permission errors on
    /// individual children are swallowed (returned list may be partial).
    fn collect(dir: &Path) -> Result<Self, (StatusCode, &'static str)> {
        let resolved = if dir.as_os_str().is_empty() {
            crate::utils::home_dir()
        } else {
            dir.to_path_buf()
        };

        // Canonicalize to collapse `..`, resolve symlinks, and obtain the real
        // absolute path. A failure here means the directory does not exist or
        // is unreachable — surface as a 400 so the UI shows a clear message.
        let canon = resolved
            .canonicalize()
            .map_err(|_| (StatusCode::BAD_REQUEST, "directory does not exist"))?;

        if !canon.is_dir() {
            return Err((StatusCode::BAD_REQUEST, "path is not a directory"));
        }

        let entries = match std::fs::read_dir(&canon) {
            Ok(rd) => {
                let mut out: Vec<DirEntry> = rd
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        // Only directories; skip files entirely.
                        let ft = e.file_type().ok()?;
                        if !ft.is_dir() {
                            return None;
                        }
                        let name = e.file_name().to_string_lossy().to_string();
                        // Reuse the canonicalized parent + child name to avoid
                        // a second canonicalize per child (symlinked dirs keep
                        // their link path, which is the expected UX).
                        let path = canon.join(&name);
                        Some(DirEntry {
                            is_hidden: name.starts_with('.'),
                            name,
                            path: path.to_string_lossy().to_string(),
                        })
                    })
                    .collect();
                out.sort_by_key(|a| a.name.to_lowercase());
                out
            }
            // Permission denied on the parent itself — return an empty listing
            // rather than erroring, so the UI can still show the parent + back.
            Err(_) => Vec::new(),
        };

        let parent = canon.parent().map(|p| p.to_string_lossy().to_string());

        Ok(Self {
            current: canon.to_string_lossy().to_string(),
            parent,
            entries,
        })
    }
}

/// `GET /api/v1/fs/dirs?path=<dir>` — list sub-directories of `<dir>`.
///
/// Omit `path` (or pass empty) to list the user's home directory. Returns 400
/// if the path does not exist or is not a directory. Individual children that
/// cannot be read are skipped — the listing may be partial but never fails.
pub async fn list_dirs(
    Query(q): Query<DirQuery>,
) -> Result<Json<DirListing>, (StatusCode, &'static str)> {
    let dir: PathBuf = q
        .path
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::utils::home_dir);
    DirListing::collect(&dir).map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_lists_subdirs_only_sorted() {
        let tmp = tempfile::tempdir().expect("tmp");
        // Mix files + dirs, hidden + visible, and upper/lower case to exercise
        // the filter + case-insensitive sort.
        std::fs::create_dir(tmp.path().join("zeta")).unwrap();
        std::fs::create_dir(tmp.path().join("Alpha")).unwrap();
        std::fs::create_dir(tmp.path().join(".hidden")).unwrap();
        std::fs::write(tmp.path().join("a-file.txt"), b"x").unwrap();

        let listing = DirListing::collect(tmp.path()).expect("ok");
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec![".hidden", "Alpha", "zeta"]);
        // File filtered out; hidden flag set on the dot-dir.
        assert!(
            listing
                .entries
                .iter()
                .find(|e| e.name == ".hidden")
                .unwrap()
                .is_hidden
        );
        assert!(
            !listing
                .entries
                .iter()
                .find(|e| e.name == "Alpha")
                .unwrap()
                .is_hidden
        );
        // Parent populated (tempdir root is not a fs root).
        assert!(listing.parent.is_some());
    }

    #[test]
    fn collect_rejects_missing_path() {
        let bogus = PathBuf::from("/this/does/not/exist/anywhere");
        let err = DirListing::collect(&bogus).expect_err("should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn collect_rejects_plain_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let err = DirListing::collect(&file).expect_err("file is not a dir");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn collect_neutralizes_dotdot_traversal() {
        // `..` must canonicalize to the real parent, not escape anywhere — the
        // resolved path is always absolute and normalized.
        let tmp = tempfile::tempdir().expect("tmp");
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let up = nested.join("..");
        let listing = DirListing::collect(&up).expect("ok");
        // canonicalize(..) resolves to tmp/a, whose name is "a".
        assert_eq!(
            PathBuf::from(&listing.current).file_name().unwrap(),
            std::ffi::OsStr::new("a")
        );
    }
}
