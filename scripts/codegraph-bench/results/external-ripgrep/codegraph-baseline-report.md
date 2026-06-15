# Codegraph 基线报告

> 生成日期: 2026-06-15
> 测量环境: Darwin arm64 / 10 核
> wgenty-code 版本: wgenty code 0.1.0
> 目标 commit: 82313cf95849bfe425109ad9506a52154879b1b1

## 1. 测量环境

| 字段 | 值 |
|------|-----|
| 操作系统 | Darwin arm64 |
| CPU 核数 | 10 |
| wgenty-code 版本 | wgenty code 0.1.0 |
| 目标 commit | 82313cf95849bfe425109ad9506a52154879b1b1 |
| 测量时间 | 2026-06-15T11:48:11Z |

## 2. 性能基线

### 全量索引

| 指标 | 值 |
|------|-----|
| 中位数 | 0.247 秒 |
| p95 | 0.247 秒 |
| 样本数 | 1 |

### 增量索引

| 规模 | 中位数 |
|------|--------|
| 1 文件 | N/A 秒 |
| 10 文件 | N/A 秒 |
| 100 文件 | N/A 秒 |

### 索引体积

| 指标 | 值 |
|------|-----|
| index.db | 1966080 字节 |
| 源码 .rs | 1767444 字节 |
| 比率 | 1.11 |

## 3. 覆盖率基线

| 指标 | 值 |
|------|-----|
| .rs 文件总数 | 100 |
| 已索引文件 | 100 |
| 覆盖率 | 100.0% |
| 符号总数 | 3759 |
| 关系总数 | 0 |
| 解析失败率 | 0% |

## 4. Agent 使用率基线

| 指标 | 值 |
|------|-----|
| 总 session 数 | N/A |
| 使用 codegraph 的 session | N/A |
| codegraph 采纳率 | N/A% |
| codegraph 工具调用占比 | N/A% |

## 5. 根因分析

> 详见: scripts/codegraph-bench/root-cause-analysis.md

### Top 3 根因

1. **Prompt 优先级**: 系统 prompt 中 grep 排在 codegraph 之前，codegraph 未被列出
2. **工具描述缺乏场景引导**: tool description 纯功能描述，无"何时优先用"指示
3. **无懒初始化成功反馈**: Agent 不知道 codegraph 索引已就绪

## 6. 后续 change 基线 vs 目标对照表

| 指标 | 当前基线值 | 建议目标值 | 行业对标（参考） | 验证方法 | 归属 change |
|------|-----------|-----------|-----------------|---------|------------|
| agent codegraph 调用率 | N/A% | ≥ N/A | — | 重跑 agent-tasks | #1 codegraph-agent-adoption |
| codegraph_node p95 延迟 | 0.247 秒 | ≤ 基线 × 1.5 | rust-analyzer: ~10ms | bench-query.sh | #2 codegraph-query-and-explainability |
| 全量索引耗时 | 0.247 秒 | ≤ 基线 × 1.5 | sourcegraph: ~Xs | bench-perf.sh | #3 codegraph-multilang-and-deep-graph |
| 多语言覆盖率 | 0% | ≥ 70% | — | bench-coverage.sh | #3 codegraph-multilang-and-deep-graph |

**建议目标值制定规则**: 基线乘系数为主（b）、行业对标抽样为辅（a）、后续 change 在 design 阶段可调整不超过 ±20%（c）。

## 7. 附录

- 原始数据: scripts/codegraph-bench/results/external-ripgrep/
- env.json / perf.json / coverage.json / agent.json
- 根因分析: scripts/codegraph-bench/root-cause-analysis.md
- 稳定性窗口: 恒定指标 ±1%、中等波动 ±20%、高波动 ±50%
