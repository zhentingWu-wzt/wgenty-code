# Comet Design Handoff

- Change: fix-autodream-memory-consolidation
- Phase: design
- Mode: compact
- Context hash: 1d73f51e9796358febbea3b2347ae7af96d53cb0678e400d66c7ce4d191187c5

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/fix-autodream-memory-consolidation/proposal.md

- Source: openspec/changes/fix-autodream-memory-consolidation/proposal.md
- Lines: 1-42
- SHA256: c6e05fd81bd9cd83aa86d84ba41dcc6e3003a48baf04c4433b5c0ee5bd766781

```md
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
```

## openspec/changes/fix-autodream-memory-consolidation/design.md

- Source: openspec/changes/fix-autodream-memory-consolidation/design.md
- Lines: 1-130
- SHA256: 19e124fcef976d9880ca44ee2c58b22f241d256e2bebfc38f13707d279bd764e

[TRUNCATED]

```md
---
comet_change: fix-autodream-memory-consolidation
role: technical-design
canonical_spec: openspec
---

## Context

merge 后 `memory_add` 工具与 `AutoDream` 整理代码已落地,但实际不可用。本次是"让已写好的能力真正生效"的修复型 change,不引入新架构。

**当前状态(已通过代码核查确认):**

| 项 | headless | TUI/daemon |
|---|---|---|
| `memory_add` 注册 | ✅ `headless_runtime.rs:241` | ❌ `daemon/state.rs` 未注册 |
| `MemoryManager` 可用 | ✅ `headless_runtime.rs:200` | ⚠️ daemon 未持有,但可从 `app_state.settings` + `working_dir` 构造 |
| AutoDream 启动检查 | ❌ 未接入 | ⚠️ 仅 TUI app 启动 fire-and-forget 一次(`app/mod.rs:545/593`),门控过严 |
| subagent 工具过滤 | `filter_allowed_tools` 只过滤 `task`/`delegate` 和 explore 的 mutating-fs,`memory_add` 不在过滤名单 | 同左 |

**关键事实:**
- `MemoryManager::consolidate()`(context/mod.rs:709)**不调用 LLM** -- 纯本地 `ConsolidationEngine` TF-IDF 合并 + TTL 衰减 + `Storage::reconcile` 删孤儿 + 索引重建,毫秒~秒级。这是放宽门控的前提。
- `MemoryManager::consolidate()` 内部已通过 `ConsolidationFileLock::acquire(&self.project_storage)` 加**跨进程文件锁**(`~/.wgenty-code/memory/.consolidation.lock`)。AutoDream 另写 `~/.wgenty-code/.consolidation.lock`(时间戳锁)是冗余且路径不一致。
- `filter_allowed_tools`(task.rs:869)不排除 `memory_add`,故 daemon 注册后 subagent 自动可用,满足 spec "available to all agents"。
- `MemoryManager::with_settings(&settings, project_root)` 是标准构造(args.rs:436、app/mod.rs:448 均用此)。headless 当前用 `MemoryManager::new(project_root)`(headless_runtime.rs:200),本次不改 headless 的 mm 构造方式。
- `AutoDreamService::new` 的第一个参数 `_state: Arc<RwLock<AppState>>` **完全未使用**(下划线前缀,struct 不存储),可安全移除。
- daemon 是**每会话启动**(cli/args.rs:198 -> tui/util.rs:42,每次 `repl` fresh 构造 `DaemonState::new`),非长驻。将 AutoDream 移到 daemon 后,check_and_run 仍在每次 REPL 启动时触发,与原 TUI app 行为等价。
- TUI app 的 `auto_dream_service`(app/mod.rs:195/545/593)是独立实例,与 ServiceManager(services/mod.rs:51,mm=None,仅状态展示)和 CLI `memory autodream`(args.rs:473,手动 force_consolidation)无耦合,移除范围干净。

## Goals / Non-Goals

**Goals:**
- TUI/daemon 模式下 `memory_add` 可用(注册 + 注入 mm)
- AutoDream 门控放宽到 1h/1session,使 consolidate 真正周期触发
- headless 模式接入 AutoDream 启动检查
- 统一 consolidation 锁,消除 AutoDream 自管锁与 mm 内部锁的路径不一致竞态
- AutoDream 启动入口集中到 daemon(TUI/daemon 模式)和 headless(单次 CLI 模式),消除 TUI app 与 daemon 的潜在双重启动

**Non-Goals:**
- 后台周期 tick(层 2,等"长会话内膨胀"实际出现再加)
- 系统主动提取教训(会话结束提取 / tick 提取 -- 与 memory_add 冗余)
- 系统级 cron/launchd
- 重写 consolidation 算法、改记忆存储路径、改 compaction 提取
- `memory_add` 工具实现本身(已完整)
- 门控阈值 settings 化(本次保持常量)

## Decisions

### D1: daemon 注入 `MemoryManager` + 启动 AutoDream

**决策:** 在 `DaemonState::new(app_state)` 内部用 `MemoryManager::with_settings(&app_state.settings, app_state.settings.storage.working_dir.clone())` 构造 mm,注册 `MemoryAddTool::new(mm.clone())`,并将 mm 存为 `DaemonState` 字段。同时构造 `AutoDreamService::new(None, Some(mm.clone()))` 并 fire-and-forget `spawn(check_and_run())`,取代原 TUI app 侧的 AutoDream 启动职责。

**备选:**
- 从 TUI app 传 mm 给 daemon -- ✗ daemon 是独立进程,通过 HTTP 通信,不能共享 Arc。
- daemon 不持有 mm,每次工具调用临时构造 -- ✗ `MemoryManager::load()` 有 IO 成本,且 mm 内部缓存 memories + TF-IDF 索引,必须长生命周期。
- daemon 只为 memory_add 持有 mm,AutoDream 留在 TUI app -- ✗ 用户选择集中化(见 brainstorm-summary Q1),且 daemon 每会话启动,时序等价无回归。

**理由:** `DaemonState` 已是 daemon 进程的长生命周期共享状态(持有 tool_registry、coordinator 等),mm 放这里最自然。daemon 每会话启动,在 daemon init 触发 AutoDream check_and_run 与原 TUI app 启动触发语义一致。`with_settings` 是既有标准构造,保证 daemon 与 headless/CLI 的 mm 配置一致(都用 settings.json 的 `storage.memory.*`)。daemon 的 mm 既供 memory_add 又供 AutoDream,职责统一,消除 TUI app 与 daemon 双重启动 AutoDream 的潜在竞态。

### D2: 门控阈值 1h/1session

**决策:** `DEFAULT_MIN_HOURS: 1`,`DEFAULT_MIN_SESSIONS: 1`。

**备选:**
- 6h/2session(温和)-- ✗ 用户反馈"没作用",倾向更激进;consolidate 不调 LLM,激进成本可控。
- 保留 24h/5session + 仅接入 headless -- ✗ 门控过严是"没作用"主因之一,不放宽等于没修。

**理由:** consolidate 不调 LLM,频繁触发的唯一成本是持写锁瞬间阻塞 `add_memory`。1h/1session 意味着"距上次整理 ≥1h 且本会话/最近有 ≥1 个新 session"即触发,实际频率受 `SESSION_SCAN_INTERVAL_MS`(10min)节流,不会失控。这是"先解决没作用"的最直接手段。

### D3: 统一 consolidation 锁 -- 移除 AutoDream 自管锁

**决策:** AutoDream 移除 `try_acquire_lock`(时间戳锁 `~/.wgenty-code/.consolidation.lock`),`run_consolidation` 直接调 `mm.consolidate()`(内部 `ConsolidationFileLock` 已保护)。AutoDream 的 `is_consolidating` 内存标志保留(用于 `check_and_run` 防同进程重入),但不再写磁盘锁文件。

**备选:**
- 保留 AutoDream 锁 + 修路径统一 -- ✗ 两层锁冗余,mm 内部锁已是跨进程的,AutoDream 再加一层无意义。
- 让 mm 锁改用 AutoDream 路径 -- ✗ mm 锁是 `ConsolidationFileLock`(真正的 flock),更可靠,应保留 mm 的。

**理由:** `ConsolidationFileLock` 是基于 `project_storage` 的跨进程文件锁(context/mod.rs:714),已保护 `memory dream` 手动调用与 AutoDream 的并发。AutoDream 的时间戳锁是历史遗留(原 AutoDream 自管记忆时的产物,现 `run_consolidation` 已全委托 mm)。移除它消除路径不一致,且 `is_consolidating` 内存标志仍防同进程内 `check_and_run` 重入。

**注意:** `.autodream_state.json` 的 `last_consolidated_at` 仍需持久化(门控时间基准),保留 `save_state`。`is_consolidating` 不再持久化到磁盘(仅内存),因为进程崩溃后磁盘标志会永久阻塞 -- 这反而修复了原时间戳锁"1h 过期"之外的卡死风险。

```

Full source: openspec/changes/fix-autodream-memory-consolidation/design.md

## openspec/changes/fix-autodream-memory-consolidation/tasks.md

- Source: openspec/changes/fix-autodream-memory-consolidation/tasks.md
- Lines: 1-56
- SHA256: 60873b1c43641de9dfec5af87a350451369851bb9e16d04bbb7dea7c8ad8658a

```md
## 1. AutoDreamService::new 签名重构 (D6)

- [ ] 1.1 将 `AutoDreamService::new` 签名从 `new(_state, config, mm)` 改为 `new(config, mm)`（移除未使用的 `_state: Arc<RwLock<AppState>>` 参数）
- [ ] 1.2 更新 `services/mod.rs:51` 调用方（ServiceManager::initialize）
- [ ] 1.3 更新 `cli/args.rs:473` 调用方（`memory autodream` CLI 命令的 `run_memory`）
- [ ] 1.4 更新 `utils/stress_tests.rs:162` 调用方
- [ ] 1.5 更新 `services/auto_dream.rs:304` 测试调用方
- [ ] 1.6 `cargo check --lib` 确认所有调用方已更新（编译器兜底）

## 2. AutoDream 门控与锁统一 (services/auto_dream.rs, D2/D3)

- [ ] 2.1 将 `DEFAULT_MIN_HOURS` 由 24 改为 1,`DEFAULT_MIN_SESSIONS` 由 5 改为 1
- [ ] 2.2 移除 `try_acquire_lock` 方法及其对 `~/.wgenty-code/.consolidation.lock` 时间戳锁的写入逻辑
- [ ] 2.3 调整 `check_and_run`:移除 `try_acquire_lock` 调用,保留 `is_consolidating` 内存标志作同进程重入防护
- [ ] 2.4 调整 `force_consolidation`:同步移除锁相关逻辑,保留 `is_consolidating` 内存标志
- [ ] 2.5 确认 `save_state` 仅持久化 `last_consolidated_at`(及 `session_count`/`last_session_scan`),不再持久化 `is_consolidating` 到磁盘
- [ ] 2.6 更新 auto_dream 单测:门控阈值改为 1h/1session;移除/改写涉及 `try_acquire_lock` 的测试;新增"不写磁盘锁文件"测试

## 3. daemon 注入 mm + 注册 memory_add + 启动 AutoDream (daemon/state.rs, D1)

- [ ] 3.1 在 `DaemonState` 新增 `memory_manager: Arc<MemoryManager>` 字段
- [ ] 3.2 在 `DaemonState::new` 中用 `MemoryManager::with_settings(&app_state.settings, app_state.settings.storage.working_dir.clone())` 构造 mm
- [ ] 3.3 在 daemon 工具注册区注册 `MemoryAddTool::new(memory_manager.clone())`
- [ ] 3.4 构造 `AutoDreamService::new(None, Some(memory_manager.clone()))` 并 `tokio::spawn` fire-and-forget 调用 `check_and_run()`（取代 TUI app 侧职责）
- [ ] 3.5 确认 daemon 的 subagent 路径(`filter_allowed_tools`)不会过滤 `memory_add`(核查即可,预期无需改动)

## 4. 移除 TUI app 侧 AutoDream (tui/app/mod.rs, D4)

- [ ] 4.1 移除 `App` 结构体的 `auto_dream_service: Option<Arc<AutoDreamService>>` 字段(L195)
- [ ] 4.2 移除 `App::new` 中 `AutoDreamService` 的构造(L545)
- [ ] 4.3 移除启动时 `check_and_run` 的 spawn(L593)
- [ ] 4.4 更新/移除相关测试 `auto_dream_service_is_initialized_on_app_creation`(L789)
- [ ] 4.5 `cargo check` 确认无残留引用

## 5. headless 接入 AutoDream 启动检查 (cli/headless_runtime.rs, D4)

- [ ] 5.1 在构造 `memory_manager`(L200)之后,构造 `AutoDreamService::new(None, Some(memory_manager.clone()))`
- [ ] 5.2 `tokio::spawn` fire-and-forget 调用 `check_and_run()`,失败仅 log(与 daemon 侧语义一致)
- [ ] 5.3 确认 headless 启动日志含 AutoDream check_and_run 调用记录

## 6. 验证与测试

- [ ] 6.1 `cargo check --lib` 通过
- [ ] 6.2 `cargo test --lib services::auto_dream` 通过(含门控与锁的新测试)
- [ ] 6.3 `cargo test --lib tools::meta::memory_add` 通过(既有测试不回归)
- [ ] 6.4 `cargo test --lib context` 通过(consolidate 锁路径相关不回归)
- [ ] 6.5 `cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] 6.6 `cargo fmt --check` 格式一致
- [ ] 6.7 手动验证:TUI 启动,确认工具表含 `memory_add`(模型可见);日志显示 AutoDream check_and_run 来自 daemon(非 TUI app)
- [ ] 6.8 手动验证:`memory autodream` 状态显示门控 1h/1session;触发 consolidate 无锁冲突、无 `.consolidation.lock` 时间戳文件残留
- [ ] 6.9 手动验证:headless 启动触发 AutoDream check_and_run(日志可见)

## 7. 收尾

- [ ] 7.1 更新 `docs/memory-system.md` 中 AutoDream 门控值与锁路径描述(原记载 24h+5session、两把锁不一致隐患);补充 AutoDream 启动入口变更(daemon/headless,TUI app 不再启动)
- [ ] 7.2 提交(遵循每任务一提交,commit message 体现设计意图)
```

## openspec/changes/fix-autodream-memory-consolidation/specs/agent-memory/spec.md

- Source: openspec/changes/fix-autodream-memory-consolidation/specs/agent-memory/spec.md
- Lines: 1-103
- SHA256: 061416fbb556c295ab2db69fd100d2f8dae9cb517f78e848e7a4c8b1bb183434

[TRUNCATED]

```md
## MODIFIED Requirements

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall, in both TUI/daemon and headless modes. The gate thresholds SHALL be `min_hours = 1` and `min_sessions = 1` (distance from last consolidation >= 1 hour AND >= 1 new session since last consolidation). The session-scan interval throttle (`SESSION_SCAN_INTERVAL_MS = 10 minutes`) SHALL remain. When gates pass, consolidation SHALL delegate to `MemoryManager::consolidate()`, which uses `ConsolidationEngine` for deduplication and filtering. Consolidation SHALL use ConsolidationEngine's Jaccard similarity (>0.8 threshold) for duplicate detection, importance threshold (0.3) for filtering, and merge logic for similar memories.

AutoDream SHALL NOT maintain its own disk-based consolidation lock. Cross-process mutual exclusion SHALL be provided solely by `MemoryManager::consolidate()`'s internal `ConsolidationFileLock` (at `~/.wgenty-code/memory/.consolidation.lock`). AutoDream's in-memory `is_consolidating` flag SHALL be retained only to prevent same-process re-entrancy within `check_and_run()`, and SHALL NOT be persisted to disk. The `last_consolidated_at` timestamp SHALL remain persisted in `~/.wgenty-code/.autodream_state.json` as the time-gate baseline.

`MemoryManager::consolidate()` does not invoke any LLM call -- it is pure local computation (TF-IDF similarity merge, TTL decay, orphan-file reconcile, index rebuild). This is the premise that permits the aggressive 1h/1session gate.

#### Scenario: Consolidation gate passes

- **WHEN** session starts and 1 hour has passed with >= 1 new session and no active consolidation lock held by another process
- **THEN** `MemoryManager::consolidate()` is called, deduplicating and merging similar memories

#### Scenario: Consolidation gate fails on time

- **WHEN** session starts but less than 1 hour has passed since last consolidation
- **THEN** consolidation is skipped and the session continues with existing memories

#### Scenario: Consolidation gate fails on session-scan throttle

- **WHEN** session starts within the 10-minute session-scan interval since the last scan
- **THEN** consolidation is skipped without re-scanning the sessions directory

#### Scenario: Cross-process mutual exclusion via MemoryManager lock

- **WHEN** AutoDream triggers `consolidate()` while a concurrent `memory dream` invocation already holds the `ConsolidationFileLock`
- **THEN** AutoDream's `consolidate()` waits on the same lock (no separate AutoDream lock file is created) and no race occurs

#### Scenario: AutoDream does not write a separate disk lock

- **WHEN** AutoDream runs consolidation
- **THEN** no `~/.wgenty-code/.consolidation.lock` (timestamp lock) file is written; only `~/.wgenty-code/.autodream_state.json` (state) and `~/.wgenty-code/memory/.consolidation.lock` (mm internal lock) are touched

#### Scenario: Headless mode triggers AutoDream startup check

- **WHEN** a headless/CLI session starts
- **THEN** `AutoDreamService::check_and_run()` is invoked (fire-and-forget) before the agent loop, identical to TUI startup

#### Scenario: Daemon mode triggers AutoDream startup check

- **WHEN** a TUI/daemon session starts and the daemon process initializes its `DaemonState`
- **THEN** the daemon constructs `AutoDreamService` (with the daemon's `MemoryManager`) and invokes `check_and_run()` (fire-and-forget), so TUI/daemon mode triggers consolidation at session startup

#### Scenario: TUI app does not directly start AutoDream

- **WHEN** a TUI session starts
- **THEN** the TUI app does NOT construct or invoke `AutoDreamService` itself; AutoDream startup is handled solely by the daemon (avoiding duplicate consolidation triggers)

#### Scenario: Consolidation is LLM-free

- **WHEN** `check_and_run()` gates pass and `consolidate()` runs
- **THEN** no LLM call is made; consolidation completes via local TF-IDF merge, TTL decay, orphan reconcile, and index rebuild

### Requirement: Proactive memory capture via tool

The system SHALL provide a `memory_add` tool that allows the agent to proactively write a memory entry at any point during a conversation, without waiting for context compaction. The tool SHALL accept parameters: `content` (required string), `memory_type` (enum: Knowledge/Preference/Session/Conversation/Task/Error/Insight/Decision, default Knowledge), `scope` (enum: project/global, default project), `tags` (optional string array), and `importance` (optional float 0.0-1.0, default 0.5). The tool SHALL delegate to `MemoryManager::add_memory()` for storage, deduplication (0.6 similarity threshold), and scope routing. The tool SHALL declare `is_read_only() = false`. The tool SHALL be registered in BOTH the headless runtime tool registry AND the daemon tool registry (`daemon/state.rs`), so that it is available to the model in all run modes (TUI/daemon and headless). The tool SHALL be available to all agents (root + subagents); the subagent tool filter (`filter_allowed_tools`) SHALL NOT exclude `memory_add`.

#### Scenario: Agent proactively writes a project memory

- **WHEN** the agent calls `memory_add` with content "note_edit tool uses NoteStore but is registered with store:None, so it doesn't persist", memory_type "Knowledge", scope "project"
- **THEN** `MemoryManager::add_memory()` is called with a `MemoryEntry` of type Knowledge and `MemoryOrigin::Project`, and the memory is saved to `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Agent proactively writes a global memory

- **WHEN** the agent calls `memory_add` with content "Always read actual settings.json before assuming config defaults", scope "global"
- **THEN** `MemoryManager::add_memory()` is called with `MemoryOrigin::Global`, and the memory is saved to `~/.wgenty-code/memory/<id>.json`

#### Scenario: Dedup merges similar memory

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory in the same scope
- **THEN** `MemoryManager::add_memory()` merges the new content into the existing memory entry (updating timestamp/metadata) instead of creating a duplicate, and the tool output indicates a merge occurred

#### Scenario: Tool returns memory_id on success

- **WHEN** `memory_add` succeeds (new or merged)
- **THEN** the tool returns a JSON result containing `success: true`, `memory_id` (the stored entry's UUID), and `merged: boolean` indicating whether it was merged into an existing entry

#### Scenario: Invalid memory_type rejected
```

Full source: openspec/changes/fix-autodream-memory-consolidation/specs/agent-memory/spec.md

