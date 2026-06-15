# Brainstorm Summary

- Change: codegraph-baseline-spike
- Date: 2026-06-15
- 状态：5 个 OQ 全部确认 ✅

## 进度

- [x] OQ1 — Agent 驱动技术路径
- [x] OQ2 — 外部仓库选择
- [x] OQ3 — 标准任务集来源
- [x] OQ4 — 稳定性窗口阈值
- [x] OQ5 — "建议目标值"制定规则

## 已确认决策

### OQ1 — Agent 驱动技术路径：B 主 + A 辅

**决策**：双轨并行
- **路径 B（核心）**：用 `wgenty-code query --prompt "..."` 非交互入口（已确认存在于 `src/cli/mod.rs:64-68`），回放标准任务集（≥12 条），每次记录工具调用序列。产出主指标。
- **路径 A（辅助）**：扫描 `~/.wgenty-code/sessions/<uuid>.json`（已有 71 个历史 session）做静态分析，作为根因 top 3 的可追溯证据来源。
- **舍弃**：路径 C（MCP 客户端模拟）— 绕过 agent 决策行为，不能回答"agent 是否选择用 codegraph"。

**输出位置**：
- `results/<ts>/agent.json`：B 路径输出（标准任务集结果）
- `results/<ts>/transcript-analysis.json`：A 路径输出（历史 session 统计）

**关键技术依据**：
- `src/cli/mod.rs:64-68` 已有 `Query { prompt }` 子命令 + 全局 `--no-interactive` flag
- `src/context/memory_session.rs:93` 把 session 存到 `~/.wgenty-code/sessions/<uuid>.json`
- 仓库当前已有 71 个历史 session 文件可直接分析

**待 build 阶段确认**：
- B 路径在 `wgenty-code query` 输出中能否直接拿到工具调用序列，还是需要解析 session 文件
- B 跑 12 条任务的 API token 成本（先 2-3 条试跑）
- A 路径需要的 session 字段是否稳定（先看一个 session 文件的 schema）

### OQ2 — 外部仓库选择：ripgrep 必选 + tokio 可选

**决策**：分级
- **必选**：ripgrep（~30K 行 Rust，社区 baseline，下载快）→ 满足 spec S5 硬性要求
- **可选**：tokio（~100K 行，async/macro 重度使用，规模压力测试）→ 时间充裕时跑；跑不动则标记"未完成"，不阻塞 spike 完成
- **舍弃**：tree-sitter（含大量 C 代码会让覆盖率指标失真）

**待 build 阶段确认**：
- ripgrep 克隆来源（GitHub mirror、最新 release tag 还是 main HEAD）
- 在 tasks.md 5.2 中明确"tokio 跑不通时记录原因，不视为 task 失败"

### OQ3 — 标准任务集来源：工程师手写 ≥12 条（6 类）

**决策**：路径 b — 工程师手写
- **数量**：≥12 条（6 类 × 2 条；可加 1-2 条复合任务）
- **6 类**（覆盖 codegraph 现有能力）：
  1. **definition_lookup** — 定义查找（"Tool trait 定义在哪？"）
  2. **reference_lookup** — 引用查找（"谁调用了 register？"）
  3. **call_chain** — 调用链探索（"run_async 的调用链？"）
  4. **impl_enumeration** — 实现枚举（"Tool trait 谁实现了？"）
  5. **module_structure** — 模块结构（"src/tools/codegraph 有什么？"）
  6. **cross_module_path** — 跨模块路径（"events 怎么流到 tui？"）
- **任务文件格式**（YAML，便于 jq 解析）：

  ```yaml
  task_id: nav-001
  category: definition_lookup
  prompt: "Tool trait 定义在哪个文件，长什么样？"
  expected_answer_anchor:
    file: src/tools/mod.rs
    contains: "trait Tool"
  ```

- **舍弃**：
  - 路径 a（issue/PR 抽取）：项目早期 issue 少，ROI 低
  - 路径 c（自动模板生成）：模板化 prompt 偏离真实人类提问方式

**待 build 阶段确认**：
- prompt 用中文还是英文（建议中英各 6 条以避免语言偏置）
- 任务跑两遍取众数 vs 跑一遍 — 一致性如何处理

### OQ4 — 稳定性窗口阈值：分层阈值

**决策**：按指标本征波动分三层
- **恒定指标（±1%）**：索引体积、文件数、按 SymbolKind 的符号数、按 RelKind 的关系数 — 同一 commit 应近乎完全一致
- **中等波动（±20%）**：全量索引耗时（中位数）、查询延迟（中位数）、agent 调用率
- **高波动（±50%）**：增量索引耗时（少量文件）、单次查询延迟、agent token 数 — 只看趋势不看绝对值

**报告呈现**：以表格形式展示「指标 × 阈值层 × 阈值数值」，避免读者反复问"这个为什么超阈值"。

**舍弃**：
- 选项 1（统一 ±20%）：对恒定指标过宽、对高波动指标过严
- 选项 3（不设阈值，只报波动等级）：失去机械判定能力，不利后续 change 验收

### OQ5 — "建议目标值"制定规则：b 为主 + a 抽样 + c 补充

**决策**：三层混合
- **主规则（b）基线乘系数**：所有指标都给基于基线的目标值，例如：
  - agent codegraph 调用率（#1）：≥ 基线 + 30 个百分点
  - codegraph_node p95 延迟（#2）：≤ 基线 × 1.5（增加可解释性后允许变慢）
  - 全量索引耗时（#3）：≤ 基线 × 1.5（多语言成本可控）
  - 多语言覆盖率（#3）：≥ 70%（Java/Python 样例项目）
- **a 抽样**：仅对 2-3 个关键指标（索引耗时、查询延迟）做行业对标作为**参考列**（不强制），数据来源：rust-analyzer / sourcegraph 公开 benchmark
- **c 补充**：每个目标值后面括号标注"by #1/#2/#3"，给后续 change 留 1 次"在 design 阶段调整该目标值（不能放宽超过 ±20%）"的口子

**对照表样例**：

| 指标 | 基线 | 建议目标 | 行业对标（参考） | 验证方法 | 归属 change |
|------|------|----------|------------------|----------|-------------|
| agent codegraph 调用率 | X% | ≥ X% + 30pp | — | 重跑标准任务集 | #1 |
| codegraph_node p95 延迟 | Yms | ≤ Y × 1.5 | rust-analyzer goto-def: Zms | 重跑查询集 | #2 |
| 全量索引耗时（中位数） | T | ≤ T × 1.5（含 Java/Python） | sourcegraph: T' | bench-perf.sh | #3 |
| 多语言覆盖率（Java/Python） | 0% | ≥ 70% | — | 在样例项目跑覆盖率 | #3 |

## Spec Patch（待回写到 specs/codegraph-baseline-bench/spec.md）

5 个 OQ 的决策需要回填到 delta spec：

1. **Requirement 4（Agent 使用率基线测量）**：增加 Scenario 明确 B 路径用 `wgenty-code query --prompt`、A 路径扫 `~/.wgenty-code/sessions/`
2. **Requirement 4**：增加 Scenario 明确任务集 ≥12 条 + 6 类 + YAML 格式 + expected_answer_anchor
3. **Requirement 5（基线报告产出）**：将"后续 change 基线 vs 目标对照表"的字段定义补充为 6 列（指标/基线/建议目标/行业对标/验证方法/归属 change）
4. **Requirement 8（测量结果可重复）**：把"±20%"细化为分层阈值（恒定 ±1%、中等 ±20%、高波动 ±50%）

新增 Requirement（如有必要）：
- 可能新增一条「外部仓库验证」requirement 明确 ripgrep 必选 + tokio 可选

## 测试策略概要

- **单元层**：测量脚本本身相对简单（shell + jq + sqlite3），主要靠 dry-run + fixture 数据测试，例如准备一个 mini fixture 索引验证 coverage 脚本输出正确
- **集成层**：在 wgenty-code 自身仓库跑 `run-all.sh` 至少 2 次（含可重复性验证）
- **外部验证层**：ripgrep 上跑 perf + coverage（agent 测量在 wgenty-code 自身仓库跑就行）
- **报告内容验证**：用 markdown linter + 自定义 grep 检查必含章节存在
