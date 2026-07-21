## Why

`memory_add` 工具和 `AutoDream` 记忆整理在 merge 后虽已落地代码,但实际不可用:

1. **TUI/daemon 模式下 `memory_add` 不存在** -- 工具只注册在 `headless_runtime.rs`,`daemon/state.rs`(TUI 依赖的 daemon 进程)未注册,导致主力交互模式下模型根本看不到该工具,无法主动存教训。
2. **AutoDream 几乎从不触发** -- `check_and_run()` 仅在 TUI 启动时 fire-and-forget 调用一次,门控 `min_hours=24` + `min_sessions=5` 过严,且 headless 模式完全未接入。结果是记忆库只增不整理,相似记忆堆积、过时记忆不衰减、孤儿文件残留。
3. **两把 consolidation 锁路径不一致** -- AutoDream 用 `~/.wgenty-code/.consolidation.lock`(时间戳),`MemoryManager::consolidate()` 用 `~/.wgenty-code/memory/.consolidation.lock`(`ConsolidationFileLock`),互不保护,存在并发竞态隐患。

现在解决,因为 `memory_add` 是"教训存记忆"的唯一入口,而 `consolidate()` 不调用 LLM(纯本地 TF-IDF 合并 + TTL + 磁盘 reconcile + 索引重建),放宽门控的成本极低,收益是记忆库不再随高频写入而退化。

## What Changes

- **TUI/daemon 注册 `memory_add`**:在 `daemon/state.rs` 注册 `MemoryAddTool`,使 TUI/daemon 模式(主力交互模式)下模型可主动存教训。需将 `MemoryManager` 注入 daemon state。
- **放宽 AutoDream 门控**:`min_hours` 由 24 调整为 1,`min_sessions` 由 5 调整为 1。前提是 `consolidate()` 不调用 LLM,频繁触发的成本仅为短暂的写锁阻塞。
- **headless 接入 AutoDream 启动检查**:在 `headless_runtime.rs` 启动时调用 `AutoDreamService::check_and_run()`,目前该模式完全未接入。
- **统一 consolidation 锁**:AutoDream 不再自管锁(移除 `try_acquire_lock` 的时间戳锁逻辑),直接委托 `MemoryManager::consolidate()`(其内部已通过 `ConsolidationFileLock` 加跨进程锁)。
- **不改**:`memory_add` 工具实现、`consolidate()` 算法、记忆存储路径、compaction 提取逻辑、prompt 引导。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `agent-memory`: 修改"Time-gated memory consolidation"requirement -- 门控由 24h/5session 改为 1h/1session;AutoDream 锁逻辑统一到 `MemoryManager::consolidate()` 内部锁;headless 模式接入启动检查。强化"Proactive memory capture via tool"的可用性要求 -- `memory_add` 必须在 TUI/daemon 模式注册(目前 spec 已要求所有 agent 可用,但 daemon 未注册,属实现补全)。

## Impact

- **代码**:
  - `src/daemon/state.rs` -- 注册 `MemoryAddTool`,注入 `MemoryManager`(新增字段或构造参数)
  - `src/services/auto_dream.rs` -- 门控常量(`DEFAULT_MIN_HOURS`/`DEFAULT_MIN_SESSIONS`)调整;移除 `try_acquire_lock` 自管锁,`check_and_run`/`force_consolidation` 直接调 `mm.consolidate()`
  - `src/cli/headless_runtime.rs` -- 启动时构造 `AutoDreamService` 并调用 `check_and_run()`
  - `src/context/mod.rs` -- 可能无需改动(`ConsolidationFileLock` 已存在);确认锁路径统一后 `.consolidation.lock` 只剩一处
- **API**:无 public API 变更(均为内部服务/工具注册)
- **依赖**:无新增依赖
- **运行模式**:TUI/daemon + headless 均覆盖
- **风险**:
  - 门控激进(1h/1session)可能使 consolidate 频繁持写锁,短暂阻塞 `add_memory` -- 因 consolidate 不调 LLM 且毫秒~秒级,可接受
  - daemon 注入 `MemoryManager` 的路径需确认(当前 `daemon/state.rs` 无 memory_manager 字段)
  - subagent 工具过滤逻辑需确认不会排除 `memory_add`
