# Tasks: model-profiles-single-source

- [x] 1. `src/config/models.rs`:`ModelRoles`、legacy 字段 `skip_serializing`、`migrate_legacy()`、确定性 `endpoint_for_tier`
- [x] 2. `src/config/mod.rs`:`load_from_disk` 接迁移、`switch_to_profile` 非破坏、`small_model_settings` 走 tier、新增 `planner_settings()`、`set()` legacy 重映射
- [x] 3. `src/daemon/handlers/system.rs`:`list_models` 用 `active_profile` 判定,删名字反推
- [x] 4. `src/tui/app/turn.rs`:两处 planner 块替换为 `planner_settings()`
- [x] 5. 测试:migrate 各分支、switch 可切回、set 重映射、list_models active 标记、endpoint_for_tier 确定性
- [x] 6. `cargo fmt` + `cargo clippy -- -D warnings` + 定向 `cargo test`(config/daemon/tui)
- [x] 7. follow-up:`models.profiles` 反序列化对 null 条目容错(过渡期构建写出的角色绑定不阻断启动;`active_profile` 指向被丢弃条目由 `migrate_legacy` 恢复)
