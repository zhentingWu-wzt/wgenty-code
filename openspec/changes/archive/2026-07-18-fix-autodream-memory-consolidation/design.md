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

### D4: headless 接入 AutoDream 启动检查 + 移除 TUI app 侧 AutoDream

**决策:**
1. **headless 接入:** 在 `headless_runtime.rs` 构造 `memory_manager`(L200)之后、agent loop 之前,构造 `AutoDreamService::new(None, Some(mm.clone()))` 并 fire-and-forget `spawn(check_and_run())`。
2. **移除 TUI app 侧:** 移除 `tui/app/mod.rs` 的 `auto_dream_service` 字段(L195)、App::new 中的构造(L545)、启动 spawn(L593)、相关测试(L789)。AutoDream 启动职责由 daemon 接管(D1)。

**备选:**
- 同步 `check_and_run().await` 阻塞启动 -- ✗ headless 是单次 CLI,用户感知延迟;consolidate 虽快但 load+consolidate 仍可能百毫秒级,fire-and-forget 更稳。
- 不接入 headless -- ✗ headless 用户记忆永不整理,违背目标。
- 保留 TUI app 侧 AutoDream -- ✗ 与 daemon 侧双重启动,浪费且依赖锁兜底;用户选择集中化。

**理由:** headless 是单次进程,后台 tick 无意义(进程立即退出),但**启动检查**能覆盖"每次 headless 调用尝试整理一次"。fire-and-forget 与 daemon 侧一致,失败只 log 不影响主流程。移除 TUI app 侧 AutoDream 后,TUI/daemon 模式只有 daemon 一个 AutoDream 启动点,headless 模式只有 headless 一个启动点,无双重启动竞态。daemon 每会话启动,时序与原 TUI app 等价。

### D5: 不做后台 tick(层 2 推迟)

**决策:** 本次不加进程内 interval tick。

**理由:** 层 1(放宽门控 + 接入 headless + 修锁 + 集中启动)已能让 autodream 在每次启动时真正触发,解决"没作用"。后台 tick 的收益是"长会话内周期清理",但需新增调度代码 + 两处接入,且目前无证据长会话内记忆膨胀成问题。等层 1 上线后观察,确有长会话膨胀再加 tick(届时 consolidate 不调 LLM,tick 成本也低)。

### D6: 重构 `AutoDreamService::new` 移除 `_state` 参数

**决策:** 将 `AutoDreamService::new` 签名从 `new(_state: Arc<RwLock<AppState>>, config: Option<AutoDreamConfig>, memory_manager: Option<Arc<MemoryManager>>)` 改为 `new(config: Option<AutoDreamConfig>, memory_manager: Option<Arc<MemoryManager>>)`。`_state` 参数在函数体和 struct 中均未使用,移除是无副作用清理。

**备选:**
- 保留 `_state` 参数,headless 传 `AppState::new(settings)` -- ✗ 传一个未使用的参数是代码异味;且 headless 构造 AppState 仅为满足签名,增加无意义依赖。

**理由:** `_state` 下划线前缀已表明未使用,struct 也不存储该字段。移除后 API 诚实,headless/daemon 接入更简洁(`AutoDreamService::new(None, Some(mm))`)。影响调用方:`services/mod.rs:51`、`args.rs:473`、`stress_tests.rs:162`、`auto_dream.rs:304` 测试(Q1 移除的 `app/mod.rs:545` 不计),均为机械签名更新,无逻辑变更。

## Risks / Trade-offs

- **[风险] 门控激进导致 consolidate 频繁持写锁,阻塞 add_memory** -> 缓解:consolidate 不调 LLM、毫秒~秒级;`SESSION_SCAN_INTERVAL_MS=10min` 节流;`add_memory` 等锁是短暂等待非失败。可接受。若上线后观测到阻塞,调高门控或加 `try_write` 退让。
- **[风险] daemon 注入 mm 后,daemon 与 TUI app 的 mm 是不同实例** -> 缓解:daemon 与 TUI 是进程间关系,本就无法共享实例;两者都从同一磁盘目录 load,`ConsolidationFileLock` 保护跨进程并发。可接受。
- **[风险] headless fire-and-forget 整理未完成进程即退出** -> 缓解:headless 单次调用,启动检查是 best-effort;未完成则下次启动再整理。可接受(与 daemon 侧语义一致)。
- **[风险] 移除 AutoDream 磁盘锁后,`is_consolidating` 仅内存,跨进程重入靠 mm 锁** -> 缓解:这正是设计意图 -- 跨进程并发由 mm `ConsolidationFileLock` 兜底,AutoDream 内存标志只管同进程。消除了原时间戳锁"进程崩溃留下过期锁文件"的卡死隐患。
- **[风险] 移除 TUI app 侧 AutoDream 后,若 daemon 启动失败,TUI 模式无 AutoDream** -> 缓解:daemon 是 TUI 的必要依赖(daemon 启动失败 TUI 本就无法工作);且 CLI `memory autodream` 手动触发仍可用。可接受。
- **[权衡] 1h/1session 可能使低频用户每次启动都整理** -> 可接受:consolidate 不调 LLM,成本极低;且对低频用户而言"每次启动整理"反而保证记忆新鲜。
- **[权衡] D6 API 签名变更影响 4 处生产调用方** -> 可接受:均为机械改动,编译器可捕获所有调用点,无运行时风险。

## Migration Plan

1. 先在 dev 分支实现,本地用 `cargo test --lib` 验证:auto_dream 门控单测、memory_add daemon 注册、锁统一、AutoDreamService::new 签名变更。
2. 手动验证:TUI 模式启动,确认工具表含 `memory_add`;`memory autodream` 状态显示门控为 1h/1session;触发一次 consolidate 确认无锁冲突;确认 TUI app 不再直接启动 AutoDream(日志中 AutoDream check_and_run 来自 daemon)。
3. headless 验证:`wgenty-code` headless 启动日志含 AutoDream check_and_run 调用。
4. 回滚:若门控激进引发问题,改回常量即可(单点改动);若 daemon mm/AutoDream 注入出问题,移除该注册行回退到 headless-only(不破坏 headless 既有行为);若 D6 签名变更引发问题,恢复 `_state` 参数(机械回退)。

## Open Questions

1. **daemon 是否需要把 mm 也用于 AutoDream?** -> **已决策(D1/Q1):** daemon 侧启动 AutoDream,移除 TUI app 侧。daemon 每会话启动,时序等价。
2. **`AutoDreamService::new` 的 `AppState` 参数在 headless 如何构造?** -> **已决策(D6/Q2):** 重构 `AutoDreamService::new` 移除 `_state` 参数,headless/daemon 直接 `new(None, Some(mm))`。
3. **门控阈值是否要做成 settings 可配?** -> **已决策:** 本次保持常量(改值),settings 化留待后续(避免范围扩张)。
