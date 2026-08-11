# Comet Subagent Progress Checkpoint - org-graph-contract-viewer

> 协调者恢复检查点。每次派发/回报/审查/勾选后立即更新。只保存协调状态，不替代 plan 或 tasks.md 复选框。

## 当前状态

- **当前 plan task**: Task 6 - CLI 接线（`org-graph contracts` 命令）
- **映射 OpenSpec task**: 4.1 + 4.2 + 4.3（tasks.md）
- **阶段**: `implementing`（implementer 已派发，等待回报）
- **审查-修复轮次**: 0/3（Comet 上限 3 轮）

## Task 6 派发信息

- **派发时间**: 2026-08-11
- **BASE**: 2c9e8abdd97e7b3868fd1c192277c0d4b4d8cd5c
- **brief**: `.superpowers/sdd/2026-08-11-org-graph-contract-viewer/task-6-brief.md`
- **report**: `.superpowers/sdd/2026-08-11-org-graph-contract-viewer/task-6-report.md`（implementer 写入）
- **允许修改**: `src/cli/org_graph.rs`（新建）+ `src/cli/mod.rs`（OrgGraph 变体 + OrgGraphCommands 枚举 + pub mod org_graph）+ `src/cli/args.rs`（run_async match arm）
- **TDD**: 是
- **实现提交哈希**: （待回报）
- **变更文件**: （待回报）
- **RED/GREEN 证据**: （待回报）
- **已通过审查阶段**: 无
- **未解决 reviewer 反馈**: 无
- **reviewer 关注点**:
  - `OrgGraphFormatArg` 派生 `clap::ValueEnum`（cli 侧，非 org_graph 侧），实现 `From<OrgGraphFormatArg> for Format` 映射
  - `Commands::OrgGraph { action: OrgGraphCommands }` 变体 + `OrgGraphCommands::Contracts { format }` 子命令
  - `run_async` 新增 `OrgGraph` match arm
  - handler `run()` 用 `state.settings.agent.subagent` 构造 `NodeRegistry::builtin()` → `render()` → 打印 stdout
  - `--format` 缺省 `table`
  - cargo build + cargo test 通过，零回归
  - 注意 brief 文件头说改 main.rs，但实际接线点在 args.rs 的 run_async（brief Step 3(c) 明确说改 args.rs）

## 任务唯一文本（用于定向勾选）

- OpenSpec tasks.md:
  - 4.1: `在 \`src/cli/mod.rs\` 新增顶层 \`OrgGraph\` 命令组与 \`Contracts\` 子命令，带 \`--format\` value_enum（默认 \`table\`）`
  - 4.2: `命令处理逻辑：加载 \`SubagentLimits\` 配置 → 构造 \`NodeRegistry::builtin()\` → \`render()\` → 打印到 stdout`
  - 4.3: `在 \`main.rs\` 命令分派处接线 \`Commands::OrgGraph { action: OrgGraphCommands::Contracts { format } }\``（注：实际分派在 args.rs run_async，不在 main.rs）

## 上一任务记录（Task 5，已完成）

- Task 5: `render_mermaid`；commit `de674eec`；双审查 clean Approved；progress `2c9e8abd`
- 所有 4 个渲染函数现在都是真实实现，无残留桩

## 协调者勾选记录

- tasks.md 3.1 由协调者勾选（Task 2 测试 `render_json_roundtrips_to_identical_contracts` 已覆盖：render.rs:225 parsed == original field-exact）

## 之前任务（已完成）
- Task 4: render_dot；commit `2dad8a12`；progress `09f01791`
- Task 3: render_table；commit `c08db456`；progress `f2ba9322`
- Task 2: render.rs scaffold + render_json；commit `c042fbe0`；progress `ce322cd6`
- Task 1: NodeRegistry::iter()；commit `ded6d8c4`；progress `ab32fc1a`
