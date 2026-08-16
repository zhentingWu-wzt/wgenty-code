//! HTTP integration tests for the workspace file preview endpoints
//! (`GET /api/v1/fs/entries`, `GET /api/v1/fs/file`).
//!
//! Contract: `openspec/changes/workspace-file-preview/specs/workspace-file-preview/spec.md`.
//! Tests boot the real axum router + bearer auth via the shared
//! `daemon_harness` (in-process, ephemeral port) and assert HTTP-observable
//! behavior only: status codes, JSON shapes, byte fidelity, path boundary.
//! Line-splitting edge cases and root resolution are already covered by the
//! unit tests inside `src/daemon/workspace_files.rs` and are not duplicated.

use crate::daemon_harness::{spawn_daemon, TestDaemon};
use serde_json::Value;
use std::path::Path;

/// Text preview ceiling from `workspace_files.rs` (1.5 MiB).
const TEXT_LIMIT: u64 = 1_572_864;
/// Binary preview ceiling from `workspace_files.rs` (5 MiB).
const BIN_LIMIT: u64 = 5_242_880;
/// Per-request entry cap from `workspace_files.rs`.
const MAX_ENTRIES: usize = 2000;

// ── helpers ─────────────────────────────────────────────────────────────────

/// Register `dir` as an additional project root in the daemon's registry.
/// This is the real injection path used by the daemon (`POST /projects`
/// persists through the same `ProjectRegistry::add`).
fn register_project(d: &TestDaemon, dir: &Path) {
    d.state
        .projects
        .add(dir.to_str().expect("tempdir path is utf-8"))
        .expect("register project root");
}

/// GET one of the fs endpoints with `path` properly query-encoded.
async fn fs_get(d: &TestDaemon, endpoint: &str, path: &Path) -> reqwest::Response {
    let mut url =
        reqwest::Url::parse(&format!("{}/{}", d.base, endpoint)).expect("base url parses");
    url.query_pairs_mut()
        .append_pair("path", &path.to_string_lossy());
    d.client.get(url).send().await.expect("fs request")
}

/// GET one of the fs endpoints and decode the JSON body.
async fn fs_json(d: &TestDaemon, endpoint: &str, path: &Path) -> (reqwest::StatusCode, Value) {
    let resp = fs_get(d, endpoint, path).await;
    let status = resp.status();
    (status, resp.json().await.expect("json body"))
}

/// Entry names of an `/fs/entries` body in listed order.
fn entry_names(body: &Value) -> Vec<&str> {
    body["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|e| e["name"].as_str().expect("entry name"))
        .collect()
}

/// Run `git` in `dir`, asserting success (same pattern as the `worktrees.rs`
/// unit tests — the daemon itself shells out to the `git` binary).
fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Group 1: path boundary ──────────────────────────────────────────────────

/// 1a — the main project root (daemon working dir) and a registered pseudo
/// project root (text file + png + nested dir) both serve entries/file.
#[tokio::test]
async fn registered_roots_serve_entries_and_file() {
    let d = spawn_daemon().await;

    // Main project root: a file round-trips through /fs/file.
    std::fs::write(d._temp.path().join("main-note.txt"), b"from main root").unwrap();
    let (status, body) = fs_json(&d, "fs/file", &d._temp.path().join("main-note.txt")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["lines"], serde_json::json!(["from main root"]));

    // Registered (non-main) pseudo project root.
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("notes.txt"), b"hello").unwrap();
    std::fs::write(root.path().join("img.png"), b"\x89PNG-not-a-real-image").unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/entries", root.path()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // Directory first, files case-insensitively after it.
    assert_eq!(entry_names(&body), ["src", "img.png", "notes.txt"]);
    assert_eq!(body["truncated"], serde_json::json!(false));
    assert_eq!(
        body["current"],
        serde_json::json!(root.path().canonicalize().unwrap().to_string_lossy())
    );

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("notes.txt")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["lines"], serde_json::json!(["hello"]));
}

/// 1b — a linked git worktree of a registered repo is inside the boundary
/// even though it is not a registered root itself (roots are discovered via
/// `git worktree list`, the daemon's real injection path). Also asserts the
/// worktree isolation scenario: the listing reflects the worktree workspace,
/// not the main checkout.
#[tokio::test]
async fn git_worktree_root_serves_worktree_files() {
    let d = spawn_daemon().await;

    let repo = tempfile::tempdir().unwrap();
    run_git(repo.path(), &["init", "-b", "main"]);
    run_git(repo.path(), &["config", "user.email", "t@t"]);
    run_git(repo.path(), &["config", "user.name", "t"]);
    std::fs::write(repo.path().join("tracked.txt"), b"from main checkout").unwrap();
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "init"]);

    let wt_base = tempfile::tempdir().unwrap();
    let wt = wt_base.path().join("wt");
    run_git(
        repo.path(),
        &["worktree", "add", wt.to_str().unwrap(), "-b", "feat"],
    );
    std::fs::write(wt.join("only-in-wt.txt"), b"worktree only\n").unwrap();
    register_project(&d, repo.path());

    // The linked worktree directory serves entries …
    let (status, body) = fs_json(&d, "fs/entries", &wt).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(entry_names(&body), ["only-in-wt.txt", "tracked.txt"]);
    assert_eq!(
        body["current"],
        serde_json::json!(wt.canonicalize().unwrap().to_string_lossy())
    );

    // … and file content comes from the worktree workspace.
    let (status, body) = fs_json(&d, "fs/file", &wt.join("tracked.txt")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["lines"], serde_json::json!(["from main checkout"]));

    // Isolation: the worktree-only file does not appear in the main checkout.
    let (status, body) = fs_json(&d, "fs/entries", repo.path()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(entry_names(&body), ["tracked.txt"]);
}

/// 1c — absolute system paths and `..` escapes are refused with a uniform 403
/// (which does not leak whether the target exists); unresolvable paths 404.
#[tokio::test]
async fn paths_outside_registered_workspaces_are_refused() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), b"a").unwrap();
    register_project(&d, root.path());

    // `/etc/passwd` (a real system file outside every workspace) → uniform 403
    // on both endpoints.
    #[cfg(unix)]
    for endpoint in ["fs/entries", "fs/file"] {
        let (status, body) = fs_json(&d, endpoint, Path::new("/etc/passwd")).await;
        assert_eq!(status, reqwest::StatusCode::FORBIDDEN, "{endpoint}");
        assert_eq!(body["error"], "outside registered workspaces");
        assert!(body.get("size").is_none());
    }

    // `..` traversal out of the root onto an existing sibling → same uniform
    // 403, byte-identical error field (no existence leak).
    let sibling = tempfile::tempdir().unwrap(); // same parent dir as `root`
    let escape = root
        .path()
        .join("..")
        .join(sibling.path().file_name().unwrap());
    let (status, body) = fs_json(&d, "fs/file", &escape).await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "outside registered workspaces");

    // A missing path is a 404, not a 403 (non-existence is not secret).
    let (status, body) = fs_json(&d, "fs/file", &root.path().join("missing.txt")).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not found");
    let (status, _) = fs_json(&d, "fs/entries", &root.path().join("missing-dir")).await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

/// 1d — a symlink inside a workspace pointing outside is refused on read, and
/// symlink entries are skipped entirely by the listing.
#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_is_refused_and_skipped_in_entries() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("real.txt"), b"real").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        root.path().join("leak.txt"),
    )
    .unwrap();
    register_project(&d, root.path());

    // Reading through the symlink canonicalizes outside → uniform 403.
    let (status, body) = fs_json(&d, "fs/file", &root.path().join("leak.txt")).await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "outside registered workspaces");

    // The listing never contains symlink entries.
    let (status, body) = fs_json(&d, "fs/entries", root.path()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(entry_names(&body), ["real.txt"]);
}

// ── Group 2: /fs/entries behavior ───────────────────────────────────────────

/// 2a — dirs before files, case-insensitive within each group, hidden entries
/// ignored, and each entry carries name/is_dir/size.
#[tokio::test]
async fn entries_order_dirs_first_case_insensitive_and_hidden_skipped() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Zdir")).unwrap();
    std::fs::create_dir(root.path().join("adir")).unwrap();
    std::fs::create_dir(root.path().join(".hiddendir")).unwrap();
    std::fs::write(root.path().join("Bfile.txt"), b"bb").unwrap();
    std::fs::write(root.path().join("afile.txt"), b"a").unwrap();
    std::fs::write(root.path().join("cFILE.md"), b"ccc").unwrap();
    std::fs::write(root.path().join(".hidden"), b"h").unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/entries", root.path()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(
        entry_names(&body),
        ["adir", "Zdir", "afile.txt", "Bfile.txt", "cFILE.md"]
    );
    assert_eq!(body["truncated"], serde_json::json!(false));

    let entries = body["entries"].as_array().unwrap();
    for e in entries {
        assert!(e["is_dir"].is_boolean(), "entry carries is_dir");
        assert!(e["size"].is_u64(), "entry carries size");
    }
    let bfile = entries.iter().find(|e| e["name"] == "Bfile.txt").unwrap();
    assert_eq!(bfile["is_dir"], serde_json::json!(false));
    assert_eq!(bfile["size"], serde_json::json!(2));
    let adir = entries.iter().find(|e| e["name"] == "adir").unwrap();
    assert_eq!(adir["is_dir"], serde_json::json!(true));
}

/// 2b — the entry cap stops collection at 2000 and flags `truncated`. The cap
/// is a const (not injectable), so the test materializes 2100 children.
#[tokio::test]
async fn entries_truncate_at_cap_with_flag() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    for i in 0..(MAX_ENTRIES + 100) {
        std::fs::write(root.path().join(format!("f{i:04}.txt")), b"").unwrap();
    }
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/entries", root.path()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), MAX_ENTRIES);
    assert_eq!(body["truncated"], serde_json::json!(true));
    // Which 2000 survive depends on readdir order, but whatever is kept is
    // still returned sorted (case-insensitive; all entries are files here).
    let names = entry_names(&body);
    assert!(
        names
            .windows(2)
            .all(|w| w[0].to_lowercase() <= w[1].to_lowercase()),
        "kept entries must stay sorted"
    );
}

// ── Group 3: /fs/file behavior ──────────────────────────────────────────────

/// 3a — text preview: `lines` split on `\n` with CRLF stripped, `version`
/// carries `mtime_ms` and `size`.
#[tokio::test]
async fn file_text_lines_and_version() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let content = "alpha\nbeta\r\ngamma";
    std::fs::write(root.path().join("notes.txt"), content).unwrap();
    register_project(&d, root.path());

    let resp = fs_get(&d, "fs/file", &root.path().join("notes.txt")).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert!(resp.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["lines"], serde_json::json!(["alpha", "beta", "gamma"]));
    assert_eq!(body["is_binary"], serde_json::json!(false));
    assert_eq!(body["version"]["size"], serde_json::json!(content.len()));
    let mtime_ms = body["version"]["mtime_ms"].as_u64().expect("mtime_ms set");
    assert!(mtime_ms > 0, "file was just written; mtime must be nonzero");
}

/// 3b — whitelisted binary preview: raw bytes with the mapped Content-Type.
#[tokio::test]
async fn file_png_streams_bytes_with_content_type() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let png: &[u8] = b"\x89PNG\r\n\x1a\nnot-a-real-image";
    std::fs::write(root.path().join("pic.png"), png).unwrap();
    register_project(&d, root.path());

    let resp = fs_get(&d, "fs/file", &root.path().join("pic.png")).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers()[reqwest::header::CONTENT_TYPE]
            .to_str()
            .unwrap(),
        "image/png"
    );
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(&bytes[..], png);
}

/// 3c — a 2 MiB text file exceeds the 1.5 MiB ceiling: 413 whose body carries
/// the real `size` and the applicable `limit`.
#[tokio::test]
async fn file_oversized_text_returns_413_with_size_and_limit() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let big = "x".repeat(2 * 1024 * 1024); // 2 MiB > 1.5 MiB
    std::fs::write(root.path().join("big.txt"), big.as_bytes()).unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("big.txt")).await;
    assert_eq!(status, reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "file too large to preview");
    assert_eq!(body["size"], serde_json::json!(big.len()));
    assert_eq!(body["limit"], serde_json::json!(TEXT_LIMIT));
}

/// 3c' — the binary branch has its own 5 MiB ceiling: an oversized png 413s
/// with the binary limit (not the text one).
#[tokio::test]
async fn file_oversized_binary_returns_413_with_binary_limit() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let huge = vec![0u8; BIN_LIMIT as usize + 1];
    std::fs::write(root.path().join("huge.png"), &huge).unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("huge.png")).await;
    assert_eq!(status, reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "file too large to preview");
    assert_eq!(body["size"], serde_json::json!(huge.len()));
    assert_eq!(body["limit"], serde_json::json!(BIN_LIMIT));
}

/// 3d — a non-whitelisted binary file (NUL byte): JSON variant with
/// `is_binary: true` and the `lines` field omitted.
#[tokio::test]
async fn file_binary_variant_for_nul_content() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let content = b"abc\0def";
    std::fs::write(root.path().join("data.bin"), content).unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("data.bin")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["is_binary"], serde_json::json!(true));
    assert!(body.get("lines").is_none(), "binary variant omits lines");
    assert_eq!(body["version"]["size"], serde_json::json!(content.len()));
}

/// 3e — an empty file is valid UTF-8: present-but-empty `lines` array.
#[tokio::test]
async fn file_empty_yields_empty_lines() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("empty.txt"), b"").unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("empty.txt")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["lines"], serde_json::json!([]));
    assert_eq!(body["is_binary"], serde_json::json!(false));
    assert_eq!(body["version"]["size"], serde_json::json!(0));
}

/// 3f — path shape mismatches: a directory is not a file (400) and a file is
/// not a listable directory (400).
#[tokio::test]
async fn directory_and_file_path_mismatches_return_400() {
    let d = spawn_daemon().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("a.txt"), b"a").unwrap();
    register_project(&d, root.path());

    let (status, body) = fs_json(&d, "fs/file", &root.path().join("src")).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "path is not a file");

    let (status, body) = fs_json(&d, "fs/entries", &root.path().join("a.txt")).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "path is not a directory");
}
