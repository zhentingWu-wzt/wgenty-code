# Comet Subagent Progress Checkpoint - org-graph-contract-viewer

> 协调者恢复检查点。每次派发/回报/审查/勾选后立即更新。只保存协调状态，不替代 plan 或 tasks.md 复选框。

## 当前状态

- **当前 plan task**: Task 5 - `render_mermaid`（flowchart + classDef）
- **映射 OpenSpec task**: 2.5 + 3.4 + 3.2（tasks.md；3.2 在 Task 5 后全部四种格式覆盖，一并勾选）
- **阶段**: `implementing`（implementer 已派发，等待回报）
- **审查-修复轮次**: 0/3（Comet 上限 3 轮）

## Task 5 派发信息

- **派发时间**: 2026-08-11
- **BASE**: 09f01791a52df7227b19b2ed138da031153b5602
- **brief**: `.superpowers/sdd/2026-08-11-org-graph-contract-viewer/task-5-brief.md`
- **report**: `.superpowers/sdd/2026-08-11-org-graph-contract-viewer/task-5-report.md`（implementer 写入）
- **允许修改**: `src/org_graph/render.rs`（仅 - 替换 render_mermaid 桩 + mermaid_node_id/mermaid_class/format_mermaid_label 辅助 + 2 测试）
- **TDD**: 是
- **实现提交哈希**: （待回报）
- **变更文件**: （待回报）
- **RED/GREEN 证据**: （待回报）
- **已通过审查阶段**: 无
- **未解决 reviewer 反馈**: 无
- **reviewer 关注点**: render_mermaid 以 `flowchart LR` 开头；5 个节点定义 `["..."]:::<class>`；3 个 classDef（readonly/spawn/mutate）；can_spawn->spawn 类、否则 can_mutate_fs->mutate、否则 readonly；省略 system_prompt；纯函数。这是 render.rs 最后一个桩，替换后 render.rs 全部 4 格式真实实现。

## 任务唯一文本（用于定向勾选）

- OpenSpec tasks.md:
  - 2.5: `实现 \`render_mermaid\`：合法 mermaid 图定义，每个契约渲染为节点`
  - 3.4: `\`mermaid\` 输出以合法图类型声明开头，且五个契约均成为图中节点`
  - 3.2: `\`explore_readonly=true\` / \`false\` 时，Explore 与 Plan 的 \`can_mutate_fs\` 在所有四种格式中如实反映`（Task 5 完成后四种格式全部覆盖：table=Task3, dot=Task4, mermaid=Task5, json=Task2；一并勾选）

## 上一任务记录（Task 4，已完成）

- Task 4: `render_dot`；commit `2dad8a12`；双审查 clean Approved；progress `09f01791`

## 之前任务（已完成）
- Task 3: render_table；commit `c08db456`；progress `f2ba9322`
- Task 2: render.rs scaffold + render_json；commit `c042fbe0`；progress `ce322cd6`
- Task 1: NodeRegistry::iter()；commit `ded6d8c4`；progress `ab32fc1a`
