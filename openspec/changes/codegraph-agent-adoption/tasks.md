# Tasks — codegraph-agent-adoption

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图（关联 change 名）。
> 三层修复对应 D1 决策：层 A (Prompt)、层 B (Tool description)、层 C (错误反馈)。

## 1. 准备与基线确认

- [ ] 1.1 阅读 #0 archive：`openspec/changes/archive/2026-06-15-codegraph-baseline-spike/` 与 `scripts/codegraph-bench/root-cause-analysis.md`，确认根因 top 3 仍准确
- [ ] 1.2 在当前 prompt 上跑一次 14 条 nav-XXX.yaml 任务（基线复测），确认 codegraph 调用率仍在 0.05% 量级；若变化大需重新评估阈值
- [ ] 1.3 确认 `src/prompts/` 目录下所有 prompt 文件列表（base.md、collaboration.md 等），决定 D2 中 playbook 的最终位置

## 2. 层 A — Prompt 修改

- [ ] 2.1 修改 `src/prompts/base.md` 「Search」工具段落：在 grep 之前加入 `codegraph_node` 和 `codegraph_explore`，附简短说明
- [ ] 2.2 修改 `src/prompts/base.md` 「When to use each tool」表：将以下场景的推荐工具改为 codegraph
  - "Find where a function is defined" → `codegraph_node`
  - "Find callers of a function" → `codegraph_node`
  - "Find implementations of a trait" → `codegraph_explore`
  - "Understand module structure" → `codegraph_explore`
  - 保留 grep 作为兜底（"if codegraph index unavailable"）
- [ ] 2.3 在 base.md（或 build 阶段确认的位置）新增「代码导航 playbook」段落：明确 codegraph→grep→file_read 标准工作流和何时切换
- [ ] 2.4 commit 层 A：`feat(prompts): prioritize codegraph over grep for code navigation`

## 3. 层 B — Tool description 修改

- [ ] 3.1 修改 `src/tools/codegraph/tools.rs` 中 `codegraph_node` 的 `description()` 函数，按 D3 格式重写：
  - 首句：保持功能描述
  - 第二句：`PREFER FOR: finding symbol definitions, listing callers/callees, finding references.`
  - 第三句：`AVOID WHEN: searching for text patterns or non-symbol concepts (use grep instead).`
- [ ] 3.2 修改 `src/tools/codegraph/tools.rs` 中 `codegraph_explore` 的 `description()` 函数，按 D3 格式重写：
  - 首句：描述探索能力（symbols + relationships）
  - 第二句：`PREFER FOR: exploring module structure, browsing call graphs across multiple symbols, understanding cross-module relationships.`
  - 第三句：`AVOID WHEN: looking up a single known symbol (use codegraph_node) or searching text patterns (use grep).`
- [ ] 3.3 commit 层 B：`feat(codegraph): add scenario-based guidance to tool descriptions`

## 4. 层 C — 错误文案优化

- [ ] 4.1 修改 `src/tools/codegraph/tools.rs:get_engine()` 中 "No codegraph index found" 的 ToolError message：按 D4 改为：
  ```
  No codegraph index found at .codegraph/index.db. To enable: run 'wgenty-code codegraph index' in this directory, then retry. Falling back to grep is acceptable for now.
  ```
- [ ] 4.2 commit 层 C：`feat(codegraph): improve lazy-init error message with actionable guidance`

## 5. Spec 同步（modified capabilities）

- [ ] 5.1 verify 阶段 OpenSpec 归档时按 delta 语义同步 modified specs 到主 spec（symbol-query / call-graph / codegraph-lazy-init）。本阶段不直接编辑主 spec。

## 6. 评测回归脚本

- [ ] 6.1 brainstorming D5 中的 repl 自动化方式（repl + expect / daemon API / 人工跑），写入 design.md 的 Open Questions 解答
- [ ] 6.2 实现 `scripts/codegraph-bench/bench-agent-replay.sh`：
  - 读取 `agent-tasks/nav-*.yaml` 列表
  - 对每条任务调用 wgenty-code（按 6.1 选定方式）
  - 等待 session 写入 `~/.wgenty-code/sessions/`
  - 调用 `bench-agent.sh --session <new>` 提取工具序列
- [ ] 6.3 扩展脚本输出按 task category 分层聚合：strong_categories（definition_lookup / reference_lookup / impl_enumeration）vs other_categories
- [ ] 6.4 输出 JSON 报告 `results/<ts>/agent-replay.json`，含每条任务工具序列 + 分层统计 + 与 #0 基线对比

## 7. 验收测试

- [ ] 7.1 在新 prompt + 新 description 下跑一次 `bench-agent-replay.sh`，记录分层数据
- [ ] 7.2 验证强项类（nav-001/002/003/004/007/008） ≥ 60% 使用 codegraph
- [ ] 7.3 验证其他类（nav-005/006/009-014） ≥ 25% 使用 codegraph
- [ ] 7.4 验证 grep/file_read/glob 仍在合适场景被调用（未"过度纠正"到 codegraph 但场景不对）
- [ ] 7.5 跑 cargo build 确认无回归；跑现有相关测试

## 8. 不破坏现有功能验证

- [ ] 8.1 抽查 3-5 条非代码导航 session（如配置修改、文件读取），确认行为不变
- [ ] 8.2 验证 codegraph 索引未建时新错误文案出现，且 Agent 能 fallback 到 grep
- [ ] 8.3 `git diff --stat` 确认改动范围符合预期（仅 src/prompts/、src/tools/codegraph/tools.rs、scripts/codegraph-bench/）

## 9. 验证与归档

- [ ] 9.1 运行 `openspec validate codegraph-agent-adoption` 校验
- [ ] 9.2 进入 `/comet-verify`，按 spec scenarios 逐项核对
- [ ] 9.3 verify 通过后进入 `/comet-archive`，归档到 `openspec/changes/archive/`，同步 modified specs 到主 spec
