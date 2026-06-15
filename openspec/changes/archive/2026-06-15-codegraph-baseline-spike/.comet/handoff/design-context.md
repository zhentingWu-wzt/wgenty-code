# Comet Design Handoff

- Change: codegraph-baseline-spike
- Phase: design
- Mode: compact
- Context hash: db49c3e4bd1b8827203debb18a474dc5edcbf505b324343a49d44a288989df80

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/codegraph-baseline-spike/proposal.md

- Source: openspec/changes/codegraph-baseline-spike/proposal.md
- Lines: 1-31
- SHA256: 50d2073f420132417caf6a695ecb39b5979bd24efa1703ee2771f9f7979b90ae

```md
## Why

wgenty-code 已经具备一套基于 tree-sitter 的 Rust codegraph（索引、符号查询、调用图、MCP 暴露），但用户反馈三个核心痛点：Agent 不主动用 codegraph、索引覆盖/速度不足、查询结果不够好用、缺乏可解释性。后续计划通过三个独立 change（`codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph`）逐层改进，但当前**没有任何量化基线**——既不知道现在的索引耗时/体积/查询延迟，也不知道 Agent 在代码导航任务中实际有多少比例用了 codegraph，更不知道"不用"的真实根因是什么。在没有基线的前提下立项三个 change，会让"提升速度"、"提升使用率"沦为模糊目标，验收阈值无据可依，归档时无法回看 ROI。

本 change 是后续 3 个 change 的**前置 spike**：用最小成本（不动业务源码、不改 prompt、不改 spec）拿到一组可信的量化数据 + 一份"Agent 不用 codegraph 的根因 top 3"，作为后续 change 的输入和验收阈值来源。

## What Changes

- 新增基线测量套件 `scripts/codegraph-bench/`：性能、覆盖率、Agent 使用率三组测量脚本（shell 或最小 Rust 二进制，独立目录与业务源码隔离）
- 新增基线报告 `docs/superpowers/specs/<date>-codegraph-baseline-report.md`，至少包含：性能基线、覆盖率基线、Agent 使用率基线、"不用 codegraph"根因 top 3、后续 3 个 change 的"基线值 vs 建议目标值"表
- 在 wgenty-code 自身仓库 + 至少 1 个外部 Rust 项目（如 ripgrep 或同等规模 crate）上跑通测量脚本，验证脚本不绑死本仓库
- 测量脚本可重复执行（`bash scripts/codegraph-bench/run-all.sh` 或等价入口），后续 change 验收时可重跑对比

## Capabilities

### New Capabilities

- `codegraph-baseline-bench`：可重复执行的 codegraph 基线测量套件 + 量化报告。该 capability 规定测量脚本的入口、覆盖范围（性能、覆盖率、Agent 使用率）、测量产物（基线报告必含字段）、可重复性约束（同一仓库重跑结果可比对）以及外部仓库适配性。

### Modified Capabilities

无。本 change 不修改任何现有 codegraph 相关 capability 的 spec 行为；`code-indexing`、`symbol-query`、`call-graph`、`codegraph-mcp`、`codegraph-lazy-init` 在测量过程中只作为"被观测对象"，requirements 不变 — 它们的修改属于后续 3 个 change。

## Impact

- **新增文件**：`scripts/codegraph-bench/`（测量脚本）、`docs/superpowers/specs/<date>-codegraph-baseline-report.md`（报告）
- **不修改的代码**：`src/` 下所有源码、`src/prompts/` 下所有 prompt、`openspec/specs/` 下所有 capability spec
- **可能修改的依赖**：仅当测量必需时引入轻量依赖（如 hyperfine、jq、time），优先用系统自带工具
- **运行影响**：测量过程会在 `.codegraph/` 下生成临时索引；测量完成后清理或保留视脚本设计而定（在 design 阶段决定）
- **下游 change 依赖**：`codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph` 三个 change 在各自 design 阶段会引用本 spike 报告作为基线和目标值来源
- **风险**：低 — 不修改业务代码；最坏情况下报告产出延迟，但不阻塞当前 codegraph 的使用
```

## openspec/changes/codegraph-baseline-spike/design.md

- Source: openspec/changes/codegraph-baseline-spike/design.md
- Lines: 1-116
- SHA256: 152272b3f82a6d426952432f9ad7b181500d4cdb45e327083af3c096bd9ec319

[TRUNCATED]

```md
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
```

Full source: openspec/changes/codegraph-baseline-spike/design.md

## openspec/changes/codegraph-baseline-spike/tasks.md

- Source: openspec/changes/codegraph-baseline-spike/tasks.md
- Lines: 1-78
- SHA256: 7e58d04a55fc66a627846a7bec5799d020d8e537b494c4ca8c497ceb8a2294d1

```md
# Tasks — codegraph-baseline-spike

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图（关联 change 名）。
> 每个分组对应 design.md / spec.md 的一段产出，序号即执行顺序。

## 1. 测量脚手架

- [ ] 1.1 创建 `scripts/codegraph-bench/` 目录及 README，说明套件用途、入口、目录约定
- [ ] 1.2 实现 `run-all.sh` 入口脚本：解析 `--target`、`--output`、`--repeats` 参数，调度三组测量
- [ ] 1.3 实现 `lib/env-fingerprint.sh`：采集 OS、CPU 核数、`wgenty-code --version`、目标 commit hash，写入 `env.json`
- [ ] 1.4 编写"缺失二进制"兜底逻辑：找不到 `wgenty-code` 时退出码非零并打印安装建议（对应 spec 「测量套件入口/Scenario: 缺少 codegraph 二进制时的兜底」）
- [ ] 1.5 增加 `.gitignore` 规则：忽略 `scripts/codegraph-bench/results/` 下的运行产物，但保留 README 和 `.gitkeep`

## 2. 性能基线测量

- [ ] 2.1 实现 `bench-perf.sh`：全量索引计时（≥5 次，每次前清空 `.codegraph/`），输出每次的 wall-clock 秒数
- [ ] 2.2 扩展 `bench-perf.sh`：增量索引计时（修改 1/10/100 个 `.rs` 文件，各 ≥3 次）
- [ ] 2.3 扩展 `bench-perf.sh`：记录 `.codegraph/index.db` 字节数与目标项目 `.rs` 总字节数
- [ ] 2.4 实现 `bench-query.sh`：从 fixture 文件读取 ≥20 条查询，分别测 `codegraph_node` 与 `codegraph_explore` 端到端耗时
- [ ] 2.5 输出统一 JSON：性能数据写入 `results/<timestamp>/perf.json`，含 raw samples + 中位数 + p95
- [ ] 2.6 在 wgenty-code 自身仓库跑通 perf 测量，把首份原始数据存档备查

## 3. 覆盖率基线测量

- [ ] 3.1 实现 `bench-coverage.sh`：通过 `find` 统计目标项目 `.rs` 总文件数，与 `.codegraph/index.db` 中已索引文件数对比
- [ ] 3.2 扩展 `bench-coverage.sh`：用 `sqlite3` 查询符号按 `SymbolKind` 分组、关系按 `RelKind` 分组的总数
- [ ] 3.3 扩展 `bench-coverage.sh`：捕获 `wgenty-code codegraph index` 输出中的 parse 失败计数与文件，归类失败原因 top 3
- [ ] 3.4 输出统一 JSON：覆盖率数据写入 `results/<timestamp>/coverage.json`
- [ ] 3.5 在 wgenty-code 自身仓库跑通 coverage 测量，确认输出字段齐全

## 4. Agent 使用率基线测量

- [ ] 4.1 brainstorming 选择 Agent 驱动路径（transcripts 静态分析 / CLI 回放 / 临时打点），写入 design.md 的 Open Questions 解答；本步骤是 build 阶段的关键决策点，不得自动跳过
- [ ] 4.2 实现 `agent-tasks/` 目录与 README，定义任务文件格式（YAML 或 JSON 均可）
- [ ] 4.3 编写 ≥10 条种子代码导航任务（覆盖：定义查找、引用查找、调用链探索、impl 列举等）
- [ ] 4.4 实现 `bench-agent.sh`：按 4.1 选定的路径，对每条任务记录工具调用序列
- [ ] 4.5 输出统一 JSON：agent 数据写入 `results/<timestamp>/agent.json`，含每条任务的工具序列、是否使用 codegraph、失败/兜底标记
- [ ] 4.6 在 wgenty-code 自身仓库跑通 agent 测量

## 5. 外部仓库验证

- [ ] 5.1 选定外部测试仓库（候选：ripgrep）；克隆到本地或文档说明克隆步骤
- [ ] 5.2 在外部仓库执行 `bash <wgenty-path>/scripts/codegraph-bench/run-all.sh --target <repo>`，确认 perf + coverage 跑通
- [ ] 5.3 把外部仓库结果归档到 `results/<timestamp>-external/`，并记录环境差异（OS/工具版本）
- [ ] 5.4 解决脚本中任何"绑死本仓库"的硬编码（路径、配置、假设）

## 6. 根因分析

- [ ] 6.1 汇总 4.x 输出的工具调用数据，识别"agent 没用 codegraph"的高频模式
- [ ] 6.2 对每个候选根因抽取 ≥1 条可追溯证据（transcript 引用 + 行号、prompt 段落 + 文件:行）
- [ ] 6.3 输出根因 top 3 候选清单，标注影响面（哪类任务）和建议归属 change（C/B/A）
- [ ] 6.4 与用户审视根因清单，确认 top 3（决策点：用户确认后才能写入最终报告）

## 7. 基线报告产出

- [ ] 7.1 设计报告骨架（章节顺序：环境指纹 → 性能 → 覆盖率 → Agent 使用率 → 根因 → 后续 change 对照表）
- [ ] 7.2 实现 `gen-report.sh`：从 `results/<timestamp>/` 各 JSON 拼装出 Markdown 报告
- [ ] 7.3 报告日期填充为生成日期，写入 `docs/superpowers/specs/<YYYY-MM-DD>-codegraph-baseline-report.md`
- [ ] 7.4 填写「后续 change 基线 vs 目标对照表」：为 codegraph-agent-adoption / codegraph-query-and-explainability / codegraph-multilang-and-deep-graph 各列出 ≥1 条「指标 / 基线 / 目标 / 验证方法」
- [ ] 7.5 报告中明确"建议目标值"的制定规则（决策点：与用户确认规则后再填表）

## 8. 可重复性验证

- [ ] 8.1 在 wgenty-code 自身仓库跑两次完整 `run-all.sh`（间隔 ≥5 分钟），对比性能基线中位数差异
- [ ] 8.2 若差异超出脚本声明的稳定性窗口（默认 ±20%），输出告警；否则记录稳定性结论到报告
- [ ] 8.3 把两次结果保留在 `results/` 下，作为可复现性证据

## 9. 范围合规验证

- [ ] 9.1 执行 `git diff --name-only main...` 确认改动仅出现在 `scripts/codegraph-bench/`、`docs/superpowers/specs/`、`openspec/changes/codegraph-baseline-spike/` 路径下
- [ ] 9.2 若发现违规改动，回滚或转移到独立 change

## 10. 验证与归档

- [ ] 10.1 通过 `openspec validate codegraph-baseline-spike` 校验 change 完整性
- [ ] 10.2 在干净环境（新 shell、清空 `.codegraph/`）重跑 `run-all.sh` + `gen-report.sh`，验证整套套件可复现
- [ ] 10.3 进入 `/comet-verify`，按 spec 8 个 requirement 的 scenarios 逐项核对
- [ ] 10.4 verify 通过后进入 `/comet-archive`，归档到 `openspec/changes/archive/`
```

## openspec/changes/codegraph-baseline-spike/specs/codegraph-baseline-bench/spec.md

- Source: openspec/changes/codegraph-baseline-spike/specs/codegraph-baseline-bench/spec.md
- Lines: 1-175
- SHA256: 896c06e11b8a109a6d467f9cb5e3dd61cb7ee8064f1ee03a6c450d981167dd17

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: 测量套件入口

系统 SHALL 提供可重复执行的 codegraph 基线测量套件入口脚本，使任何开发者无需修改源码即可在任一 Rust 项目中复现基线测量。

#### Scenario: 一键运行全部基线测量

- **WHEN** 在 wgenty-code 仓库根目录执行 `bash scripts/codegraph-bench/run-all.sh`
- **THEN** 套件依次完成性能基线、覆盖率基线、Agent 使用率基线的测量，并把原始数据写入 `scripts/codegraph-bench/results/<timestamp>/` 目录

#### Scenario: 在外部 Rust 仓库运行

- **WHEN** 在外部 Rust 项目根目录执行 `bash <wgenty-path>/scripts/codegraph-bench/run-all.sh --target .`
- **THEN** 套件能在该外部项目上跑通性能基线和覆盖率基线，并把结果写入指定 `--output` 目录或当前目录的 `codegraph-bench-results/`

#### Scenario: 缺少 codegraph 二进制时的兜底

- **WHEN** 执行环境找不到 `wgenty-code` 二进制
- **THEN** 套件以非零退出码失败，并打印清晰提示：缺失的二进制名称、建议安装命令

### Requirement: 性能基线测量

测量套件 SHALL 测量并记录 codegraph 在指定项目上的关键性能指标。

#### Scenario: 全量索引性能

- **WHEN** 在目标项目上执行性能基线脚本
- **THEN** 脚本至少运行 5 次 `wgenty-code codegraph index`（每次前清空 `.codegraph/`），并记录每次的 wall-clock 耗时；输出至少包含中位数和 p95（或全部样本的原始值，便于事后计算）

#### Scenario: 增量索引性能

- **WHEN** 已存在索引，脚本通过 `touch` 或可重复的修改方式分别变更 1、10、100 个 `.rs` 文件后重跑索引
- **THEN** 脚本记录三种规模下的增量索引耗时，至少各取 3 个样本

#### Scenario: 索引体积

- **WHEN** 全量索引完成
- **THEN** 脚本记录 `.codegraph/index.db` 的字节数和目标项目 `.rs` 文件总字节数，并输出索引体积比

#### Scenario: 查询延迟

- **WHEN** 索引已就绪，脚本对 `codegraph_node` 和 `codegraph_explore` 各发起至少 20 次代表性查询（用 fixture 文件提供查询列表）
- **THEN** 脚本记录每次查询的端到端耗时，输出每个工具的中位数和 p95

### Requirement: 覆盖率基线测量

测量套件 SHALL 量化当前 codegraph 索引对目标项目的覆盖程度。

#### Scenario: 文件覆盖率

- **WHEN** 全量索引完成
- **THEN** 脚本输出目标项目 `.rs` 文件总数 vs 索引中已记录文件数，并列出未被索引的文件 top 10（如有）

#### Scenario: 符号与关系统计

- **WHEN** 全量索引完成
- **THEN** 脚本通过 `.codegraph/index.db` 输出按 `SymbolKind` 分组的符号总数和按 `RelKind` 分组的关系总数

#### Scenario: 解析失败统计

- **WHEN** 全量索引运行期间 codegraph 报告 parse 失败
- **THEN** 脚本汇总 parse 失败的文件数、占比，并归类失败原因 top 3（按错误信息归类）

### Requirement: Agent 使用率基线测量

测量套件 SHALL 量化 Agent 在代码导航类任务中实际调用 codegraph 工具的比例，采用「CLI 回放为主 + 历史 session 静态分析为辅」双轨方案。

#### Scenario: B 路径——CLI 回放标准任务集

- **WHEN** 测量脚本对 `scripts/codegraph-bench/agent-tasks/` 提供的标准代码导航任务集逐条调用 `wgenty-code query --prompt "..."`（或带 `--no-interactive` 等价非交互入口）
- **THEN** 脚本记录每条任务中 agent 实际调用的工具序列、是否使用 codegraph、调用 codegraph 时的 RelKind 分布、grep/find/Read 调用次数

#### Scenario: A 路径——历史 session 静态分析

- **WHEN** 测量脚本扫描 `~/.wgenty-code/sessions/` 下的历史 session JSON 文件
- **THEN** 脚本统计每个 session 中代码导航相关任务的工具调用分布，作为 B 路径之外的辅助证据，特别用于支撑根因分析

#### Scenario: 标准任务集规模与结构

```

Full source: openspec/changes/codegraph-baseline-spike/specs/codegraph-baseline-bench/spec.md

