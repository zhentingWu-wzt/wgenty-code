# Scoped Viewer Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the main-window subagent selector by automatically obtaining and refreshing scoped UI viewer tokens without weakening agent isolation.

**Architecture:** Keep viewer authentication inside `DaemonClient`: every scoped request ensures a token exists, retries once with a fresh token after `401` or `404`, and returns the final response to the typed endpoint method. Keep both task/delegate pollers alive after individual view-fetch failures so transient daemon errors cannot permanently disable selector updates.

**Tech Stack:** Rust, Tokio, Reqwest, Axum test server, Anyhow, Cargo test/fmt/clippy.

---

## File Structure

- Modify `src/tui/client.rs`: centralize scoped request authentication and bounded viewer refresh; add real HTTP regression tests.
- Modify `src/tui/agent/core.rs`: keep the sequential subagent view poller alive after a failed request.
- Modify `src/tui/agent/tool_dispatch.rs`: keep the parallel/static subagent view poller alive after a failed request.

### Task 1: Automatically create a viewer before the first scoped request

**Files:**
- Modify: `src/tui/client.rs:5-170`
- Test: `src/tui/client.rs` test module

- [ ] **Step 1: Add a failing real-HTTP test**

Add a `#[cfg(test)] mod tests` to `src/tui/client.rs` with a small Axum server. The server must count viewer creations and reject `/api/v1/agents/self` unless the request carries the issued token.

```rust
#[derive(Clone, Default)]
struct ScopedServerState {
    viewer_creations: Arc<AtomicUsize>,
    scoped_requests: Arc<AtomicUsize>,
}

#[tokio::test]
async fn root_view_creates_viewer_before_first_scoped_request() {
    let (base_url, state, server) = spawn_scoped_server().await;
    let client = DaemonClient::new(base_url);

    let view = client.get_root_agent_view("session-a").await.unwrap();

    assert_eq!(view.self_view.agent_id, "root");
    assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
    assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 1);
    server.abort();
}
```

The test server's viewer route returns `{"viewer_token":"viewer-1"}`. Its scoped route returns a serialized `LocalAgentViewResponse` only when `X-Wgenty-Viewer-Token: viewer-1` is present.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test --lib tui::client::tests::root_view_creates_viewer_before_first_scoped_request -- --nocapture
```

Expected: FAIL because `get_root_agent_view` sends no viewer header and receives `404 Not Found`; `viewer_creations` remains zero.

- [ ] **Step 3: Implement the minimal viewer initialization path**

Import `anyhow::Context` and `reqwest::{Method, Response, StatusCode}`. Add these private methods to `DaemonClient`:

```rust
async fn ensure_viewer(&self) -> anyhow::Result<()> {
    if self.viewer_token.read().await.is_some() {
        return Ok(());
    }
    self.create_viewer()
        .await
        .context("create trusted UI viewer before scoped agent request")
}

async fn scoped_request(&self, method: Method, url: &str) -> anyhow::Result<Response> {
    let token = self
        .viewer_token
        .read()
        .await
        .clone()
        .ok_or_else(|| anyhow::anyhow!("trusted UI viewer token is unavailable"))?;
    self.http_tools
        .request(method, url)
        .header("X-Wgenty-Viewer-Token", token)
        .send()
        .await
        .context("send capability-scoped agent request")
}
```

Update `get_root_agent_view` to call `ensure_viewer()`, then `scoped_request(Method::GET, &url)`. Keep its existing endpoint-specific non-success error and JSON decoding behavior, adding `.context(...)` to fallible operations.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
cargo test --lib tui::client::tests::root_view_creates_viewer_before_first_scoped_request -- --nocapture
```

Expected: PASS with one viewer creation and one scoped request.

- [ ] **Step 5: Run the client test module**

Run:

```bash
cargo test --lib tui::client::tests -- --nocapture
```

Expected: all client tests pass.

### Task 2: Refresh a stale viewer token exactly once

**Files:**
- Modify: `src/tui/client.rs:65-170`
- Test: `src/tui/client.rs` test module

- [ ] **Step 1: Add failing stale-token and bounded-retry tests**

Add a test-server mode that initially accepts no cached token, returns a new token from the viewer endpoint, and records request counts.

```rust
#[tokio::test]
async fn root_view_refreshes_stale_viewer_after_not_found() {
    let (base_url, state, server) = spawn_scoped_server().await;
    let client = DaemonClient::new(base_url);
    *client.viewer_token.write().await = Some("stale-viewer".to_string());

    let view = client.get_root_agent_view("session-a").await.unwrap();

    assert_eq!(view.self_view.agent_id, "root");
    assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
    assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn root_view_retries_auth_failure_only_once() {
    let (base_url, state, server) = spawn_always_not_found_server().await;
    let client = DaemonClient::new(base_url);

    let error = client.get_root_agent_view("session-a").await.unwrap_err();

    assert!(error.to_string().contains("404 Not Found"));
    assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 2);
    assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 2);
    server.abort();
}
```

The always-not-found case creates the initial viewer, receives `404`, refreshes once, retries once, and then returns the final failure. It must not loop.

- [ ] **Step 2: Run both tests and verify RED**

Run:

```bash
cargo test --lib tui::client::tests::root_view_refreshes_stale_viewer_after_not_found -- --nocapture
cargo test --lib tui::client::tests::root_view_retries_auth_failure_only_once -- --nocapture
```

Expected: stale-token test fails on the first `404`; bounded-retry test reports only the pre-request viewer creation and one scoped request.

- [ ] **Step 3: Implement one-refresh retry behavior**

Add a single entry point for scoped requests:

```rust
async fn send_scoped_request(&self, method: Method, url: &str) -> anyhow::Result<Response> {
    self.ensure_viewer().await?;
    let response = self.scoped_request(method.clone(), url).await?;
    if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND) {
        self.create_viewer()
            .await
            .context("refresh trusted UI viewer after scoped request rejection")?;
        return self.scoped_request(method, url).await;
    }
    Ok(response)
}
```

Use `send_scoped_request` from all four scoped endpoints:

- `get_root_agent_view`
- `navigate_agent_view`
- `get_child_transcript`
- `cancel_child`

Do not retry any second response, even when it is also `401` or `404`.

- [ ] **Step 4: Add and pass the non-authentication error test**

```rust
#[tokio::test]
async fn root_view_does_not_refresh_viewer_after_server_error() {
    let (base_url, state, server) = spawn_status_server(StatusCode::INTERNAL_SERVER_ERROR).await;
    let client = DaemonClient::new(base_url);

    let error = client.get_root_agent_view("session-a").await.unwrap_err();

    assert!(error.to_string().contains("500 Internal Server Error"));
    assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
    assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 1);
    server.abort();
}
```

Run:

```bash
cargo test --lib tui::client::tests -- --nocapture
```

Expected: all viewer initialization, refresh, bounded retry, and non-authentication tests pass.

- [ ] **Step 5: Refactor test helpers while keeping tests green**

Keep one `spawn_test_server` helper parameterized by scoped response behavior. Avoid sleeps: bind `127.0.0.1:0`, obtain `local_addr`, spawn `axum::serve`, and return the `JoinHandle` after the listener is ready.

Run:

```bash
cargo test --lib tui::client::tests -- --nocapture
```

Expected: all client tests remain green.

### Task 3: Keep progress pollers alive after transient errors

**Files:**
- Modify: `src/tui/agent/core.rs:424-440`
- Modify: `src/tui/agent/tool_dispatch.rs:40-54`

- [ ] **Step 1: Confirm both permanent-stop branches**

Run:

```bash
rg -n -U "get_root_agent_view[\\s\\S]{0,220}Err\\(_\\) => break" src/tui/agent/core.rs src/tui/agent/tool_dispatch.rs
```

Expected: two matches, one in each poller.

- [ ] **Step 2: Replace permanent termination with logged continuation**

In both loops, replace `Err(_) => break` with:

```rust
Err(error) => {
    tracing::warn!(
        session_id = %session_id,
        error = %error,
        "Failed to poll scoped subagent view; retrying"
    );
}
```

Use the local variable name `sid` in `tool_dispatch.rs`. Do not add another sleep; the loop already sleeps 500 ms before each request and remains bounded by `max_duration`/`max_poll_duration`.

- [ ] **Step 3: Verify the permanent-stop pattern is gone**

Run:

```bash
rg -n -U "get_root_agent_view[\\s\\S]{0,220}Err\\(_\\) => break" src/tui/agent/core.rs src/tui/agent/tool_dispatch.rs
```

Expected: no matches.

- [ ] **Step 4: Compile the affected TUI agent modules**

Run:

```bash
cargo test --lib tui::agent --no-run
```

Expected: compilation succeeds without warnings.

### Task 4: Full verification and documentation

**Files:**
- Modify if required by project convention: `CHANGELOG.md`

- [ ] **Step 1: Format the code**

Run:

```bash
cargo fmt
cargo fmt -- --check
```

Expected: formatting check exits successfully.

- [ ] **Step 2: Run Clippy with CI settings**

Run:

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: zero warnings and exit code 0.

- [ ] **Step 3: Run the complete test suite**

Run:

```bash
cargo test --all
```

Expected: all tests pass with zero failures.

- [ ] **Step 4: Record the user-visible bug fix**

Under the current unreleased section of `CHANGELOG.md`, add a concise bullet equivalent to:

```markdown
- Fixed the subagent selector disappearing when scoped UI viewer credentials were missing or invalidated by a daemon restart.
```

Run:

```bash
git diff --check
git status --short
```

Expected: only the planned client, poller, test, changelog, design, and plan files are changed.

- [ ] **Step 5: Commit the implementation**

```bash
git add src/tui/client.rs src/tui/agent/core.rs src/tui/agent/tool_dispatch.rs CHANGELOG.md docs/superpowers/plans/2026-07-11-fix-scoped-viewer-recovery.md
git commit -m "fix(tui): recover scoped subagent viewer"
```
