# Tasks — codegraph-baseline-spike

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图（关联 change 名）。
> 每个分组对应 design.md / spec.md 的一段产出，序号即执行顺序。

## 1. 测量脚手架

- [x] 1.1 创建 `scripts/codegraph-bench/` 目录及 README，说明套件用途、入口、目录约定
- [x] 1.2 实现 `run-all.sh` 入口脚本：解析 `--target`、`--output`、`--repeats` 参数，调度三组测量
- [x] 1.3 实现 `lib/env-fingerprint.sh`：采集 OS、CPU 核数、`wgenty-code --version`、目标 commit hash，写入 `env.json`
- [x] 1.4 编写"缺失二进制"兜底逻辑：找不到 `wgenty-code` 时退出码非零并打印安装建议（对应 spec 「测量套件入口/Scenario: 缺少 codegraph 二进制时的兜底」）
- [x] 1.5 增加 `.gitignore` 规则：忽略 `scripts/codegraph-bench/results/` 下的运行产物，但保留 README 和 `.gitkeep`

## 2. 性能基线测量

- [x] 2.1 实现 `bench-perf.sh`：全量索引计时（≥5 次，每次前清空 `.codegraph/`），输出每次的 wall-clock 秒数
- [x] 2.2 扩展 `bench-perf.sh`：增量索引计时（修改 1/10/100 个 `.rs` 文件，各 ≥3 次）
- [x] 2.3 扩展 `bench-perf.sh`：记录 `.codegraph/index.db` 字节数与目标项目 `.rs` 总字节数
- [x] 2.4 实现 `bench-query.sh`：从 fixture 文件读取 ≥20 条查询，分别测 `codegraph_node` 与 `codegraph_explore` 端到端耗时
- [x] 2.5 输出统一 JSON：性能数据写入 `results/<timestamp>/perf.json`，含 raw samples + 中位数 + p95
- [x] 2.6 在 wgenty-code 自身仓库跑通 perf 测量，把首份原始数据存档备查

## 3. 覆盖率基线测量

- [x] 3.1 实现 `bench-coverage.sh`：通过 `find` 统计目标项目 `.rs` 总文件数，与 `.codegraph/index.db` 中已索引文件数对比
- [x] 3.2 扩展 `bench-coverage.sh`：用 `sqlite3` 查询符号按 `SymbolKind` 分组、关系按 `RelKind` 分组的总数
- [x] 3.3 扩展 `bench-coverage.sh`：捕获 `wgenty-code codegraph index` 输出中的 parse 失败计数与文件，归类失败原因 top 3
- [x] 3.4 输出统一 JSON：覆盖率数据写入 `results/<timestamp>/coverage.json`
- [x] 3.5 在 wgenty-code 自身仓库跑通 coverage 测量，确认输出字段齐全

## 4. Agent 使用率基线测量

- [x] 4.1 brainstorming 选择 Agent 驱动路径（transcripts 静态分析 / CLI 回放 / 临时打点），写入 design.md 的 Open Questions 解答；本步骤是 build 阶段的关键决策点，不得自动跳过
- [x] 4.2 实现 `agent-tasks/` 目录与 README，定义任务文件格式（YAML 或 JSON 均可）
- [x] 4.3 编写 ≥10 条种子代码导航任务（覆盖：定义查找、引用查找、调用链探索、impl 列举等）
- [x] 4.4 实现 `bench-agent.sh`：按 4.1 选定的路径，对每条任务记录工具调用序列
- [x] 4.5 输出统一 JSON：agent 数据写入 `results/<timestamp>/agent.json`，含每条任务的工具序列、是否使用 codegraph、失败/兜底标记
- [x] 4.6 在 wgenty-code 自身仓库跑通 agent 测量

## 5. 外部仓库验证

- [x] 5.1 选定外部测试仓库（候选：ripgrep）；克隆到本地或文档说明克隆步骤
- [ ] 5.2 在外部仓库执行 `bash <wgenty-path>/scripts/codegraph-bench/run-all.sh --target <repo>`，确认 perf + coverage 跑通
- [ ] 5.3 把外部仓库结果归档到 `results/<timestamp>-external/`，并记录环境差异（OS/工具版本）
- [ ] 5.4 解决脚本中任何"绑死本仓库"的硬编码（路径、配置、假设）

## 6. 根因分析

- [x] 6.1 汇总 4.x 输出的工具调用数据，识别"agent 没用 codegraph"的高频模式
- [x] 6.2 对每个候选根因抽取 ≥1 条可追溯证据（transcript 引用 + 行号、prompt 段落 + 文件:行）
- [x] 6.3 输出根因 top 3 候选清单，标注影响面（哪类任务）和建议归属 change（C/B/A）
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
