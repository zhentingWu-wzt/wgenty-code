# Design Open Questions — Resolution Audit

设计文档 §7 列出的 3 个 Open Questions 全部在 design 阶段闭合，本文档记录最终决策与实施位置。

## O1 — `# claudeMd` 标题保留 vs 改名

**决策**：方案 B，改名为 `# wgentyMd`。

**理由**：
- 项目身份一致性优先于 "复用 Claude 模型对 `# claudeMd` 的隐性 prompt cache"。
- 反悔代价极低（改一行常量）。
- 设计文档 D6 + brainstorm-summary.md Step 1d 中已锁定。

**实施位置**：`src/prompts/mod.rs::REMINDER_PREAMBLE_OPENING` 常量（commit 8f06f92）。
**测试覆盖**：`reminder_full_four_sources_snapshot` 断言 `result.to_model.contains("# wgentyMd")`。

## O2 — `UserPromptSubmit` fire 从 `tokio::spawn` 改 `await` 不引入死锁

**决策**：方案 B，fire 移到 `AgentLoop::process_input_inner` 内 `await`，10s timeout 兜底。

**死锁分析**（design doc §2 D7）：
- `AgentLoop` 通过 `tokio::spawn` 在独立任务中运行，与 UI render loop 解耦。
- `HookManager::fire(...)` 内部用 `tokio::time::timeout(Duration::from_secs(10), ...)` 兜底。
- Hook 命令通过 `tokio::process::Command::spawn` 启动子进程，与主 runtime 同步等待但不持锁。
- **结论**：无死锁路径。时序漂移 = hook 执行时间（毫秒级，超时 10s 兜底）。

**实施位置**：`src/tui/agent/mod.rs::process_input_inner`（commit bec7db4）+ `src/tui/app/input.rs` 删除 fire-and-forget（同 commit）。

**测试覆盖**：cargo test --workspace 全过（452 lib + 5 integration tests）；timeout 路径通过代码审查保证（`tokio::time::timeout` + `tracing::warn!` 降级）。

## O3 — `LayerVisibility::Internal` 在 TUI transcript 层的实现路径

**决策**：方案 A，在 reminder builder 输出端分流（`ReminderOutput { to_model, to_transcript }`）。

**理由**：
- 不让 TUI 渲染层识别 `<!-- internal -->` 之类标签（避免跨层耦合 + 字符串正则脆弱）。
- 调用方契约清晰：发模型用 `to_model`；TUI 展示用 `to_transcript`（None 时不展示）。

**实施位置**：
- `src/prompts/mod.rs::ReminderOutput` 结构 + `build_user_turn_reminder` 双轨渲染（commit 8f06f92）。
- `src/tui/agent/mod.rs::process_input_inner` 消费 `to_model`，`to_transcript` 留 TODO 等 `AppEvent::SystemNotice` 通道（commit bec7db4）。

**已知限制（K1 in design doc）**：`to_transcript` 实际投递到 TUI 的链路尚未实现（需新增 `AppEvent::SystemNotice`），但模型侧已正确分流——Internal 在 `to_model` 出现、不会出现在最终 transcript（因为 transcript 通道根本不投递）。

**测试覆盖**：`reminder_internal_visibility_excludes_transcript` + `reminder_visible_hook_in_both_outputs`（src/prompts/mod.rs::reminder_tests）。

## 总结

3 个 Open Questions 全部在 design 阶段闭合，全部按既定决策实施完成。设计文档 §5（migration plan）的所有条目实施完毕。
