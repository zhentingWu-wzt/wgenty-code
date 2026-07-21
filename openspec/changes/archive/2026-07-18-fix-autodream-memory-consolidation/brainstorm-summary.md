# Brainstorm Summary

- Change: fix-autodream-memory-consolidation
- Phase: design
- Date: 2026-07-18
- Mode: compact

## 议题

merge 后 `memory_add` 工具与 `AutoDream` 整理代码已落地但不可用。本次是"让已写好的能力真正生效"的修复型 change，不引入新架构。brainstorming 聚焦两个未决架构选择与一个 API 重构决策。

## 前置代码核查（brainstorming 输入）

设计文档的关键事实均已通过 CodeGraph 核查确认：

| 事实 | 核查结果 |
|---|---|
| `DEFAULT_MIN_HOURS=24` / `DEFAULT_MIN_SESSIONS=5` | ✅ `services/auto_dream.rs:20-21` |
| `consolidate()` 不调 LLM（纯 TF-IDF + reconcile + rebuild） | ✅ `context/mod.rs:709`，`ConsolidationEngine::consolidate` `context/consolidation.rs:72` |
| `ConsolidationFileLock` 跨进程锁已保护 `consolidate()` | ✅ `context/mod.rs:714`，路径 `~/.wgenty-code/memory/.consolidation.lock` |
| `filter_allowed_tools` 不排除 `memory_add` | ✅ `task.rs:869`，只过滤 `task`/`delegate` + explore 的 `MUTATING_FS_TOOLS` |
| daemon 完全未注册 `memory_add` / 无 `MemoryManager` | ✅ `daemon/state.rs` grep 零命中 |
| headless 已注册 `memory_add` | ✅ `headless_runtime.rs:241` |
| `AutoDreamService::new` 第一个参数 `_state` 未使用 | ✅ 下划线前缀，struct 不存储该字段 |
| daemon 是每会话启动（非长驻） | ✅ `cli/args.rs:198` -> `tui/util.rs:42`，每次 `repl` fresh 构造 |

## 决策记录

### Q1: AutoDream 启动入口 -- daemon 侧 vs TUI app 侧

**选项呈现：**
1. 保持 TUI app 侧 AutoDream，daemon 只为 memory_add 持有 mm（原设计倾向）
2. daemon 侧启动 AutoDream，移除 TUI app 侧（用户选择 ✅）
3. 两者都启动（靠锁兜底）

**用户决策：选项 2** -- daemon 成为 TUI/daemon 模式的唯一 AutoDream 入口。

**代码核查支撑：**
- daemon 是每会话启动（`cli/args.rs:198` -> `tui/util.rs:42`），非长驻。移到 daemon 后 check_and_run 仍在每次 REPL 启动时触发，与原 TUI app 行为等价，无时序回归。
- TUI app 的 `auto_dream_service`（`app/mod.rs:195/545/593`）是独立实例，与 ServiceManager（`services/mod.rs:51`，mm=None，仅状态展示）和 CLI `memory autodream`（`args.rs:473`，手动 force_consolidation）无耦合，移除范围干净。
- `force_consolidation` 调用方仅 `args.rs:432`（CLI 手动命令）和测试，不涉及 TUI app，移除不影响手动触发路径。

**取舍：**
- 收益：架构集中，TUI/daemon 模式只有一个 AutoDream 启动点；daemon 的 mm 既供 memory_add 又供 AutoDream，职责统一。
- 成本：需移除 TUI app 的 `auto_dream_service` 字段 + 构造 + spawn + 测试（4 处改动）；daemon 需新增 AutoDream 构造与 spawn。

### Q2: headless 构造 AutoDreamService -- 传 AppState vs 重构 API

**选项呈现：**
1. 传 `AppState::new(settings)`（不改 API，但传未使用参数）
2. 重构 `AutoDreamService::new` 移除 `_state` 依赖（用户选择 ✅）

**用户决策：选项 2** -- 移除 `AutoDreamService::new` 的 `_state` 参数。

**代码核查支撑：**
- `_state: Arc<RwLock<AppState>>` 在 `new` 中完全未使用（下划线前缀），struct 也不存储它。移除是无副作用清理。
- 影响调用方：`app/mod.rs:545`（Q1 已移除）、`services/mod.rs:51`、`args.rs:473`、`stress_tests.rs:162`、`auto_dream.rs:304` 测试。共 4 处生产调用方需更新签名 + 1 处测试。

**取舍：**
- 收益：API 诚实（不要求调用方提供不用的参数）；headless 接入更简洁（`AutoDreamService::new(None, Some(mm))`）。
- 成本：5 处调用方签名更新（机械改动，无逻辑变更）。轻微超出原 Non-Goals（"不改 memory_add 工具实现"不涉及，但 AutoDream API 签名变更属新增范围）。

## 最终设计变更（相对原 design.md 的 delta）

| 决策 | 原 design.md | 最终（brainstorming 后） |
|---|---|---|
| D1 daemon 注入 mm | mm 用于 memory_add | mm 用于 memory_add **+ AutoDream**（daemon 侧启动） |
| D2 门控 1h/1session | 不变 | 不变 |
| D3 移除 AutoDream 自管锁 | 不变 | 不变 |
| D4 headless 接入 | headless 新增 check_and_run | headless 新增 **+ 移除 TUI app 侧 AutoDream** |
| D5 不做后台 tick | 不变 | 不变 |
| **D6（新增）** | — | 重构 `AutoDreamService::new` 移除 `_state` 参数 |

## 用户确认

- [x] Q1: daemon 侧启动 AutoDream，移除 TUI app 侧 -- 用户选择选项 2
- [x] Q2: 重构 AutoDreamService::new 移除 _state -- 用户选择选项 2
- [x] D2/D3/D5 保持原设计（无异议）

## 非目标（重申）

- 后台周期 tick（层 2 推迟）
- 系统主动提取教训（与 memory_add 冗余）
- 系统级 cron/launchd
- 重写 consolidation 算法、改记忆存储路径、改 compaction 提取
- `memory_add` 工具实现本身（已完整）
- 门控阈值 settings 化（本次保持常量）
