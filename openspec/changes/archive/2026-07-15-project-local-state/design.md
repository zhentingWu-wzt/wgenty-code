# Design: 项目本地状态存储与记忆分层

## Overview

本设计将 wgenty-code 的会话与记忆从「全局扁平存储」改为「项目本地 + 全局分层」模型。核心引入 **scope 物理分离**：存储位置即 scope，`MemoryManager` 双源加载。项目根 = CWD，不向上查找。

```
~/.wgenty-code/                      # 全局（跨项目）
├── memory/<id>.json                 # 全局记忆（永久注入）
├── sessions/                        # [迁移源] 旧会话，迁移后清空
├── history.jsonl                    # 命令历史（不变）
├── settings.json
├── WGENTY.md / rules/               # 静态指令（不变）
└── daemon.token

<CWD>/.wgenty-code/                  # 项目本地
├── sessions/{id}.json               # 项目会话
└── memory/<id>.json                 # 项目记忆（按需召回）
```

## Goals & Constraints

- **项目根 = CWD**：不向上查找 `.git`/`Cargo.toml`。简单且符合用户预期。
- **scope = 物理位置**：不给 `MemoryEntry` 加 schema 字段，避免序列化兼容负担。`MemoryManager` 内部用 `MemoryOrigin` 枚举追踪来源。
- **全局记忆每轮注入**：不走 TF-IDF 阈值过滤，保证「永久记住」语义。
- **项目记忆按需召回**：保留现有 TF-IDF + importance 阈值机制，但只搜项目记忆。
- **向后兼容**：迁移后旧路径双读，防止迁移失败丢数据。
- **最小侵入**：命令历史、daemon token、settings 不动。

## Data Model

### MemoryOrigin（新增，内部枚举，不序列化）

```rust
/// Tracks where a memory was loaded from. Not serialized - derived from
/// storage path at load time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOrigin {
    Project,
    Global,
}
```

`MemoryManager` 内部将 `memories: Arc<RwLock<Vec<MemoryEntry>>>` 改为带 origin 的结构：

```rust
struct LoadedMemory {
    entry: MemoryEntry,
    origin: MemoryOrigin,
}
```

TF-IDF 索引只索引 **项目记忆**（全局记忆不需召回）。这样 `search_memories()` 天然只返回项目记忆，无需运行时过滤。

### SessionManager 构造变更

```rust
pub struct SessionManager {
    sessions_dir: PathBuf,   // <CWD>/.wgenty-code/sessions/
    // ... 不变
}

impl SessionManager {
    /// 新增：接受项目根目录
    pub fn with_project_root(project_root: PathBuf) -> Self {
        let sessions_dir = project_root.join(".wgenty-code").join("sessions");
        // ...
    }
}
```

保留 `new()` 作为兼容入口（内部调用 `with_project_root(config_dir())` 或标记 deprecated），但主路径改用 `with_project_root`。

## API Changes

### `src/utils/mod.rs`

```rust
/// 项目本地 .wgenty-code 目录：<project_root>/.wgenty-code/
pub fn project_local_dir(project_root: &Path) -> PathBuf {
    project_root.join(".wgenty-code")
}

/// 项目本地 memory 目录
pub fn project_memory_dir(project_root: &Path) -> PathBuf {
    project_local_dir(project_root).join("memory")
}

/// 项目本地 sessions 目录
pub fn project_sessions_dir(project_root: &Path) -> PathBuf {
    project_local_dir(project_root).join("sessions")
}

/// 全局 memory 目录（保持原有 ~/.wgenty-code/memory/）
pub fn global_memory_dir() -> PathBuf {
    config_dir().join("memory")
}
```

### `MemoryManager`

- `with_settings(settings, project_root)`：接收项目根，初始化双 `Storage`（project + global）。
- `load()`：分别从项目目录与全局目录加载，全局记忆标记 `MemoryOrigin::Global`。
- `search_memories(query)`：只搜项目记忆（索引只含项目记忆）。
- `add_memory(entry, scope)`：按 scope 写入对应目录。
- `global_memories() -> Vec<MemoryEntry>`：返回全部全局记忆（供每轮注入）。
- `add_memory` 的 dedup 逻辑需考虑 scope：同 scope 内 dedup，跨 scope 不合并。

### `MemoryContextInjector`

- `recall(user_input, manager, ...)`：保持不变，只召回项目记忆。
- 新增 `inject_global(manager) -> String`：返回 `<global-memory>` 块，包含所有全局记忆。每轮调用，拼到 system prompt。

### Compaction scope 判定

`ConsolidationEngine` 的提取 prompt 增加 scope 字段：

```json
{
  "summary": "...",
  "memories": [
    {"type": "preference", "scope": "global", "content": "始终用中文回复"},
    {"type": "decision", "scope": "project", "content": "本项目用 axum 0.7"}
  ]
}
```

- `scope` 缺失时默认 `project`（保守，避免误升级为全局）。
- 提取后按 scope 调用 `add_memory(entry, scope)`。

## Storage Layer

`Storage` 当前是单目录。两种方案：

**方案 A（推荐）**：`MemoryManager` 持有两个 `Storage` 实例（project_storage + global_storage），各自独立。`Storage` 本身不改。

**方案 B**：`Storage` 改为多目录。侵入大，不推荐。

选 A：最小改动，`Storage` 复用，`MemoryManager` 负责路由。

## Migration Strategy

启动时（`MemoryManager::with_settings` 或显式 `migrate()` 调用）执行一次性迁移：

### Session 迁移

```
for each ~/.wgenty-code/sessions/{id}.json:
    session = load(id)
    target_root = session.project_path.unwrap_or(current_cwd)
    target = target_root/.wgenty-code/sessions/{id}.json
    if target 不存在:
        copy to target
        remove original
    else:
        skip (已迁移或冲突)
```

迁移幂等：目标存在则跳过。迁移后原文件删除（避免双读混淆）。失败时保留原文件并 warn。

### Memory 迁移

现有 `~/.wgenty-code/memory/*.json` 无 scope 元数据。**全部保留在全局目录**（即不移动），天然成为全局记忆。新记忆按 scope 写入对应目录。无需迁移动作，只需确保 `MemoryManager::load()` 仍从全局目录读取。

### 迁移触发

- 首次检测到旧 sessions 目录非空时触发 session 迁移。
- 用 `~/.wgenty-code/.migrated-v2` 标记文件避免重复扫描（或检查旧目录是否为空）。

## Injection Flow

```
每轮 user message:
  system_prompt = base + permissions + ... 
    + <global-memory>           # 全局记忆，每轮注入
        - [Preference] 始终用中文回复
        - [Decision] 提交前跑 cargo clippy
      </global-memory>
    + <memory-context>          # 项目记忆，按需召回（可能为空）
        - [Decision] 本项目用 axum 0.7
      </memory-context>
    + skills + wgenty_md
```

全局记忆块放在 Environment 与 Skills 层之间，与现有项目记忆召回块相邻但独立。

## Edge Cases

1. **CWD 不可写**：项目本地目录创建失败时，session/memory 回退到全局目录并 warn（降级，不 panic）。
2. **CWD 为 home 根目录**：`<home>/.wgenty-code/` 与全局 `~/.wgenty-code/` 重合--此时项目记忆与全局记忆物理同目录。`MemoryManager` 检测到 `project_root == home_dir` 时，将项目记忆写入全局目录的 `project-memory/` 子目录以避免混淆，或直接合并为全局。选择：检测重合时 warn 并将项目记忆也写入全局 `memory/`（视为同一池）。
3. **无 CWD（被删除）**：`std::env::current_dir()` 失败时回退到全局目录。
4. **迁移中崩溃**：迁移非原子，但幂等--重启后重新扫描，已迁移的跳过，未完成的继续。
5. **全局记忆过多**：全局记忆每轮注入，数量大时会撑大 prompt。加软上限（如 top 50 by importance）并 warn。
6. **子代理 scope**：子代理继承主 agent 的项目根，共享同一项目记忆池（non-goal: 子代理独立 scope）。

## Trade-offs

- **物理分离 vs schema 字段**：选物理分离。优点：零序列化兼容风险、scope 天然隔离、迁移简单。缺点：`MemoryManager` 需双源管理、跨 scope 查询需显式合并。
- **CWD vs 向上查找**：选 CWD。优点：简单、可预测、子目录运行时显式隔离。缺点：子目录运行会创建多个 `.wgenty-code/`（用户已接受）。
- **全局记忆每轮注入 vs 按需召回**：选每轮注入。符合「永久记住」语义，但占用固定 token。软上限缓解。
- **现有 memory 全部归全局**：无法可靠路由到项目，保守归全局最安全。用户可后续手动迁移到项目目录。

## Open Questions

- 全局记忆软上限值（暂定 50 条，按 importance 排序）需实测调整。
- `<global-memory>` 块的具体 system prompt 位置与措辞，待 build 阶段细化。
