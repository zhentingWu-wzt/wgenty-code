## Context

wgenty-code 已有一套基于 tree-sitter 的 Rust codegraph 实现（`src/tools/codegraph/`，1777 行），覆盖 5 个 capability：`code-indexing`、`symbol-query`、`call-graph`、`codegraph-mcp`、`codegraph-lazy-init`。后续规划了三个改进 change（`codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph`），但当前缺乏量化基线。本 change 是这三个改进 change 的前置 spike。

现状关键事实：
- tree-sitter 已挂载，但仅 `tree-sitter-rust 0.24`
- `Symbol` / `Reference` / `Relationship` / `Confidence` 模型已存在；关系仅 `calls` / `implements` / `contains` / `imports` 四种
- 索引持久化在 SQLite (`.codegraph/index.db`)
- `codegraph_node` 和 `codegraph_explore` 已通过 MCP 暴露
- 系统 prompt（`src/prompts/base.md`）仍把 grep 列为代码搜索首选；codegraph 工具描述在 `src/tools/codegraph/tools.rs` 中较中性，没有"何时优先用"的强引导
- 现有 capability 中 `Confidence` 字段定义了但实际产出薄弱

约束：spike 不修改业务源码、不修改 prompt、不修改既有 spec；产物为测量套件 + 量化报告；**不在本阶段实施修复方案**。

## Goals / Non-Goals

**Goals:**

- 建立一组可重跑的测量脚本，可在 wgenty-code 自身和外部 Rust 项目上产出一致格式的结果
- 量化三组数据：性能（耗时/体积/延迟）、覆盖率（文件/符号/关系/解析失败）、Agent 使用率（标准任务集上的工具调用分布）
- 给出"Agent 不主动使用 codegraph"的根因 top 3，每条带可追溯证据
- 产出一份 baseline 报告，包含「基线值 vs 建议目标值」对照表，作为后续 3 个 change 的输入

**Non-Goals:**

- 不修改 codegraph 任何业务代码、prompt 或 capability spec
- 不实施任何修复（修复归后续 3 个 change）
- 不构建 LSP / 编译器级别的测量基础设施
- 不引入大型新依赖（hyperfine 等可选；优先系统工具）
- 不在本 spike 中跨语言（多语言归 `codegraph-multilang-and-deep-graph`）

## Decisions

### D1：测量目录与业务代码隔离

**决策**：测量代码集中放在 `scripts/codegraph-bench/`，不在 `src/`、`crates/` 或 `tests/` 下。

**理由**：
- 严格遵守"不修改业务源码"约束
- 后续 3 个 change 各自迭代时，这个目录可作为统一的 regression 基线
- 单独目录便于通过 `git diff --name-only` 验证范围合规

**替代方案**：
- 放在 `tests/bench/` —— 拒绝。会被 `cargo test` 自动发现，可能误触发或误判失败
- 用 `cargo bench` —— 拒绝。`criterion` 仅适合微基准，难以表达索引/查询/agent 三种异质测量

### D2：测量脚本以 shell 为主，必要时辅以小型 Rust 二进制

**决策**：性能/覆盖率测量用 bash + jq + sqlite3 即可；Agent 使用率测量需要驱动 agent，可能需要小型 Rust 二进制（或调用 wgenty-code 现有 CLI）。

**理由**：
- bash 不需要新编译，迭代快
- 性能测量本质是计时和读 SQLite，shell 足够
- Agent 驱动可能复用 `wgenty-code run --task <file>` 之类已有入口（build 阶段确认）

**替代方案**：
- 全部用 Rust 写 —— 拒绝。增加编译时间、迭代慢、且需要把 spike 代码加进 workspace
- 全部用 Python —— 可选，但 wgenty-code 自身不依赖 Python，引入 Python 会增加运行环境约束

**Open**：Agent 驱动的具体技术路径（CLI 回放 vs transcripts 静态分析 vs 临时打点）由 build 阶段 brainstorming 决定。

### D3：测量结果分两层产出

**决策**：原始数据 → `scripts/codegraph-bench/results/<timestamp>/`；最终报告 → `docs/superpowers/specs/<YYYY-MM-DD>-codegraph-baseline-report.md`。

**理由**：
- 原始数据留痕，方便事后核查或重新统计
- 最终报告进入 docs 目录，归档时与三个后续 change 的 design doc 同处，方便引用
- 报告与原始数据分离，避免报告本身被自动覆盖

### D4：标准任务集存放与扩展

**决策**：标准代码导航任务集放在 `scripts/codegraph-bench/agent-tasks/`，每个任务一个文件，README 说明格式。

**理由**：
- 文件级粒度便于增删
- 后续 change 在验收时也可复用该任务集
- 任务集可在 build 阶段先放种子任务（10 条），后续 change 可追加

### D5：外部仓库测量目标

**决策**：外部仓库测量至少跑通一个，候选：`ripgrep` 或同等规模 Rust crate（万行级、无 build.rs 重型操作）。

**理由**：
- 验证脚本不绑死 wgenty-code 自身
- ripgrep 是社区熟知的稳定 baseline
- 允许 build 阶段根据下载/索引耗时选择更合适的目标

### D6：报告日期使用生成日，不写动态文案

**决策**：报告文件名和正文中的日期均为生成时的实际日期（如 `2026-06-15`），由脚本在生成时填充；正文不写"今天"、"近期"等相对时间。

**理由**：
- 归档后报告会被三个后续 change 引用，写动态时间会引入歧义
- 生成日是事实信息，未来回看时可对应到当时的环境指纹

## Risks / Trade-offs

- **[风险] Agent 使用率测量方法选择不当导致数据失真** → 在 build 阶段先用 brainstorming 比较 transcripts 静态分析 / CLI 回放 / 临时打点三条路径，选成本与可信度最优解；报告中说明所选方法的局限
- **[风险] 测量脚本在外部仓库跑不通（依赖 wgenty 内部假设）** → 验收 S5 强制要求外部仓库跑通；用 `--target` 显式参数化目标路径，避免硬编码
- **[风险] 性能基线波动大，目标值难以制定** → 接受 ±20% 波动窗口；同一指标采多次取中位数 + p95；建议目标值用相对值（如"减少 50%"）而非绝对值
- **[风险] "标准任务集"代表性不足** → 在 build 阶段用 brainstorming 推导任务来源（issue 历史、典型 PR、典型 grep 行为），并在报告中明确标注"任务集仅 N 条，外推需谨慎"
- **[Trade-off] 不引入新 Cargo 依赖 vs 测量精度**：性能测量精度受限于 shell `time` 的毫秒级精度。Rust 索引耗时通常秒级，毫秒精度足够；查询延迟若进入毫秒级再考虑用 hyperfine（可选依赖）
- **[Trade-off] 不修改 src/ vs Agent 使用率精确测量**：如果通过临时打点是最优路径，则需要在 src/ 加打点；本 spike 拒绝，转用 transcript 分析或 CLI 回放。代价是数据更粗，但符合范围约束。若 build 阶段确认"不打点不可行"，需重新评估范围（可能升级为先做"打点 spike → 测量 spike"两步）

## Migration Plan

不适用 —— spike 仅新增 `scripts/codegraph-bench/` 和 `docs/superpowers/specs/<date>-codegraph-baseline-report.md`，无运行时迁移、无回滚需求。归档时报告进入 docs，脚本继续保留以便后续 change 重跑对比。

## Open Questions

- **Agent 驱动技术路径**：transcripts 静态分析 vs CLI 回放 vs 临时打点 —— 由 build 阶段 brainstorming 决定（D2）
- **外部仓库选择**：ripgrep 是否合适？是否需要测试两个不同规模的仓库？由 build 阶段确定（D5）
- **标准任务集来源**：从哪里抽取代表性的"代码导航"任务？issue 历史 / 典型 PR / 工程师经验访谈？由 build 阶段确定
- **稳定性窗口阈值**：±20% 是否合理？还是按指标分别定？由 build 阶段确定
- **报告"建议目标值"如何制定**：是基于行业基准、基线乘系数、还是后续 change 反向制定？建议在 build 阶段先列规则、再填表
