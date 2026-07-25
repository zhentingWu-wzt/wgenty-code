---
change: brain-inspired-memory-system
design-doc: docs/superpowers/specs/2026-07-25-brain-inspired-memory-system-design.md
base-ref: 479ff8026237a601d9caf33b25b858ac39a86899
---

# Memory Reliability Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `agent-memory` 补上可读时校准的反馈回路：effective importance、Tier-1 tombstone supersede、幂等 path staleness、默认关闭的 exploration，并使 recall / retention / CLI list 表面一致。

**Architecture:** 不新开子系统。在现有 `MemoryEntry` 上增加 serde-default 反馈字段；`effective_importance` 为纯读时函数；`add_memory` 相似分支改为 classify→merge/reinforce|tombstone|flag；`inject::recall` 用 effective 排序并在注入后持久化 `recall_count`；`consolidate` 在 LLM-free 路径做 anchor 与全-missing stale 标记；配置挂在 `MemorySettings`。

**Tech Stack:** Rust, tokio, serde, chrono, anyhow, existing context memory tests

## Global Constraints

- `consolidate()` **永远 LLM-free**（无 dream Tier-2 / replay）
- 语义记忆文件名 = 稳定 UUID `id`（不改 storage 命名）
- tombstone 不硬删；supersede 不改旧 base `importance`
- stale：仅当提取路径非空且**全部 missing**；只设 `stale_marked_at`，不叠乘 base
- `exploration_epsilon` 默认 `0.0`
- 旧 JSON 零迁移（`#[serde(default)]`）
- 不做 engagement / episodic / 符号多线索 / pain / 热路径 restate
- Canonical spec：`openspec/changes/brain-inspired-memory-system/specs/agent-memory/spec.md`
- Design：`docs/superpowers/specs/2026-07-25-brain-inspired-memory-system-design.md`
- 每完成一个 Task：更新 `openspec/changes/brain-inspired-memory-system/tasks.md` 对应勾选 + git commit

---

## 文件结构

| 文件 | 操作 | 职责 |
|------|------|------|
| `src/context/mod.rs` | 修改 | `MemoryEntry` 字段、`reinforce`/`effective_importance`、`add_memory` classify、`list_memories`、`consolidate` anchor/stale、cfg 线程 |
| `src/context/consolidation.rs` | 修改 | `type_half_life` 共享、`should_keep` 用 effective、`classify_relation`（或同文件 helper）、ConsolidationConfig 扩展 |
| `src/context/inject.rs` | 修改 | recall effective 排序/过滤、recall_count 持久化、exploration、format_global |
| `src/config/services.rs` | 修改 | `MemorySettings` 新字段 + defaults |
| `src/context/mod.rs` / manager ctor | 修改 | 从 settings 读入 epsilon/staleness_* |
| `WGENTY.md` | 修改 | 配置表 |
| 单测 | 同上各文件 `#[cfg(test)]` | TDD 金标 |

可选小拆：若 `mod.rs` 过大，可将 `classify_relation` / path extract 放到 `src/context/memory_relation.rs` — **仅当**单测与引用清晰时再拆，默认先放 `consolidation.rs` / `mod.rs`。

---

### Task 1: MemoryEntry 字段 + effective_importance + serde ✅

**Files:**
- Modify: `src/context/mod.rs`
- Modify: `src/context/consolidation.rs`（half-life helper 可先放这里或 mod）

**对齐 tasks.md:** 1.1–1.6

- [x] **Step 1: 写失败测试 — legacy JSON 与 effective 曲线**

在 `src/context/mod.rs` 测试模块增加：

```rust
#[test]
fn legacy_memory_json_defaults_feedback_fields() {
    let raw = r#"{
        "id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "memory_type":"Knowledge",
        "content":"old",
        "timestamp":"2020-01-01T00:00:00Z",
        "importance":0.8,
        "tags":[],
        "metadata":{}
    }"#;
    let e: MemoryEntry = serde_json::from_str(raw).unwrap();
    assert_eq!(e.recall_count, 0);
    assert_eq!(e.hit_count, 0);
    assert!(e.last_reinforced_at.is_none());
    assert!(e.superseded_by.is_none());
    assert!(e.stale_marked_at.is_none());
}

#[test]
fn effective_importance_superseded_is_zero() { /* ... */ }

#[test]
fn effective_importance_never_recalled_hitrate_neutral() { /* hitrate factor == 1.0 */ }

#[test]
fn effective_importance_decays_with_age() { /* same base, different anchor */ }

#[test]
fn effective_importance_stale_multiplier() { /* stale_marked_at Some → * penalty */ }
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p wgenty-code legacy_memory_json_defaults_feedback_fields effective_importance -- --nocapture
```

（按实际 package 名调整；若是 bin/lib 一体则 `cargo test legacy_memory_json`）

- [x] **Step 3: 实现字段与 API**

```rust
// MemoryEntry 新增（均 #[serde(default)]）
pub recall_count: u32,
pub hit_count: u32,
pub last_reinforced_at: Option<DateTime<Utc>>,
pub superseded_by: Option<String>,
pub stale_marked_at: Option<DateTime<Utc>>,
```

`new()` 初始化为 0/None。

```rust
pub fn reinforce(&mut self, now: DateTime<Utc>) {
    self.hit_count = self.hit_count.saturating_add(1);
    self.last_reinforced_at = Some(now);
}

pub fn effective_importance(&self, now: DateTime<Utc>, cfg: &EffectiveImportanceCfg) -> f32 {
    if self.superseded_by.is_some() {
        return 0.0;
    }
    let anchor = self.last_reinforced_at.unwrap_or(self.timestamp);
    let hours = (now - anchor).num_minutes().max(0) as f64 / 60.0;
    let half = type_half_life_hours(self.memory_type, cfg.age_threshold_hours).max(1e-6);
    let decay = (-std::f64::consts::LN_2 * hours / half).exp() as f32;
    let hitrate = (self.hit_count as f32 + 1.0) / (self.recall_count as f32 + 2.0);
    let hit_factor = 0.5 + 0.5 * hitrate;
    let stale_mul = if self.stale_marked_at.is_some() {
        cfg.staleness_penalty
    } else {
        1.0
    };
    self.importance * decay * hit_factor * stale_mul
}
```

`type_half_life_hours` 与现有 `should_keep` TTL 倍率一致（Knowledge/Preference×4, Decision/Insight×2, Error max(base/2,1), else×1）。

`EffectiveImportanceCfg { age_threshold_hours, staleness_penalty }` 可先简单 struct。

- [x] **Step 4: 测试通过**

```bash
cargo test legacy_memory_json effective_importance
```

- [x] **Step 5: 勾选 tasks.md 1.1–1.6 并 commit**

```bash
git add src/context/mod.rs src/context/consolidation.rs openspec/changes/brain-inspired-memory-system/tasks.md
git commit -m "$(cat <<'EOF'
feat(memory): add feedback fields and effective_importance

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 配置键 + MemoryManager 线程 cfg ✅

**Files:**
- Modify: `src/config/services.rs` (`MemorySettings`)
- Modify: `src/context/mod.rs` (`with_settings` / test ctor)
- Modify: `src/context/consolidation.rs` (`ConsolidationConfig` 如需承载 penalty/check)

**对齐 tasks.md:** 6.2 的配置部分可先做结构，文档放 Task 6

- [x] **Step 1: 扩展 MemorySettings**

```rust
#[serde(default = "default_exploration_epsilon")]
pub exploration_epsilon: f32, // 0.0
#[serde(default = "default_staleness_check")]
pub staleness_check: bool, // true
#[serde(default = "default_staleness_penalty")]
pub staleness_penalty: f32, // 0.5
```

更新 `Default` / 任何手工 struct 字面量（编译器会指路）。

- [x] **Step 2: MemoryManager 保存 cfg 字段**，供 inject/consolidate 读取（getter 或 pub(crate)）

- [x] **Step 3: `cargo test` 相关 config/context 编译通过**

- [x] **Step 4: commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): add exploration and staleness settings

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Wire effective into recall / global / should_keep / list ✅

**Files:**
- Modify: `src/context/inject.rs`
- Modify: `src/context/consolidation.rs` (`should_keep`)
- Modify: `src/context/mod.rs` (`list_memories`)

**对齐 tasks.md:** 2.1–2.3, 2.5–2.6（2.4 在 Task 4）

- [x] **Step 1: 失败测试** — superseded 不进 recall 排序；list 按 effective；should_keep 对 superseded/effective 低者

复用 `MemoryManager::new_for_test`。

- [x] **Step 2: 实现**

`inject::recall`:

```rust
.filter(|m| m.superseded_by.is_none())
.filter(|m| m.effective_importance(now, &cfg) >= threshold_f32)
// sort by effective desc
```

`format_global` / `list_memories`：同样 effective；list **保留** superseded 行。

`should_keep`：高 effective 保；否则用 age vs type TTL（与 half-life 一致）。注意：原先 `importance >= threshold` 永留 → 改为 effective 比较。

- [x] **Step 3: 修现有 inject 测试**（它们按 raw importance 断言）

- [x] **Step 4: commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): rank recall, retention, and list by effective importance

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: inject 持久化 recall_count ✅

**Files:**
- Modify: `src/context/inject.rs`
- Modify: `src/context/mod.rs`（如需 `bump_recall_counts(&[id])` API，推荐集中在 manager 以免 inject 碰锁细节）

**对齐 tasks.md:** 2.4, 2.6 中 persist 部分

- [x] **Step 1: 失败集成测试**

```rust
// add project memory, call recall with matching keywords, reload/get_memory
// assert recall_count == 1 and disk reflects it
```

- [x] **Step 2: 实现 `MemoryManager::record_recall_injections(&self, ids: &[str])`**

- 等待 `!consolidating`（与 add_memory 相同）
- 写锁 `memories`，对每个 id：`recall_count += 1`，`project_storage.save_memory`
- **不要**为 count-only 强制 index rebuild（content 未变）

`recall` 在确定 top 列表后调用该 API，再拼 block。

- [x] **Step 3: 测试通过 + commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): persist recall_count on project memory injection

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Tier-1 classify_relation + add_memory 分支 ✅

**Files:**
- Modify: `src/context/consolidation.rs` 或 `mod.rs`
- Modify: `src/context/mod.rs` `add_memory`

**对齐 tasks.md:** 3.1–3.4

- [x] **Step 1: 失败单测金标**

```rust
// state-change → Contradicts
// max_tokens drift → Contradicts  
// subset "use jwt" vs "use jwt authentication" → Compatible
// unrelated similar-ish → Ambiguous (construct carefully)
```

- [x] **Step 2: 实现 `pub enum MemoryRelation { Compatible, Contradicts, Ambiguous }` + `classify_relation`**

保守：状态词列表见 design；数值漂移：共享非数字 token 且数字 token 集合差非空。

- [x] **Step 3: 改 `add_memory` 相似分支**

```rust
match classify_relation(&entry, &mem[existing_idx]) {
  Compatible => { merge; reinforce(now); save; replace_entry; merged=true }
  Ambiguous => { merge; metadata relation_ambiguous=true; save; replace_entry; merged=true }
  Contradicts => {
    mem[existing_idx].superseded_by = Some(entry.id.clone());
    storage.save_memory(&mem[existing_idx]).await?;
    // push new entry standalone...
    merged=false
  }
}
```

跳过已被 superseded 的 existing 作为 merge 目标（find_similar 时可 filter，或 classify 前检查）。

- [x] **Step 4: 集成：supersede 后 recall 不含旧 content**

- [x] **Step 5: commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): tier-1 relation classify with tombstone supersede

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: consolidate anchor + idempotent all-missing staleness ✅

**Files:**
- Modify: `src/context/mod.rs` `consolidate`（及 global prune 路径如对称）
- 可选 helper：`extract_paths(content) -> Vec<PathBuf>`、`paths_all_missing`

**对齐 tasks.md:** 4.1–4.4, 6.1

- [x] **Step 1: 失败测试**

- 无 `last_reinforced_at` → consolidate 后 Some  
- content `src/does_not_exist_12345.rs` only → stale_marked  
- 一条存在 + 一条 missing → **不** stale  
- 二次 consolidate stale 不叠加、base importance 不变  

使用 tempdir project_root + `new_for_test`。

- [x] **Step 2: 在 consolidate 写锁内、engine 前**

```rust
let now = Utc::now();
for m in memories.iter_mut() {
  if m.last_reinforced_at.is_none() {
    m.last_reinforced_at = Some(now);
  }
  if self.staleness_check {
    let paths = extract_memory_paths(&m.content);
    if !paths.is_empty() && paths.iter().all(|p| !p.exists()) && m.stale_marked_at.is_none() {
      m.stale_marked_at = Some(now);
    }
  }
}
// then consolidation.consolidate(&memories) — engine may need &memories with updates
```

Path regex：保守匹配相对路径 / `src/` + 常见后缀；绝对路径若出现可 probe。

- [x] **Step 3: should_keep 已用 effective（Task 3）— 确认 stale 降权影响保留**

- [x] **Step 4: commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): anchor last_reinforced_at and idempotent path staleness

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Optional exploration (default off) ✅

**Files:**
- Modify: `src/context/inject.rs`

**对齐 tasks.md:** 5.1–5.3

- [x] **Step 1: 测试 epsilon=0 永不替换**（可用固定 seed 或注入 `Rng`；最简单：epsilon=0 断言集合不变）

- [x] **Step 2: epsilon=1 且存在冷候选时替换最低档**

实现可用 `rand` 若项目已有；否则 `use std::collections::hash_map::DefaultHasher` 基于 turn 不稳定亦可，但测试需可注入布尔 `force_explore` 测试钩 `#[cfg(test)]`。

**推荐：** `recall(..., explore_draw: Option<bool>)` 仅测试覆盖；生产路径 `explore_draw=None` → 内部 bernoulli(epsilon)。

- [x] **Step 3: commit**

```bash
git commit -m "$(cat <<'EOF'
feat(memory): optional recall exploration gated by epsilon

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: WGENTY.md + 全量质量门 + tasks 收口

**Files:**
- Modify: `WGENTY.md`
- Modify: `openspec/changes/brain-inspired-memory-system/tasks.md`（全部勾选核对）

**对齐 tasks.md:** 6.3–6.7

- [ ] **Step 1: 文档** — 配置表增加 `exploration_epsilon` / `staleness_check` / `staleness_penalty` 默认值说明

- [ ] **Step 2: 质量门**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

- [ ] **Step 3: Spec compliance 笔记** — 在 tasks 6.7 旁或 commit body 列出 scenario→test 映射

- [ ] **Step 4: 最终 commit**

```bash
git commit -m "$(cat <<'EOF'
docs(memory): document reliability foundation settings

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## 执行备注

- **TDD：** 每个 Task 先红后绿（若用户选 `tdd_mode: tdd`）
- **锁序：** memories write → storage save；勿在持有 index 锁时再取 memories write
- **find_similar：** 相似候选应跳过 `superseded_by.is_some()` 的行，避免叠 tombstone 链混乱（实现时加一句 filter）
- **merge_into：** 确认不会擦掉新反馈字段；合并后保留较高 hit/recall 或相加策略在实现时选 **max(hit)/max(recall)** 或 sum — **推荐 sum 封顶合理上限不强制**，简单：保留 existing counters 再 reinforce（Compatible）
- **回滚：** 仅新字段，旧二进制可忽略

## 完成定义

- tasks.md M1–M6 全勾  
- 测试/clippy/fmt 绿  
- 无 consolidate LLM  
- 行为符合 delta spec scenarios  
