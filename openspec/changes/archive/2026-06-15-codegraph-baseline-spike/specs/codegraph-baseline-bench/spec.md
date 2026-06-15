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

- **WHEN** 阅读 `scripts/codegraph-bench/agent-tasks/` 目录
- **THEN** 任务集包含至少 12 条任务，覆盖 6 类（definition_lookup、reference_lookup、call_chain、impl_enumeration、module_structure、cross_module_path），每类至少 2 条；每条任务为独立 YAML 文件，至少包含 `task_id`、`category`、`prompt`、`expected_answer_anchor: { file, contains }`

#### Scenario: 失败兜底路径

- **WHEN** agent 在某条任务中调用 codegraph 失败或返回空结果
- **THEN** 脚本记录失败/空结果次数，以及该任务后续是否兜底到 grep / 是否最终完成任务

#### Scenario: 标准任务集可扩展

- **WHEN** 在 `scripts/codegraph-bench/agent-tasks/` 下新增任务文件
- **THEN** 重跑脚本无需修改其他文件，新任务自动纳入统计；任务文件格式在该目录的 README 中明确

### Requirement: 基线报告产出

测量套件 SHALL 产出一份包含规定字段的基线报告，作为后续 change 的引用源。

#### Scenario: 报告位置与命名

- **WHEN** 测量套件完成全部测量
- **THEN** 在 `docs/superpowers/specs/<YYYY-MM-DD>-codegraph-baseline-report.md` 生成 Markdown 报告（日期为生成日，不写"今天"等动态文案）

#### Scenario: 报告必含章节

- **WHEN** 打开生成的报告
- **THEN** 报告至少包含以下章节：测量环境（OS、CPU、wgenty-code 版本、commit hash）、性能基线、覆盖率基线、Agent 使用率基线、根因分析、后续 change 基线 vs 目标对照表

#### Scenario: 后续 change 基线对照表

- **WHEN** 阅读报告的"后续 change 基线 vs 目标对照表"章节
- **THEN** 表格至少为 `codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph` 三个 change 各列出至少 1 条记录；每行包含 6 列：「指标 / 当前基线值 / 建议目标值 / 行业对标参考 / 验证方法 / 归属 change」；建议目标值的制定规则在报告中明确说明（基线乘系数为主、行业对标抽样为辅、允许后续 change 在 design 阶段做不超过 ±20% 的调整）

### Requirement: 根因分析

报告 SHALL 给出"Agent 不主动使用 codegraph"的根因 top 3，每条带可追溯证据。

#### Scenario: 根因条目格式

- **WHEN** 阅读报告的根因分析章节
- **THEN** 至少给出 3 条根因，每条包含：根因描述、影响面（哪类任务受影响）、证据（transcript 引用、查询日志摘录或具体 prompt 段落）、建议修复方向（指向 codegraph-agent-adoption / codegraph-query-and-explainability / codegraph-multilang-and-deep-graph 之一）

#### Scenario: 证据可追溯

- **WHEN** 报告引用任意一条 transcript / 查询日志 / prompt 段落作为证据
- **THEN** 引用同时包含来源文件相对路径和行号或时间戳，使读者可独立复核

### Requirement: 不修改业务代码与 spec

测量套件的实现 SHALL 不修改 `src/` 下任何源码、`src/prompts/` 下任何 prompt、以及 `openspec/specs/` 下任何已有 capability spec。

#### Scenario: 仓库改动局限于规定目录

- **WHEN** 本 change 进入 verify 阶段执行 `git diff --name-only main...`
- **THEN** 改动文件仅出现在 `scripts/codegraph-bench/`、`docs/superpowers/specs/`、`openspec/changes/codegraph-baseline-spike/` 这几条路径下；任何 `src/` 或 `openspec/specs/` 下文件出现在差异列表中即视为违反

#### Scenario: 必要的依赖最小化

- **WHEN** 测量需要新增依赖
- **THEN** 优先使用系统自带工具（`time`、`du`、`sqlite3`、`jq`）；只有在系统工具无法满足时才允许新增 Cargo/包管理器依赖，且需在报告中说明引入原因

### Requirement: 测量结果可重复

测量套件 SHALL 让同一仓库在同一 commit 下的重复测量结果可比对，并按指标本征波动分层声明稳定性窗口。

#### Scenario: 同一仓库重跑

- **WHEN** 在同一 commit 下相隔 5 分钟以上重跑 `run-all.sh`
- **THEN** 脚本按以下分层阈值判定稳定性，超出对应阈值时输出告警并提示读者考虑系统负载：
  - 恒定指标（索引体积、文件数、按 SymbolKind 分组的符号数、按 RelKind 分组的关系数）：相对差异 ≤ ±1%
  - 中等波动指标（全量索引耗时中位数、查询延迟中位数、agent codegraph 调用率）：相对差异 ≤ ±20%
  - 高波动指标（增量索引耗时、单次查询延迟、agent token 数）：相对差异 ≤ ±50%，仅看趋势不强约束绝对值

#### Scenario: 测量产物含环境指纹

- **WHEN** 任一次测量完成
- **THEN** 结果目录中保存一份 `env.json`，包含：操作系统、CPU 核数、`wgenty-code --version`、目标仓库 commit hash、运行时间戳；报告引用该指纹作为环境上下文

### Requirement: 外部仓库验证

测量套件 SHALL 在至少一个外部 Rust 仓库上跑通核心测量，证明脚本不绑死 wgenty-code 自身仓库。

#### Scenario: ripgrep 必跑

- **WHEN** 本 change 进入 verify 阶段
- **THEN** 必须存在一份在 `ripgrep` 仓库上跑出的性能基线和覆盖率基线结果（归档到 `results/<timestamp>-external-ripgrep/`），且 `run-all.sh` 在 ripgrep 上能直接以 `--target <ripgrep-path>` 跑通

#### Scenario: tokio 可选

- **WHEN** 在 `tokio` 仓库上跑测量套件
- **THEN** 若跑通则归档到 `results/<timestamp>-external-tokio/`；若因规模、构建依赖或耗时跑不通，需在报告中记录失败原因，但不视为 spike 失败

#### Scenario: 不绑死 wgenty-code 路径

- **WHEN** 阅读 `run-all.sh` 与 `bench-perf.sh`、`bench-coverage.sh`
- **THEN** 脚本通过 `--target` 参数显式接受目标仓库路径，不写死任何 `wgenty-code/` 子路径；外部仓库使用时不需要修改脚本
