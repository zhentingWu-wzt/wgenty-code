---
change: fix-subagent-focus-nav
design-doc: docs/superpowers/specs/2026-07-05-fix-subagent-focus-nav-design.md
base-ref: 7568e2974f81459a78fe8ac47931302a7b830e9d
archived-with: 2026-07-05-fix-subagent-focus-nav
---

# Subagent Focus View 导航与选择器重做 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重做 subagent focus view 的键位与选择器交互（移除 Tab 双焦点、↑↓ 导航选择器、鼠标滚 timeline、滚动跟随、光标对齐），加完成态灰显+延迟移除，源头移除 "task" 包装节点，TUI 层过滤 "delegate" 分组节点，并简化状态栏/主聊天键位 + 修复输入框样式竞态。

**Architecture:** 改动集中在 TUI 层（`subagent_tree` / `subagent_focus_view` / `subagent_status_bar` / `input` / `app::event` / `app::mod`）与 `task` 工具。新增 `SubagentTree::is_grouping_node` + `real_node_list` 过滤分组节点；App 新增 `completed_at: HashMap<String, Instant>` 跟踪完成时刻；`FocusView::render` 接收 `completed_at` + `now` 计算可见列表与灰显。`FocusArea`/`active_area` 全部移除，选择器为唯一键盘交互区。

**Tech Stack:** Rust, ratatui（Layout/Constraint/Frame/Paragraph/Block/Borders/Style），`std::time::Instant`，`std::collections::HashMap`。

**已有进度（工作区未提交，本计划在其上完成，不重做）：**
- `input.rs`：D12 `update_style()` 已抽出，`render()` 已纯净化 ✓（仅需核对调用点完整）
- `event.rs`：状态栏 ↑↓ 自动激活 + 移除 Tab（D10）✓；主聊天 ↑↓ 滚动移除（D11）✓；focus view 选择器导航 `len+1` + Enter 退出（D5 部分）✓；`update_style()` 调用点 ✓
- `subagent_focus_view.rs`：`build_selector_lines` 已加 "main" 条目 + `i+scroll+1` ✓（但 `scroll=0` 写死、`take(available)` 越界未修）
- **未做**：D1（`active_area` 守卫）、D3（滚动跟随）、D4（高度）、D6（边框）、D7（task 包装）、D8（completed_at）、D9（grouping 过滤）

**验收 spec：** `openspec/changes/fix-subagent-focus-nav/specs/subagent-focus-view/spec.md` + `specs/subagent-status-display/spec.md`。

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 1: 移除 "task" 包装节点（D7）

### Task 1.1: 移除 task.rs 包装根节点创建块

**Files:**
- Modify: `src/tools/meta/task.rs:226-254`（背景模式 root 节点块）

- [x] **Step 1: 删除 root 包装节点创建块**

  删除 `src/tools/meta/task.rs` 第 226-254 行整个块（`let root_node_id = ...` + `store.insert(root_node_id, SubagentProgress{...})`）。保留下方 `let tool_registry = ...` 等逻辑。

  注意：`root_node_id` 变量后续在 404、429、532、557 行仍被引用（作为子节点 parent_id）——这些引用在 Task 1.2 中改为 `None`，所以 `root_node_id` 的声明需保留为不生成包装节点。**改为**：保留 `let root_node_id` 声明但删除包装节点 insert 块，或者直接在 Task 1.2 把所有 `Some(root_node_id.clone())` 改为 `None` 并删除 `root_node_id` 声明。本步采用后者：删除 226-254 整块（含 `let root_node_id` 声明）。

- [x] **Step 2: 验证编译错误点**

  Run: `cargo build 2>&1 | grep "root_node_id" | head`
  Expected: 报错 404、429、532、557 行 `root_node_id` not found（Task 1.2 修复）。

### Task 1.2: subagent 子节点改为 root（parent_id: None）

**Files:**
- Modify: `src/tools/meta/task.rs:404`（背景模式 child parent_id）
- Modify: `src/tools/meta/task.rs:429`（背景模式 callback parent_id）
- Modify: `src/tools/meta/task.rs:532`（同步模式 child parent_id）
- Modify: `src/tools/meta/task.rs:557`（同步模式 callback parent_id）

- [x] **Step 1: 背景模式 child parent_id 改 None**

  `src/tools/meta/task.rs:404`：`parent_id: Some(root_node_id.clone()),` → `parent_id: None,`

- [x] **Step 2: 背景模式 callback parent_id 改 None**

  `src/tools/meta/task.rs:429`：`Some(root_node_id.clone()),` → `None,`

- [x] **Step 3: 同步模式 child parent_id 改 None**

  `src/tools/meta/task.rs:532`：`parent_id: Some(root_node_id.clone()),` → `parent_id: None,`

- [x] **Step 4: 同步模式 callback parent_id 改 None**

  `src/tools/meta/task.rs:557`：`Some(root_node_id.clone()),` → `None,`

- [x] **Step 5: 验证编译**

  Run: `cargo build 2>&1 | tail -5`
  Expected: 无 `root_node_id` 报错（可能仍有其它阶段的编译错误，记录但不阻塞本任务）。

- [x] **Step 6: Commit**

  ```bash
  git add src/tools/meta/task.rs
  git commit -m "refactor(task): remove wrapper root node — subagent becomes tree root (D7)"
  ```

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 2: SubagentTree 分组节点过滤（D9）

### Task 2.1: 新增 is_grouping_node + real_node_list

**Files:**
- Modify: `src/tui/components/subagent_tree.rs`（impl SubagentTree）
- Test: `src/tui/components/subagent_tree.rs`（#[cfg(test)] mod tests）

- [x] **Step 1: 写失败测试 — is_grouping_node + real_node_list**

  在 `src/tui/components/subagent_tree.rs` 的 `mod tests` 末尾加：

  ```rust
  #[test]
  fn test_is_grouping_node_and_real_node_list() {
      let mut tree = SubagentTree::default();
      // delegate wrapper: has children, no events/messages
      tree.upsert(make_progress("delegate-root", None, SubagentStatus::Running));
      // 给 wrapper 加 children 字段需要通过 upsert 子节点实现
      tree.upsert(make_progress("sub1", Some("delegate-root"), SubagentStatus::Running));
      tree.upsert(make_progress("sub2", Some("delegate-root"), SubagentStatus::Running));
      // real leaf subagent (no children)
      tree.upsert(make_progress("task-root", None, SubagentStatus::Running));

      // wrapper is grouping node (has children, no events/messages)
      assert!(tree.is_grouping_node("delegate-root"));
      // real subagents are not grouping nodes
      assert!(!tree.is_grouping_node("sub1"));
      assert!(!tree.is_grouping_node("sub2"));
      assert!(!tree.is_grouping_node("task-root"));

      // real_node_list excludes grouping nodes
      let real = tree.real_node_list();
      assert!(real.contains(&"sub1".to_string()));
      assert!(real.contains(&"sub2".to_string()));
      assert!(real.contains(&"task-root".to_string()));
      assert!(!real.contains(&"delegate-root".to_string()));
  }

  #[test]
  fn test_active_count_excludes_grouping_nodes() {
      let mut tree = SubagentTree::default();
      tree.upsert(make_progress("delegate-root", None, SubagentStatus::Running));
      tree.upsert(make_progress("sub1", Some("delegate-root"), SubagentStatus::Running));
      tree.upsert(make_progress("sub2", Some("delegate-root"), SubagentStatus::Completed));
      // active = Running + Pending, excluding grouping node
      assert_eq!(tree.active_count(), 1); // only sub1 (sub2 completed)
      assert_eq!(tree.total_count(), 2); // sub1 + sub2, not delegate-root
  }
  ```

  注意：`make_progress` helper 需确保 `events` 和 `messages` 为 `Vec::new()`（已是）。grouping node 判据：`!children.is_empty() && events.is_empty() && messages.is_empty()`。

- [x] **Step 2: 运行测试验证失败**

  Run: `cargo test --lib subagent_tree::tests::test_is_grouping_node_and_real_node_list 2>&1 | tail -10`
  Expected: FAIL — `is_grouping_node` / `real_node_list` 方法不存在。

- [x] **Step 3: 实现 is_grouping_node + real_node_list**

  在 `impl SubagentTree` 中（`node_list` 方法后）加：

  ```rust
  /// Whether a node is a grouping/wrapper node with no execution info of its own.
  /// Grouping nodes have children but no events or messages (e.g., `delegate`
  /// 1:N wrapper). Real subagents — even Pending leaves with no events yet —
  /// are never grouping nodes (they have no children).
  pub fn is_grouping_node(&self, node_id: &str) -> bool {
      match self.nodes.get(node_id) {
          Some(n) => !n.children.is_empty()
              && n.progress.events.is_empty()
              && n.progress.messages.is_empty(),
          None => false,
      }
  }

  /// Depth-first list of all REAL node IDs (grouping nodes excluded).
  pub fn real_node_list(&self) -> Vec<String> {
      self.node_list()
          .into_iter()
          .filter(|id| !self.is_grouping_node(id))
          .collect()
  }
  ```

- [x] **Step 4: 运行测试验证通过**

  Run: `cargo test --lib subagent_tree::tests::test_is_grouping_node 2>&1 | tail -10`
  Expected: PASS。

### Task 2.2: count 方法过滤分组节点

**Files:**
- Modify: `src/tui/components/subagent_tree.rs:51-95`（count_by_status / active_count / total_count）

- [x] **Step 1: count_by_status 改用 real_node_list**

  ```rust
  pub fn count_by_status(&self, status: SubagentStatus) -> usize {
      self.real_node_list()
          .iter()
          .filter_map(|id| self.nodes.get(id))
          .filter(|n| n.progress.status == status)
          .count()
  }
  ```

- [x] **Step 2: total_count 改用 real_node_list**

  ```rust
  pub fn total_count(&self) -> usize {
      self.real_node_list().len()
  }
  ```

  注意：`active_count` / `completed_count` / `failed_count` 都基于 `count_by_status`，自动继承过滤。`is_empty` / `is_complete` 保持基于 `nodes`（不过滤，因空树/完成判定需看所有节点）——但 `is_complete` 应排除分组节点（分组节点永 Running 会破坏判定）。改为：

  ```rust
  pub fn is_complete(&self) -> bool {
      if self.nodes.is_empty() {
          return false;
      }
      self.real_node_list()
          .iter()
          .filter_map(|id| self.nodes.get(id))
          .all(|n| {
              matches!(
                  n.progress.status,
                  SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
              )
          })
  }
  ```

- [x] **Step 3: 验证现有测试 + 新测试通过**

  Run: `cargo test --lib subagent_tree 2>&1 | tail -15`
  Expected: PASS（含 `test_count_by_status`、`test_is_complete`、新增的 grouping 测试）。注意 `test_count_by_status` 旧用例：root + a + b + c 都无 children → 都不是 grouping node → count 不变，应仍 PASS。

- [x] **Step 4: Commit**

  ```bash
  git add src/tui/components/subagent_tree.rs
  git commit -m "feat(subagent_tree): filter grouping nodes from counts and lists (D9)"
  ```

### Task 2.3: status_bar active_node_ids 走 real_node_list

**Files:**
- Modify: `src/tui/components/subagent_status_bar.rs`（`active_node_ids` 实现或调用点）

- [x] **Step 1: 定位 active_node_ids 实现**

  Run: `grep -n "active_node_ids\|node_list\|real_node_list" src/tui/components/subagent_status_bar.rs src/tui/app/event.rs`
  预期：`active_node_ids` 在 `event.rs` 或 status_bar.rs 中遍历 `node_list()`。

- [x] **Step 2: 改用 real_node_list**

  把 `active_node_ids(&self.subagent_tree)` 实现（或在 status_bar.rs 中遍历处）的 `tree.node_list()` 改为 `tree.real_node_list()`。若 `active_node_ids` 是 free function 接收 `&SubagentTree`，内部改用 `real_node_list()`。

- [x] **Step 3: 验证编译**

  Run: `cargo build 2>&1 | tail -5`
  Expected: 编译通过（或仅剩其它阶段未完成错误）。

- [x] **Step 4: Commit**

  ```bash
  git add src/tui/components/subagent_status_bar.rs src/tui/app/event.rs
  git commit -m "fix(status_bar): active_node_ids excludes grouping nodes (D9)"
  ```

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 3: App completed_at 字段 + 写入时机（D8 状态层）

### Task 3.1: App 新增 completed_at 字段

**Files:**
- Modify: `src/tui/app/mod.rs:107-126`（App struct 字段）、`src/tui/app/mod.rs:405-414`（App::new 初始化）

- [x] **Step 1: 加字段**

  在 `src/tui/app/mod.rs` App struct 中 `turn_started_at` 附近加：

  ```rust
  /// Completion timestamps for subagent nodes — used by the focus view
  /// selector to dim completed subagents and remove them after a delay.
  pub completed_at: HashMap<String, std::time::Instant>,
  ```

- [x] **Step 2: 初始化**

  在 `App::new`（约 405-414 行）初始化处加 `completed_at: HashMap::new(),`

- [x] **Step 3: 验证编译**

  Run: `cargo build 2>&1 | tail -5`
  Expected: 编译通过（字段未使用会有 warning，正常）。

### Task 3.2: SubagentUpdate 写入 completed_at（transition 时刻）

**Files:**
- Modify: `src/tui/app/event.rs:939-945`（`AppEvent::SubagentUpdate` 分支）

- [x] **Step 1: 定位 SubagentUpdate 分支**

  Run: `sed -n '935,950p' src/tui/app/event.rs`
  预期：`AppEvent::SubagentUpdate(progress) => { self.subagent_tree.upsert(*progress); ... focus.rebuild ... }`

- [x] **Step 2: 写入 completed_at transition**

  在 `upsert` 前后加 transition 检测（仅当状态转为完成态且之前非完成态时写入）：

  ```rust
  AppEvent::SubagentUpdate(progress) => {
      // Track completion time on transition to a terminal status
      let is_terminal = matches!(
          progress.status,
          SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
      );
      let was_terminal = self
          .subagent_tree
          .nodes
          .get(&progress.node_id)
          .map(|n| {
              matches!(
                  n.progress.status,
                  SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
              )
          })
          .unwrap_or(false);
      if is_terminal && !was_terminal {
          self.completed_at
              .insert(progress.node_id.clone(), std::time::Instant::now());
      }
      self.subagent_tree.upsert((*progress).clone());
      if let Some(ref mut focus) = self.subagent_focus {
          focus.rebuild(&self.subagent_tree);
      }
  }
  ```

  注意：需 import `SubagentStatus`（`use crate::agent::progress::SubagentStatus;`，若未导入）。`upsert` 接收 `SubagentProgress`（非 Clone 不可再借用？检查签名——`upsert(&mut self, progress: SubagentProgress)` 接收所有权）。因此需在 upsert **前**读取旧状态（上面 `was_terminal` 在 upsert 前读 ✓），upsert 用 `(*progress).clone()` 或重构。检查 `SubagentProgress` 是否 Clone —— `SubagentTree::upsert` 签名是 `progress: SubagentProgress`（所有权），`AppEvent::SubagentUpdate(Box<SubagentProgress>)`。原代码 `self.subagent_tree.upsert(*progress)` 移动所有权。为在 upsert 前读旧状态，`was_terminal` 已在 upsert 前读 ✓，upsert 仍用 `*progress`（但 progress 已被 `&progress.node_id` 借用完毕，移动安全）。实际写：先 `let node_id = progress.node_id.clone();` + `let is_terminal = ...`，再 upsert `*progress`。改写：

  ```rust
  AppEvent::SubagentUpdate(progress) => {
      let node_id = progress.node_id.clone();
      let is_terminal = matches!(
          progress.status,
          SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
      );
      let was_terminal = self
          .subagent_tree
          .nodes
          .get(&node_id)
          .map(|n| matches!(n.progress.status, SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled))
          .unwrap_or(false);
      if is_terminal && !was_terminal {
          self.completed_at.insert(node_id.clone(), std::time::Instant::now());
      }
      self.subagent_tree.upsert(*progress);
      if let Some(ref mut focus) = self.subagent_focus {
          focus.rebuild(&self.subagent_tree);
      }
  }
  ```

- [x] **Step 3: 验证编译**

  Run: `cargo build 2>&1 | tail -10`
  Expected: 编译通过。

### Task 3.3: Submit/clear 清空 completed_at

**Files:**
- Modify: `src/tui/app/event.rs:524`（`AppEvent::Submit` 中 `subagent_tree.clear()` 处）

- [x] **Step 1: 清空 completed_at**

  在 `self.subagent_tree.clear();` 旁加 `self.completed_at.clear();`。

- [x] **Step 2: 验证编译 + Commit Phase 3**

  Run: `cargo build 2>&1 | tail -5`
  ```bash
  git add src/tui/app/mod.rs src/tui/app/event.rs
  git commit -m "feat(app): track subagent completion time for focus view dim+removal (D8)"
  ```

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 4: subagent_focus_view.rs 状态与渲染重构（D1, D2, D3, D4, D6, D8 渲染层）

### Task 4.1: 移除 FocusArea / active_area（D1）

**Files:**
- Modify: `src/tui/components/subagent_focus_view.rs:149-176`（enum + struct 字段）、`181-202`（build）、`208-227`（rebuild）、`256-260`/`332-353`/`376-404`（render 边框/help 条件分支）
- Modify: `src/tui/app/event.rs:6`（import）、`38-127`（键位 active_area 守卫）

- [x] **Step 1: 删除 FocusArea enum + active_area 字段**

  删除 `subagent_focus_view.rs:149-154` 的 `FocusArea` enum。删除 struct 中 `pub active_area: FocusArea,` 字段（174 行）。删除 `build` 中 `active_area: FocusArea::Timeline,`（200 行）。

- [x] **Step 2: 修正 render 中的 active_area 引用**

  - `256-260`：`header_border` 改为恒 `inactive_border`（header 非交互区）。
  - `332-353`：`timeline_border` 恒 `inactive_border`；`selector_border` 恒 `active_border`（移除条件分支）。
  - `376-404`：`help_text` 的 `match state.active_area` 改为单一字符串：`"↑↓ navigate · Enter switch/exit · t fold · Esc back · wheel scroll timeline".to_string()`。

- [x] **Step 3: 删除 event.rs 的 FocusArea import + active_area 守卫**

  - `event.rs:6`：`use crate::tui::components::subagent_focus_view::{FocusArea, FocusViewState};` → `use crate::tui::components::subagent_focus_view::FocusViewState;`
  - `38-44`：删除 `KeyCode::Tab => { focus.active_area = ...; return; }` 块。
  - `48-71`：删除 `KeyCode::Up/Down/PageUp/PageDown if focus.active_area == FocusArea::Timeline =>` 四个 timeline 滚动分支（D5：focus view 内不再键盘滚 timeline）。
  - `72`：`KeyCode::Char('t') if focus.active_area == FocusArea::Timeline =>` → 去掉守卫：`KeyCode::Char('t') =>`。
  - `94-103`：`KeyCode::Up/Down if focus.active_area == FocusArea::Selector =>` → 去掉守卫：`KeyCode::Up =>` / `KeyCode::Down =>`。
  - `104`：`KeyCode::Enter if focus.active_area == FocusArea::Selector =>` → `KeyCode::Enter =>`。
  - `451-489`（MouseScrolled）：`if focus.active_area == FocusArea::Timeline {` 守卫去掉，鼠标滚轮始终滚 timeline。

- [x] **Step 4: 验证编译**

  Run: `cargo build 2>&1 | tail -15`
  Expected: 无 `FocusArea` / `active_area` 报错（可能仍有未完成的 D3/D8 渲染错误）。

### Task 4.2: build 中 selector_index 对齐当前 node_id（D2）

**Files:**
- Modify: `src/tui/components/subagent_focus_view.rs:181-202`（build）

- [x] **Step 1: selector_index 初始化为 pos+1**

  ```rust
  pub fn build(node_id: &str, tree: &SubagentTree) -> Option<Self> {
      let node = tree.nodes.get(node_id)?;
      let p = &node.progress;
      let pos = tree
          .real_node_list()
          .iter()
          .position(|id| id == node_id);
      let selector_index = pos.map(|i| i + 1).unwrap_or(0);
      Some(Self {
          node_id: node_id.to_string(),
          // ... 其余字段不变 ...
          scroll_offset: 0,
          auto_scroll: true,
          selector_index,
      })
  }
  ```

  注意：用 `real_node_list()`（D9）而非 `node_list()`，确保分组节点不计入偏移。

- [x] **Step 2: 写测试 — 光标对齐**

  在 `subagent_focus_view.rs` 的 `mod tests` 加：

  ```rust
  #[test]
  fn test_build_selector_index_aligns_with_current_node() {
      let mut tree = SubagentTree::default();
      tree.upsert(make_node("a", vec![]));
      tree.upsert(make_node("b", vec![]));
      tree.upsert(make_node("c", vec![]));
      // real_node_list = [a, b, c]; opening "c" (pos 2) → selector_index 3
      let state = FocusViewState::build("c", &tree).unwrap();
      assert_eq!(state.selector_index, 3);
      // opening "a" (pos 0) → selector_index 1
      let state_a = FocusViewState::build("a", &tree).unwrap();
      assert_eq!(state_a.selector_index, 1);
  }
  ```

  （`make_node` helper 已存在于该 test 模块，598 行。）

- [x] **Step 3: 运行测试验证**

  Run: `cargo test --lib subagent_focus_view::tests::test_build_selector_index 2>&1 | tail -10`
  Expected: PASS。

### Task 4.3: build_selector_lines 滚动跟随 + 完成态过滤 + 灰显（D3, D8 渲染层）

**Files:**
- Modify: `src/tui/components/subagent_focus_view.rs:468-540`（build_selector_lines）
- Modify: `src/tui/components/subagent_focus_view.rs:235-251`（render 签名 + selector 高度）
- Modify: `src/tui/app/render.rs`（传 completed_at + now 给 render）

- [x] **Step 1: 加 visible_node_ids helper + COMPLETED_REMOVE_DELAY 常量**

  在 `subagent_focus_view.rs` 顶部（use 之后）加：

  ```rust
  use std::time::Instant;

  /// Completed subagents are removed from the selector after this delay.
  pub const COMPLETED_REMOVE_DELAY_SECS: u64 = 10;

  /// Real subagent node IDs visible in the selector: grouping nodes excluded
  /// (via `real_node_list`), and completed nodes past the removal delay
  /// excluded — except the currently-viewed node, which is always kept.
  pub fn visible_node_ids(
      tree: &SubagentTree,
      completed_at: &HashMap<String, Instant>,
      now: Instant,
      current_node_id: &str,
  ) -> Vec<String> {
      tree.real_node_list()
          .into_iter()
          .filter(|id| {
              if id == current_node_id {
                  return true;
              }
              match completed_at.get(id) {
                  Some(t) => now.duration_since(*t).as_secs() < COMPLETED_REMOVE_DELAY_SECS,
                  None => true,
              }
          })
          .collect()
  }
  ```

- [x] **Step 2: 重构 build_selector_lines — 统一列表 + 滑动窗口 + 灰显**

  改 `build_selector_lines` 签名加 `completed_at` + `now`，重写为：

  ```rust
  fn build_selector_lines(
      state: &FocusViewState,
      tree: &SubagentTree,
      completed_at: &HashMap<String, Instant>,
      now: Instant,
      inner: Rect,
  ) -> Vec<Line<'static>> {
      let visible = visible_node_ids(tree, completed_at, now, &state.node_id);
      // unified list: ["main", ...visible]
      let total_len = 1 + visible.len();
      let available = inner.height as usize;
      let avail = available.min(total_len);

      // scroll_start: keep selector_index visible (sliding window)
      let mut scroll_start = 0usize;
      if state.selector_index < scroll_start {
          scroll_start = state.selector_index;
      }
      if state.selector_index >= scroll_start + avail {
          scroll_start = state.selector_index.saturating_sub(avail) + 1;
      }
      scroll_start = scroll_start.min(total_len.saturating_sub(avail));

      let mut lines: Vec<Line<'static>> = Vec::new();

      // "main" entry at index 0
      if avail > 0 && scroll_start == 0 {
          let is_main_selected = state.selector_index == 0;
          let selector = if is_main_selected { "▶ " } else { "  " };
          lines.push(Line::from(vec![
              Span::styled(selector, Style::default().fg(Color::Rgb(249, 226, 175))),
              Span::styled(
                  "main",
                  Style::default()
                      .fg(Color::Rgb(180, 180, 200))
                      .add_modifier(Modifier::BOLD),
              ),
          ]));
      }

      // subagent entries
      for (i, node_id) in visible.iter().enumerate() {
          let abs_index = i + 1; // +1 for main
          if abs_index < scroll_start || (abs_index >= scroll_start + avail && scroll_start > 0) {
              // skip items outside window when scrolled past main
          }
          if abs_index < scroll_start || abs_index >= scroll_start + avail {
              continue;
          }
          let is_current = node_id == &state.node_id;
          let is_selected = abs_index == state.selector_index;
          let node = tree.nodes.get(node_id);
          let (icon, icon_color) = if let Some(n) = node {
              selector_status_icon(&n.progress.status)
          } else {
              ("?", Color::DarkGray)
          };
          let current_marker = if is_current { " ●" } else { "" };
          let selector = if is_selected { "▶ " } else { "  " };
          let label = if let Some(n) = node {
              n.progress.label.clone()
          } else {
              node_id.clone()
          };
          let max_w = inner.width.saturating_sub(8) as usize;
          let display = truncate(&label, max_w);

          // dim completed-but-not-removed nodes
          let is_completed = matches!(
              node.map(|n| &n.progress.status),
              Some(SubagentStatus::Completed) | Some(SubagentStatus::Failed) | Some(SubagentStatus::Cancelled)
          );
          let label_color = if is_completed {
              Color::Rgb(90, 90, 110) // dim gray
          } else {
              Color::Rgb(220, 220, 235)
          };

          lines.push(Line::from(vec![
              Span::styled(selector, Style::default().fg(Color::Rgb(249, 226, 175))),
              Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
              Span::styled(display, Style::default().fg(label_color).add_modifier(Modifier::BOLD)),
              Span::styled(current_marker, Style::default().fg(Color::Rgb(249, 226, 175))),
          ]));
      }

      lines
  }
  ```

  注意：`truncate` 与 `selector_status_icon` 是已有 helper。窗口逻辑：当 `scroll_start > 0` 时 main 已滚出，从 visible 里取 `[scroll_start-1 .. scroll_start-1+avail]`。上面循环用 `abs_index` 与 `[scroll_start, scroll_start+avail)` 比较过滤——当 `scroll_start==0` 时 main 占第 0 行，visible 从 abs_index 1 起，`abs_index < 0+avail` 即 `abs_index < avail` 取前 `avail-1` 个（main 占 1 行）。需保证总行数 ≤ avail。仔细核算：`scroll_start==0` 时渲染 main + visible 中 `abs_index in [1, avail)` 即 `abs_index < avail` → 取 `avail-1` 个 visible，总行 = 1 + (avail-1) = avail ✓。`scroll_start>0` 时不渲染 main，取 `abs_index in [scroll_start, scroll_start+avail)` → avail 个 visible ✓。上面 `continue` 条件 `abs_index < scroll_start || abs_index >= scroll_start + avail` 正确。

- [x] **Step 3: render 签名加 completed_at + now，传给 build_selector_lines**

  `FocusView::render` 签名改为：

  ```rust
  pub fn render(
      f: &mut Frame,
      area: Rect,
      state: &FocusViewState,
      tree: &SubagentTree,
      completed_at: &HashMap<String, Instant>,
      now: Instant,
      spinner_frame: u8,
  ) {
  ```

  在调用 `build_selector_lines` 处传 `completed_at, now`。

- [x] **Step 4: 选择器高度 Length(6) → Length(8)（D4）**

  `subagent_focus_view.rs:247`：`Constraint::Length(6),` → `Constraint::Length(8),`

- [x] **Step 5: render.rs 传 completed_at + now**

  在 `src/tui/app/render.rs` 调用 `FocusView::render(...)` 处，加 `&self.completed_at, std::time::Instant::now()`。Run: `grep -n "FocusView::render" src/tui/app/render.rs` 定位。

- [x] **Step 6: 写测试 — 滚动跟随 + 不越界 + 完成态过滤**

  ```rust
  #[test]
  fn test_build_selector_lines_no_overflow() {
      use std::time::Instant;
      let mut tree = SubagentTree::default();
      for i in 0..10 { tree.upsert(make_node(&format!("s{}", i), vec![])); }
      let state = FocusViewState::build("s0", &tree).unwrap();
      let inner = Rect::new(0, 0, 80, 4); // available=4
      let now = Instant::now();
      let completed_at = HashMap::new();
      let lines = build_selector_lines(&state, &tree, &completed_at, now, inner);
      assert!(lines.len() <= 4, "lines must not exceed available height");
  }
  ```

  完成态过滤测试 + 滚动跟随测试见 tasks.md 6.4/6.6，此处可一并加。

- [x] **Step 7: 验证编译 + 测试**

  Run: `cargo build 2>&1 | tail -10 && cargo test --lib subagent_focus_view 2>&1 | tail -15`
  Expected: 编译通过，新测试 PASS。

- [x] **Step 8: Commit**

  ```bash
  git add src/tui/components/subagent_focus_view.rs src/tui/app/render.rs
  git commit -m "feat(focus_view): scroll-follow selector, completion dim+removal, remove FocusArea (D1,D3,D4,D6,D8)"
  ```

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 5: event.rs focus view 键位 + 可见列表导航（D5, D8 导航层）

### Task 5.1: ↑↓ 导航基于可见列表 + Enter 切换

**Files:**
- Modify: `src/tui/app/event.rs:94-118`（focus view ↑↓/Enter 分支）

- [x] **Step 1: ↑↓ 用 visible_node_ids 长度**

  ```rust
  KeyCode::Up => {
      let now = std::time::Instant::now();
      let visible = visible_node_ids(&self.subagent_tree, &self.completed_at, now, &focus.node_id);
      let len = visible.len() + 1; // +1 for main
      focus.selector_index = wrap_prev(focus.selector_index, len);
      return;
  }
  KeyCode::Down => {
      let now = std::time::Instant::now();
      let visible = visible_node_ids(&self.subagent_tree, &self.completed_at, now, &focus.node_id);
      let len = visible.len() + 1;
      focus.selector_index = wrap_next(focus.selector_index, len);
      return;
  }
  KeyCode::Enter => {
      if focus.selector_index == 0 {
          exit_focus = true;
      } else {
          let now = std::time::Instant::now();
          let visible = visible_node_ids(&self.subagent_tree, &self.completed_at, now, &focus.node_id);
          let new_state = visible
              .get(focus.selector_index - 1)
              .and_then(|id| FocusViewState::build(id, &self.subagent_tree));
          if let Some(state) = new_state {
              *focus = state;
          }
          return;
      }
  }
  ```

  需 import `visible_node_ids`：`use crate::tui::components::subagent_focus_view::visible_node_ids;`（在 event.rs 顶部）。

- [x] **Step 2: 验证编译 + Commit**

  Run: `cargo build 2>&1 | tail -10`
  ```bash
  git add src/tui/app/event.rs
  git commit -m "feat(focus_view): arrow keys navigate visible selector list (D5,D8 nav)"
  ```

### Task 5.2: 核对 D10/D11/D12 已有实现完整

**Files:**
- Modify: `src/tui/app/event.rs`（状态栏 + 主聊天 + update_style 调用点）、`src/tui/components/input.rs`

- [x] **Step 1: 核对状态栏 ↑↓ 自动激活 + 移除 Tab（D10）**

  Run: `sed -n '305,360p' src/tui/app/event.rs` 确认：Tab 切换已删，`KeyCode::Up || KeyCode::Down` 自动置 `subagent_status_bar_focused = true`，Esc 取消，Enter 打开 focus view。若有缺失补全。

- [x] **Step 2: 核对主聊天 ↑↓ 滚动移除（D11）**

  Run: `sed -n '355,375p' src/tui/app/event.rs` 确认：`KeyCode::Up`/`Down` 单行滚动分支已删，仅留 `PageUp`/`PageDown`。

- [x] **Step 3: 核对 input.rs update_style + 调用点（D12）**

  Run: `grep -n "update_style" src/tui/components/input.rs src/tui/app/event.rs` 确认 `update_style()` 已抽出，`render()` 不再设 slash 样式；event.rs 在 `textarea.input`、补全 `insert_str`、粘贴 `insert_char`、`take_text`、Shift+Enter 后均调用。补全任何漏调点。

- [x] **Step 4: 验证编译 + Commit（若有补全）**

  Run: `cargo build 2>&1 | tail -5`
  若有改动：`git add -A && git commit -m "chore: complete D10/D11/D12 call sites"`

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 6: 测试更新与新增

### Task 6.1: 更新现有断言 + 新增测试

**Files:**
- Modify: `src/tui/components/subagent_focus_view.rs`（mod tests）

- [x] **Step 1: 更新 test_build_from_node（若有 active_area 断言）**

  Run: `grep -n "active_area\|selector_index" src/tui/components/subagent_focus_view.rs` 定位测试。移除 `active_area` 断言，`selector_index` 期望值改 `pos+1`。

- [x] **Step 2: 新增完成态过滤测试**

  ```rust
  #[test]
  fn test_visible_node_ids_filters_completed_after_delay() {
      use std::time::{Instant, Duration};
      let mut tree = SubagentTree::default();
      tree.upsert(make_node("a", vec![]));
      tree.upsert(make_node("b", vec![]));
      let now = Instant::now();
      let mut completed_at = HashMap::new();
      // b completed 20s ago — past delay
      completed_at.insert("b".to_string(), now - Duration::from_secs(20));
      let visible = visible_node_ids(&tree, &completed_at, now, "a");
      assert!(visible.contains(&"a".to_string()));
      assert!(!visible.contains(&"b".to_string())); // removed
      // current node exempt even if completed+past delay
      let visible_cur = visible_node_ids(&tree, &completed_at, now, "b");
      assert!(visible_cur.contains(&"b".to_string()));
  }
  ```

- [x] **Step 3: 新增状态栏自动激活测试（若可单测）**

  状态栏键位在 event.rs 集成层，若难以单测则跳过单测、靠手动验收。在 tasks.md 标注。

- [x] **Step 4: 运行全部测试**

  Run: `cargo test --lib 2>&1 | tail -20`
  Expected: 全 PASS。

- [x] **Step 5: Commit**

  ```bash
  git add src/tui/components/subagent_focus_view.rs src/tui/components/subagent_tree.rs
  git commit -m "test: cursor alignment, scroll-follow, completion filter, grouping filter"
  ```

### Task 6.2: task.rs 测试确认无包装节点

**Files:**
- Modify: `src/tools/meta/task.rs`（mod tests，若有断言包装节点）

- [x] **Step 1: 检查 task.rs 测试**

  Run: `grep -n "root_node_id\|task:\|parent_id" src/tools/meta/task.rs | grep test` 定位。若有断言包装节点存在，更新为断言 subagent 为 root（`parent_id: None`）。

- [x] **Step 2: 运行 task 测试**

  Run: `cargo test --lib task 2>&1 | tail -10`
  Expected: PASS。

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Phase 7: 构建与验收

### Task 7.1: cargo build + cargo test

- [x] **Step 1: cargo build**

  Run: `cargo build 2>&1 | tail -10`
  Expected: 无错误。

- [x] **Step 2: cargo test**

  Run: `cargo test 2>&1 | tail -20`
  Expected: 全 PASS。

- [x] **Step 3: clippy（可选）**

  Run: `cargo clippy --lib 2>&1 | tail -15`
  Expected: 无 error（warning 记录但不阻塞）。

### Task 7.2: 手动验收（按 spec 场景）

- [x] **Step 1: 启动 TUI，触发 subagent**

  `cargo run`，输入需要 subagent 的请求（如 `@explore` 搜索代码），等待 subagent 启动。

- [x] **Step 2: 验收 subagent-focus-view 场景**

  状态栏 Enter 打开 focus view，逐项核对：
  - ↑↓ 导航选择器（含 main），wrap ✓
  - ↑↓ 不滚 timeline ✓
  - Enter 切换 subagent / 在 main 退出 ✓
  - 鼠标滚轮滚 timeline ✓
  - PageUp/PageDn 不响应 ✓
  - Tab no-op ✓
  - 't' 折叠工具调用 ✓
  - 选择器滚动跟随（多 subagent 时 ▶ 始终可见）✓
  - 光标 ▶ 进入时对齐当前 ● ✓
  - 选择器边框高亮、timeline 边框暗淡 ✓
  - 完成态 subagent 灰显 ✓
  - 完成超 10s 后从 selector 移除（当前查看的例外）✓
  - 无 "task:" 空条目 ✓
  - delegate 分组节点不出现 ✓

- [x] **Step 3: 验收 subagent-status-display 场景**

  - 状态栏可见时 ↑↓ 自动激活并导航（无需 Tab）✓
  - Esc 取消焦点 ✓
  - Tab 无效 ✓
  - Enter 打开 focus view ✓
  - 字符输入仍进输入框 ✓
  - active 计数不含分组节点（delegate 包装不虚高）✓

- [x] **Step 4: 验收主聊天 + 输入框**

  - PageUp/PageDn 滚聊天 ✓
  - 鼠标滚轮滚聊天 ✓
  - ↑↓ 无活跃 subagent 时不滚聊天（不响应）✓
  - 输入框在 subagent 频繁更新时不闪烁/消失 ✓
  - slash 命令（`@`/`/`）着色正常 ✓

- [x] **Step 5: 记录验收结果**

  在 tasks.md 验收任务旁标注通过/失败；失败项回 Phase 修复。

### Task 7.3: 最终提交 + 准备 verify

- [x] **Step 1: 确认所有任务勾选**

  Run: `grep -c '\- \[ \]' openspec/changes/fix-subagent-focus-nav/tasks.md`
  Expected: `0`（全勾选）。

- [x] **Step 2: 加载 requesting-code-review（executing-plans 模式）**

  使用 Skill 工具加载 `superpowers:requesting-code-review`，请求至少一次代码审查。CRITICAL 发现先修复。

- [x] **Step 3: 运行 build guard**

  Run: `bash "$COMET_GUARD" fix-subagent-focus-nav build --apply`
  Expected: ALL CHECKS PASSED，phase → verify。

archived-with: 2026-07-05-fix-subagent-focus-nav
---

## Self-Review

**Spec coverage:**
- subagent-focus-view delta：↑↓ 导航（Task 4.1/5.1）、Enter 切换/退出（5.1）、鼠标滚 timeline（4.1）、Tab no-op（4.1）、't' 折叠（4.1）、滚动跟随（4.3）、光标对齐（4.2）、wrap 含 main（5.1）、▶/● 区分（4.3）、边框（4.1）、完成态灰显（4.3）、延迟移除（4.3）、当前 node 例外（4.3）、分组节点不显示（2.1/2.2）✓
- subagent-status-display delta：↑↓ 自动激活（5.2 核对）、Esc 取消（5.2）、Tab 无效（5.1 删 Tab）、Enter focus view（5.2 核对）、字符输入（5.2 核对）✓
- D7 task 包装移除（1.1/1.2）、D11 主聊天滚动（5.2）、D12 input.rs（5.2）✓

**Placeholder scan:** 无 TBD/TODO；每个 code step 含完整代码或精确行号编辑。

**Type consistency:** `visible_node_ids(tree, completed_at, now, current_node_id) -> Vec<String>` 在 4.3 定义，5.1 调用签名一致；`is_grouping_node(&str) -> bool`、`real_node_list() -> Vec<String>` 一致；`completed_at: HashMap<String, Instant>` 在 3.1 定义，4.3/5.1 使用一致。

**缺口：** 状态栏自动激活（D10）单测缺（集成层难单测），靠手动验收 Task 7.2 Step 3 覆盖。
