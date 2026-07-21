# Tasks: 项目本地状态存储与记忆分层

## 1. 基础设施：项目本地路径工具函数

- [x] 1.1 在 `src/utils/mod.rs` 新增 `project_local_dir(project_root)`、`project_memory_dir(project_root)`、`project_sessions_dir(project_root)`、`global_memory_dir()` 函数
- [x] 1.2 新增 `current_project_root()` 工具函数：封装 `std::env::current_dir()`，失败时回退到全局 `config_dir()` 并 warn
- [x] 1.3 添加单元测试覆盖路径构造与 CWD 回退逻辑

## 2. Session 项目化

- [x] 2.1 `SessionManager` 新增 `with_project_root(project_root: PathBuf)` 构造函数，`sessions_dir` 指向 `<project_root>/.wgenty-code/sessions/`
- [x] 2.2 `SessionManager::new()` 标记 deprecated 或改为调用 `with_project_root(config_dir())` 保持测试兼容
- [x] 2.3 `SessionManager::create()` 时自动创建项目本地 sessions 目录（含父目录）
- [x] 2.4 CWD 不可写时降级：目录创建失败回退到全局 `~/.wgenty-code/sessions/` 并 warn
- [x] 2.5 更新 `MemoryManager::with_settings()` 调用链，传入项目根初始化 `MemorySessionManager`
- [x] 2.6 更新 TUI 启动路径（`src/tui/app/mod.rs`）与 CLI `run_query`/`run_agent` 路径，传入 CWD 作为项目根
- [x] 2.7 测试：项目本地 session 创建/加载/列表只含当前项目会话

## 3. Memory 双源存储与 scope 追踪

- [x] 3.1 新增 `MemoryOrigin` 枚举（Project/Global）与 `LoadedMemory { entry, origin }` 内部结构
- [x] 3.2 `MemoryManager` 持有双 `Storage`（`project_storage` + `global_storage`），`with_settings(settings, project_root)` 初始化
- [x] 3.3 `MemoryManager::load()` 分别从项目与全局目录加载，全局记忆标记 `MemoryOrigin::Global`，项目记忆标记 `Project`
- [x] 3.4 TF-IDF `MemoryIndex` 只索引项目记忆（全局记忆不参与召回）
- [x] 3.5 `search_memories(query)` 只返回项目记忆（天然由索引限定）
- [x] 3.6 新增 `global_memories() -> Vec<MemoryEntry>` 返回全部全局记忆（供每轮注入）
- [x] 3.7 `add_memory(entry, scope)` 按 scope 写入对应 `Storage`；dedup 逻辑改为同 scope 内去重
- [x] 3.8 CWD 为 home 目录（项目根 == 全局根）时 warn 并将项目记忆写入全局目录（合并为同一池）
- [x] 3.9 更新所有 `add_memory()` 调用方传入 scope（compaction、手动添加路径）
- [x] 3.10 测试：双源加载、scope 隔离、跨 scope 不 dedup、home 重合降级

## 4. 全局记忆每轮注入

- [x] 4.1 `MemoryContextInjector` 新增 `inject_global(manager) -> String`，生成 `<global-memory>` 块（含全部全局记忆，按 importance 排序，软上限 50）
- [x] 4.2 全局记忆软上限：超过 50 条时取 top 50 by importance 并 warn
- [x] 4.3 在 system prompt 组装流程（`src/prompts/` 或 TUI turn spawn）中，每轮调用 `inject_global` 拼入，位置在 Environment 与 Skills 层之间
- [x] 4.4 `<global-memory>` 块为空时不注入（避免空块噪声）
- [x] 4.5 更新 daemon/headless 路径（`run_query`/`run_agent`）同样注入全局记忆
- [x] 4.6 测试：全局记忆注入格式、软上限、空块不注入

## 5. Compaction 自动 scope 判定

- [x] 5.1 `ConsolidationEngine` 提取 prompt 增加 `scope` 字段要求（JSON schema：`memories[].scope = "project"|"global"`）
- [x] 5.2 解析 compaction 响应时读取 `scope`，缺失时默认 `project`
- [x] 5.3 提取的记忆按 scope 调用 `add_memory(entry, scope)`
- [x] 5.4 prompt 指引模型：跨项目通用的偏好/行为准则归 global，项目特定决策/知识归 project
- [x] 5.5 测试：scope 解析、缺失默认 project、按 scope 路由写入

## 6. 数据迁移

- [x] 6.1 新增 `migrate_legacy_sessions()`：扫描 `~/.wgenty-code/sessions/`，按 `project_path` 路由到 `<project_path>/.wgenty-code/sessions/`，`project_path` 为 None 归当前 CWD
- [x] 6.2 迁移幂等：目标已存在则跳过；迁移成功后删除原文件；失败保留原文件并 warn
- [x] 6.3 用 `~/.wgenty-code/.migrated-project-local` 标记文件避免重复扫描
- [x] 6.4 现有 `~/.wgenty-code/memory/*.json` 不迁移（天然成为全局记忆），确认 `load()` 仍从全局目录读取
- [x] 6.5 启动时触发迁移（`MemoryManager::with_settings` 或 TUI/CLI 启动入口）
- [x] 6.6 测试：迁移幂等性、project_path 路由、None 归当前、目标冲突跳过

## 7. CLI 与配置集成

- [x] 7.1 `memory status` 命令输出区分项目记忆数与全局记忆数
- [x] 7.2 `memory` CLI 新增手动添加记忆时支持 `--scope project|global` 参数（若现有命令结构允许）
- [x] 7.3 更新 WGENTY.md 文档：配置/架构章节补充项目本地 `.wgenty-code/` 布局说明
- [x] 7.4 更新 `agent-memory` spec（delta，见 specs/）

## 8. 验证与收尾

- [x] 8.1 `cargo fmt --check` 通过
- [x] 8.2 `cargo clippy --all-targets -- -D warnings` 零 warning
- [x] 8.3 `cargo test --all` 全部通过
- [x] 8.4 手动验证：新项目首次运行创建 `<CWD>/.wgenty-code/`；全局记忆每轮注入；项目记忆按需召回；迁移旧数据
- [x] 8.5 性能验证：启动时间增量 ≤ 5%，内存增量 ≤ 2%（按 AGENTS.md 性能约束）
