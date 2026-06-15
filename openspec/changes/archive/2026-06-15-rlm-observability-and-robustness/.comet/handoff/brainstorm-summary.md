# Brainstorm Summary

- Change: rlm-observability-and-robustness
- Date: 2026-06-15

## 确认的技术方案

### 1. TUI 输入框补全
- **方案 C**：内联 + 参数提示（混合式）— 补全列表出现在输入框上方，选定 skill 后内联展开参数提示（如 `<change-name>`）
- 触发：`@` → skill 补全，`/` → plugin command 补全
- 扫描源：`~/.claude/skills/` 目录 + `PluginRegistry.commands`
- 交互：↑↓/Tab 导航，Enter 确认，Esc 取消
- 新文件：`src/tui/components/completion_panel.rs`

### 2. Subagent 错误可视化与恢复
- **方案 A**：内联展开 — 选中 Failed 节点后原地展开错误详情 + 操作按钮（[r] retry，[d] details，[Esc] close）
- **方案 A**：时间线视图 DetailView — 左侧 header（状态/耗时/tokens/rounds） + 右侧滚动事件时间线（💭思考 / 🔧工具 / ✓结果 / 🏁完成，+Xs 时间偏移）
- **方案 B**：选择性回滚 — 只回滚出错步骤涉及的文件，保留之前成功的修改。需要跟踪每个工具调用的文件修改记录

### 3. Transcript 持久化
- **方案 A**：完成时批量写入 + 每 10 轮 checkpoint — 正常完成/失败时整批写入 SQLite，长任务每 10 轮 checkpoint 中间状态
- SQLite schema：`subagent_transcripts` + `subagent_events` 两张表
- 新模块：`src/transcript/`（`mod.rs` + `store.rs`）

### 4. 结构化归约
- **分层格式**：分析任务 → `structured-claims/1`（`{claim, evidence, confidence, conflicts_with}`），修改任务 → `unified-diff/1`
- **容错**：方案 A（分层降级）— 结构化解析 → regex 提取 → 保留原文 + `[unstructured]` 标记
- **Aggregator 合并**：Jaccard >0.8 去重，conflicts_with 冲突检测，同文件 diff 标记 write_conflict，仅冲突项 fallback LLM

### 5. 进展跟踪与预算
- **方案 A**：progress_delta 基于信息增益 — delta = 本轮新增工具调用类型数 / 已用类型总数，连续 3 轮 < 0.05 → NoProgress abort
- Per-subagent token budget：`task` 工具新增 `token_budget` 参数，每轮累计检查，超限 kill

## 关键取舍与风险

| 风险 | 缓解 |
|------|------|
| Jaccard 0.8 阈值可能误合并相似 claims | 阈值可配置，被合并项保留在 metadata |
| 选择性回滚可能留下不一致中间状态 | 仅回滚独立文件修改，不涉及跨文件依赖 |
| 10 轮 checkpoint 间隔可能丢失中间数据 | 关键事件（Failed/Completed）立即 flush |
| 补全面板增加渲染复杂度 | 复用 PermissionState 模式，不引入新渲染路径 |

## 测试策略

- 单元测试：CompletionEngine、claims/diff 解析、Jaccard 相似度、progress_delta、TranscriptStore CRUD、token budget
- 集成测试：补全触发链、subagent 完整生命周期、超时/budget 失败场景、RLM pipeline 冲突检测、选择性回滚
- 手动验证：TUI 补全交互、实时事件时间线、错误恢复流程、detail view 导航

## Spec Patch

无（brainstorming 确认现有 delta spec 覆盖充分）
