# Web 多会话指挥中心实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 web 前端从单会话瘦客户端重构为多会话 Agent 指挥中心（三栏布局 + 每会话独立运行时 + worktree/skill 面板 + 视觉系统升级）。

**Architecture:** 每会话一个 zustand store 实例（`createSessionStore` 工厂）+ 全局 `sessionManager` 注册表；agent loop 仍在浏览器驱动，每会话独立 `runAgentLoop`。daemon 仅新增 3 个轻量端点（worktrees list/create/delete、skills list），包装 `git` CLI 和现有 `SkillLoader`，不动 agent 核心。

**Tech Stack:** React 18 + TypeScript + Vite + zustand（web）；Rust + axum（daemon）；vitest + @testing-library（web 测试）；cargo test（Rust 测试）。

**Spec:** `docs/superpowers/specs/2026-08-02-web-command-center-design.md`

## Global Constraints

- Rust：`cargo clippy --all-targets -- -D warnings` 零 warning；`cargo fmt`；commit 用英文 Conventional Commits。
- Web：`npm run lint` / `npm test` / `npm run typecheck` / `npm run build` 全绿才算任务完成（工作目录 `web/`）。
- 每个任务单独 commit；Rust 任务 `cargo test <模块>` 通过，web 任务相关 vitest 通过。
- daemon 已知限制（不在本期修复，UI 如实标注）：审批规则 / todos / tasks / 模型切换是进程级全局共享。
- 所有会话共享 daemon 单一 working_dir；worktree 面板只是管理入口，不与会话绑定。
- skill 启停不在本期（knowledge 模块无启用概念），SkillPanel 只读。
- Web 端文本沿用现有硬编码英文风格（i18n 不在本期范围）。

## 文件结构

**Daemon（Rust，新增 2 个模块，改 2 个文件）：**

| 文件 | 职责 |
|:--|:--|
| `src/daemon/worktrees.rs`（新建） | worktree 端点 handlers + porcelain 解析 + git 命令封装 + 单元测试 |
| `src/daemon/skills_api.rs`（新建） | skills 列表端点 handler + 映射函数 + 单元测试 |
| `src/daemon/mod.rs`（改） | `mod worktrees; mod skills_api;` 声明 |
| `src/daemon/routes.rs`（改） | 注册 4 条新路由 |

**Web（新增 9 个文件，改 ~10 个）：**

| 文件 | 职责 |
|:--|:--|
| `web/src/api/types.ts`（改） | WorktreeInfo / SkillInfoDto / CheckpointInfo / UndoTurnResult 类型 |
| `web/src/api/client.ts`（改） | listWorktrees / createWorktree / deleteWorktree / listSkills / listCheckpoints / undoTurns |
| `web/src/state/sessionStore.ts`（新建） | `createSessionStore()` 工厂（现 chatStore 内容），abort controller 每实例隔离 |
| `web/src/state/sessionManager.ts`（新建） | 会话注册表：entries、activeId、status 状态机、connection/modelName |
| `web/src/state/sessionContext.tsx`（新建） | `SessionStoreContext` + `useSessionStore(selector)` hook |
| `web/src/agent/sessionRunner.ts`（新建） | `runSessionTurn(client, sessionId, text)`：从 App.tsx 抽出的发送逻辑，绑会话 store + 更新 meta |
| `web/src/components/LeftRail.tsx`（新建） | 左栏容器：SessionList + WorktreePanel + SkillPanel + RailFooter |
| `web/src/components/SessionList.tsx`（新建） | 会话卡片列表（打开中的 + daemon 保存的） |
| `web/src/components/WorktreePanel.tsx` / `SkillPanel.tsx`（新建） | 两个只读/轻操作面板 |
| `web/src/components/ContextPanel.tsx`（新建） | 右栏容器：Todos/Tasks/Memory/Subagent/Checkpoints |
| `web/src/components/SubagentPanel.tsx` / `CheckpointsPanel.tsx`（新建） | trace 时间线（按会话过滤）/ checkpoint 列表 + undo |
| `web/src/components/SessionHeader.tsx`（新建） | 中栏顶：会话名 + 状态 pill |
| `web/src/components/ChatView.tsx` / `Composer.tsx` / `PermissionModal.tsx` / `StatusBar.tsx`（改） | 订阅从全局单例改为 context 注入 |
| `web/src/state/chatStore.ts`（Task 5 改、Task 7 删） | 过渡期 re-export 单例 |
| `web/src/components/Sidebar.tsx`（Task 9 删） | 被 LeftRail 替代 |
| `web/src/App.tsx`（改） | 多会话接线 + beforeunload |
| `web/src/styles.css`（改） | 三栏布局 + accent 色 + 视觉细节 |

---

### Task 1: daemon — GET /api/v1/worktrees（列表）

**Files:**
- Create: `src/daemon/worktrees.rs`
- Modify: `src/daemon/mod.rs`（加 `pub(crate) mod worktrees;`）
- Modify: `src/daemon/routes.rs`（注册路由）

**Interfaces:**
- Consumes: `DaemonState.app_state.settings.storage.working_dir`（`PathBuf`，项目根，git 命令的 cwd）
- Produces: `WorktreeInfo { path: String, head: String, branch: Option<String>, is_main: bool }`（serde Serialize）；`parse_worktree_list(input: &str) -> Vec<WorktreeInfo>`；handler `list_worktrees`。Task 2 复用 `git()` 帮助函数和 `WorktreeInfo`。

- [ ] **Step 1: 写失败测试**

新建 `src/daemon/worktrees.rs`，先只放测试和类型：

```rust
//! Git worktree endpoints for the web command center's WorktreePanel.
//! Thin wrappers around `git worktree` run in the daemon's working_dir.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorktreeInfo {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    /// The first entry in `git worktree list` is always the main worktree.
    pub is_main: bool,
}

/// Parse `git worktree list --porcelain` output. Blocks are separated by blank
/// lines; `branch` is absent for detached HEAD entries.
pub(crate) fn parse_worktree_list(input: &str) -> Vec<WorktreeInfo> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;

    let mut flush = |path: &mut Option<String>, head: &mut String, branch: &mut Option<String>| {
        if let Some(p) = path.take() {
            out.push(WorktreeInfo {
                path: p,
                head: std::mem::take(head),
                branch: branch.take(),
                is_main: out.is_empty(), // first block = main worktree
            });
        }
    };

    for line in input.lines() {
        if line.is_empty() {
            flush(&mut path, &mut head, &mut branch);
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = h.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = Some(b.to_string());
        }
    }
    flush(&mut path, &mut head, &mut branch);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_and_linked_worktrees() {
        let input = "worktree /repo\nHEAD 111aaa\nbranch refs/heads/main\n\nworktree /repo/.worktrees/feat\nHEAD 222bbb\nbranch refs/heads/feature/x\n\n";
        let wt = parse_worktree_list(input);
        assert_eq!(wt.len(), 2);
        assert!(wt[0].is_main);
        assert_eq!(wt[0].branch.as_deref(), Some("main"));
        assert!(!wt[1].is_main);
        assert_eq!(wt[1].path, "/repo/.worktrees/feat");
    }

    #[test]
    fn detached_head_has_no_branch() {
        let input = "worktree /repo\nHEAD 111aaa\ndetached\n\n";
        let wt = parse_worktree_list(input);
        assert_eq!(wt.len(), 1);
        assert_eq!(wt[0].branch, None);
        assert_eq!(wt[0].head, "111aaa");
    }

    #[test]
    fn empty_input_yields_empty_list() {
        assert!(parse_worktree_list("").is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认通过（解析器是纯函数，一次到位）**

Run: `cargo test daemon::worktrees`
Expected: 3 个测试 PASS（若失败，修解析器直到通过）

- [ ] **Step 3: 加 git 封装和 handler**

在 `src/daemon/worktrees.rs` 追加：

```rust
use crate::daemon::state::DaemonState;
use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

/// Run `git` in the daemon's working_dir; on non-zero exit return the stderr
/// text so the web panel can show why (e.g. "already exists").
pub(crate) async fn git(args: &[&str], state: &DaemonState) -> Result<String, (StatusCode, String)> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(&state.app_state.settings.storage.working_dir)
        .output()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn git: {e}"),
            )
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err((StatusCode::BAD_REQUEST, stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// GET /api/v1/worktrees — list git worktrees (main first).
pub async fn list_worktrees(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<WorktreeInfo>>, (StatusCode, String)> {
    let stdout = git(&["worktree", "list", "--porcelain"], &state).await?;
    Ok(Json(parse_worktree_list(&stdout)))
}
```

在 `src/daemon/mod.rs` 的模块声明区加：

```rust
pub(crate) mod worktrees;
```

在 `src/daemon/routes.rs` 的 `protected` router 链上加（放在 Sessions 路由附近）：

```rust
        // Worktrees (web command center)
        .route("/api/v1/worktrees", get(worktrees::list_worktrees))
```

注意：`routes.rs` 顶部已有 `use crate::daemon::handlers;`，handlers 是单个文件模块；新模块不挂在 handlers 下，在顶部另加 `use crate::daemon::worktrees;`。

- [ ] **Step 4: 编译 + 测试 + lint**

Run: `cargo test daemon::worktrees && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/daemon/worktrees.rs src/daemon/mod.rs src/daemon/routes.rs
git commit -m "feat(daemon): add GET /worktrees endpoint"
```

---

### Task 2: daemon — POST/DELETE /api/v1/worktrees（创建/删除）

**Files:**
- Modify: `src/daemon/worktrees.rs`
- Modify: `src/daemon/routes.rs`

**Interfaces:**
- Consumes: Task 1 的 `git()`、`parse_worktree_list()`、`WorktreeInfo`
- Produces: handlers `create_worktree` / `delete_worktree`；请求体 `CreateWorktreeRequest { path, branch }`；query `DeleteWorktreeQuery { path }`

- [ ] **Step 1: 写失败测试（集成级：临时 git 仓库跑真实 git 命令）**

先确认 dev-dependency 有 tempfile：`grep '^tempfile' Cargo.toml`；没有则 `cargo add --dev tempfile`。

在 `src/daemon/worktrees.rs` 的 tests 模块追加（测试直接驱动 git 命令序列 + 解析，不走 HTTP——HTTP 层只是参数透传）：

```rust
    async fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);
        dir
    }

    #[tokio::test]
    async fn add_list_remove_roundtrip() {
        let dir = init_repo().await;
        let wt_path = dir.path().join(".worktrees").join("feat");
        std::fs::create_dir_all(wt_path.parent().unwrap()).unwrap();

        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {:?}: {:?}", args, out);
            String::from_utf8_lossy(&out.stdout).to_string()
        };

        // create
        run(&["worktree", "add", wt_path.to_str().unwrap(), "-b", "feat"]);
        // list: main + linked
        let list = parse_worktree_list(&run(&["worktree", "list", "--porcelain"]));
        assert_eq!(list.len(), 2);
        assert!(list[0].is_main);
        assert_eq!(list[1].branch.as_deref(), Some("feat"));
        // remove
        run(&["worktree", "remove", wt_path.to_str().unwrap()]);
        let list = parse_worktree_list(&run(&["worktree", "list", "--porcelain"]));
        assert_eq!(list.len(), 1);
    }
```

- [ ] **Step 2: 跑测试**

Run: `cargo test daemon::worktrees`
Expected: PASS（若环境无 git 则跳过本测试的执行，本地/CI 有 git）

- [ ] **Step 3: 实现 handlers**

在 `src/daemon/worktrees.rs` 追加：

```rust
use axum::extract::Query;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateWorktreeRequest {
    /// Target directory for the new worktree (absolute or relative to working_dir).
    pub path: String,
    /// New branch name to create at HEAD (`git worktree add <path> -b <branch>`).
    pub branch: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteWorktreeQuery {
    pub path: String,
}

/// POST /api/v1/worktrees — create a worktree on a new branch at HEAD.
pub async fn create_worktree(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CreateWorktreeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if body.path.trim().is_empty() || body.branch.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path and branch are required".into()));
    }
    git(&["worktree", "add", &body.path, "-b", &body.branch], &state).await?;
    Ok(StatusCode::CREATED)
}

/// DELETE /api/v1/worktrees?path=… — remove a linked worktree. The main
/// worktree is refused: deleting it would destroy the repo checkout.
pub async fn delete_worktree(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<DeleteWorktreeQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let stdout = git(&["worktree", "list", "--porcelain"], &state).await?;
    let entries = parse_worktree_list(&stdout);
    let target = entries
        .iter()
        .find(|w| w.path == q.path)
        .ok_or((StatusCode::NOT_FOUND, format!("no such worktree: {}", q.path)))?;
    if target.is_main {
        return Err((StatusCode::BAD_REQUEST, "refusing to remove the main worktree".into()));
    }
    git(&["worktree", "remove", &q.path], &state).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

在 `src/daemon/routes.rs` 把 Task 1 加的那行扩展为：

```rust
        // Worktrees (web command center)
        .route(
            "/api/v1/worktrees",
            get(worktrees::list_worktrees).post(worktrees::create_worktree).delete(worktrees::delete_worktree),
        )
```

（`routing::{get, post}` 的 import 已存在；链式 `.post().delete()` 不需要额外 import。）

- [ ] **Step 4: 编译 + 测试 + lint**

Run: `cargo test daemon::worktrees && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/daemon/worktrees.rs src/daemon/routes.rs
git commit -m "feat(daemon): add create/delete worktree endpoints"
```

---

### Task 3: daemon — GET /api/v1/skills（列表）

**Files:**
- Create: `src/daemon/skills_api.rs`
- Modify: `src/daemon/mod.rs`（加 `pub(crate) mod skills_api;`）
- Modify: `src/daemon/routes.rs`（注册路由）

**Interfaces:**
- Consumes: `DaemonState.skill_loader: Arc<SkillLoader>`（`src/knowledge/loader.rs`：`skill_names() -> Vec<String>`、`load_skill(name) -> Option<&SkillInfo>`，`SkillInfo { name, description, source_path }`）
- Produces: `SkillEntry { name: String, description: String, source_path: String }`（Serialize）；`collect_skills(loader: &SkillLoader) -> Vec<SkillEntry>`；handler `list_skills`

- [ ] **Step 1: 写失败测试**

新建 `src/daemon/skills_api.rs`：

```rust
//! Skills list endpoint for the web command center's SkillPanel (read-only).

use crate::daemon::state::DaemonState;
use crate::knowledge::loader::SkillLoader;
use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub source_path: String,
}

/// Map the loader's skills into response entries, sorted by name for a stable
/// panel display. (`skill_names` + `load_skill` are the loader's only public
/// accessors; every name resolves, so the filter_map never drops in practice.)
pub(crate) fn collect_skills(loader: &SkillLoader) -> Vec<SkillEntry> {
    let mut out: Vec<SkillEntry> = loader
        .skill_names()
        .into_iter()
        .filter_map(|n| loader.load_skill(&n))
        .map(|s| SkillEntry {
            name: s.name.clone(),
            description: s.description.clone(),
            source_path: s.source_path.display().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// GET /api/v1/skills — list skills visible to the daemon (read-only).
pub async fn list_skills(State(state): State<Arc<DaemonState>>) -> Json<Vec<SkillEntry>> {
    Json(collect_skills(&state.skill_loader))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_and_sorts_skills() {
        let dir = tempfile::tempdir().unwrap();
        for (name, desc) in [("zeta", "last"), ("alpha", "first")] {
            let skill_dir = dir.path().join("skills").join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
            )
            .unwrap();
        }
        let loader = SkillLoader::load_from_dir(dir.path());
        let entries = collect_skills(&loader);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[0].description, "first");
        assert!(entries[0].source_path.ends_with("SKILL.md"));
    }
}
```

（tempfile 已在 Task 2 确认/添加为 dev-dependency。注意 `parse_frontmatter` 依赖 SKILL.md 的 frontmatter 格式，若该测试失败说明格式假设不符——打开 `src/knowledge/loader.rs` 的 `parse_frontmatter` 核对实际格式并修正测试输入。）

- [ ] **Step 2: 跑测试**

Run: `cargo test daemon::skills_api`
Expected: PASS

- [ ] **Step 3: 注册模块和路由**

`src/daemon/mod.rs` 加：

```rust
pub(crate) mod skills_api;
```

`src/daemon/routes.rs` 的 protected router 加：

```rust
        // Skills (web command center, read-only)
        .route("/api/v1/skills", get(skills_api::list_skills))
```

（`use crate::daemon::skills_api;` 加到顶部。）

- [ ] **Step 4: 编译 + 测试 + lint**

Run: `cargo test daemon:: && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add src/daemon/skills_api.rs src/daemon/mod.rs src/daemon/routes.rs
git commit -m "feat(daemon): add GET /skills endpoint"
```

---

### Task 4: web — api client/types 新增方法

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Test: `web/src/api/client.test.ts`（新建）

**Interfaces:**
- Consumes: Task 1–3 的端点；现成 `GET /api/v1/checkpoints`（返回 `[{ turn_id, created_at, file_count }]`，newest-first）和 `POST /api/v1/tools/undo-turn`（body `{ turn_ids: string[] }`，返回 `{ restored, skipped, failed, rewound_turns }`，见 `src/daemon/handlers.rs:1002-1056`）
- Produces: 类型 `WorktreeInfo` / `SkillInfoDto` / `CheckpointInfo` / `UndoTurnResult`；方法 `listWorktrees` / `createWorktree` / `deleteWorktree` / `listSkills` / `listCheckpoints` / `undoTurns`——Task 10、11 的面板只用这些签名

- [ ] **Step 1: 写失败测试**

新建 `web/src/api/client.test.ts`：

```ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonClient } from "./client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(payload), {
      status,
      headers: { "content-type": "application/json" },
    }),
  );
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("DaemonClient command-center endpoints", () => {
  afterEach(() => vi.unstubAllGlobals());
  const client = new DaemonClient();

  it("listWorktrees GETs /worktrees", async () => {
    const spy = mockFetch([{ path: "/repo", head: "abc", branch: "main", is_main: true }]);
    const wt = await client.listWorktrees();
    expect(spy).toHaveBeenCalledWith("/api/v1/worktrees");
    expect(wt[0].is_main).toBe(true);
  });

  it("createWorktree POSTs path+branch", async () => {
    const spy = mockFetch(null, 201);
    await client.createWorktree({ path: "/repo/.worktrees/f", branch: "f" });
    const [url, init] = spy.mock.calls[0];
    expect(url).toBe("/api/v1/worktrees");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body)).toEqual({ path: "/repo/.worktrees/f", branch: "f" });
  });

  it("deleteWorktree DELETEs with ?path= query", async () => {
    const spy = mockFetch(undefined, 204);
    await client.deleteWorktree("/repo/.worktrees/f");
    const [url, init] = spy.mock.calls[0];
    expect(url).toBe(`/api/v1/worktrees?path=${encodeURIComponent("/repo/.worktrees/f")}`);
    expect(init.method).toBe("DELETE");
  });

  it("listSkills GETs /skills", async () => {
    mockFetch([{ name: "alpha", description: "d", source_path: "/x/SKILL.md" }]);
    const skills = await client.listSkills();
    expect(skills[0].name).toBe("alpha");
  });

  it("listCheckpoints GETs /checkpoints", async () => {
    mockFetch([{ turn_id: "t1", created_at: 123, file_count: 2 }]);
    const cps = await client.listCheckpoints();
    expect(cps[0].turn_id).toBe("t1");
  });

  it("undoTurns POSTs turn_ids", async () => {
    const spy = mockFetch({ restored: 1, skipped: 0, failed: 0, rewound_turns: 1 });
    const res = await client.undoTurns(["t2", "t3"]);
    expect(JSON.parse(spy.mock.calls[0][1].body)).toEqual({ turn_ids: ["t2", "t3"] });
    expect(res.restored).toBe(1);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/api/client.test.ts`
Expected: FAIL（方法不存在，TypeScript 报错）

- [ ] **Step 3: 实现类型和方法**

在 `web/src/api/types.ts` 末尾追加：

```ts
// ── Command center (worktrees / skills / checkpoints) ────────────────────────

export interface WorktreeInfo {
  path: string;
  head: string;
  branch: string | null;
  is_main: boolean;
}

export interface SkillInfoDto {
  name: string;
  description: string;
  source_path: string;
}

export interface CheckpointInfo {
  turn_id: string;
  created_at: number;
  file_count: number;
}

export interface UndoTurnResult {
  restored: number;
  skipped: number;
  failed: number;
  rewound_turns: number;
}
```

在 `web/src/api/client.ts` 的 import 列表加入新类型，并在类末尾（`traceStream` 之后）追加：

```ts
  // ── Command center: worktrees / skills / checkpoints ───────────────────────

  async listWorktrees(): Promise<WorktreeInfo[]> {
    return jsonOrThrow(await fetch(`${this.base}/worktrees`));
  }

  async createWorktree(req: { path: string; branch: string }): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/worktrees`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async deleteWorktree(path: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/worktrees?path=${encodeURIComponent(path)}`, {
        method: "DELETE",
      }),
    );
  }

  async listSkills(): Promise<SkillInfoDto[]> {
    return jsonOrThrow(await fetch(`${this.base}/skills`));
  }

  async listCheckpoints(): Promise<CheckpointInfo[]> {
    return jsonOrThrow(await fetch(`${this.base}/checkpoints`));
  }

  async undoTurns(turnIds: string[]): Promise<UndoTurnResult> {
    return jsonOrThrow(
      await fetch(`${this.base}/tools/undo-turn`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ turn_ids: turnIds }),
      }),
    );
  }
```

注意 `createWorktree` 在 daemon 返回 201 且无 body——`jsonOrThrow` 只对 204 特判无 body，201 + 空 body 会 JSON parse 失败。实现时在 `jsonOrThrow` 里把 201 也纳入无 body 特判（改为 `if (res.status === 204 || res.status === 201) return undefined as T;`），或让测试 mock 返回带 JSON body 的 201。选前者，并在测试中覆盖。

- [ ] **Step 4: 跑测试 + lint + typecheck**

Run: `cd web && npx vitest run src/api/client.test.ts && npm run lint && npm run typecheck`
Expected: 6 个测试 PASS，lint/typecheck 绿

- [ ] **Step 5: Commit**

```bash
git add web/src/api/types.ts web/src/api/client.ts web/src/api/client.test.ts
git commit -m "feat(web): add worktree/skill/checkpoint client methods"
```

---

### Task 5: web — sessionStore 工厂（chatStore 改造）

**Files:**
- Create: `web/src/state/sessionStore.ts`
- Modify: `web/src/state/chatStore.ts`（改为 re-export 单例，过渡期保留）
- Test: `web/src/state/sessionStore.test.ts`（新建）

**Interfaces:**
- Consumes: 现 `chatStore.ts` 的全部类型与实现（`DisplayMessage` / `ConnectionStatus` / `TurnError` / actions）
- Produces: `createSessionStore(): SessionStore`；`type SessionStore = ReturnType<typeof createSessionStore>`；re-export `ChatState`（从 chatStore 移过来的接口改名）、`DisplayMessage`、`TurnError`。Task 6/7 依赖这些名字

- [ ] **Step 1: 写失败测试**

新建 `web/src/state/sessionStore.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { createSessionStore } from "./sessionStore";

describe("createSessionStore", () => {
  it("two instances are fully isolated", () => {
    const a = createSessionStore();
    const b = createSessionStore();
    a.getState().pushUserMessage("hello a");
    expect(a.getState().messages).toHaveLength(1);
    expect(b.getState().messages).toHaveLength(0);
  });

  it("abort registration is per-instance (stopRunning only aborts its own)", () => {
    const a = createSessionStore();
    const b = createSessionStore();
    const ctrlA = new AbortController();
    const ctrlB = new AbortController();
    a.getState().registerAbort(ctrlA);
    b.getState().registerAbort(ctrlB);
    a.getState().stopRunning();
    expect(ctrlA.signal.aborted).toBe(true);
    expect(ctrlB.signal.aborted).toBe(false);
  });

  it("streaming round: begin → append → finalize", () => {
    const s = createSessionStore();
    const id = s.getState().beginAssistantRound(0);
    s.getState().appendAssistant(id, { type: "contentDelta", text: "hi" });
    s.getState().finalizeAssistant(id);
    const msg = s.getState().messages.find((m) => m.id === id)!;
    expect(msg.content).toBe("hi");
    expect(msg.streaming).toBe(false);
  });
});
```

（`StreamEvent` 的具体变体名以 `web/src/api/sseParser.ts` 实际导出为准；若 `contentDelta` 签名不同，按实际类型调整测试。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/state/sessionStore.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现工厂**

把 `web/src/state/chatStore.ts` 的全部内容移到 `web/src/state/sessionStore.ts`，做三处改造：

1. `interface ChatState` 改为 `export interface SessionState`（所有引用同步改）。
2. 顶层改为工厂：`export function createSessionStore() { return create<SessionState>((set, get) => ({ ...原 store 体... })); }`，并加 `export type SessionStore = ReturnType<typeof createSessionStore>;`
3. **关键**：模块级的 `let currentAbort: AbortController | null = null;` 移进工厂函数体内（`create` 调用之前），使每个实例有独立的 abort 槽位：

```ts
export function createSessionStore() {
  // Per-instance holder for the running turn's AbortController (was module-
  // level in the singleton era — must be per-session now).
  let currentAbort: AbortController | null = null;
  return create<SessionState>((set, get) => ({
    // …原 store 体，registerAbort/stopRunning 读写这个闭包变量…
  }));
}
```

（`nextId`/`genId` 消息 id 计数器保留在模块级——跨会话唯一即可，无需隔离。）

把 `web/src/state/chatStore.ts` 改成过渡层（现有 import 全部不断）：

```ts
/**
 * Back-compat singleton. Pre-command-center components still import
 * `useChatStore`; Task 7 migrates them to `useSessionStore` (context) and
 * deletes this file.
 */
import { createSessionStore } from "./sessionStore";

export const useChatStore = createSessionStore();
export type { DisplayMessage, ConnectionStatus, TurnError, SessionState } from "./sessionStore";
```

注意原 chatStore.ts 里 `DisplayMessage`、`ConnectionStatus`、`TurnError` 是 export 的——确认 sessionStore.ts 里它们保持 export，chatStore.ts 的 re-export 列表与实际导出一致。

- [ ] **Step 4: 跑全部测试 + lint + typecheck**

Run: `cd web && npm test && npm run lint && npm run typecheck`
Expected: 全绿（含旧测试：singleton 行为不变）

- [ ] **Step 5: Commit**

```bash
git add web/src/state/sessionStore.ts web/src/state/chatStore.ts web/src/state/sessionStore.test.ts
git commit -m "refactor(web): turn chatStore into per-session store factory"
```

---

### Task 6: web — sessionManager（会话注册表）

**Files:**
- Create: `web/src/state/sessionManager.ts`
- Test: `web/src/state/sessionManager.test.ts`（新建）

**Interfaces:**
- Consumes: Task 5 的 `createSessionStore` / `SessionStore` / `ConnectionStatus`
- Produces:
  ```ts
  export type SessionStatus = "running" | "awaiting_approval" | "idle" | "error";
  export interface SessionEntry {
    id: string;             // 本地 id，也作为 /tools/execute 的 session_id
    daemonId: string | null; // POST /sessions 拿到的持久化 id（未保存为 null）
    name: string;
    store: SessionStore;
    status: SessionStatus;
    lastPreview: string;
    updatedAt: number;
  }
  export const useSessionManager: // zustand hook，见下
  ```
  manager state：`{ entries: Record<string, SessionEntry>, order: string[], activeId: string | null, connection: ConnectionStatus, modelName: string | null }`；actions：`createLocalSession(name?: string): string`、`removeSession(id)`、`setActive(id)`、`setStatus(id, status)`、`setPreview(id, text)`、`setDaemonId(id, daemonId)`、`setConnection(s)`、`setModelName(n)`。Task 7/8/9 只认这些名字。

- [ ] **Step 1: 写失败测试**

新建 `web/src/state/sessionManager.test.ts`：

```ts
import { beforeEach, describe, expect, it } from "vitest";
import { useSessionManager } from "./sessionManager";

describe("sessionManager", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "unknown",
      modelName: null,
    });
  });

  it("createLocalSession registers an idle entry and makes it active", () => {
    const id = useSessionManager.getState().createLocalSession("test");
    const s = useSessionManager.getState();
    expect(s.entries[id].status).toBe("idle");
    expect(s.entries[id].store).toBeDefined();
    expect(s.activeId).toBe(id);
    expect(s.order).toContain(id);
  });

  it("entries have independent stores", () => {
    const a = useSessionManager.getState().createLocalSession("a");
    const b = useSessionManager.getState().createLocalSession("b");
    const entries = useSessionManager.getState().entries; // 重新取最新 state
    entries[a].store.getState().pushUserMessage("hi");
    expect(entries[b].store.getState().messages).toHaveLength(0);
  });

  it("setStatus / setPreview update only the target entry", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a");
    const b = m.createLocalSession("b");
    m.setStatus(a, "running");
    m.setPreview(a, "working…");
    const s = useSessionManager.getState();
    expect(s.entries[a].status).toBe("running");
    expect(s.entries[a].lastPreview).toBe("working…");
    expect(s.entries[b].status).toBe("idle");
  });

  it("removeSession drops the entry and fixes activeId", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a");
    const b = m.createLocalSession("b");
    m.setActive(b);
    m.removeSession(b);
    const s = useSessionManager.getState();
    expect(s.entries[b]).toBeUndefined();
    expect(s.activeId).toBe(a);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/state/sessionManager.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

新建 `web/src/state/sessionManager.ts`：

```ts
/**
 * Session registry — the multi-session backbone of the command center.
 *
 * Owns one SessionStore per session (created via `createSessionStore`), the
 * active-session pointer, and per-session display metadata (status, preview)
 * that the LeftRail session list renders. Status is derived from loop events
 * by `agent/sessionRunner.ts` — this module only stores it.
 *
 * Also holds app-level connection/modelName (moved off the old singleton
 * chatStore: they're global facts, not per-session).
 */
import { create } from "zustand";
import { createSessionStore, type SessionStore } from "./sessionStore";
import type { ConnectionStatus } from "./sessionStore";

export type SessionStatus = "running" | "awaiting_approval" | "idle" | "error";

export interface SessionEntry {
  id: string;
  daemonId: string | null;
  name: string;
  store: SessionStore;
  status: SessionStatus;
  lastPreview: string;
  updatedAt: number;
}

interface SessionManagerState {
  entries: Record<string, SessionEntry>;
  /** Display order (creation order; LeftRail renders in this order). */
  order: string[];
  activeId: string | null;
  connection: ConnectionStatus;
  modelName: string | null;

  createLocalSession: (name?: string) => string;
  removeSession: (id: string) => void;
  setActive: (id: string) => void;
  setStatus: (id: string, status: SessionStatus) => void;
  setPreview: (id: string, text: string) => void;
  setDaemonId: (id: string, daemonId: string) => void;
  setConnection: (s: ConnectionStatus) => void;
  setModelName: (n: string | null) => void;
}

let counter = 1;

function patchEntry(
  entries: Record<string, SessionEntry>,
  id: string,
  patch: Partial<SessionEntry>,
): Record<string, SessionEntry> {
  const e = entries[id];
  if (!e) return entries;
  return { ...entries, [id]: { ...e, ...patch, updatedAt: Date.now() } };
}

export const useSessionManager = create<SessionManagerState>((set, get) => ({
  entries: {},
  order: [],
  activeId: null,
  connection: "unknown",
  modelName: null,

  createLocalSession: (name) => {
    const id = `web-${Date.now()}-${counter++}`;
    const entry: SessionEntry = {
      id,
      daemonId: null,
      name: name ?? `Session ${counter - 1}`,
      store: createSessionStore(),
      status: "idle",
      lastPreview: "",
      updatedAt: Date.now(),
    };
    set((s) => ({
      entries: { ...s.entries, [id]: entry },
      order: [...s.order, id],
      activeId: id,
    }));
    return id;
  },

  removeSession: (id) =>
    set((s) => {
      const entries = { ...s.entries };
      delete entries[id];
      const order = s.order.filter((x) => x !== id);
      const activeId = s.activeId === id ? (order[order.length - 1] ?? null) : s.activeId;
      return { entries, order, activeId };
    }),

  setActive: (id) => {
    if (get().entries[id]) set({ activeId: id });
  },

  setStatus: (id, status) => set((s) => ({ entries: patchEntry(s.entries, id, { status }) })),

  setPreview: (id, text) =>
    set((s) => ({ entries: patchEntry(s.entries, id, { lastPreview: text.slice(0, 120) }) })),

  setDaemonId: (id, daemonId) => set((s) => ({ entries: patchEntry(s.entries, id, { daemonId }) })),

  setConnection: (connection) => set({ connection }),
  setModelName: (modelName) => set({ modelName }),
}));

/** Selector helper: count of sessions waiting on a permission decision. */
export const selectPendingApprovalCount = (s: SessionManagerState): number =>
  Object.values(s.entries).filter((e) => e.status === "awaiting_approval").length;
```

- [ ] **Step 4: 跑测试 + lint**

Run: `cd web && npx vitest run src/state/ && npm run lint`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add web/src/state/sessionManager.ts web/src/state/sessionManager.test.ts
git commit -m "feat(web): add sessionManager registry for multi-session"
```

---

### Task 7: web — sessionContext + 组件订阅迁移（行为不变）

**Files:**
- Create: `web/src/state/sessionContext.tsx`
- Modify: `web/src/components/ChatView.tsx`、`Composer.tsx`、`PermissionModal.tsx`、`StatusBar.tsx`
- Modify: `web/src/App.tsx`（根部挂 Provider）
- Delete: `web/src/state/chatStore.ts`（迁移完成后）

**Interfaces:**
- Consumes: Task 5 的 `SessionStore` / `SessionState`；Task 6 的 `useSessionManager`
- Produces: `SessionStoreContext.Provider` 和 `useSessionStore<T>(selector: (s: SessionState) => T): T`——后续所有会话内组件（含 Task 8+ 新组件）只能用这个 hook 订阅会话状态

- [ ] **Step 1: 实现 context（此任务以"全部测试仍绿"为验收，先写基础设施）**

新建 `web/src/state/sessionContext.tsx`：

```tsx
/**
 * Per-session store injection. CenterPane wraps the active session's UI in a
 * Provider; components inside subscribe via `useSessionStore` instead of a
 * global singleton — which is what makes concurrent sessions possible.
 */
import { createContext, useContext } from "react";
import { useStore } from "zustand";
import type { SessionState, SessionStore } from "./sessionStore";

export const SessionStoreContext = createContext<SessionStore | null>(null);

export function useSessionStore<T>(selector: (s: SessionState) => T): T {
  const store = useContext(SessionStoreContext);
  if (!store) throw new Error("useSessionStore must be used inside SessionStoreContext.Provider");
  return useStore(store, selector);
}
```

- [ ] **Step 2: 迁移组件订阅**

对 `ChatView.tsx` / `Composer.tsx` / `PermissionModal.tsx`：把 `import { useChatStore } from "../state/chatStore"` 换成 `import { useSessionStore } from "../state/sessionContext"`，把所有 `useChatStore(sel)` 调用改成 `useSessionStore(sel)`。选择器引用的字段名不变。

`StatusBar.tsx` 特殊：`connection` / `modelName` 已移到 sessionManager——改从 `useSessionManager((s) => s.connection)` / `(s) => s.modelName` 读；`isRunning` 改为"活跃会话的 isRunning"（也用 `useSessionStore`，因为 StatusBar 在 Provider 内渲染，见 Step 3）。

`hooks/usePermissionTrace.ts` 也要迁移（它 import 了被删除的 chatStore）：此任务只做最小改动——把 `useChatStore.getState()` 换成"活跃会话的 store"：

```ts
const m = useSessionManager.getState();
const entry = m.activeId ? m.entries[m.activeId] : null;
entry?.store.getState().pushSubagentPermission(approval);
```

（Task 8 再把它升级为按 trace 事件的 `session_id` 精确路由。）

`PermissionModal.tsx` 顺手加一句文案：审批说明下加 `<p className="modal-global-note">Approvals are global — they apply to all sessions.</p>`（daemon 审批规则进程级共享，UI 如实告知）。

`Composer.test.tsx`：渲染处包一层 Provider：

```tsx
import { SessionStoreContext } from "../state/sessionContext";
import { createSessionStore } from "../state/sessionStore";

function renderComposer(onSend: (t: string) => void, store = createSessionStore()) {
  return render(
    <SessionStoreContext.Provider value={store}>
      <Composer onSend={onSend} />
    </SessionStoreContext.Provider>,
  );
}
```

测试里原来 `useChatStore.getState()` 的调用改为操作传入的 `store`（如 `store.getState().setRunning(true)`）。

- [ ] **Step 3: App.tsx 挂 Provider（仍单会话）**

`App.tsx` 目前有一个 `useState(() => new DaemonClient())` 的 client。此任务不改多会话逻辑，只做：

```tsx
import { useSessionManager } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";

// App 组件体内：
const activeId = useSessionManager((s) => s.activeId);
const activeStore = useSessionManager((s) => (s.activeId ? s.entries[s.activeId].store : null));
// 首次渲染创建一个本地会话：
if (!activeId) useSessionManager.getState().createLocalSession();

// JSX：用 Provider 包住 StatusBar + app-body + PermissionModal
<SessionStoreContext.Provider value={activeStore}>
  ...
</SessionStoreContext.Provider>
```

同时把 App 里的 `setConnection`/`setModelName` 订阅从旧 chatStore 换成 `useSessionManager`。`handleSend` 此任务保持原样（仍写活跃 store——context 后它读写的就是活跃会话）。

注意：`if (!activeId) createLocalSession()` 是渲染期副作用，react-hooks v7 会报 purity 错误。改成 `useEffect(() => { if (!useSessionManager.getState().activeId) useSessionManager.getState().createLocalSession(); }, [])`，并在 `activeStore` 为 null 时渲染空 div。

删除 `web/src/state/chatStore.ts`，全局搜索 `state/chatStore` 确认无残留 import。

- [ ] **Step 4: 全部验证**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add -A web/src
git commit -m "refactor(web): inject session store via context"
```

---

### Task 8: web — sessionRunner + App 多会话接线

**Files:**
- Create: `web/src/agent/sessionRunner.ts`
- Create: `web/src/components/SessionHeader.tsx`
- Modify: `web/src/App.tsx`、`web/src/hooks/usePermissionTrace.ts`、`web/src/components/StatusBar.tsx`
- Test: `web/src/agent/sessionRunner.test.ts`（新建）

**Interfaces:**
- Consumes: Task 5 `SessionStore`；Task 6 `useSessionManager` / `SessionStatus`；Task 7 context；`runAgentLoop`（`web/src/agent/loop.ts`，签名 `runAgentLoop({ client, messages, sessionId, signal, callbacks })`，callbacks 为 `onStreamEvent(round, ev)` / `onToolExecution(exec)` / `onPermissionRequired(info)`）
- Produces: `runSessionTurn(client: DaemonClient, sessionId: string, text: string): Promise<void>`——App 和后续会话 UI 的唯一发送入口；`SessionHeader` 组件

- [ ] **Step 1: 写失败测试**

新建 `web/src/agent/sessionRunner.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionManager } from "../state/sessionManager";

// Mock the loop: capture the callbacks the runner wires up.
vi.mock("./loop", () => ({
  runAgentLoop: vi.fn(),
}));
import { runAgentLoop } from "./loop";
import { runSessionTurn } from "./sessionRunner";
import { DaemonClient } from "../api/client";

const client = new DaemonClient();

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

describe("runSessionTurn", () => {
  beforeEach(() => {
    reset();
    vi.clearAllMocks();
  });

  it("marks the session running, streams into its own store, then goes idle", async () => {
    vi.mocked(runAgentLoop).mockImplementation(async ({ callbacks }) => {
      callbacks.onStreamEvent(0, { type: "contentDelta", text: "hi" });
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client, id, "hello");
    const e = useSessionManager.getState().entries[id];
    expect(e.status).toBe("idle");
    expect(e.store.getState().messages.some((m) => m.content === "hi")).toBe(true);
    expect(e.store.getState().isRunning).toBe(false);
  });

  it("permission prompt flips status to awaiting_approval during the loop", async () => {
    const id = useSessionManager.getState().createLocalSession("s1");
    vi.mocked(runAgentLoop).mockImplementation(async ({ callbacks }) => {
      callbacks.onPermissionRequired({
        tool_name: "exec_command",
        reason: "needs approval",
        session_rule: "bash:*",
      } as never);
      // 循环进行中：状态必须是 awaiting_approval
      expect(useSessionManager.getState().entries[id].status).toBe("awaiting_approval");
    });
    await runSessionTurn(client, id, "x");
    // 循环结束后归位 idle
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("loop error marks the session error and does not touch other sessions", async () => {
    vi.mocked(runAgentLoop).mockRejectedValue(new Error("stream error: boom"));
    const a = useSessionManager.getState().createLocalSession("a");
    const b = useSessionManager.getState().createLocalSession("b");
    await runSessionTurn(client, a, "x");
    const s = useSessionManager.getState();
    expect(s.entries[a].status).toBe("error");
    expect(s.entries[a].store.getState().lastError?.kind).toBe("upstream");
    expect(s.entries[b].status).toBe("idle");
    expect(s.entries[b].store.getState().lastError).toBeNull();
  });

  it("aborted turns are silent (no error state)", async () => {
    vi.mocked(runAgentLoop).mockRejectedValue(new Error("aborted"));
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client, id, "x");
    const e = useSessionManager.getState().entries[id];
    expect(e.status).toBe("idle");
    expect(e.store.getState().lastError).toBeNull();
  });
});
```

`onPermissionRequired` 的 info 类型以 `web/src/api/types.ts` 的 `PermissionRequiredInfo` 实际字段为准调整。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/agent/sessionRunner.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 sessionRunner**

新建 `web/src/agent/sessionRunner.ts`。内容 = 现 `App.tsx` 的 `handleSend` + `toWireMessages` + `lastAssistantId` 逻辑，参数化 sessionId，并在关键节点写 manager meta：

```ts
/**
 * Runs one agent turn for a session: pushes the user message, drives
 * runAgentLoop, mirrors progress into the sessionManager meta (status /
 * preview), and autosaves a snapshot to the daemon after the turn.
 */
import type { DaemonClient } from "../api/client";
import type { ChatMessage } from "../api/types";
import { runAgentLoop } from "./loop";
import { useSessionManager } from "../state/sessionManager";
import type { DisplayMessage } from "../state/sessionStore";

export async function runSessionTurn(
  client: DaemonClient,
  sessionId: string,
  text: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  const store = entry.store;

  store.getState().pushUserMessage(text);
  store.getState().setError(null);
  store.getState().setRunning(true);
  m.setStatus(sessionId, "running");

  const messages: ChatMessage[] = [
    ...toWireMessages(store.getState().messages),
    { role: "user", content: text },
  ];

  let currentAssistantId: string | null = null;
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  try {
    await runAgentLoop({
      client,
      messages,
      sessionId,
      signal: abort.signal,
      callbacks: {
        onStreamEvent: (round, ev) => {
          if (currentAssistantId === null) {
            currentAssistantId = store.getState().beginAssistantRound(round);
          }
          store.getState().appendAssistant(currentAssistantId, ev);
          if (ev.type === "contentDelta") {
            useSessionManager.getState().setPreview(sessionId, ev.text);
          }
        },
        onToolExecution: (exec) => {
          const id =
            currentAssistantId ??
            lastAssistantId(store.getState().messages) ??
            store.getState().beginAssistantRound(0);
          store.getState().attachToolExec(id, exec);
        },
        onPermissionRequired: (info) => {
          useSessionManager.getState().setStatus(sessionId, "awaiting_approval");
          return store.getState().requestPermission(info);
        },
      },
    });
    useSessionManager.getState().setStatus(sessionId, "idle");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg !== "aborted") {
      const isTransport =
        /fetch|network|failed to fetch|stream interrupted|aborted/i.test(msg) &&
        !msg.startsWith("stream error:");
      store.getState().setError({
        message: msg,
        kind: isTransport ? "transport" : "upstream",
        retry: isTransport ? () => runSessionTurn(client, sessionId, text) : undefined,
      });
      useSessionManager.getState().setStatus(sessionId, "error");
    } else {
      useSessionManager.getState().setStatus(sessionId, "idle");
    }
  } finally {
    store.getState().registerAbort(null);
    if (currentAssistantId) store.getState().finalizeAssistant(currentAssistantId);
    store.getState().setRunning(false);
  }

  await autosave(client, sessionId);
}

/** Persist a snapshot: create the daemon session on first save, then PUT. */
async function autosave(client: DaemonClient, sessionId: string): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  try {
    let daemonId = entry.daemonId;
    if (!daemonId) {
      const created = await client.createSession({ name: entry.name });
      daemonId = created.id;
      m.setDaemonId(sessionId, daemonId);
    }
    await client.saveSession(daemonId, {
      messages: toWireMessages(entry.store.getState().messages),
    });
  } catch {
    // Autosave is best-effort; the next turn retries. Don't flip the session
    // to error over a persistence hiccup.
  }
}

function toWireMessages(display: DisplayMessage[]): ChatMessage[] {
  // …从现 App.tsx 原样搬移（user → wire；assistant → content + tool_calls + tool results）…
}

function lastAssistantId(messages: DisplayMessage[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return messages[i].id;
  }
  return null;
}
```

`toWireMessages` 从 App.tsx **原样搬移**（计划中不重复其 25 行实现，执行者直接剪切）。`createSession` / `saveSession` 的请求字段名以 `web/src/api/types.ts` 的 `CreateSessionRequest` / `UpdateSessionRequest` 为准（含 `ui_messages` 的话先不传）。

注意 `onPermissionRequired`：现 loop.ts 的回调签名若是 `(info) => void` 而非返回 Promise，保持原签名、只加 setStatus 一行——以 `web/src/agent/loop.ts` 实际类型为准。

- [ ] **Step 4: App.tsx 改接 + SessionHeader + trace 路由 + beforeunload**

`App.tsx`：
- 删除 `handleSend` / `toWireMessages` / `lastAssistantId`（已搬去 sessionRunner）。
- Composer 的 `onSend` 改为 `(text) => activeId && runSessionTurn(client, activeId, text)`。
- useEffect 里 daemon 断连时（catch 分支）除 `setConnection("disconnected")` 外，把所有 `running` 会话 `setStatus(id, "error")`。

新建 `web/src/components/SessionHeader.tsx`：

```tsx
import { useSessionManager } from "../state/sessionManager";

const STATUS_LABEL: Record<string, string> = {
  running: "Running",
  awaiting_approval: "Needs approval",
  idle: "Idle",
  error: "Error",
};

/** Center-pane header: active session name + live status pill. */
export function SessionHeader() {
  const entry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : null));
  if (!entry) return null;
  return (
    <div className="session-header">
      <span className="session-header-name">{entry.name}</span>
      <span className={`session-header-status session-status-${entry.status}`}>
        {STATUS_LABEL[entry.status]}
      </span>
    </div>
  );
}
```

`usePermissionTrace.ts` 升级为按事件 `session_id` 路由（trace 事件带 session_id，见 daemon `handlers.rs` trace 端点）：

```ts
// permission_pending 事件到达时：
const m = useSessionManager.getState();
const target = m.entries[event.session_id] ?? (m.activeId ? m.entries[m.activeId] : null);
if (target) {
  target.store.getState().pushSubagentPermission(approval);
  m.setStatus(target.id, "awaiting_approval");
}
// permission_resolved：target.store.getState().clearSubagentPermission()，
// 若该会话无其他 pending 则 setStatus(id, "running")
```

（事件字段名以 `web/src/hooks/usePermissionTrace.ts` 里解析出的实际结构为准；`StructuredApproval` 无 session 维度，路由依赖事件外壳的 session_id。）

`StatusBar.tsx`：加全局待审批角标——`useSessionManager(selectPendingApprovalCount)`（Task 6 导出的 selector），> 0 时渲染 `<span className="topbar-approval-badge">{n}</span>`。

`App.tsx` 加 beforeunload：

```tsx
useEffect(() => {
  const handler = (e: BeforeUnloadEvent) => {
    const running = Object.values(useSessionManager.getState().entries).filter(
      (x) => x.status === "running" || x.status === "awaiting_approval",
    ).length;
    if (running > 0) e.preventDefault();
  };
  window.addEventListener("beforeunload", handler);
  return () => window.removeEventListener("beforeunload", handler);
}, []);
```

`styles.css` 追加（SessionHeader + 角标 + 状态 pill）：

```css
.session-header {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.5rem 1.5rem;
  border-bottom: 1px solid var(--border);
  font-size: 0.85rem;
}
.session-header-name {
  font-weight: 600;
}
.session-header-status {
  font-size: 0.68rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding: 0.1rem 0.45rem;
  border-radius: 10px;
  background: var(--bg-elev2);
  color: var(--fg-dim);
}
.session-status-running {
  color: var(--ok);
}
.session-status-awaiting_approval {
  color: var(--warn);
}
.session-status-error {
  color: var(--bad);
}
.topbar-approval-badge {
  background: var(--warn);
  color: var(--bg);
  border-radius: 10px;
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  font-weight: 600;
}
```

- [ ] **Step 5: 全部验证**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿

- [ ] **Step 6: Commit**

```bash
git add -A web/src
git commit -m "feat(web): run each session on its own loop via sessionRunner"
```

---

### Task 9: web — LeftRail + SessionList（删除旧 Sidebar）

**Files:**
- Create: `web/src/components/LeftRail.tsx`、`web/src/components/SessionList.tsx`
- Modify: `web/src/App.tsx`（Sidebar → LeftRail）、`web/src/styles.css`
- Delete: `web/src/components/Sidebar.tsx`
- Test: `web/src/components/SessionList.test.tsx`（新建）

**Interfaces:**
- Consumes: Task 6 `useSessionManager` / `SessionEntry`；Task 8 `runSessionTurn` 无关；`client.listSessions()` / `loadSession()` / `deleteSession()`（现有）；Task 5 `DisplayMessage`
- Produces: `<LeftRail client>`（App 唯一挂载点）；`<SessionList client>`；session 打开语义：`openDaemonSession(client, daemonId)`（在 SessionList 内实现，把 daemon 会话灌入新的本地 entry）

- [ ] **Step 1: 写失败测试**

新建 `web/src/components/SessionList.test.tsx`：

```tsx
import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SessionList } from "./SessionList";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";

const client = new DaemonClient();

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

describe("SessionList", () => {
  beforeEach(reset);

  it("renders open sessions with status dot and preview", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("fix bug");
    m.setPreview(a, "reading files…");
    render(<SessionList client={client} />);
    expect(screen.getByText("fix bug")).toBeInTheDocument();
    expect(screen.getByText("reading files…")).toBeInTheDocument();
  });

  it("clicking a session card makes it active", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("first");
    const b = m.createLocalSession("second");
    m.setActive(a);
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByText("second"));
    expect(useSessionManager.getState().activeId).toBe(b);
  });

  it("new-session button creates and activates a session", async () => {
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /new session/i }));
    expect(useSessionManager.getState().order).toHaveLength(1);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/components/SessionList.test.tsx`
Expected: FAIL（组件不存在）

- [ ] **Step 3: 实现 SessionList**

```tsx
import { useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { SessionInfo } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

/**
 * Open sessions (live, with status) on top; daemon-saved sessions below —
 * clicking one loads its history into a new local entry.
 */
export function SessionList({ client }: { client: DaemonClient }) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const [saved, setSaved] = useState<SessionInfo[]>([]);

  useEffect(() => {
    client
      .listSessions()
      .then(setSaved)
      .catch(() => setSaved([])); // daemon down → hide the section, not an error storm
  }, [client, order.length]); // refresh after autosave creates daemon sessions

  const openDaemonSession = async (info: SessionInfo) => {
    // Already open? Just focus it.
    const existing = Object.values(useSessionManager.getState().entries).find(
      (e) => e.daemonId === info.id,
    );
    if (existing) {
      useSessionManager.getState().setActive(existing.id);
      return;
    }
    const full = await client.loadSession(info.id);
    const m = useSessionManager.getState();
    const localId = m.createLocalSession(info.name ?? "Session");
    m.setDaemonId(localId, info.id);
    const store = useSessionManager.getState().entries[localId].store;
    for (const msg of full.messages ?? []) {
      store.getState().pushLoadedMessage({
        id: `loaded-${Math.random().toString(36).slice(2)}`,
        role: msg.role === "user" ? "user" : "assistant",
        content: typeof msg.content === "string" ? msg.content : "",
      });
    }
  };

  return (
    <div className="session-list-panel">
      <div className="session-list-head">
        <span className="rail-section-title">Sessions</span>
        <button
          type="button"
          className="btn-xs"
          onClick={() => useSessionManager.getState().createLocalSession()}
        >
          + New session
        </button>
      </div>
      <ul className="session-cards">
        {order.map((id) => {
          const e = entries[id];
          return (
            <li key={id}>
              <button
                type="button"
                className={`session-card ${id === activeId ? "active" : ""}`}
                onClick={() => useSessionManager.getState().setActive(id)}
              >
                <span className={`session-dot session-status-${e.status}`} />
                <span className="session-card-main">
                  <span className="session-card-name">{e.name}</span>
                  {e.lastPreview && <span className="session-card-preview">{e.lastPreview}</span>}
                </span>
                {e.status === "awaiting_approval" && (
                  <span className="session-card-badge">!</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      {saved.length > 0 && (
        <>
          <div className="rail-section-title">Saved</div>
          <ul className="session-cards">
            {saved.map((info) => (
              <li key={info.id}>
                <button
                  type="button"
                  className="session-card"
                  onClick={() => openDaemonSession(info)}
                >
                  <span className="session-card-main">
                    <span className="session-card-name">{info.name ?? info.id}</span>
                    <span className="session-card-preview">
                      {info.message_count} messages
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
```

（`full.messages` 的元素类型以 `SessionResponse` 实际类型为准做窄化；`pushLoadedMessage` 的签名见 sessionStore。）

- [ ] **Step 4: 实现 LeftRail，接入 App，删除旧 Sidebar**

```tsx
import type { DaemonClient } from "../api/client";
import { SessionList } from "./SessionList";
import { WorktreePanel } from "./WorktreePanel";
import { SkillPanel } from "./SkillPanel";
import { ModelPanel } from "./ModelPanel";

/**
 * Left column of the command center: sessions, worktrees, skills, and a
 * footer with global controls (model). Replaces the old tabbed Sidebar.
 */
export function LeftRail({ client }: { client: DaemonClient }) {
  return (
    <aside className="leftrail">
      <div className="leftrail-scroll">
        <SessionList client={client} />
        <WorktreePanel client={client} />
        <SkillPanel client={client} />
      </div>
      <div className="leftrail-footer">
        <ModelPanel client={client} />
      </div>
    </aside>
  );
}
```

WorktreePanel / SkillPanel 在 Task 10 才实现——本任务先建占位组件（`export function WorktreePanel(_: { client: DaemonClient }) { return null; }`），Task 10 填实。**注意**：若选择把 Task 10 提前合并也可，但 commit 保持分开。

`App.tsx`：`<Sidebar client={client} />` 换成 `<LeftRail client={client} />`，删除 Sidebar import 和文件。移动端 drawer CSS（`@media (max-width: 768px)` 的 `.sidebar` 规则）改挂到 `.leftrail` 类名上（简单起见：`.leftrail` 复用原 `.sidebar` 的全部响应式规则，把选择器改名即可）。

`styles.css` 追加：

```css
.leftrail {
  width: 280px;
  flex-shrink: 0;
  background: var(--bg-elev);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.leftrail-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.leftrail-footer {
  border-top: 1px solid var(--border);
  padding: 0.5rem;
}
.rail-section-title {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--fg-dim);
  padding: 0.3rem 0.2rem;
}
.session-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.session-cards {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.session-card {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0.45rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.35rem 0.5rem;
  color: var(--fg);
  cursor: pointer;
  text-align: left;
}
.session-card:hover {
  border-color: var(--border-strong);
}
.session-card.active {
  border-color: var(--fg-dim);
  background: var(--bg-elev2);
}
.session-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--fg-mute);
  flex-shrink: 0;
}
.session-card-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.session-card-name {
  font-size: 0.8rem;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.session-card-preview {
  font-size: 0.7rem;
  color: var(--fg-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.session-card-badge {
  background: var(--warn);
  color: var(--bg);
  border-radius: 50%;
  width: 14px;
  height: 14px;
  font-size: 0.65rem;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
```

- [ ] **Step 5: 全部验证**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿

- [ ] **Step 6: Commit**

```bash
git add -A web/src
git commit -m "feat(web): replace sidebar with LeftRail session list"
```

---

### Task 10: web — WorktreePanel + SkillPanel

**Files:**
- Modify: `web/src/components/WorktreePanel.tsx`、`web/src/components/SkillPanel.tsx`（Task 9 的占位填实）
- Modify: `web/src/styles.css`
- Test: `web/src/components/panels.test.tsx`（新建）

**Interfaces:**
- Consumes: Task 4 的 `listWorktrees` / `createWorktree` / `deleteWorktree` / `listSkills`；类型 `WorktreeInfo` / `SkillInfoDto`
- Produces: 无新导出（面板是自包含叶子组件）

- [ ] **Step 1: 写失败测试**

```tsx
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WorktreePanel } from "./WorktreePanel";
import { SkillPanel } from "./SkillPanel";
import { DaemonClient } from "../api/client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(payload), { status }),
  );
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("WorktreePanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists worktrees with branch names", async () => {
    mockFetch([
      { path: "/repo", head: "a", branch: "main", is_main: true },
      { path: "/repo/.worktrees/f", head: "b", branch: "feat", is_main: false },
    ]);
    render(<WorktreePanel client={new DaemonClient()} />);
    expect(await screen.findByText("main")).toBeInTheDocument();
    expect(screen.getByText("feat")).toBeInTheDocument();
  });

  it("delete button calls the API and refreshes (main worktree has no delete)", async () => {
    const spy = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (init?.method === "DELETE") return Promise.resolve(new Response(null, { status: 204 }));
      return Promise.resolve(
        new Response(
          JSON.stringify([{ path: "/repo", head: "a", branch: "main", is_main: true }]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);
    render(<WorktreePanel client={new DaemonClient()} />);
    await screen.findByText("main");
    expect(screen.queryByRole("button", { name: /remove/i })).not.toBeInTheDocument();
  });
});

describe("SkillPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists skills with descriptions", async () => {
    mockFetch([{ name: "brainstorming", description: "explore intent", source_path: "/x" }]);
    render(<SkillPanel client={new DaemonClient()} />);
    expect(await screen.findByText("brainstorming")).toBeInTheDocument();
    expect(screen.getByText("explore intent")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/components/panels.test.tsx`
Expected: FAIL（占位组件返回 null）

- [ ] **Step 3: 实现两个面板**

`WorktreePanel.tsx`：

```tsx
import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { WorktreeInfo } from "../api/types";

/** Git worktree list + create/remove. Data: GET/POST/DELETE /api/v1/worktrees. */
export function WorktreePanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<WorktreeInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listWorktrees()
      .then((w) => {
        setItems(w);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const create = async () => {
    const branch = window.prompt("New branch name:");
    if (!branch?.trim()) return;
    const path = `.worktrees/${branch.trim().replaceAll("/", "-")}`;
    try {
      await client.createWorktree({ path, branch: branch.trim() });
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (path: string) => {
    if (!window.confirm(`Remove worktree ${path}?`)) return;
    try {
      await client.deleteWorktree(path);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <section className="rail-panel">
      <div className="session-list-head">
        <span className="rail-section-title">Worktrees</span>
        <button type="button" className="btn-xs" onClick={create}>
          + New
        </button>
      </div>
      {error && <div className="panel-error">{error}</div>}
      {items && items.length === 0 && <div className="panel-empty">No worktrees</div>}
      <ul className="wt-list">
        {(items ?? []).map((w) => (
          <li key={w.path} className="wt-item">
            <span className="wt-branch">{w.branch ?? "(detached)"}</span>
            {w.is_main && <span className="wt-main-tag">main</span>}
            {!w.is_main && (
              <button
                type="button"
                className="btn-xs wt-remove"
                onClick={() => remove(w.path)}
              >
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
```

`SkillPanel.tsx`：

```tsx
import { useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { SkillInfoDto } from "../api/types";

/** Read-only skill list (GET /api/v1/skills). Enable/disable is out of scope:
 *  the knowledge layer has no enabled concept. */
export function SkillPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<SkillInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    client
      .listSkills()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  return (
    <section className="rail-panel">
      <span className="rail-section-title">Skills</span>
      {error && <div className="panel-error">{error}</div>}
      <ul className="skill-list">
        {(items ?? []).map((s) => (
          <li key={s.name} className="skill-item" title={s.source_path}>
            <span className="skill-name">{s.name}</span>
            <span className="skill-desc">{s.description}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
```

`styles.css` 追加：

```css
.wt-list,
.skill-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.wt-item,
.skill-item {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.3rem 0.45rem;
  font-size: 0.78rem;
}
.wt-branch,
.skill-name {
  font-family: var(--mono);
  font-weight: 500;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.skill-item {
  flex-direction: column;
  align-items: flex-start;
  gap: 0.1rem;
}
.skill-desc {
  color: var(--fg-dim);
  font-size: 0.72rem;
  line-height: 1.3;
}
.wt-main-tag {
  font-size: 0.62rem;
  text-transform: uppercase;
  background: var(--bg-elev2);
  color: var(--fg-dim);
  padding: 0.05rem 0.3rem;
  border-radius: 3px;
}
```

- [ ] **Step 4: 全部验证**

Run: `cd web && npm test && npm run lint && npm run typecheck`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add -A web/src
git commit -m "feat(web): add worktree and skill panels to LeftRail"
```

---

### Task 11: web — ContextPanel（右栏）+ SubagentPanel + CheckpointsPanel

**Files:**
- Create: `web/src/components/ContextPanel.tsx`、`SubagentPanel.tsx`、`CheckpointsPanel.tsx`
- Modify: `web/src/App.tsx`（挂右栏 + 开合按钮）、`web/src/styles.css`
- Modify: `web/src/components/TodosPanel.tsx`、`TasksPanel.tsx`（标题加"全局共享"标注）
- Test: `web/src/components/CheckpointsPanel.test.tsx`（新建）

**Interfaces:**
- Consumes: 现有 TodosPanel / TasksPanel / MemoryPanel；Task 4 的 `listCheckpoints` / `undoTurns`；trace SSE（`usePermissionTrace` 同款 `client.traceStream()`，事件结构见 `web/src/hooks/usePermissionTrace.ts` 解析逻辑）
- Produces: `<ContextPanel client>`（App 挂载点）

- [ ] **Step 1: 写失败测试（CheckpointsPanel——两个新面板里逻辑最多的）**

```tsx
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CheckpointsPanel } from "./CheckpointsPanel";
import { DaemonClient } from "../api/client";

describe("CheckpointsPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists checkpoints and undoes the selected turn", async () => {
    const spy = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (init?.method === "POST") {
        return Promise.resolve(
          new Response(
            JSON.stringify({ restored: 2, skipped: 0, failed: 0, rewound_turns: 1 }),
            { status: 200 },
          ),
        );
      }
      return Promise.resolve(
        new Response(
          JSON.stringify([
            { turn_id: "t2", created_at: 200, file_count: 1 },
            { turn_id: "t1", created_at: 100, file_count: 3 },
          ]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));

    render(<CheckpointsPanel client={new DaemonClient()} />);
    const btn = await screen.findByRole("button", { name: /undo t2/i });
    await userEvent.setup().click(btn);

    const post = spy.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(post![1].body)).toEqual({ turn_ids: ["t2"] });
    expect(await screen.findByText(/restored 2/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd web && npx vitest run src/components/CheckpointsPanel.test.tsx`
Expected: FAIL

- [ ] **Step 3: 实现**

`CheckpointsPanel.tsx`：

```tsx
import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { CheckpointInfo, UndoTurnResult } from "../api/types";

/** Per-turn file checkpoints with undo (GET /checkpoints, POST /tools/undo-turn).
 *  Global store — not per-session (daemon limitation, same as Todos). */
export function CheckpointsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<CheckpointInfo[]>([]);
  const [result, setResult] = useState<UndoTurnResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listCheckpoints()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const undo = async (turnId: string) => {
    if (!window.confirm(`Undo turn ${turnId}? Files return to their pre-turn state.`)) return;
    try {
      setResult(await client.undoTurns([turnId]));
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <section className="ctx-section">
      <span className="rail-section-title">Checkpoints (global)</span>
      {error && <div className="panel-error">{error}</div>}
      {result && (
        <div className="cp-result">
          restored {result.restored}, skipped {result.skipped}, failed {result.failed}
        </div>
      )}
      {items.length === 0 && <div className="panel-empty">No checkpoints</div>}
      <ul className="cp-list">
        {items.map((c) => (
          <li key={c.turn_id} className="cp-item">
            <span className="cp-turn">{c.turn_id}</span>
            <span className="cp-meta">{c.file_count} files</span>
            <button type="button" className="btn-xs" onClick={() => undo(c.turn_id)}>
              Undo {c.turn_id}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
```

`SubagentPanel.tsx`：订阅 `client.traceStream()`，把事件按"活跃会话 session_id"过滤后渲染为时间线（spawn / progress / complete / permission 事件各一行）。实现骨架：

```tsx
import { useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";

interface TraceLine {
  ts: number;
  text: string;
}

/** Live subagent activity for the active session, from the trace SSE stream.
 *  Line-oriented: each event becomes one row; capped at 100. */
export function SubagentPanel({ client }: { client: DaemonClient }) {
  const [lines, setLines] = useState<TraceLine[]>([]);
  const activeId = useSessionManager((s) => s.activeId);

  useEffect(() => {
    setLines([]);
    if (!activeId) return;
    const abort = new AbortController();
    (async () => {
      try {
        const { body } = await client.traceStream();
        const reader = body.getReader();
        const decoder = new TextDecoder();
        let buf = "";
        for (;;) {
          const { done, value } = await reader.read();
          if (done || abort.signal.aborted) break;
          buf += decoder.decode(value, { stream: true });
          const rows = buf.split("\n");
          buf = rows.pop() ?? "";
          for (const row of rows) {
            if (!row.trim()) continue;
            try {
              const ev = JSON.parse(row);
              if (ev.session_id !== activeId) continue;
              const text = `${ev.kind ?? "event"}: ${ev.summary ?? ev.message ?? ""}`;
              setLines((prev) => [...prev.slice(-99), { ts: Date.now(), text }]);
            } catch {
              // 半行/非 JSON 行忽略
            }
          }
        }
      } catch {
        // daemon down — the panel just stays empty; StatusBar shows connection
      }
    })();
    return () => abort.abort();
  }, [client, activeId]);

  return (
    <section className="ctx-section">
      <span className="rail-section-title">Subagents</span>
      {lines.length === 0 && <div className="panel-empty">No subagent activity</div>}
      <ul className="trace-list">
        {lines.map((l, i) => (
          <li key={i} className="trace-line">
            {l.text}
          </li>
        ))}
      </ul>
    </section>
  );
}
```

（trace 事件的实际字段——`kind` / `summary` / `session_id`——以 `web/src/hooks/usePermissionTrace.ts` 已解析的结构为准调整；保持防御式 `??` 访问。）

`ContextPanel.tsx`：

```tsx
import type { DaemonClient } from "../api/client";
import { TodosPanel } from "./TodosPanel";
import { TasksPanel } from "./TasksPanel";
import { MemoryPanel } from "./MemoryPanel";
import { SubagentPanel } from "./SubagentPanel";
import { CheckpointsPanel } from "./CheckpointsPanel";

/** Right column: active-session context + global panels (marked as such). */
export function ContextPanel({ client }: { client: DaemonClient }) {
  return (
    <aside className="contextpanel">
      <SubagentPanel client={client} />
      <TodosPanel client={client} />
      <TasksPanel client={client} />
      <CheckpointsPanel client={client} />
      <MemoryPanel client={client} />
    </aside>
  );
}
```

TodosPanel / TasksPanel 标题改为 `Todos (global)` / `Tasks (global)`（只改标题文本，其他不动）。

`App.tsx`：ContextPanel 挂到 `.app-main` 右侧（`.app-body` 内、CenterPane 之后），加一个右栏开合按钮（复用 sidebarStore 或本地 useState 均可——用本地 state，少动全局）。CSS：

```css
.contextpanel {
  width: 300px;
  flex-shrink: 0;
  background: var(--bg-elev);
  border-left: 1px solid var(--border);
  overflow-y: auto;
  padding: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.ctx-section {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.cp-list,
.trace-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.cp-item {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.3rem 0.45rem;
  font-size: 0.78rem;
}
.cp-turn {
  font-family: var(--mono);
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cp-meta,
.cp-result {
  color: var(--fg-dim);
  font-size: 0.72rem;
}
.trace-line {
  font-size: 0.76rem;
  color: var(--fg-dim);
  font-family: var(--mono);
  line-height: 1.4;
}
@media (max-width: 1024px) {
  .contextpanel {
    display: none;
  }
}
```

- [ ] **Step 4: 全部验证**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿

- [ ] **Step 5: Commit**

```bash
git add -A web/src
git commit -m "feat(web): add ContextPanel with subagent trace and checkpoints"
```

---

### Task 12: web — 视觉系统（accent 色 / 图标 / 字体 / toast / 消息流细节）

**Files:**
- Modify: `web/src/styles.css`、`web/src/main.tsx`、`web/src/App.tsx`
- Modify: `web/src/components/Composer.tsx`、`SessionList.tsx`、`LeftRail.tsx`、`ChatView.tsx`（图标替换）
- Modify: `web/package.json`

**Interfaces:**
- Consumes: 无（纯表现层）
- Produces: 无新导出

- [ ] **Step 1: 装依赖**

```bash
cd web && npm install lucide-react sonner @fontsource-variable/inter @fontsource/jetbrains-mono
```

- [ ] **Step 2: 字体 + accent 色**

`web/src/main.tsx` 顶部（`import "./styles.css"` 之前）：

```ts
import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono";
```

`styles.css` 的 `:root` 改 accent 组（其余变量不动）：

```css
  --accent: #6e8efb; /* brand accent: CTAs, active states, links, streaming cursor */
  --accent-fg: #0d0d0d;
  --accent-soft: rgba(110, 142, 251, 0.14);
```

全局检查 accent 的旧用法（`.btn-primary`、`.msg-markdown a`、`.sidebar-tab.active` 等）现在都自动继承新色——逐项目视确认不需要额外调整。`.diff-path` 的 `color: var(--accent)` 改 `var(--fg)`（文件路径不该是 accent 色）。

- [ ] **Step 3: 图标替换**

替换点（每个都是 1-2 行改动）：
- `Composer.tsx`：Send/Stop 按钮文字前加 `<Send size={14} />` / `<Square size={14} />`（lucide-react）
- `SessionList.tsx`："+ New session" → `<Plus size={12} />`；待审批 `!` 角标 → `<CircleAlert size={14} />`
- `LeftRail.tsx`：各 section title 前加图标（`<MessagesSquare />` / `<GitBranch />` / `<Puzzle />`）
- `SessionHeader.tsx`：状态 pill 前加对应图标
- `ChatView.tsx` / `PermissionModal.tsx` 里的 `▸`/`◂` 等字符图标 → lucide 对应物

- [ ] **Step 4: sonner toast**

`App.tsx` 根部挂 `<Toaster theme="dark" position="bottom-right" />`（`import { Toaster, toast } from "sonner"`）。接三处反馈：
- daemon 断连：`setConnection("disconnected")` 处 `toast.error("Daemon disconnected")`（用 ref 去重，避免 10s 轮询反复弹）
- 会话完成/出错：sessionRunner 的 finally 后不对——在 `setStatus(id, "error")` 处 `toast.error(\`\${entry.name}: turn failed\`)`
- worktree 创建/删除成功：WorktreePanel 的 create/remove 成功后 `toast.success(...)`

- [ ] **Step 5: 消息流细节**

`styles.css` 追加：

```css
/* Streaming cursor on the in-flight assistant message. */
.msg-assistant[data-streaming="true"] .msg-markdown > :last-child::after {
  content: "▍";
  color: var(--accent);
  animation: cursor-blink 0.9s step-end infinite;
}
@keyframes cursor-blink {
  50% {
    opacity: 0;
  }
}
/* User messages: subtle surface to distinguish roles at a glance. */
.msg-user .msg-content {
  background: var(--bg-elev);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.5rem 0.7rem;
}
/* Tool cards: status color bar on the left edge. */
.tool-card {
  border-left: 3px solid var(--border);
}
.tool-card[data-status="ok"] {
  border-left-color: var(--ok);
}
.tool-card[data-status="error"] {
  border-left-color: var(--bad);
}
.tool-card[data-status="denied"] {
  border-left-color: var(--warn);
}
```

`ChatView.tsx`：给 assistant 消息容器加 `data-streaming={m.streaming}`；`ToolCallCard.tsx` 给 `.tool-card` 加 `data-status`。

- [ ] **Step 6: 全部验证 + 目视检查**

Run: `cd web && npm test && npm run lint && npm run typecheck && npm run build`
Expected: 全绿。再 `npm run dev` 起页面目视一遍 accent 色、图标、toast（连接 daemon 实测）。

- [ ] **Step 7: Commit**

```bash
git add -A web
git commit -m "feat(web): visual system — accent color, icons, fonts, toasts"
```

---

## 验收清单（全部任务完成后）

- [ ] `cargo test --all` 通过，`cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] `cd web && npm run lint && npm test && npm run typecheck && npm run build` 全绿
- [ ] 手动冒烟：起 daemon + `npm run dev`，开两个会话同时跑，左栏状态点各自更新；切会话不中断后台流式；worktree 面板列出 `.worktrees/`；skill 面板列出 skills；审批弹窗只出现在对应会话
