# Comet Design Handoff

- Change: project-local-state
- Phase: design
- Mode: full
- Context hash: e21b449df8430fa5f9ff4df7cca9d508689c7d36174dc348a33132555bf8a0ed

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/project-local-state/proposal.md

- Source: openspec/changes/project-local-state/proposal.md
- Lines: 1-48
- SHA256: 1cf17d03553c8101511c140eab91ee61dd7ba40e4a80ad83635ca040eda8defb

```md
# Proposal: 项目本地状态存储与记忆分层

## Why

当前 wgenty-code 的所有会话与记忆均扁平存储在全局 `~/.wgenty-code/` 下：

- **Session 会话**：`~/.wgenty-code/sessions/{id}.json`——所有项目的会话混在一个目录，虽然 `Session` 结构有 `project_path` 字段，但仅作元数据，不参与存储路径或列表过滤。
- **Memory 记忆**：`~/.wgenty-code/memory/<id>.json`——所有记忆无 scope 之分，`MemoryContextInjector::recall()` 用 TF-IDF 关键词匹配搜索全部记忆，把 `cwd_project_name()` 当作查询词。这是关键词匹配，不是真正的项目隔离。
- **命令历史**：`~/.wgenty-code/history.jsonl`——同样全局混合。

这带来三个问题：

1. **项目隔离缺失**：在项目 A 工作时，会召回项目 B 的记忆（只要关键词命中），造成跨项目信息泄漏与噪声。
2. **会话列表混乱**：`SessionManager::list()` 返回所有项目的会话，用户无法快速定位当前项目的会话。
3. **全局记忆无法显式表达**：用户希望大模型永久记住的跨项目行为准则（如"始终用中文回复"、"提交前跑 clippy"），目前只能塞进 `rules/*.md` 静态文件，无法通过记忆系统结构化管理，也无法在 compaction 时自动沉淀。

## What Changes

1. **Session 项目化**：会话存储从 `~/.wgenty-code/sessions/` 迁移到 `<CWD>/.wgenty-code/sessions/`。项目根 = CWD（主 worktree 的 cwd），不向上查找。`SessionManager::list()` 默认只列出当前项目的会话。

2. **Memory 分层**：记忆按 scope 物理分离——
   - **项目记忆**：`<CWD>/.wgenty-code/memory/`，记录当前项目特有的记忆，按需 TF-IDF 召回。
   - **全局记忆**：`~/.wgenty-code/memory/`，存储用户需要大模型永久记住的跨项目行为/信息，**每轮注入**，不过 TF-IDF 阈值过滤。
   - `MemoryManager` 双源加载并追踪每条记忆的来源 scope。

3. **Compaction 自动 scope 判定**：compaction 提取记忆时，增强 LLM prompt 让模型判断每条记忆的 scope（project/global），自动归档到对应目录。用户也可通过现有 `memory` CLI 或 agent 工具手动添加到任一 scope。

4. **全局记忆注入**：全局记忆每轮作为独立 `<global-memory>` 块注入 system prompt，区别于项目记忆的 `<memory-context>` 按需召回块。

5. **数据迁移**：启动时自动迁移现有数据——
   - 现有 session：按 `project_path` 字段路由到对应项目 `.wgenty-code/sessions/`；`project_path` 为 None 的归入当前项目。
   - 现有 memory：无 scope 元数据，全部保留为全局记忆（无法可靠路由到具体项目）。

6. **命令历史**：保持全局 `~/.wgenty-code/history.jsonl` 不变（不在本次范围）。

## Impact

- **Affected specs**: `agent-memory`（修改 Memory storage、Memory recall 要求；新增 Session 项目化、Memory scope、数据迁移要求）
- **Affected code**:
  - `src/utils/mod.rs`：新增 `project_local_dir()` 工具函数
  - `src/context/memory_session.rs`：`SessionManager` 接受项目根参数，存储路径改为项目本地
  - `src/context/mod.rs`：`MemoryManager` 双源存储（项目 + 全局），`search_memories` 区分 scope
  - `src/context/inject.rs`：全局记忆每轮注入 + 项目记忆按需召回
  - `src/context/consolidation.rs`：compaction prompt 增加 scope 判定
  - `src/context/storage.rs`：`Storage` 支持双目录
  - TUI / CLI 启动路径：传入项目根
- **Data migration**: 启动时一次性自动迁移，向后兼容旧路径双读
- **Non-goals**: 命令历史项目化、向上查找项目根、子代理独立 scope、memory 跨项目共享
```

## openspec/changes/project-local-state/design.md

- Source: openspec/changes/project-local-state/design.md
- Lines: 1-206
- SHA256: 851a5f8c85d40b819e779b573772a15ced8baa0540c885db1d25de3005f87f06

```md
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
```

## openspec/changes/project-local-state/tasks.md

- Source: openspec/changes/project-local-state/tasks.md
- Lines: 1-71
- SHA256: 491f6904c2a41fd30aa6a160a92b647fe6290406874cd66b091b7786b0e0efa6

```md
# Tasks: 项目本地状态存储与记忆分层

## 1. 基础设施：项目本地路径工具函数

- [ ] 1.1 在 `src/utils/mod.rs` 新增 `project_local_dir(project_root)`、`project_memory_dir(project_root)`、`project_sessions_dir(project_root)`、`global_memory_dir()` 函数
- [ ] 1.2 新增 `current_project_root()` 工具函数：封装 `std::env::current_dir()`，失败时回退到全局 `config_dir()` 并 warn
- [ ] 1.3 添加单元测试覆盖路径构造与 CWD 回退逻辑

## 2. Session 项目化

- [ ] 2.1 `SessionManager` 新增 `with_project_root(project_root: PathBuf)` 构造函数，`sessions_dir` 指向 `<project_root>/.wgenty-code/sessions/`
- [ ] 2.2 `SessionManager::new()` 标记 deprecated 或改为调用 `with_project_root(config_dir())` 保持测试兼容
- [ ] 2.3 `SessionManager::create()` 时自动创建项目本地 sessions 目录（含父目录）
- [ ] 2.4 CWD 不可写时降级：目录创建失败回退到全局 `~/.wgenty-code/sessions/` 并 warn
- [ ] 2.5 更新 `MemoryManager::with_settings()` 调用链，传入项目根初始化 `MemorySessionManager`
- [ ] 2.6 更新 TUI 启动路径（`src/tui/app/mod.rs`）与 CLI `run_query`/`run_agent` 路径，传入 CWD 作为项目根
- [ ] 2.7 测试：项目本地 session 创建/加载/列表只含当前项目会话

## 3. Memory 双源存储与 scope 追踪

- [ ] 3.1 新增 `MemoryOrigin` 枚举（Project/Global）与 `LoadedMemory { entry, origin }` 内部结构
- [ ] 3.2 `MemoryManager` 持有双 `Storage`（`project_storage` + `global_storage`），`with_settings(settings, project_root)` 初始化
- [ ] 3.3 `MemoryManager::load()` 分别从项目与全局目录加载，全局记忆标记 `MemoryOrigin::Global`，项目记忆标记 `Project`
- [ ] 3.4 TF-IDF `MemoryIndex` 只索引项目记忆（全局记忆不参与召回）
- [ ] 3.5 `search_memories(query)` 只返回项目记忆（天然由索引限定）
- [ ] 3.6 新增 `global_memories() -> Vec<MemoryEntry>` 返回全部全局记忆（供每轮注入）
- [ ] 3.7 `add_memory(entry, scope)` 按 scope 写入对应 `Storage`；dedup 逻辑改为同 scope 内去重
- [ ] 3.8 CWD 为 home 目录（项目根 == 全局根）时 warn 并将项目记忆写入全局目录（合并为同一池）
- [ ] 3.9 更新所有 `add_memory()` 调用方传入 scope（compaction、手动添加路径）
- [ ] 3.10 测试：双源加载、scope 隔离、跨 scope 不 dedup、home 重合降级

## 4. 全局记忆每轮注入

- [ ] 4.1 `MemoryContextInjector` 新增 `inject_global(manager) -> String`，生成 `<global-memory>` 块（含全部全局记忆，按 importance 排序，软上限 50）
- [ ] 4.2 全局记忆软上限：超过 50 条时取 top 50 by importance 并 warn
- [ ] 4.3 在 system prompt 组装流程（`src/prompts/` 或 TUI turn spawn）中，每轮调用 `inject_global` 拼入，位置在 Environment 与 Skills 层之间
- [ ] 4.4 `<global-memory>` 块为空时不注入（避免空块噪声）
- [ ] 4.5 更新 daemon/headless 路径（`run_query`/`run_agent`）同样注入全局记忆
- [ ] 4.6 测试：全局记忆注入格式、软上限、空块不注入

## 5. Compaction 自动 scope 判定

- [ ] 5.1 `ConsolidationEngine` 提取 prompt 增加 `scope` 字段要求（JSON schema：`memories[].scope = "project"|"global"`）
- [ ] 5.2 解析 compaction 响应时读取 `scope`，缺失时默认 `project`
- [ ] 5.3 提取的记忆按 scope 调用 `add_memory(entry, scope)`
- [ ] 5.4 prompt 指引模型：跨项目通用的偏好/行为准则归 global，项目特定决策/知识归 project
- [ ] 5.5 测试：scope 解析、缺失默认 project、按 scope 路由写入

## 6. 数据迁移

- [ ] 6.1 新增 `migrate_legacy_sessions()`：扫描 `~/.wgenty-code/sessions/`，按 `project_path` 路由到 `<project_path>/.wgenty-code/sessions/`，`project_path` 为 None 归当前 CWD
- [ ] 6.2 迁移幂等：目标已存在则跳过；迁移成功后删除原文件；失败保留原文件并 warn
- [ ] 6.3 用 `~/.wgenty-code/.migrated-project-local` 标记文件避免重复扫描
- [ ] 6.4 现有 `~/.wgenty-code/memory/*.json` 不迁移（天然成为全局记忆），确认 `load()` 仍从全局目录读取
- [ ] 6.5 启动时触发迁移（`MemoryManager::with_settings` 或 TUI/CLI 启动入口）
- [ ] 6.6 测试：迁移幂等性、project_path 路由、None 归当前、目标冲突跳过

## 7. CLI 与配置集成

- [ ] 7.1 `memory status` 命令输出区分项目记忆数与全局记忆数
- [ ] 7.2 `memory` CLI 新增手动添加记忆时支持 `--scope project|global` 参数（若现有命令结构允许）
- [ ] 7.3 更新 WGENTY.md 文档：配置/架构章节补充项目本地 `.wgenty-code/` 布局说明
- [ ] 7.4 更新 `agent-memory` spec（delta，见 specs/）

## 8. 验证与收尾

- [ ] 8.1 `cargo fmt --check` 通过
- [ ] 8.2 `cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] 8.3 `cargo test --all` 全部通过
- [ ] 8.4 手动验证：新项目首次运行创建 `<CWD>/.wgenty-code/`；全局记忆每轮注入；项目记忆按需召回；迁移旧数据
- [ ] 8.5 性能验证：启动时间增量 ≤ 5%，内存增量 ≤ 2%（按 AGENTS.md 性能约束）
```

## openspec/changes/project-local-state/specs/agent-memory/spec.md

- Source: openspec/changes/project-local-state/specs/agent-memory/spec.md
- Lines: 1-124
- SHA256: c44eca50519c5865b53b083abada95e37f04a7d028dfce56f6eca231b35c0c07

```md
# agent-memory Delta: 项目本地状态存储与记忆分层

## MODIFIED Requirements

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend. Memories SHALL be physically separated by scope:
- **Project memories** SHALL be stored at `<project_root>/.wgenty-code/memory/<id>.json`
- **Global memories** SHALL be stored at `~/.wgenty-code/memory/<id>.json`

`project_root` SHALL equal the current working directory (CWD), with no upward search for project markers. Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata. `MemoryManager` SHALL track each loaded memory's origin (Project/Global) internally without serializing the origin field. The TF-IDF index SHALL index only project memories so that `search_memories()` naturally returns only project-scoped results. Deduplication SHALL occur within the same scope only; cross-scope duplicates SHALL NOT be merged.

#### Scenario: Project memory persisted to project-local directory

- **WHEN** `MemoryManager::add_memory(entry, Project)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Global memory persisted to global directory

- **WHEN** `MemoryManager::add_memory(entry, Global)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `~/.wgenty-code/memory/<id>.json`

#### Scenario: CWD unavailable degrades to global storage

- **WHEN** the project-local memory directory cannot be created (e.g. CWD deleted or unwritable)
- **THEN** project memories SHALL fall back to the global memory directory and a warning SHALL be logged

#### Scenario: CWD equals home directory

- **WHEN** `project_root` resolves to the user's home directory (project root coincides with global root)
- **THEN** project memories SHALL be written to the global memory directory (merged pool) and a warning SHALL be logged

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load project memories from `<CWD>/.wgenty-code/memory/` and global memories from `~/.wgenty-code/memory/`. `MemoryManager::search_memories(query)` SHALL retrieve only project memories matching the query via the TF-IDF index (global memories are not indexed for recall). Global memories SHALL be injected every turn as a `<global-memory>` block, NOT filtered by the importance threshold. Global memories exceeding a soft cap (default 50) SHALL be truncated to the top entries by importance with a warning logged. The `<global-memory>` block SHALL NOT be injected when no global memories exist.

#### Scenario: Global memories injected every turn

- **WHEN** a turn is processed and global memories exist in `~/.wgenty-code/memory/`
- **THEN** a `<global-memory>` block containing all global memories (sorted by importance, capped at 50) is injected into the system prompt between the Environment and Skills layers

#### Scenario: Project memories recalled by keyword

- **WHEN** a user message is processed and project memories match the extracted keywords with importance >= threshold
- **THEN** a `<memory-context>` block containing the matched project memories is injected (global memories are excluded from this block)

#### Scenario: No global memories

- **WHEN** a turn is processed but no global memories exist
- **THEN** no `<global-memory>` block is injected

#### Scenario: Global memory soft cap exceeded

- **WHEN** more than 50 global memories exist
- **THEN** only the top 50 by importance are injected and a warning is logged

## ADDED Requirements

### Requirement: Project-local session storage

`SessionManager` SHALL store sessions at `<CWD>/.wgenty-code/sessions/{id}.json` instead of the global `~/.wgenty-code/sessions/`. `SessionManager::list()` SHALL return only sessions belonging to the current project. `project_root` SHALL equal CWD with no upward search. When the project-local sessions directory cannot be created, sessions SHALL fall back to `~/.wgenty-code/sessions/` with a warning logged.

#### Scenario: Session created in project-local directory

- **WHEN** a new session is created via `SessionManager::create()`
- **THEN** the session is persisted at `<CWD>/.wgenty-code/sessions/{id}.json`

#### Scenario: Session list scoped to current project

- **WHEN** `SessionManager::list()` is called
- **THEN** only sessions stored in `<CWD>/.wgenty-code/sessions/` are returned

#### Scenario: Unwritable CWD falls back to global

- **WHEN** the project-local sessions directory cannot be created
- **THEN** sessions are stored in `~/.wgenty-code/sessions/` and a warning is logged

### Requirement: Memory scope classification during compaction

During `do_auto_compact()`, the LLM summarization prompt SHALL request each extracted memory entry to include a `scope` field with value `"project"` or `"global"`. The prompt SHALL instruct the model to classify cross-project preferences and behavioral conventions as `global`, and project-specific decisions/knowledge as `project`. When the `scope` field is absent or unparseable, it SHALL default to `project`. Extracted memories SHALL be persisted via `MemoryManager::add_memory(entry, scope)` to the directory corresponding to their scope.

#### Scenario: Scope classified and routed

- **WHEN** compaction extracts a memory with `scope: "global"`
- **THEN** the memory is stored at `~/.wgenty-code/memory/<id>.json`

#### Scenario: Missing scope defaults to project

- **WHEN** compaction extracts a memory without a `scope` field
- **THEN** the memory is treated as project-scoped and stored at `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Manual memory addition with scope

- **WHEN** a user manually adds a memory via CLI or agent tool with an explicit scope
- **THEN** the memory is stored in the directory corresponding to the specified scope

### Requirement: Legacy data migration to project-local storage

On startup, if legacy sessions exist at `~/.wgenty-code/sessions/` and migration has not been performed (tracked by a `~/.wgenty-code/.migrated-project-local` marker file), `migrate_legacy_sessions()` SHALL move each session to `<project_path>/.wgenty-code/sessions/{id}.json` using the session's `project_path` field. Sessions with `project_path == None` SHALL be moved to the current CWD's project-local directory. Migration SHALL be idempotent: if the target file already exists, the source is skipped. On successful migration of a file, the original SHALL be deleted; on failure, the original SHALL be preserved with a warning. Existing memories at `~/.wgenty-code/memory/` SHALL NOT be migrated--they naturally become global memories.

#### Scenario: Sessions migrated by project_path

- **WHEN** startup detects legacy sessions at `~/.wgenty-code/sessions/` and migration marker is absent
- **THEN** each session is moved to its `project_path`'s `.wgenty-code/sessions/` directory

#### Scenario: Session without project_path migrated to CWD

- **WHEN** a legacy session has `project_path == None`
- **THEN** it is moved to `<CWD>/.wgenty-code/sessions/{id}.json`

#### Scenario: Migration is idempotent

- **WHEN** migration runs and a target file already exists at the destination
- **THEN** the source session is skipped (not overwritten)

#### Scenario: Migration marker prevents re-scan

- **WHEN** the `~/.wgenty-code/.migrated-project-local` marker file exists
- **THEN** session migration is not re-run on subsequent startups

#### Scenario: Existing memories remain global

- **WHEN** startup loads memories after migration
- **THEN** all pre-existing `~/.wgenty-code/memory/*.json` files are loaded as global memories (not moved)
```

