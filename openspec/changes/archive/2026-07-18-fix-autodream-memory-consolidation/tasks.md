## 1. AutoDreamService::new 签名重构 (D6)

- [x] 1.1 将 `AutoDreamService::new` 签名从 `new(_state, config, mm)` 改为 `new(config, mm)`（移除未使用的 `_state: Arc<RwLock<AppState>>` 参数）
- [x] 1.2 更新 `services/mod.rs:51` 调用方（ServiceManager::initialize）
- [x] 1.3 更新 `cli/args.rs:473` 调用方（`memory autodream` CLI 命令的 `run_memory`）
- [x] 1.4 更新 `utils/stress_tests.rs:162` 调用方
- [x] 1.5 更新 `services/auto_dream.rs:304` 测试调用方
- [x] 1.6 `cargo check --lib` 确认所有调用方已更新（编译器兜底）

## 2. AutoDream 门控与锁统一 (services/auto_dream.rs, D2/D3)

- [x] 2.1 将 `DEFAULT_MIN_HOURS` 由 24 改为 1,`DEFAULT_MIN_SESSIONS` 由 5 改为 1
- [x] 2.2 移除 `try_acquire_lock` 方法及其对 `~/.wgenty-code/.consolidation.lock` 时间戳锁的写入逻辑
- [x] 2.3 调整 `check_and_run`:移除 `try_acquire_lock` 调用,保留 `is_consolidating` 内存标志作同进程重入防护
- [x] 2.4 调整 `force_consolidation`:同步移除锁相关逻辑,保留 `is_consolidating` 内存标志
- [x] 2.5 确认 `save_state` 仅持久化 `last_consolidated_at`(及 `session_count`/`last_session_scan`),不再持久化 `is_consolidating` 到磁盘
- [x] 2.6 更新 auto_dream 单测:门控阈值改为 1h/1session;移除/改写涉及 `try_acquire_lock` 的测试;新增"不写磁盘锁文件"测试

## 3. daemon 注入 mm + 注册 memory_add + 启动 AutoDream (daemon/state.rs, D1)

- [x] 3.1 在 `DaemonState` 新增 `memory_manager: Arc<MemoryManager>` 字段
- [x] 3.2 在 `DaemonState::new` 中用 `MemoryManager::with_settings(&app_state.settings, app_state.settings.storage.working_dir.clone())` 构造 mm
- [x] 3.3 在 daemon 工具注册区注册 `MemoryAddTool::new(memory_manager.clone())`
- [x] 3.4 构造 `AutoDreamService::new(None, Some(memory_manager.clone()))` 并 `tokio::spawn` fire-and-forget 调用 `check_and_run()`（取代 TUI app 侧职责）
- [x] 3.5 确认 daemon 的 subagent 路径(`filter_allowed_tools`)不会过滤 `memory_add`(核查即可,预期无需改动)

## 4. 移除 TUI app 侧 AutoDream (tui/app/mod.rs, D4)

- [x] 4.1 移除 `App` 结构体的 `auto_dream_service: Option<Arc<AutoDreamService>>` 字段(L195)
- [x] 4.2 移除 `App::new` 中 `AutoDreamService` 的构造(L545)
- [x] 4.3 移除启动时 `check_and_run` 的 spawn(L593)
- [x] 4.4 更新/移除相关测试 `auto_dream_service_is_initialized_on_app_creation`(L789)
- [x] 4.5 `cargo check` 确认无残留引用

## 5. headless 接入 AutoDream 启动检查 (cli/headless_runtime.rs, D4)

- [x] 5.1 在构造 `memory_manager`(L200)之后,构造 `AutoDreamService::new(None, Some(memory_manager.clone()))`
- [x] 5.2 `tokio::spawn` fire-and-forget 调用 `check_and_run()`,失败仅 log(与 daemon 侧语义一致)
- [x] 5.3 确认 headless 启动日志含 AutoDream check_and_run 调用记录

## 6. 验证与测试

- [x] 6.1 `cargo check --lib` 通过
- [x] 6.2 `cargo test --lib services::auto_dream` 通过(含门控与锁的新测试)
- [x] 6.3 `cargo test --lib tools::meta::memory_add` 通过(既有测试不回归)
- [x] 6.4 `cargo test --lib context` 通过(consolidate 锁路径相关不回归)
- [x] 6.5 `cargo clippy --all-targets -- -D warnings` 零 warning
- [x] 6.6 `cargo fmt --check` 格式一致
- [x] 6.7 手动验证:TUI 启动,确认工具表含 `memory_add`(模型可见);日志显示 AutoDream check_and_run 来自 daemon(非 TUI app)
- [x] 6.8 手动验证:`memory autodream` 状态显示门控 1h/1session;触发 consolidate 无锁冲突、无 `.consolidation.lock` 时间戳文件残留
- [x] 6.9 手动验证:headless 启动触发 AutoDream check_and_run(日志可见)

## 7. 收尾

- [x] 7.1 更新 `docs/memory-system.md` 中 AutoDream 门控值与锁路径描述(原记载 24h+5session、两把锁不一致隐患);补充 AutoDream 启动入口变更(daemon/headless,TUI app 不再启动)
- [x] 7.2 提交(遵循每任务一提交,commit message 体现设计意图)
