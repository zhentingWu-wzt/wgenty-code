# 会话绑定 Worktree（N:1）+ 会话归档实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 会话可绑定 git worktree（多会话可绑同一个），其工具调用落在该目录；会话可归档/取消归档以控制列表可见性。

**Architecture:** daemon 持有 `session_workdirs` 映射（session_id → worktree 路径），绑定/归档状态持久化到 `Session.metadata`；`execute_tool` 按 session_id 注入 `ToolContext.workdir`（未绑定 → None → 现状行为）。web 端新建会话三选一（主检出/现有 worktree/新建 worktree），会话列表按工作区分组，归档项在 `/sessions` 浏览器的 Archived 折叠区管理。

**Tech Stack:** Rust + axum（daemon）；React 18 + TS + zustand（web）；cargo test；vitest + @testing-library。

**Spec:** `docs/superpowers/specs/2026-08-02-session-worktree-binding-design.md`

## Global Constraints

- Rust：`cargo clippy --all-targets -- -D warnings` 零 warning；`cargo fmt`；commit 英文 Conventional Commits。
- Web（cwd `web/`）：`npm run lint && npm test && npm run typecheck && npm run build` 全绿（现有 56 个测试保持通过）。
- 绑定路径必须 canonicalize 后位于 canonicalized `settings.storage.working_dir` 内；仓库外路径一律 400。
- 未绑定会话（含 session_id 缺省 `"default"`）行为必须与现状完全一致（向后兼容 TUI/旧客户端）。
- 归档是可见性标志：daemon 不过滤 `GET /sessions`，只透传 `archived` 字段。
- 绑定会话单一身份：daemon session id 同时是 `/tools/execute` 的 session_id。
- 每个任务单独 commit。

## 文件结构

**Daemon：**

| 文件 | 职责 |
|:--|:--|
| `src/daemon/state.rs`（改） | `session_workdirs` map + bind/unbind/session_workdir/worktree_sessions helpers |
| `src/daemon/session_admin.rs`（新建） | 绑定/解绑/归档端点 handlers + 测试 |
| `src/daemon/models.rs`（改） | BindWorktreeRequest/Response、SetArchivedRequest、SessionResponse/SessionInfoResponse 加 worktree/archived 字段 |
| `src/daemon/handlers.rs`（改） | execute_tool 两处注入 workdir；sessions GET 响应带新字段 |
| `src/daemon/routes.rs`（改） | 注册 3 条新路由 |
| `src/context/memory_session.rs`（改） | metadata 读写的 worktree/archived 存取 + 加载时回填（若已有现成 metadata API 则复用） |
| `src/tools/search/grep.rs`、`glob_search.rs`（改） | 相对 path 走 `resolve_path(_, context.workdir)` |

**Web：**

| 文件 | 职责 |
|:--|:--|
| `web/src/api/types.ts`（改） | WorktreeBinding、SetArchived 请求/响应；SessionInfo/SessionResponse 加 worktree/archived |
| `web/src/api/client.ts`（改） | bindWorktree/unbindWorktree/setSessionArchived |
| `web/src/state/sessionManager.ts`（改） | `SessionEntry.worktree?`；`createLocalSession` 支持显式 id；`closeSession`（removeSession 已有） |
| `web/src/components/NewSessionModal.tsx`（新建） | 新建会话三选一弹窗 |
| `web/src/components/SessionList.tsx`（改） | 按工作区分组、分支 tag、归档/删除按钮、接 NewSessionModal |
| `web/src/components/SessionsBrowserModal.tsx`（改） | 默认隐藏归档 + Archived 折叠区 + 行内归档/取消归档 |
| `web/src/components/WorktreePanel.tsx`（改） | 删除前反查绑定会话，确认后先解绑再删 |

---

### Task 1: daemon — session_workdirs 状态与 helpers

**Files:**
- Modify: `src/daemon/state.rs`

**Interfaces:**
- Consumes: 现有 `DaemonState` 结构（`src/daemon/state.rs:35-98`）
- Produces（Task 2/3 依赖，签名必须一致）：
  ```rust
  // DaemonState 新字段：
  pub session_workdirs: Arc<RwLock<std::collections::HashMap<String, std::path::PathBuf>>>,
  // impl DaemonState:
  pub fn bind_session_worktree(&self, session_id: &str, path: std::path::PathBuf);
  pub fn unbind_session_worktree(&self, session_id: &str);
  pub fn session_workdir(&self, session_id: &str) -> Option<std::path::PathBuf>;
  pub fn worktree_sessions(&self, path: &std::path::Path) -> Vec<String>;
  ```

- [ ] **Step 1: 写失败测试**

在 `src/daemon/state.rs` 的 `#[cfg(test)]` 模块（没有则新建）追加：

```rust
    #[test]
    fn session_workdirs_bind_query_unbind() {
        // 用一个轻量方式构造 DaemonState 或直接测试 map 语义：
        // 若构造 DaemonState 成本高，将四个 helper 实现为
        // `pub(crate) fn` 自由函数操作 map，再测自由函数。
        let map = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        crate::daemon::state::bind_in(&map, "s1", std::path::PathBuf::from("/repo/.worktrees/a"));
        crate::daemon::state::bind_in(&map, "s2", std::path::PathBuf::from("/repo/.worktrees/a"));
        crate::daemon::state::bind_in(&map, "s3", std::path::PathBuf::from("/repo/.worktrees/b"));

        assert_eq!(
            crate::daemon::state::workdir_of(&map, "s1").unwrap(),
            std::path::PathBuf::from("/repo/.worktrees/a")
        );
        assert!(crate::daemon::state::workdir_of(&map, "nobody").is_none());

        let mut sessions = crate::daemon::state::sessions_of(&map, std::path::Path::new("/repo/.worktrees/a"));
        sessions.sort();
        assert_eq!(sessions, vec!["s1".to_string(), "s2".to_string()]);

        crate::daemon::state::unbind_in(&map, "s1");
        assert!(crate::daemon::state::workdir_of(&map, "s1").is_none());
        assert_eq!(
            crate::daemon::state::sessions_of(&map, std::path::Path::new("/repo/.worktrees/a")),
            vec!["s2".to_string()]
        );
    }
```

实现提示：把四个操作做成 `pub(crate)` 自由函数（`bind_in` / `unbind_in` / `workdir_of` / `sessions_of`，操作 `Arc<RwLock<HashMap<String, PathBuf>>>`），`DaemonState` 的方法做薄封装——这样无需构造重量级 DaemonState 即可单测。注意 `state.rs` 已有的 `RwLock` 是 tokio 还是 std 的，跟随现有 import。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test daemon::state`
Expected: FAIL（函数不存在）

- [ ] **Step 3: 实现**

- `DaemonState` 加字段 `session_workdirs`（构造处初始化为空 map）
- 四个自由函数 + 四个同名薄封装方法

- [ ] **Step 4: 验证**

Run: `cargo test daemon::state && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/daemon/state.rs
git commit -m "feat(daemon): add session_workdirs binding state"
```

---

### Task 2: daemon — 绑定/解绑/归档端点 + 响应字段

**Files:**
- Create: `src/daemon/session_admin.rs`
- Modify: `src/daemon/mod.rs`（`pub(crate) mod session_admin;`）、`src/daemon/routes.rs`、`src/daemon/models.rs`
- Modify: `src/daemon/handlers.rs`（sessions GET 响应带 worktree/archived 字段）
- Modify: `src/context/memory_session.rs`（metadata 存取；加载回填）

**Interfaces:**
- Consumes: Task 1 的 bind/unbind/session_workdir helpers；`Session.metadata: HashMap<String, Value>`（`src/context/memory_session.rs:25`）；现有 sessions handlers（`src/daemon/handlers.rs:838-988`）
- Produces（web Task 6 依赖的 wire 契约）：
  - `PUT /api/v1/sessions/:id/worktree` body `{"path": "...", "branch": "..."}` → 200 `{"session_id":"...","worktree":{"path":"...","branch":"..."}}`；400（路径逃逸/worktree 不存在）；404（session 不存在）
  - `DELETE /api/v1/sessions/:id/worktree` → 204
  - `PUT /api/v1/sessions/:id/archive` body `{"archived": true|false}` → 200 `{"session_id":"...","archived":bool}`；404
  - `SessionInfoResponse` 与 `SessionResponse` 新增 `worktree: Option<{path, branch}>` 和 `archived: bool`（serde default，缺省 false/None——旧 session 文件兼容）

- [ ] **Step 1: 写失败测试**

`src/daemon/session_admin.rs` 内 `#[cfg(test)]`：

```rust
    // 路径校验纯函数（handlers 复用）：
    // resolve_worktree_path(root: &Path, input: &str) -> Result<PathBuf, String>
    #[test]
    fn resolve_rejects_escape_and_outside() {
        let root = std::path::Path::new("/repo").canonicalize().unwrap();
        // 用 tempdir 更稳：root = tempdir；输入 ".worktrees/a" → Ok(root.join(...))
        // 输入 "../outside" / 绝对路径 "/etc/x" → Err
        // 输入不存在的 ".worktrees/nope" → Err（不存在）
    }
```

用 `tempfile::tempdir()` 造真实目录（dev-dependency 已有）：在 tempdir 里 `mkdir .worktrees/a`，断言合法输入返回 canonical 路径、逃逸/不存在返回 Err。metadata 的 worktree/archived 读写若 `memory_session.rs` 已有 metadata setter 则直接复用并测往返；没有则加 `set_metadata_value(id, key, value)` 类似的窄方法并测。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test daemon::session_admin`
Expected: FAIL

- [ ] **Step 3: 实现**

`session_admin.rs`：

```rust
//! Session worktree binding + archive endpoints (project v1: single repo).

use crate::daemon::state::DaemonState;
use axum::{extract::{Path as AxumPath, State}, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct BindWorktreeRequest {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Serialize)]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Serialize)]
pub struct BindWorktreeResponse {
    pub session_id: String,
    pub worktree: WorktreeRef,
}

#[derive(Debug, Deserialize)]
pub struct SetArchivedRequest {
    pub archived: bool,
}

#[derive(Debug, Serialize)]
pub struct SetArchivedResponse {
    pub session_id: String,
    pub archived: bool,
}

/// Canonicalize `input` (relative to `root` when not absolute) and require it
/// to exist and stay inside `root`. Err carries a client-facing message.
pub(crate) fn resolve_worktree_path(root: &Path, input: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|e| format!("bad working_dir: {e}"))?;
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        root.join(input)
    };
    let canon = joined
        .canonicalize()
        .map_err(|_| format!("worktree does not exist: {input}"))?;
    if !canon.starts_with(&root) {
        return Err(format!("worktree path escapes the project: {input}"));
    }
    Ok(canon)
}

// handlers: bind_worktree / unbind_worktree / set_archived
// - bind: resolve_worktree_path(working_dir, body.path)? → 400 on Err；
//   session 不存在 → 404；state.bind_session_worktree(id, canon)；
//   持久化 Session.metadata["worktree"] = {"path": canon(显示用相对或原样), "branch": body.branch}
// - unbind: state.unbind_session_worktree(id)；清 metadata["worktree"] → 204
// - set_archived: session 不存在 → 404；metadata["archived"] = body.archived
```

models.rs：`SessionInfoResponse`/`SessionResponse` 加 `#[serde(default)] pub archived: bool` 与 `#[serde(default, skip_serializing_if = "Option::is_none")] pub worktree: Option<WorktreeRef>`（WorktreeRef 放 models.rs 并从 session_admin re-export，或直接定义在 models.rs——选 models.rs，避免循环依赖）。

handlers.rs 的 sessions GET（list/get）响应填充这两个字段（从 Session.metadata 读）。`MemorySessionManager` 加载 session 时：若 `metadata["worktree"]` 存在则回填 `session_workdirs`（回填发生在 daemon 启动或 session 首次 load 时——选启动后首次访问 sessions 端点时惰性回填，实现最简单：GET /sessions 时顺带 reconcile map）。

routes.rs 注册：

```rust
.route("/api/v1/sessions/:id/worktree", axum::routing::put(session_admin::bind_worktree).delete(session_admin::unbind_worktree))
.route("/api/v1/sessions/:id/archive", axum::routing::put(session_admin::set_archived))
```

- [ ] **Step 4: 验证**

Run: `cargo test daemon:: && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/daemon/ src/context/memory_session.rs
git commit -m "feat(daemon): add worktree binding and archive endpoints"
```

---

### Task 3: daemon — execute_tool 注入 workdir

**Files:**
- Modify: `src/daemon/handlers.rs`（:516 与 :576 两处 ToolContext 构建点）

**Interfaces:**
- Consumes: Task 1 的 `state.session_workdir(&session_id)`
- Produces: 无新导出——行为契约：绑定会话的工具调用以 worktree 为 cwd/根；未绑定（含 `"default"`）不变

- [ ] **Step 1: 写失败测试（集成级）**

在 `src/daemon/session_admin.rs` 或 `handlers.rs` 测试模块：tempdir 里 `git init` + 建 `.worktrees/a`（用 Task 1/2 同款 helper），`bind_in(&map, "s1", <canon path>)`，然后直接测试注入逻辑对应的 helper——把两处构建点共用的 workdir 解析抽成一个小函数：

```rust
/// Resolve the effective workdir for a tool call: the session's bound
/// worktree, or None (= daemon cwd, current behavior).
pub(crate) fn effective_workdir(state: &DaemonState, session_id: &str) -> Option<PathBuf> {
    state.session_workdir(session_id)
}
```

测试聚焦：`bind_in` 后 `session_workdir` 返回 Some；未绑定返回 None。真正的"文件落在 worktree"端到端验证放在手动冒烟（spec 验收节），此处不造完整 ToolRegistry（成本过高，且 ToolContext.workdir 的消费路径已有工具层测试覆盖）。

- [ ] **Step 2: 实现注入**

两处构建点把 `workdir: None` 改为：

```rust
let session_wd = effective_workdir(&state, &session_id);
// ToolContext { ..., workdir: session_wd.as_deref(), ... }
```

注意 `session_id` 变量在两处的来源（`body.session_id.unwrap_or("default")` 之类），沿用现有解析结果。借用生命周期：把 `session_wd` 绑定在 ToolContext 构建之前。

- [ ] **Step 3: 验证**

Run: `cargo test daemon:: && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add src/daemon/
git commit -m "feat(daemon): execute tools in the session's bound worktree"
```

---

### Task 4: daemon — grep/glob 支持 workdir

**Files:**
- Modify: `src/tools/search/grep.rs`（:33 附近）、`src/tools/search/glob_search.rs`（:64 附近）

**Interfaces:**
- Consumes: `resolve_path`（`src/tools/mod.rs:29-38`）、`ToolContext.workdir`
- Produces: 无新导出——行为契约：相对 path 在绑定 workdir 下解析

- [ ] **Step 1: 写失败测试**

两文件各自的测试模块：构造带 `workdir: Some(tempdir)` 的 ToolContext，相对 path 指向 tempdir 内文件/目录，断言结果来自 tempdir 而非进程 cwd。参照 list_files 现有测试的写法（它已支持 workdir）。

- [ ] **Step 2: 跑测试确认失败 → 实现**

相对 path 改走 `resolve_path(path, context.workdir)`（一处一行，模式同 list_files）。

- [ ] **Step 3: 验证**

Run: `cargo test tools:: && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add src/tools/search/
git commit -m "feat(tools): resolve grep/glob paths against session workdir"
```

---

### Task 5: web — client/types + sessionManager 扩展

**Files:**
- Modify: `web/src/api/types.ts`、`web/src/api/client.ts`、`web/src/state/sessionManager.ts`
- Test: `web/src/api/client.test.ts`（追加）、`web/src/state/sessionManager.test.ts`（追加）

**Interfaces:**
- Consumes: Task 2 的 wire 契约
- Produces（Task 7/8/9 依赖）：
  ```ts
  interface WorktreeBinding { path: string; branch: string }
  client.bindWorktree(sessionId: string, req: WorktreeBinding): Promise<void>
  client.unbindWorktree(sessionId: string): Promise<void>
  client.setSessionArchived(sessionId: string, archived: boolean): Promise<void>
  // SessionInfo 增加 worktree?: WorktreeBinding | null; archived: boolean
  // sessionManager:
  SessionEntry.worktree?: WorktreeBinding
  createLocalSession(name?: string, opts?: { id?: string; daemonId?: string; worktree?: WorktreeBinding }): string
  setWorktree(id: string, wt: WorktreeBinding | null): void
  ```

- [ ] **Step 1: 写失败测试**

`client.test.ts` 追加（沿用现有 mockFetch 模式）：

```ts
  it("bindWorktree PUTs the binding", async () => {
    const spy = mockFetch({ session_id: "s1", worktree: { path: "/r/.worktrees/a", branch: "a" } });
    await client.bindWorktree("s1", { path: ".worktrees/a", branch: "a" });
    const [url, init] = spy.mock.calls[0];
    expect(url).toBe("/api/v1/sessions/s1/worktree");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body)).toEqual({ path: ".worktrees/a", branch: "a" });
  });

  it("unbindWorktree DELETEs the binding", async () => {
    const spy = mockFetch(undefined, 204);
    await client.unbindWorktree("s1");
    expect(spy.mock.calls[0][0]).toBe("/api/v1/sessions/s1/worktree");
    expect(spy.mock.calls[0][1].method).toBe("DELETE");
  });

  it("setSessionArchived PUTs the flag", async () => {
    const spy = mockFetch({ session_id: "s1", archived: true });
    await client.setSessionArchived("s1", true);
    expect(spy.mock.calls[0][0]).toBe("/api/v1/sessions/s1/archive");
    expect(JSON.parse(spy.mock.calls[0][1].body)).toEqual({ archived: true });
  });
```

`sessionManager.test.ts` 追加：

```ts
  it("createLocalSession accepts explicit id and worktree", () => {
    const id = useSessionManager
      .getState()
      .createLocalSession("bound", { id: "daemon-1", daemonId: "daemon-1", worktree: { path: ".worktrees/a", branch: "a" } });
    const e = useSessionManager.getState().entries[id];
    expect(id).toBe("daemon-1");
    expect(e.daemonId).toBe("daemon-1");
    expect(e.worktree?.branch).toBe("a");
  });

  it("setWorktree updates and clears the binding", () => {
    const id = useSessionManager.getState().createLocalSession("x");
    useSessionManager.getState().setWorktree(id, { path: ".worktrees/a", branch: "a" });
    expect(useSessionManager.getState().entries[id].worktree?.branch).toBe("a");
    useSessionManager.getState().setWorktree(id, null);
    expect(useSessionManager.getState().entries[id].worktree).toBeUndefined();
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/api/client.test.ts src/state/sessionManager.test.ts`
Expected: FAIL

- [ ] **Step 3: 实现**

types.ts：

```ts
export interface WorktreeBinding {
  path: string;
  branch: string;
}
// SessionInfo 增加：
//   worktree?: WorktreeBinding | null;
//   archived: boolean;
```

client.ts（追加在 checkpoints 区之后）：

```ts
  async bindWorktree(sessionId: string, req: WorktreeBinding): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/worktree`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async unbindWorktree(sessionId: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/worktree`, {
        method: "DELETE",
      }),
    );
  }

  async setSessionArchived(sessionId: string, archived: boolean): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/archive`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ archived }),
      }),
    );
  }
```

sessionManager.ts：`SessionEntry` 加 `worktree?: WorktreeBinding`；`createLocalSession(name?, opts?)`——`opts.id` 优先于 `web-` 前缀生成；`setWorktree` 用现有 `patchEntry`（注意 worktree 清除时设 undefined）。

- [ ] **Step 4: 验证**

Run: `cd web && npm test && npm run lint && npm run typecheck`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add web/src/api web/src/state
git commit -m "feat(web): client and store support for worktree binding and archive"
```

---

### Task 6: web — NewSessionModal（三选一）

**Files:**
- Create: `web/src/components/NewSessionModal.tsx`
- Modify: `web/src/components/SessionList.tsx`（"+ New session" 改为打开弹窗）
- Test: `web/src/components/NewSessionModal.test.tsx`（新建）

**Interfaces:**
- Consumes: Task 5 的 `createLocalSession(name, opts)`、`bindWorktree`、现有 `createSession`/`createWorktree`/`listWorktrees`；`CommandModal` 壳
- Produces: `<NewSessionModal client onClose />`——SessionList 唯一使用点

- [ ] **Step 1: 写失败测试**

```tsx
// NewSessionModal.test.tsx
// mock fetch：/sessions POST → {id:"d1",...}；/worktrees GET → [main, feat]；/worktrees POST → 201；/worktree PUT → 200
// 1. 默认主检出：点 Create → 不调任何 API，createLocalSession() 普通本地会话，onClose 被调
// 2. 选 "Existing worktree"：下拉显示 feat；Create → createSession → bindWorktree；
//    entry id === "d1" 且 entry.worktree.branch === "feat"
// 3. 选 "New worktree" 输入分支名 "feat-x"：Create → createSession → createWorktree({path:".worktrees/feat-x", branch:"feat-x"}) → bindWorktree
// 4. createWorktree 失败（400 "already exists"）→ 显示内联错误，不建本地会话
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/components/NewSessionModal.test.tsx`
Expected: FAIL

- [ ] **Step 3: 实现**

```tsx
// 关键流程（绑定路径）：
async function createBound(daemonClient, name, wt: { path: string; branch: string }) {
  const created = await daemonClient.createSession({ name });
  await daemonClient.bindWorktree(created.id, wt);
  useSessionManager.getState().createLocalSession(name ?? created.name ?? "Session", {
    id: created.id,
    daemonId: created.id,
    worktree: wt,
  });
}
```

- radio 三选：`main`（默认）/ `existing`（下拉 `listWorktrees()` 过滤 `!is_main`，显示 branch）/ `new`（文本输入分支名）
- new 的 path 推导与 WorktreePanel 一致：`.worktrees/${branch.trim().replaceAll("/", "-")}`
- 失败：内联 `panel-error`，不建本地 entry、不关弹窗
- 成功后 onClose()
- SessionList 的 "+ New session" 按钮从直接 `createLocalSession()` 改为打开此弹窗（本地 useState）

- [ ] **Step 4: 验证**

Run: `cd web && npm test && npm run lint && npm run typecheck`
Expected: 全绿（现有 SessionList 测试同步更新：new-session 按钮现在开弹窗）

- [ ] **Step 5: Commit**

```bash
git add web/src/components
git commit -m "feat(web): new session dialog with worktree binding options"
```

---

### Task 7: web — 会话列表分组 + 归档/删除按钮

**Files:**
- Modify: `web/src/components/SessionList.tsx`、`web/src/styles.css`
- Test: `web/src/components/SessionList.test.tsx`（追加分组/操作用例）

**Interfaces:**
- Consumes: Task 5 的 `SessionEntry.worktree`、`setSessionArchived`、`deleteSession`（已有）；`removeSession`（已有）
- Produces: 无新导出

- [ ] **Step 1: 写失败测试**

```tsx
// 1. 分组渲染：两个主检出会话 + 两个绑定同一 worktree 的会话 →
//    出现 "Main checkout" 组标题和 "⎇ feat-x" 组标题，各自组内 2 张卡片
// 2. 卡片归档按钮：点击 → setSessionArchived(daemonId, true) 被调 + 本地 entry 关闭
// 3. 卡片删除按钮：confirm 后 → deleteSession(daemonId) 被调 + removeSession
```

- [ ] **Step 2: 跑测试确认失败 → 实现**

- 分组：`Main checkout`（未绑定）+ 每个不同 `worktree.branch` 一组（组标题 mono 分支名 + 会话数）
- 卡片 hover 操作区：归档（Archive icon）/ 删除（Trash2 icon）两个 `btn-xs`
  - 归档：`client.setSessionArchived(daemonId, true)` 后 `removeSession(id)`（本地关闭；无 daemonId 的纯本地会话直接 removeSession）
  - 删除：`window.confirm` 后 `client.deleteSession(daemonId)`（有 daemonId 时）+ `removeSession(id)`
- CSS：`.session-card-actions`（默认 hidden，card hover 时 flex）、`.session-group-title`

- [ ] **Step 3: 验证**

Run: `cd web && npm test && npm run lint && npm run typecheck`
Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add web/src/components web/src/styles.css
git commit -m "feat(web): group sessions by workspace with archive/delete actions"
```

---

### Task 8: web — SessionsBrowser 归档视图 + WorktreePanel 解绑流程

**Files:**
- Modify: `web/src/components/SessionsBrowserModal.tsx`、`web/src/components/WorktreePanel.tsx`
- Test: 两组件测试追加

**Interfaces:**
- Consumes: Task 5 的 client 方法与 `SessionInfo.archived`、`worktree` 字段；daemon `worktree_sessions` 反查不可达（web 端用 `GET /sessions` 的 worktree 字段自行过滤反查）
- Produces: 无新导出

- [ ] **Step 1: 写失败测试**

```tsx
// SessionsBrowserModal：
// 1. 默认视图隐藏 archived 会话；底部 "Archived (N)" 折叠区存在
// 2. 展开 Archived → 显示归档会话；点 Unarchive → setSessionArchived(id, false) + 刷新
// 3. 行内 Archive 按钮 → setSessionArchived(id, true) + 从默认视图消失
// WorktreePanel：
// 4. 删除有绑定会话的 worktree：mock /sessions 返回一条 worktree 匹配的会话 →
//    确认文案含"will be unbound"；确认后先 unbindWorktree 再 DELETE worktree
```

- [ ] **Step 2: 跑测试确认失败 → 实现**

- SessionsBrowserModal：`const active = saved.filter(s => !s.archived); const archived = saved.filter(s => s.archived);` 默认渲染 active；底部复用 `RailSection`（`title={\`Archived (${archived.length})\`}` defaultCollapsed）渲染 archived，行内 Unarchive 按钮；每行加 Archive 按钮。行内操作后 `refresh()`。
- WorktreePanel.remove：先 `listSessions()`，过滤 `s.worktree?.path === w.path`（注意 daemon 存的是 canonical 路径，web 列表返回的路径可能与输入不同——以 `branch` 匹配为主、path 兜底）；有匹配则 confirm 文案改为 `"<branch>" has N bound session(s); they will be unbound and return to the main checkout. Remove the worktree?`，确认后逐个 `unbindWorktree(s.id)`（并对本地打开的 entry `setWorktree(id, null)`），再 `deleteWorktree(path)`。

- [ ] **Step 3: 验证**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add web/src/components
git commit -m "feat(web): archived sessions view and unbind-before-remove flow"
```

---

## 验收清单（全部任务完成后）

- [ ] `cargo test --all` 通过，`cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] `cd web && npm run lint && npm test && npm run typecheck && npm run build` 全绿
- [ ] 手动冒烟（spec 验收节）：建绑定 worktree 的会话让 agent 改文件 → 改动落在 `.worktrees/<name>`，主检出干净；第二个会话绑同一 worktree → 两会话同目录工作；归档会话 → 列表消失，`/sessions` 的 Archived 区可恢复
