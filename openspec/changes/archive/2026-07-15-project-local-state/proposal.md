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
