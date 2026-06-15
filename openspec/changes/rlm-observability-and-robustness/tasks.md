## 1. TUI 输入框命令补全

- [x] 1.1 在 `src/tui/` 下创建 `CompletionEngine`，启动时扫描 `~/.claude/skills/` 目录和 `PluginRegistry` 加载候选项
- [x] 1.2 在 `src/tui/input_reader.rs` 增加补全触发逻辑：检测 `@` 和 `/` 前缀，发送 `AppEvent::CompletionTrigger` 事件
- [x] 1.3 在 `src/tui/app/event.rs` 增加补全相关事件类型：`CompletionTrigger`、`CompletionSelect`、`CompletionDismiss`
- [x] 1.4 在 `src/tui/components/` 创建 `completion_panel.rs`，实现内联补全面板（复用 PermissionState 的 inline panel 模式）
- [x] 1.5 在 `src/tui/app/` 增加补全状态管理：过滤、导航（Up/Down/Tab/Enter/Esc）、选中替换输入框内容
- [ ] 1.6 在 `packages/cli/src/components/input-box.tsx` 增加 Ink CLI 侧的 `@`/`/` 补全触发支持

## 2. Subagent Transcript 持久化

- [x] 2.1 在 `src/` 下创建 `transcript/` 模块（`mod.rs` + `store.rs`），实现 `SubagentTranscriptStore`（SQLite CRUD）
- [x] 2.2 定义数据库 schema（`subagent_transcripts` + `subagent_events` 表），实现 auto-migration
- [x] 2.3 在 `SubagentTranscriptStore` 实现 `list_by_session`、`get_by_id`、`search` 查询方法
- [x] 2.4 在 `src/config/settings.rs` 增加 `max_transcript_age_days: u32` 配置字段（默认 30）
- [x] 2.5 在 `run_subagent_loop()` 完成/失败时调用 `TranscriptStore::save()` 批量写入所有 events
- [x] 2.6 在 `TranscriptStore::save()` 中实现保留策略：删除超过 `max_transcript_age_days` 的旧记录

## 3. Subagent 执行时间线完整记录

- [x] 3.1 扩展 `SubagentEvent` 枚举，增加 `ToolResult` 和 `Error` 事件类型
- [x] 3.2 修改 `run_subagent_loop()`：移除 action_log 的 50 条截断限制，保留完整事件列表直到 subagent 完成
- [x] 3.3 修改 `run_subagent_loop()`：移除 text_snapshot 的 200 字符截断（改为完整文本存储，TUI 层截断显示）
- [x] 3.4 修改 `SubagentProgress` 结构：增加 `progress_delta: Option<f32>` 字段

## 4. Subagent 错误可视化与恢复

- [x] 4.1 在 `subagent_panel.rs` 增加 Failed 节点的错误详情展示（红色高亮 + 错误消息 + `[r] retry  [d] details` 提示）
- [x] 4.2 在 `subagent_panel_state.rs` 增加 detail view 切换状态和快捷键处理（Enter → 全屏 detail，d → detail，r → retry）
- [x] 4.3 创建 `SubagentDetailView` 组件：从 SQLite 读取 transcript 并以分页方式渲染完整事件时间线
- [x] 4.4 在 `tool_dispatch.rs` 或 `task.rs` 实现重试逻辑：读取失败 subagent 的 prompt → 注入 `previous_attempt_error` → 重新 spawn
- [x] 4.5 实现回滚机制：subagent 修改文件前创建 git stash，重试时 revert 到父节点状态

## 5. RLM 结构化归约

- [x] 5.1 在 `src/tools/meta/rlm/` 下创建 `formats.rs`：定义 `StructuredClaims` 和 `UnifiedDiff` 的 Rust struct（含 serde 序列化）
- [x] 5.2 修改 RLM planner prompt：根据任务类型（analysis/modification/mixed）在 sub-task 描述中注入输出格式指令
- [x] 5.3 修改 `run_subagent_loop()` 的 system prompt：当父任务要求结构化输出时，追加格式规范指令
- [x] 5.4 实现 Aggregator 结构化合并逻辑：Jaccard 相似度去重（阈值 0.8）、conflicts_with 冲突检测、同文件 diff 冲突标记
- [x] 5.5 Aggregator 在结构化合并后，仅对无法 resolve 的冲突项 fallback 到 LLM merge

## 6. RLM 预算控制与进展跟踪

- [ ] 6.1 在 `task` 工具的 `input_schema` 增加 `token_budget: Option<u64>` 字段
- [ ] 6.2 在 `src/config/settings.rs` 增加 `default_subagent_token_budget_k: usize` 配置（默认 0 = 不限）
- [ ] 6.3 在 `run_subagent_loop()` 每轮 API 调用后累加 `cumulative_tokens`，超限立即返回 `Err("Token budget exceeded")`
- [ ] 6.4 实现 RLM pipeline 预算分配逻辑：planner 10% + sub-tasks 80% + aggregator 10%，未用完预算滚动到下阶段
- [ ] 6.5 在 `run_subagent_loop()` 实现 progress_delta 计算：每轮比较新发现数 vs 总发现数，连续 3 轮 delta < 0.05 → `StuckStatus::NoProgress` abort

## 7. TUI 集成与渲染

- [ ] 7.1 更新 `status.rs`（status bar 组件）：显示 subagent 失败计数（红色）、token 预算使用情况
- [ ] 7.2 更新 `subagent_tree.rs`：存储并暴露 progress_delta、budget、error_details 字段
- [ ] 7.3 更新 `subagent_panel.rs`：渲染 token 预算信息（"1.5k/10k tokens"）、progress_delta 低警告、完整 action timeline
- [ ] 7.4 更新 `render.rs`：集成补全面板渲染、detail view 全屏模式

## 8. 配置与 CLI 入口

- [ ] 8.1 在 `settings.rs` 的 `set()` 方法增加新配置项的 setter
- [ ] 8.2 在 Ink CLI (`packages/cli/`) 侧更新 `use-agent.ts` 的 `AgentStatus` 类型以支持新的事件状态
- [ ] 8.3 确保配置热加载（`ConfigChanged` 事件）能正确传播新字段到运行中的 agent

## 9. 验证与测试

- [ ] 9.1 运行现有测试套件（`cargo test`），确保无回归
- [ ] 9.2 手动验证：TUI 输入框 `@` 触发 skills 补全 → 选择 skill → 提交
- [ ] 9.3 手动验证：spawn subagent → 查看 subagent panel 完整时间线 → 查看 transcript detail view
- [ ] 9.4 手动验证：强制 subagent 失败（timeout/budget exceeded）→ 查看错误详情 → 重试
- [ ] 9.5 手动验证：RLM pipeline 中两个 subagent 产出冲突 claims → Aggregator 正确标记冲突
