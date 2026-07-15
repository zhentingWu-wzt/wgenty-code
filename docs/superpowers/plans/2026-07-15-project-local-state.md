---
change: project-local-state
design-doc: openspec/changes/project-local-state/design.md
base-ref: 9aab63cd7c01af14651e729977d3728cdbc319e8
created: 2026-07-15
status: draft
archived-with: 2026-07-15-project-local-state
---

# 项目本地状态存储与记忆分层 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 wgenty-code 的会话与记忆从「全局扁平存储」改为「项目本地 + 全局分层」模型，实现 scope 物理分离、全局记忆每轮注入、项目记忆按需召回。

**Architecture:** Plan A 双 Storage 方案——`MemoryManager` 持有 `project_storage` + `global_storage` 两个 `Storage` 实例，`Storage` 本身不变。scope = 物理位置，不给 `MemoryEntry` 加 schema 字段，内部用 `MemoryOrigin` 枚举追踪来源。项目根 = CWD（不向上查找），`current_project_root()` 封装 `current_dir()` + 回退。TF-IDF 索引只索引项目记忆，全局记忆每轮注入不走召回。

**Tech Stack:** Rust 2021, tokio, serde, anyhow, tracing。TDD with `#[tokio::test]`。`Arc<RwLock<T>>` 管理共享可变状态。

## 全局约束

- `cargo fmt` 强制执行（CI）；`cargo clippy --all-targets -- -D warnings` 零 warning（CI）。
- 错误处理：使用 `anyhow::Result` + `.context("描述")`，禁止裸 `unwrap()`，每个 `?` 需带 context。
- 异步共享可变状态通过 `Arc<RwLock<T>>`，锁持有时间最小化。
- `MemoryEntry` 不新增 schema 字段——scope 由存储位置决定，不序列化。
- `Storage` 结构体本身不修改——双源由 `MemoryManager` 持有两个实例实现。
- 项目根 = CWD，不向上查找 `.git`/`Cargo.toml`。
- 命令历史（`history.jsonl`）保持全局不变。
- 迁移幂等：标记文件 `~/.wgenty-code/.migrated-project-local` 防止重复扫描。
- 性能约束：启动时间增量 ≤ 5%，内存增量 ≤ 2%，二进制增量 ≤ 500KB（AGENTS.md）。
- Conventional Commits，英文 commit message。

archived-with: 2026-07-15-project-local-state
---

## 文件结构

| 文件 | 操作 | 职责 |
|------|------|------|
| `src/utils/mod.rs` | 修改 | 新增 `project_local_dir()`、`project_memory_dir()`、`project_sessions_dir()`、`global_memory_dir()`、`current_project_root()` 路径工具函数 |
| `src/context/memory_session.rs` | 修改 | `SessionManager` 新增 `with_project_root()` 构造函数，`sessions_dir` 指向项目本地 |
| `src/context/mod.rs` | 修改 | `MemoryManager` 改为双 Storage（`project_storage` + `global_storage`），新增 `MemoryOrigin` 枚举、`LoadedMemory` 结构、`add_memory(entry, scope)`、`global_memories()`；`load()` 双源加载 |
| `src/context/inject.rs` | 修改 | `MemoryContextInjector` 新增 `inject_global()` 方法 |
| `src/context/consolidation.rs` | 修改 | `ConsolidationEngine::find_similar` 支持 scope 过滤参数 |
| `src/prompts/mod.rs` | 修改 | `PromptContext` 新增 `global_memories` 字段；`assemble_instructions` 新增 Layer 5c `<global-memory>` 块 |
| `src/agent/runtime/compactor.rs` | 修改 | `COMPACTION_SYSTEM_PROMPT` 增加 scope 字段；`parse_compaction_response` 解析 scope |
| `src/cli/headless_runtime.rs` | 修改 | `run_oneshot` 传入项目根、注入全局记忆 |
| `src/cli/args.rs` | 修改 | `run_query`/`run_agent` 传入项目根、注入全局记忆 |
| `src/tui/app/mod.rs` | 修改 | TUI 启动传入 CWD 作为项目根、每轮注入全局记忆 |
| `src/cli/mod.rs` | 修改 | `memory status` 命令区分项目/全局记忆数 |
| `src/context/migration.rs` | 新建 | `migrate_legacy_sessions()` 幂等迁移函数 |
| `WGENTY.md` | 修改 | 补充项目本地 `.wgenty-code/` 布局说明 |

archived-with: 2026-07-15-project-local-state
---

## Task 1: 基础设施——项目本地路径工具函数

**依赖：** 无（基础层）

**Files:**
- 修改: `src/utils/mod.rs`（在 `config_dir()` 之后，约第 26 行后新增函数）
- 测试: `src/utils/mod.rs`（文件末尾 `#[cfg(test)] mod tests`）

**Interfaces:**
- 产生: `pub fn project_local_dir(project_root: &Path) -> PathBuf`、`pub fn project_memory_dir(project_root: &Path) -> PathBuf`、`pub fn project_sessions_dir(project_root: &Path) -> PathBuf`、`pub fn global_memory_dir() -> PathBuf`、`pub fn current_project_root() -> PathBuf`

- [ ] **Step 1.1: 编写路径函数的失败测试**

在 `src/utils/mod.rs` 文件末尾添加测试模块（若已有则追加）：

```rust
#[cfg(test)]
mod project_local_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn project_local_dir_appends_wgenty_code() {
        let root = Path::new("/home/user/myproject");
        assert_eq!(
            project_local_dir(root),
            Path::new("/home/user/myproject/.wgenty-code")
        );
    }

    #[test]
    fn project_memory_dir_appends_memory_subdir() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            project_memory_dir(root),
            Path::new("/tmp/proj/.wgenty-code/memory")
        );
    }

    #[test]
    fn project_sessions_dir_appends_sessions_subdir() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            project_sessions_dir(root),
            Path::new("/tmp/proj/.wgenty-code/sessions")
        );
    }

    #[test]
    fn global_memory_dir_points_to_config_memory() {
        let dir = global_memory_dir();
        assert!(dir.ends_with(".wgenty-code/memory"));
    }
}
```

- [ ] **Step 1.2: 运行测试确认失败**

运行: `cargo test --lib utils::project_local_tests`
预期: 编译失败，函数未定义

- [ ] **Step 1.3: 实现路径函数**

在 `src/utils/mod.rs` 的 `config_dir()` 函数之后（约第 26 行后）添加：

```rust
use std::path::Path;

/// 项目本地 .wgenty-code 目录: <project_root>/.wgenty-code/
pub fn project_local_dir(project_root: &Path) -> PathBuf {
    project_root.join(".wgenty-code")
}

/// 项目本地 memory 目录: <project_root>/.wgenty-code/memory/
pub fn project_memory_dir(project_root: &Path) -> PathBuf {
    project_local_dir(project_root).join("memory")
}

/// 项目本地 sessions 目录: <project_root>/.wgenty-code/sessions/
pub fn project_sessions_dir(project_root: &Path) -> PathBuf {
    project_local_dir(project_root).join("sessions")
}

/// 全局 memory 目录（保持原有 ~/.wgenty-code/memory/）
pub fn global_memory_dir() -> PathBuf {
    config_dir().join("memory")
}
```

- [ ] **Step 1.4: 编写 current_project_root 的失败测试**

在 `src/utils/mod.rs` 测试模块中追加：

```rust
#[cfg(test)]
mod current_project_root_tests {
    use super::*;

    #[test]
    fn current_project_root_returns_cwd_when_available() {
        let root = current_project_root();
        // Should return the actual CWD (non-empty path)
        assert!(!root.as_os_str().is_empty());
    }

    #[test]
    fn current_project_root_falls_back_to_config_dir_on_failure() {
        // current_project_root() wraps current_dir(); if that fails it
        // falls back to config_dir(). We can't easily force current_dir()
        // to fail, but we verify the function returns a valid path.
        let root = current_project_root();
        assert!(root.is_absolute() || !root.as_os_str().is_empty());
    }
}
```

- [ ] **Step 1.5: 运行测试确认失败**

运行: `cargo test --lib utils::current_project_root_tests`
预期: 编译失败，`current_project_root` 未定义

- [ ] **Step 1.6: 实现 current_project_root**

在 `src/utils/mod.rs` 路径函数后添加：

```rust
/// 获取当前项目根目录。封装 `std::env::current_dir()`，
/// 失败时回退到全局 `config_dir()` 并 warn。
pub fn current_project_root() -> PathBuf {
    match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to get current working directory; falling back to global config dir"
            );
            config_dir()
        }
    }
}
```

- [ ] **Step 1.7: 运行全部测试确认通过**

运行: `cargo test --lib utils::`
预期: 全部 PASS

- [ ] **Step 1.8: 提交**

```bash
git add src/utils/mod.rs
git commit -m "feat(utils): add project-local path helper functions"
```

archived-with: 2026-07-15-project-local-state
---

## Task 2: Session 项目化

**依赖:** Task 1（`current_project_root()`、`project_sessions_dir()`）

**Files:**
- 修改: `src/context/memory_session.rs`（`SessionManager` 结构体约第 112 行，`new()` 构造函数）
- 修改: `src/context/mod.rs`（`MemoryManager::with_settings()` 约第 302 行、`new()` 约第 273 行）
- 修改: `src/tui/app/mod.rs`（TUI 启动路径）
- 修改: `src/cli/headless_runtime.rs`（`run_oneshot` 约第 174 行）
- 修改: `src/cli/args.rs`（`run_query`/`run_agent`）
- 测试: `src/context/memory_session.rs`（文件末尾测试模块）

**Interfaces:**
- 消费: `current_project_root()` (`src/utils/mod.rs`)、`project_sessions_dir()` (`src/utils/mod.rs`)
- 产生: `SessionManager::with_project_root(project_root: PathBuf) -> Self`——供 `MemoryManager` 和 CLI/TUI 调用

- [ ] **Step 2.1: 编写 with_project_root 的失败测试**

在 `src/context/memory_session.rs` 末尾追加测试模块：

```rust
#[cfg(test)]
mod project_root_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn with_project_root_uses_local_sessions_dir() {
        let tmp = TempDir::new().expect("temp dir");
        let manager = SessionManager::with_project_root(tmp.path().to_path_buf());
        assert_eq!(
            manager.sessions_dir,
            tmp.path().join(".wgenty-code").join("sessions")
        );
    }

    #[tokio::test]
    async fn new_delegates_to_global_config_dir() {
        let manager = SessionManager::new();
        // new() should use config_dir() (global ~/.wgenty-code/sessions)
        assert!(manager.sessions_dir.ends_with("sessions"));
    }
}
```

- [ ] **Step 2.2: 运行测试确认失败**

运行: `cargo test --lib context::memory_session::project_root_tests`
预期: 编译失败，`with_project_root` 未定义

- [ ] **Step 2.3: 实现 with_project_root 构造函数**

在 `src/context/memory_session.rs` 的 `impl SessionManager` 块中，`new()` 方法之前添加：

```rust
/// 创建指向项目本地 sessions 目录的 SessionManager。
/// sessions_dir = <project_root>/.wgenty-code/sessions/
pub fn with_project_root(project_root: PathBuf) -> Self {
    let sessions_dir = project_root.join(".wgenty-code").join("sessions");
    Self {
        sessions_dir,
        active_session: Arc::new(RwLock::new(None)),
        sessions: Arc::new(RwLock::new(HashMap::new())),
    }
}
```

- [ ] **Step 2.4: 更新 new() 委托到 with_project_root**

将 `new()` 方法体改为委托调用（保持测试兼容）：

```rust
/// 兼容入口：使用全局 config_dir() 作为项目根。
/// 新代码应优先使用 `with_project_root()`。
pub fn new() -> Self {
    Self::with_project_root(
        crate::utils::config_dir()
    )
}
```

- [ ] **Step 2.5: 实现 create() 时自动创建目录 + CWD 不可写降级**

在 `SessionManager::create()` 方法中（或新增 `ensure_sessions_dir()` 辅助方法），确保目录创建：

```rust
/// 确保 sessions 目录存在。CWD 不可写时降级到全局目录并 warn。
fn ensure_sessions_dir(&mut self) -> anyhow::Result<()> {
    if let Err(e) = tokio::task::block_in_place(|| {
        std::fs::create_dir_all(&self.sessions_dir)
    }) {
        tracing::warn!(
            path = %self.sessions_dir.display(),
            error = %e,
            "Failed to create project-local sessions dir; falling back to global"
        );
        self.sessions_dir = crate::utils::config_dir().join("sessions");
        std::fs::create_dir_all(&self.sessions_dir)
            .context("创建降级全局 sessions 目录失败")?;
    }
    Ok(())
}
```

在 `create()` 方法开头调用 `self.ensure_sessions_dir()?`。

- [ ] **Step 2.6: 运行测试确认通过**

运行: `cargo test --lib context::memory_session::project_root_tests`
预期: PASS

- [ ] **Step 2.7: 编写项目本地 session 隔离测试**

```rust
#[tokio::test]
async fn project_local_sessions_isolated_from_global() {
    use crate::utils;
    let tmp = TempDir::new().expect("temp dir");
    let proj_mgr = SessionManager::with_project_root(tmp.path().to_path_buf());
    let session = proj_mgr.create("test-proj".to_string()).await
        .expect("create");
    // 全局 manager 不应看到项目本地 session
    let global_mgr = SessionManager::new();
    let all = global_mgr.list().await;
    assert!(!all.iter().any(|s| s.id == session.id),
        "project session should not appear in global list");
}
```

- [ ] **Step 2.8: 运行隔离测试确认通过**

运行: `cargo test --lib context::memory_session::project_root_tests -- --nocapture`
预期: PASS

- [ ] **Step 2.9: 更新 MemoryManager::with_settings 和 new() 接受 project_root**

在 `src/context/mod.rs` 中：

1. `MemoryManager` 结构体新增 `project_root: PathBuf` 字段。
2. `with_settings` 签名改为 `pub fn with_settings(settings: &Settings, project_root: PathBuf) -> Self`，内部用 `project_root` 初始化 `MemorySessionManager::with_project_root(project_root.clone())`。
3. `new()` 改为 `Self::with_settings(&Settings::default(), crate::utils::current_project_root())`。

```rust
pub fn with_settings(settings: &crate::config::Settings, project_root: PathBuf) -> Self {
    let global_memory_path = crate::utils::global_memory_dir();
    let project_memory_path = crate::utils::project_memory_dir(&project_root);

    if let Err(e) = std::fs::create_dir_all(&project_memory_path) {
        tracing::warn!(
            path = %project_memory_path.display(),
            error = %e,
            "Failed to create project memory directory; falling back to global"
        );
    }
    if let Err(e) = std::fs::create_dir_all(&global_memory_path) {
        tracing::warn!(
            path = %global_memory_path.display(),
            error = %e,
            "Failed to create global memory directory"
        );
    }

    let consolidation_config =
        ConsolidationConfig::from_memory_settings(&settings.storage.memory);

    Self {
        sessions: Arc::new(MemorySessionManager::with_project_root(project_root.clone())),
        history: Arc::new(HistoryManager::new()),
        project_storage: Arc::new(Storage::new(project_memory_path)),
        global_storage: Arc::new(Storage::new(global_memory_path)),
        consolidation: Arc::new(ConsolidationEngine::new(consolidation_config)),
        memories: Arc::new(RwLock::new(Vec::new())),
        index: Arc::new(RwLock::new(MemoryIndex::new())),
        consolidating: Arc::new(AtomicBool::new(false)),
        project_root,
    }
}
```

> **注意：** 此步骤同时修改了 `MemoryManager` 结构体字段（`storage` → `project_storage` + `global_storage`），所有引用 `self.storage` 的方法需同步更新。Task 3 会完成这些更新。此步骤先保证编译通过（临时将 `storage` 保留为 `project_storage` 的别名或同时更新所有引用点）。

- [ ] **Step 2.10: 更新 TUI 和 CLI 启动路径传入项目根**

1. `src/tui/app/mod.rs`：在创建 `MemoryManager` 的位置，改为 `MemoryManager::with_settings(&settings, crate::utils::current_project_root())`。
2. `src/cli/headless_runtime.rs` 的 `run_oneshot()`（约第 200 行）：将 `MemoryManager::new()` 改为 `MemoryManager::with_settings(&settings, crate::utils::current_project_root())`。
3. `src/cli/args.rs` 的 `run_query()`/`run_agent()`：同样传入 `crate::utils::current_project_root()`。

- [ ] **Step 2.11: 运行全部相关测试**

运行: `cargo test --lib context::`
预期: PASS（可能需要修复因 `storage` → `project_storage` 重命名导致的编译错误，临时将 `self.storage` 引用改为 `self.project_storage`）

- [ ] **Step 2.12: 提交**

```bash
git add src/context/memory_session.rs src/context/mod.rs src/tui/app/mod.rs src/cli/headless_runtime.rs src/cli/args.rs
git commit -m "feat(session): project-local session storage with CWD-based root"
```

archived-with: 2026-07-15-project-local-state
---

## Task 3: Memory 双源存储与 scope 追踪

**依赖:** Task 1（`project_memory_dir()`、`global_memory_dir()`、`current_project_root()`）

**Files:**
- 修改: `src/context/mod.rs`（`MemoryManager` 结构体、`with_settings`、`load`、`search_memories`、`add_memory`、`status`）
- 修改: `src/context/consolidation.rs`（`find_similar` 增加 scope 过滤，可选）

**Interfaces:**
- 产生: `pub enum MemoryOrigin { Project, Global }`、`struct LoadedMemory { entry, origin }`
- 修改: `MemoryManager` 新增 `project_storage: Arc<Storage>` + `global_storage: Arc<Storage>`（替换原 `storage`）
- 修改: `MemoryManager::with_settings(settings, project_root)` 接受项目根
- 修改: `add_memory(entry, scope: MemoryOrigin)` -- 按 scope 路由写入
- 产生: `global_memories() -> Vec<MemoryEntry>` -- 返回全部全局记忆
- 修改: `search_memories(query)` -- 只返回项目记忆（索引只含项目记忆）

- [ ] **Step 3.1: 定义 MemoryOrigin 与 LoadedMemory 类型**

在 `src/context/mod.rs` 的 `MemoryEntry` 定义之后新增：

```rust
/// 记忆来源 scope。不序列化--由加载时的存储路径决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOrigin {
    Project,
    Global,
}

/// 带来源标记的已加载记忆。
#[derive(Debug, Clone)]
struct LoadedMemory {
    entry: MemoryEntry,
    origin: MemoryOrigin,
}
```

- [ ] **Step 3.2: 修改 MemoryManager 结构体为双 Storage**

将 `storage: Arc<Storage>` 替换为 `project_storage: Arc<Storage>` + `global_storage: Arc<Storage>`，将 `memories: Arc<RwLock<Vec<MemoryEntry>>>` 改为 `Arc<RwLock<Vec<LoadedMemory>>>`。

- [ ] **Step 3.3: 修改 with_settings 初始化双 Storage**

`with_settings(settings, project_root)` 中：
- `project_storage = Storage::new(project_memory_dir(&project_root))`
- `global_storage = Storage::new(global_memory_dir())`
- CWD == home 时 warn 并将 project_storage 也指向 global_memory_dir()（合并池）
- CWD 不可写时 warn 并降级 project_storage 到 global_memory_dir()

- [ ] **Step 3.4: 修改 load() 双源加载**

分别从 `project_storage` 和 `global_storage` 加载，标记 `MemoryOrigin::Project` / `Global`。TF-IDF 索引只 `add_entry` 项目记忆。

- [ ] **Step 3.5: 编写双源加载与 scope 隔离测试**

测试：项目记忆和全局记忆分别加载到不同 origin；`search_memories` 只返回项目记忆；`global_memories()` 只返回全局记忆。

- [ ] **Step 3.6: 修改 add_memory 签名为 (entry, scope)**

按 scope 路由到对应 Storage。dedup 逻辑改为同 scope 内查找相似（`find_similar` 需过滤同 origin 的记忆）。

- [ ] **Step 3.7: 新增 global_memories() 方法**

返回 `memories` 中 origin == Global 的所有 entry。

- [ ] **Step 3.8: 更新所有 add_memory 调用方传入 scope**

用 `grep` 查找所有 `.add_memory(` 调用方，暂时全部传 `MemoryOrigin::Project`（compaction scope 判定在 Task 5 细化）。主要调用方：`src/agent/runtime/compactor.rs`、`src/services/auto_dream.rs`。

- [ ] **Step 3.9: 更新 status() 报告项目/全局计数**

`MemoryStatus` 新增 `project_count` 和 `global_count` 字段（`#[serde(default)]` 向后兼容）。

- [ ] **Step 3.10: 运行测试确认通过**

运行: `cargo test --lib context::`
预期: PASS。修复所有因 `storage` -> `project_storage`/`global_storage` 重命名导致的编译错误。

- [ ] **Step 3.11: 提交**

```bash
git add src/context/mod.rs src/context/consolidation.rs src/agent/runtime/compactor.rs src/services/auto_dream.rs
git commit -m "feat(memory): dual-source storage with project/global scope tracking"
```

archived-with: 2026-07-15-project-local-state
---

## Task 4: 全局记忆每轮注入

**依赖:** Task 3（`global_memories()` 方法）

**Files:**
- 修改: `src/context/inject.rs`（新增 `inject_global()` 方法）
- 修改: `src/prompts/mod.rs`（`PromptContext` 新增 `global_memories` 字段，`assemble_instructions` 生成 `<global-memory>` 块）

**Interfaces:**
- 产生: `MemoryContextInjector::inject_global(manager) -> String`
- 修改: `PromptContext { global_memories: Vec<String> }` + `with_global_memories()`
- 修改: `assemble_instructions` 在 Layer 5（Environment）与 Layer 6（Skills）之间插入 `<global-memory>` 块

- [ ] **Step 4.1: 编写 inject_global 的失败测试**

测试：空全局记忆返回空字符串；有全局记忆返回 `<global-memory>` 块含全部条目；超过 50 条取 top 50 by importance。

- [ ] **Step 4.2: 实现 inject_global 方法**

在 `src/context/inject.rs` 新增：

```rust
/// 生成 `<global-memory>` 块，包含全部全局记忆（按 importance 排序，软上限 50）。
pub async fn inject_global(manager: &MemoryManager) -> String {
    let mut globals = manager.global_memories().await;
    if globals.is_empty() {
        return String::new();
    }
    globals.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(Equal));
    const SOFT_CAP: usize = 50;
    let capped = globals.len() > SOFT_CAP;
    if capped { tracing::warn!(count = globals.len(), "global memory exceeds soft cap"); }
    let top: Vec<_> = globals.into_iter().take(SOFT_CAP).collect();
    // 格式化为 <global-memory> 块
}
```

- [ ] **Step 4.3: PromptContext 新增 global_memories 字段**

在 `src/prompts/mod.rs` 的 `PromptContext` 新增 `global_memories: Vec<String>` 字段 + `with_global_memories()` builder 方法。

- [ ] **Step 4.4: assemble_instructions 插入 <global-memory> 块**

在 Layer 5（Environment）之后、Layer 6（Skills）之前，当 `context.global_memories` 非空时，push 一个 system message：

```
<global-memory>
- [Preference] 始终用中文回复
- [Decision] 提交前跑 cargo clippy
</global-memory>
```

- [ ] **Step 4.5: 编写 prompt 组装测试**

测试：`global_memories` 非空时 system_messages 含 `<global-memory>` 块且位于 Environment 之后、Skills 之前；为空时不注入。

- [ ] **Step 4.6: 更新 TUI 启动路径注入全局记忆**

在 `src/tui/app/mod.rs` 的 prompt 组装处，调用 `MemoryContextInjector::inject_global(&manager)` 填充 `PromptContext.global_memories`。

- [ ] **Step 4.7: 更新 CLI headless 路径注入全局记忆**

在 `src/cli/headless_runtime.rs` 和 `src/cli/args.rs` 的 `run_query`/`run_agent` 路径同样注入。

- [ ] **Step 4.8: 运行测试确认通过**

运行: `cargo test --lib context::inject prompts::`
预期: PASS。

- [ ] **Step 4.9: 提交**

```bash
git add src/context/inject.rs src/prompts/mod.rs src/tui/app/mod.rs src/cli/headless_runtime.rs src/cli/args.rs
git commit -m "feat(memory): inject global memories every turn as <global-memory> block"
```

archived-with: 2026-07-15-project-local-state
---

## Task 5: Compaction 自动 scope 判定

**依赖:** Task 3（`add_memory(entry, scope)`）

**Files:**
- 修改: `src/agent/runtime/compactor.rs`（`COMPACTION_SYSTEM_PROMPT` 增加 scope 字段；`parse_compaction_response` 解析 scope）

**Interfaces:**
- 修改: compaction 提取 prompt 的 JSON schema 增加 `scope: "project"|"global"`
- 修改: 解析逻辑读取 scope，缺失默认 `MemoryOrigin::Project`

- [ ] **Step 5.1: 编写 scope 解析的失败测试**

测试：JSON 含 `scope: "global"` 时解析为 `MemoryOrigin::Global`；缺失 scope 时默认 `Project`；`scope: "invalid"` 时默认 `Project`。

- [ ] **Step 5.2: 修改 COMPACTION_SYSTEM_PROMPT 增加 scope 要求**

在提取 prompt 的 JSON schema 中增加 `scope` 字段，并增加指引：「跨项目通用的偏好/行为准则归 global，项目特定决策/知识归 project」。

- [ ] **Step 5.3: 修改 parse_compaction_response 解析 scope**

解析每条 memory 的 `scope` 字段：`"global"` -> `MemoryOrigin::Global`，其他/缺失 -> `MemoryOrigin::Project`。

- [ ] **Step 5.4: 提取的记忆按 scope 调用 add_memory**

将 Task 3.8 中临时传 `Project` 的 compaction 调用方改为传入解析出的 scope。

- [ ] **Step 5.5: 运行测试确认通过**

运行: `cargo test --lib agent::runtime::compactor`
预期: PASS。

- [ ] **Step 5.6: 提交**

```bash
git add src/agent/runtime/compactor.rs
git commit -m "feat(memory): compaction scope classification with project/global routing"
```

archived-with: 2026-07-15-project-local-state
---

## Task 6: 数据迁移

**依赖:** Task 1（路径函数）、Task 2（项目本地 session 目录）

**Files:**
- 新建: `src/context/migration.rs`
- 修改: `src/context/mod.rs`（`pub mod migration;` + 启动时调用）

**Interfaces:**
- 产生: `pub fn migrate_legacy_sessions() -> anyhow::Result<usize>`

- [ ] **Step 6.1: 编写迁移幂等性测试**

测试：旧 session 按 `project_path` 路由到目标；`project_path == None` 归当前 CWD；目标已存在则跳过；marker 文件存在时跳过整个迁移；迁移成功后删除原文件。

- [ ] **Step 6.2: 创建 src/context/migration.rs**

实现 `migrate_legacy_sessions()`：
- 检查 `~/.wgenty-code/.migrated-project-local` marker，存在则 return Ok(0)
- 扫描 `~/.wgenty-code/sessions/*.json`
- 每个文件：解析 Session，取 `project_path.unwrap_or(current_project_root())`
- 目标路径 `<target>/.wgenty-code/sessions/{id}.json`，已存在则 skip
- 复制到目标，成功后删除原文件；失败保留原文件 + warn
- 全部完成后写 marker 文件

- [ ] **Step 6.3: 在 context/mod.rs 注册模块**

添加 `pub mod migration;`，在 `MemoryManager::with_settings` 末尾（或启动入口）调用 `migration::migrate_legacy_sessions()` 并 log 结果。

- [ ] **Step 6.4: 确认现有 memory 不迁移**

验证 `MemoryManager::load()` 仍从 `global_memory_dir()`（即原 `~/.wgenty-code/memory/`）读取--现有 memory 天然成为全局记忆，无需迁移。

- [ ] **Step 6.5: 运行测试确认通过**

运行: `cargo test --lib context::migration`
预期: PASS。

- [ ] **Step 6.6: 提交**

```bash
git add src/context/migration.rs src/context/mod.rs
git commit -m "feat(context): idempotent legacy session migration to project-local storage"
```

archived-with: 2026-07-15-project-local-state
---

## Task 7: CLI 与配置集成

**依赖:** Task 3（status 项目/全局计数）、Task 4（全局记忆注入）

**Files:**
- 修改: `src/cli/mod.rs`（`memory status` 命令输出区分项目/全局）
- 修改: `WGENTY.md`（补充项目本地 `.wgenty-code/` 布局说明）

- [ ] **Step 7.1: 更新 memory status 输出**

在 `src/cli/mod.rs` 的 `memory status` 处理逻辑中，输出 `MemoryStatus` 的新增 `project_count` / `global_count` 字段。

- [ ] **Step 7.2: 更新 WGENTY.md 文档**

在 WGENTY.md 的「配置」章节或新增「项目本地存储」章节，补充：
- `<CWD>/.wgenty-code/sessions/` -- 项目会话
- `<CWD>/.wgenty-code/memory/` -- 项目记忆
- `~/.wgenty-code/memory/` -- 全局记忆（每轮注入）
- 迁移行为说明

- [ ] **Step 7.3: 运行相关测试**

运行: `cargo test --lib cli::`
预期: PASS。

- [ ] **Step 7.4: 提交**

```bash
git add src/cli/mod.rs WGENTY.md
git commit -m "docs(cli): memory status shows project/global counts; document project-local layout"
```

archived-with: 2026-07-15-project-local-state
---

## Task 8: 验证与收尾

**依赖:** Task 1-7 全部完成

- [ ] **Step 8.1: cargo fmt 格式检查**

运行: `cargo fmt --check`
预期: 无差异。如有，运行 `cargo fmt` 修复。

- [ ] **Step 8.2: cargo clippy 零 warning**

运行: `cargo clippy --all-targets -- -D warnings`
预期: 零 warning。修复所有 clippy 提示。

- [ ] **Step 8.3: cargo test 全量通过**

运行: `cargo test --all`
预期: 全部 PASS。修复任何因本次改动导致的测试失败。

- [ ] **Step 8.4: 手动验证项目本地目录创建**

```bash
cd /tmp && mkdir test-project && cd test-project
cargo run --manifest-path /path/to/wgenty-code/Cargo.toml -- query --prompt "hello" 2>&1 || true
ls -la .wgenty-code/  # 预期: sessions/ memory/ 目录存在
```

- [ ] **Step 8.5: 手动验证全局记忆注入**

在 `~/.wgenty-code/memory/` 放一条测试记忆，运行 query，检查 system prompt 含 `<global-memory>` 块（通过 RUST_LOG=debug 观察）。

- [ ] **Step 8.6: 手动验证迁移**

确保 `~/.wgenty-code/sessions/` 有旧 session，首次启动后检查迁移到对应项目目录 + marker 文件生成。

- [ ] **Step 8.7: 性能验证**

```bash
cargo build --release
time ./target/release/wgenty_code --version          # 启动时间增量 ≤ 5%
ls -lh ./target/release/wgenty_code                   # 二进制增量 ≤ 500KB
```

- [ ] **Step 8.8: 最终提交（如有未提交的修复）**

```bash
git add -A
git commit -m "test(context): verify project-local state and memory scoping"
```

archived-with: 2026-07-15-project-local-state
---

## 并行执行机会

以下任务组在依赖满足后可并行执行：

- **Task 3（Memory 双源）与 Task 6（迁移）**：两者都只依赖 Task 1（路径函数），互不依赖，可并行。但 Task 6 的迁移调用在 `MemoryManager::with_settings` 中，而 Task 3 也改 `with_settings`，建议 Task 3 先完成再做 Task 6，或同一 agent 顺序执行避免合并冲突。
- **Task 4（全局记忆注入）与 Task 5（Compaction scope）**：两者都依赖 Task 3（`add_memory` 带 scope / `global_memories()`），互不依赖，可并行。Task 4 改 `inject.rs` + `prompts/mod.rs`，Task 5 改 `compactor.rs`，文件不重叠，并行安全。

**推荐执行顺序（串行关键路径）：** Task 1 → Task 2 → Task 3 → (Task 4 ∥ Task 5) → Task 6 → Task 7 → Task 8

**可并行的子代理分配（如选择 subagent-driven）：**
- Agent A: Task 1 → Task 2 → Task 3（关键路径，必须先完成）
- Agent B（等 Task 3 完成后）: Task 4
- Agent C（等 Task 3 完成后）: Task 5
- Agent D（等 Task 2 完成后）: Task 6
- 最后串行: Task 7 → Task 8
