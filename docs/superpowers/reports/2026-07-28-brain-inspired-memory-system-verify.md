---
comet_change: brain-inspired-memory-system
role: verification-report
verify_mode: full
verify_result: pass
verify_failures: 1
language: zh-CN
---

# 验证报告：brain-inspired-memory-system

## 摘要

| 维度 | 状态 |
|------|------|
| 完整性 (Completeness) | 29/29 任务完成；4/4 产物齐全（proposal、design、specs、tasks） |
| 正确性 (Correctness) | 17 个 ADDED + 11 个 MODIFIED 场景均有具名测试覆盖；50 个场景/回归测试通过 |
| 一致性 (Coherence) | Design Doc 与 delta spec 一致；无 spec 漂移；代码审查问题已全部解决 |

**验证模式：** full（29 个任务、73 个变更文件、1 个 delta capability）—— 由 `comet state scale` 判定。

**结论：** PASS，经过 1 次 verify-fail 修复循环（verify_failures=1，在 3 次自动修复预算内）。

## 验证范围

- Base ref：`4fbbf4d8`（取自 plan frontmatter）
- 实现区间：`25f47eaf`..`702814d5`，合并于 `11e358e4`
- 修复提交：`9574972b`（修复 C1 + I1 + I2 + I3 + I4）

## 维度 1 — 完整性

- **tasks.md：** 29/29 任务已勾选 `[x]`（M1–M6 全部完成）。
- **产物：** proposal.md、design.md、specs/agent-memory/spec.md、tasks.md 均存在且非空。
- **Spec 覆盖：** delta spec 中每个 ADDED/MODIFIED requirement 都有实现证据（file:line），通过两次独立的代码审查确认。

## 维度 2 — 正确性

5 个里程碑（M1–M6）全部验证存在且行为与 spec 精确匹配：

| 里程碑 | 实现位置 | 状态 |
|--------|----------|------|
| M1 effective importance | `mod.rs:137-157`，公式与 spec 逐字一致 | ✅ |
| M2 接线（recall/global/should_keep/list） | `inject.rs:42-52,93-102`、`consolidation.rs:598-630`、`mod.rs:980-1012` | ✅ |
| M3 Tier-1 classify + supersede | `consolidation.rs:139-163`、`mod.rs:674-734` | ✅ |
| M4 幂等 staleness | `consolidation.rs:85-117`（prepass） | ✅ |
| M5 exploration（epsilon） | `inject.rs:143-209` | ✅ |
| M6 配置 + anchor 迁移 | `services.rs:107-133`、`consolidation.rs:91-94` | ✅ |

**场景 → 测试映射**（来自 tasks.md 第 6.6 节）：17 个 ADDED 场景和 11 个变更的 MODIFIED 场景都有明确的具名测试；19 个未变更的 MODIFIED 场景标注为既有基线。

## 维度 3 — 一致性

- **设计遵循：** 5 个设计决策（D1 惰性衰减、D2 hitrate 阻尼、D3 anchor 迁移、D4 保守 classify、D5 无 Tier-2 LLM、D6 tombstone 不硬删、D7 LLM-free staleness、D8 staleness_check 门闸）均在代码中体现。
- **Spec 漂移：** 无。Delta spec requirements 与 design doc（`docs/superpowers/specs/2026-07-25-brain-inspired-memory-system-design.md`）完全一致，无矛盾。
- **代码模式一致性：** 遵循项目 Rust 约定（snake_case、`thiserror`/`anyhow`、`Arc<RwLock<T>>`、`#[serde(default)]` 零迁移）。

## 标准代码审查（review_mode=standard）

通过 `requesting-code-review` 派发了聚焦代码审查（正确性/安全/边界）。首轮发现 1 个 Critical + 4 个 Important 问题。所有发现均按 `receiving-code-review` 原则独立对照代码库验证后再修复。

### 已发现并解决的问题

| 编号 | 严重程度 | 问题 | 解决方案 |
|------|----------|------|----------|
| C1 | CRITICAL | `consolidate()` 通过 `should_keep`→false → `reconcile` 孤儿删除，硬删除了 tombstone 记忆。违反 spec 不变式："the memory is NOT hard-deleted" / "JSON file remains on disk (auditable)"。 | Tombstone 现在跳过 similarity/merge，始终保留在 consolidated Vec 中；`should_keep` 对其返回 true。同时暴露了一个潜在 bug：`search_memories` 未过滤 superseded（之前仅因删除而"偶然正确"）—— 现已在 search 边界显式过滤。新增回归测试。 |
| I1 | IMPORTANT | TUI recall 硬编码阈值 `0.5`（`turn.rs:136`），静默忽略 `recall_similarity_threshold` 配置；headless 路径正确读取。 | TUI 现从 settings 读取 `storage.memory.recall_similarity_threshold`。 |
| I2 | IMPORTANT | `format_global` 在 soft cap 50 截断时未记录日志（spec 场景："a warning is logged"）。 | 超限时新增 `tracing::warn!`。 |
| I3 | IMPORTANT | staleness 路径提取 regex 允许 `..` 段 → 可能探测 `project_root` 之外的文件存在性（纵深防御缺口）。 | 含 `Component::ParentDir` 的路径在提取阶段被拒绝。 |
| I4 | IMPORTANT | `recall_similarity_threshold` 配置名语义错误（实际用作 effective importance 门槛，非 Jaccard 相似度）。 | 在 WGENTY.md 配置表和字段 doc comment 中记录真实语义。重命名推迟（会构成破坏性配置变更）。 |

**未接受任何 WARNING/SUGGESTION 偏差。** 所有问题均已修复，或如 I4 通过文档解决（底层行为正确，仅名称有误导性）。

## 质量门禁（新鲜证据）

| 门禁 | 命令 | 结果 |
|------|------|------|
| 构建 | `cargo build`（via guard） | exit 0 |
| 测试（context） | `cargo test --lib context::` | **143 passed, 0 failed** |
| 场景回归 | `cargo test --lib -- context::tests::...` | **50 passed, 0 failed** |
| Clippy | `cargo clippy --all-targets -- -D warnings` | exit 0，零 warning |
| 格式 | `cargo fmt -- --check` | exit 0，clean |

所有证据均在 verify session 中采集（铁律：无新鲜验证输出不得声称完成）。

## 最终评估

**可归档：** 是。

Memory Reliability Foundation（M1–M6）已完整实现、符合 spec、测试覆盖充分。单次 verify-fail 循环捕获并修复了关键的 tombstone 保留不变式违反（C1）、配置静默忽略 bug（I1）以及纵深防御缺口（I2–I4）。无未解决的 CRITICAL 或 IMPORTANT 问题。branch_status 保持 `pending`（由 archive 阶段处理）。
