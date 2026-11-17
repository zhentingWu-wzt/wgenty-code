# 验证报告：multi-provider-models

- 日期：2026-11-17
- 阶段：Verify
- 模式：full（20 tasks / 4 delta capabilities / 50 changed files）
- 分支：`feature/20261111/multi-provider-models`
- Plan base：`e3c59ae51e32c5898715d387cb559113647e44d7`
- 验证 HEAD：`99927568dc1b79fe0420574c5370db53c945adfe`
- 实现末梢：`9eabb17a728bfb090defb86c7111ef36d54b2d07`；其后仅为 Comet/Plan/验证产物
- 结论：**PASS — 可进入归档阶段，但归档仍需用户明确授权**

## 摘要

| 维度 | 结果 |
|---|---|
| Completeness | 20/20 tasks；4 capabilities / 16 Requirements 均有实现 |
| Correctness | 23/23 Scenarios 有具名自动化测试或最终审查核对；Rust 2284、Web 293 通过 |
| Coherence | 遵循已批准的 Plan 与 Technical Design；1 项历史端点文档偏差已记录 |
| 独立集成审查 | C1/I1-I9 集中修复后 scoped re-review 全部 ADDRESSED，无新 Critical/Important |
| 质量门禁 | fmt、严格 Clippy、TypeScript、ESLint、Prettier、diff check 通过 |
| 未执行项 | 手动 daemon/浏览器冒烟 not-run；真实厂商网络 E2E not-run |

## 1. OpenSpec 完整验证 7 项

1. **tasks.md 全部完成：PASS**
   - `comet classic openspec -- instructions apply --change multi-provider-models --json` 返回 `progress.total=20`、`complete=20`、`remaining=0`、`state=all_done`。
2. **实现符合 OpenSpec design.md 高层决策：PASS（文档偏差见第 4 节）**
   - D1 providers 真源与迁移备份、D2 配置厂商/协议分离、D3 session overlay 与 spawn 快照、D4 三级目录合并/TTL、D5 per-entry Web 状态与 Composer chips 均已落地。
3. **实现符合 Superpowers Technical Design：PASS**
   - `docs/superpowers/specs/2026-11-11-multi-provider-models-design.md` 可定位，`comet_change` 正确。
   - 配置/endpoint/tier、daemon resolver、目录合并、Web per-entry 和 chips 的模块归属一致。
4. **能力规格场景全部通过：PASS**
   - 4 个 capability、16 个 Requirement、23 个 Scenario 的覆盖见第 2 节。
5. **proposal.md 目标已满足：PASS**
   - 厂商分组配置、旧 profiles 迁移、session 模型/权限覆盖、两级 chips、动态目录、事件 session 路由均已实现；Non-Goals 未擅自扩大。
6. **delta spec 与设计文档无未处理矛盾：PASS（已记录偏差）**
   - delta spec 未要求新增 `/providers`；已批准 tasks 3.2 与 Plan Global Constraints 明确要求 `GET /models` 原地升级，实现和 Technical Design 均遵循该决定。
7. **关联设计文档可定位：PASS**
   - OpenSpec `design.md` 与 Superpowers Technical Design 均存在且属于本 change。

## 2. Requirement / Scenario 覆盖

### provider-model-config

- **厂商分组配置结构（2 Scenarios）**：`ProviderGroup`/`ProviderModel` serde、模型端点覆盖和分组配置测试；`src/config/models.rs`、`src/config/tests.rs`。
- **API key 解析顺序（1 Scenario）**：`grouped_load_and_reload_preserve_api_key_precedence_through_api_client`、`legacy_env_only_active_profile_uses_provider_env_through_api_client`，覆盖 model > provider > provider env > global fallback，并走到真实 `ApiClient::get_api_key`。
- **旧 profiles 自动迁移（2 Scenarios）**：`migrate_` 18 项覆盖共享 host、active 映射、幂等、二次合入与备份；`load_from_disk_rejects_conflicting_same_name_profiles_without_rewriting` 和 `load_from_disk_rejects_second_merge_conflict_without_rewriting` 保证冲突时不写坏或丢失旧文件。
- **tier 路由跨厂商归并（1 Scenario）**：`endpoint_for_tier_cross_provider_deterministic` 与迁移前后选择不变测试；session override 下 spawn 另由 `spawned_tier_model_stays_inside_session_override_provider` 验证限定厂商范围。

### session-scoped-model-mode

- **模型 session 覆盖（2 Scenarios）**：`switch_model_with_session_writes_override_only`、`resolve_session_model_applies_override_without_global_write`、`resolve_session_model_falls_back_for_unknown_override`；轮次边界读取由 run loop 测试与最终审查核对。
- **权限模式 session 覆盖（2 Scenarios）**：`set_permission_mode_scopes_override_to_session`、`root_execution_uses_each_sessions_permission_for_approval_and_mode`、`execute_tool_session_` 两方向直接 handler 测试；Plan 由 `set_permission_mode_keeps_runtime_plan_but_persists_derived_mode` 验证当前进程 Plan、磁盘 derived、新 manager derived。
- **覆盖重置（1 Scenario）**：`model_reset_clears_override`、`permission_mode_reset_restores_fallback_without_touching_other_sessions`。
- **Subagent 继承会话覆盖（1 Scenario）**：真实 daemon TaskTool + 本地模型 mock 的 `spawned_tier_model_stays_inside_session_override_provider`，验证 child 请求模型和 spawn 边界快照；权限 resolver 由 `tasktool_permission_resolver_overrides_project_store` 等 4 项覆盖。
- **事件广播携带会话标识（1 Scenario）**：模型/模式 handler payload 测试、Web `sessionManager` 定向事件与 `usePermissionModeSync` 测试。

### model-catalog-fetch

- **厂商模型目录拉取（2 Scenarios）**：`daemon::catalog::tests::` 10 项覆盖成功合并、configured 标记、协议跳过、身份隔离和单飞。
- **拉取失败回退（2 Scenarios）**：失败/旧缓存/TTL 测试；`hanging_catalogs_do_not_block_manual_models_and_fetch_concurrently` 用 60 秒挂起 mock 验证手工+内置目录在 1 秒内可见、两厂商并发为 2、手工模型 500ms 内可切换。
- **内置目录可维护（1 Scenario）**：内置静态目录、首用兜底及手工优先级测试。
- **不可用 fetched-only override 的边界**：`fetched_only_override_restart_failure_does_not_activate_global_provider` 与 resolver 500 测试验证 fail-closed，不静默跨厂商。

### model-mode-input-chips

- **输入框上方的模型 chip（2 Scenarios）**：`ModelModeChips.test.tsx` 覆盖厂商→模型、完整含 `/` 模型名、当前 routed session、默认展示、reset 与真实事件 fallback。
- **输入框上方的权限模式 chip（1 Scenario）**：set/reset 当前 session、响应真实 mode/override、revision 防旧响应覆盖。
- **StatusBar 瘦身（1 Scenario）**：`StatusBar.modelMode.test.tsx` 断言无模型/模式控件，保留连接、审批、workspace/context/cache。
- **会话切换时状态同步（1 Scenario）**：两 entry 展示和隔离、active 切换、A→B→A、catalog generation、mutation revision 与 reconnect/seq 测试。

## 3. 最终集成审查

- 全分支审查范围：`e3c59ae5..cf9b38d3`，31 commits；完整阅读 519341-byte package。
- 初审：Critical 1 / Important 9，结论 `Ready to merge: No`。
- 唯一集中修复：`3a617d8e..9eabb17a`，11 commits；每项均有真实 RED/GREEN，详见 `.superpowers/sdd/2026-11-11-multi-provider-models/final-fix-report.md`。
- 独立 scoped re-review：完整阅读 2405 行修复 diff，C1/I1-I9 全部 **ADDRESSED**，无新 Critical/Important。
- 已关闭的高风险项：旧 `/tools/execute` 权限绕过、API key 优先级、迁移冲突数据保护、planner reload、运行时 Plan、spawn 厂商范围、session 写原子性、metadata/auto-title 覆盖、fetched-only fail-closed、目录挂起/串行等待。
- 非阻塞 Minor：重复但语义相同仅 profile key 不同的迁移条目也保守报错；M1 active main 设置目标、M2 host substring、M3 含 `/` legacy key、M4 热路径重复读取留作后续改进，未被伪称为已修复。

## 4. 设计文档偏差

OpenSpec `design.md` D4 一处写“新增 `GET /api/v1/providers`”，但后续已批准的 `tasks.md` 3.2 和 Plan Global Constraints 明确要求“`GET /models` 原地升级（无新 `/providers` 端点）”。delta spec 只要求组合目录响应，未要求新路由；Technical Design §2.2 同样采用 `GET /models` 原地升级。

因此实现遵循后批准且更具体的任务与计划约束，未新增 `/providers`。这是历史高层文档与后续批准计划的文字偏差，不是 delta spec 缺失或未处理实现漂移。

## 5. 实际检查证据

以下最终候选命令由主会话在 Build 收尾重新执行；之后仅修改 Plan/Comet/验证产物，未修改实现或测试，证据仍适用：

| 命令 | cwd | 结果 |
|---|---|---|
| `cargo test --workspace --quiet` | repo root | exit 0；lib 2065 passed / 1 ignored，integration 219 passed / 3 ignored；合计 **2284 passed / 0 failed / 4 ignored** |
| `cargo fmt -- --check` | repo root | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | repo root | exit 0，零 warning |
| `npx vitest run` | `web/` | exit 0；43 files，**293/293 passed** |
| `npx tsc --noEmit` | `web/` | exit 0 |
| `npx eslint src --quiet` | `web/` | exit 0 |
| scoped Prettier | `web/` | 全分支 Web 变更文件通过 |
| `git diff --check` | repo root | exit 0 |
| `comet guard multi-provider-models build --apply` | repo root | 12/12 PASS，进入 Verify |

### 测试可靠性说明

主会话第一次 workspace 复验曾出现一次 `load_from_disk_rejects_conflicting_same_name_profiles_without_rewriting` 失败（2064 passed / 1 failed / 1 ignored）。随后同一用例单独运行、全量 lib 单线程、全量 lib 默认并行以及最终完整 workspace 均通过。源码检查未发现未串行化的 `WGENTY_HOME` 写入测试；未修改实现来掩盖该异常，根因未证实。最终命令有明确 exit 0，但不声称该一次性测试可靠性问题已被根因修复，列为 **SUGGESTION** 持续观察。

## 6. 未执行项与边界

- 手动 daemon/浏览器冒烟：**not-run**。没有能预先保证完整手工序列所有模型端点均被本地 mock 拦截的独立脚本，为避免真实凭据、厂商访问及外部副作用未启动手工 daemon。
- 真实厂商 subagent 全生命周期网络 E2E：**not-run**。spawn 使用真实 daemon/TaskTool 与本地 HTTP mock 验证 child 请求边界。
- 冲突同名 profile 采用 fail-safe：启动报错、保留旧文件，由用户先手工解决冲突；没有声称支持同名不同端点身份的完整无损自动合并。
- 不可用 fetched-only override 采用 fail-closed：提示刷新目录或重新选择，不回退到其他厂商。
- 动态目录单厂商拉取预算 250ms，并发拉取多厂商；手工/内置目录不受挂起阻塞。

## 最终评估

无未解决 CRITICAL 或 IMPORTANT。OpenSpec 完整性、正确性、设计一致性和当前候选自动化门禁均通过；独立集成审查及修复复查有效。**Verify PASS；branch_status 保持 pending，交由 Archive 阶段取得用户决定。**
