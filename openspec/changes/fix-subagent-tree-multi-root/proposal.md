## Why

`fix-subagent-focus-nav` 的 D7 移除了 `task` 工具的包装根节点，让每个 task subagent 直接以 `parent_id: None` 成为 tree root。但 `SubagentTree::upsert` 只把**第一个** `parent_id: None` 节点写入 `root_id`，`node_list()` 只从 `root_id` 开始 DFS——后续顶层 subagent（第二个 task subagent、或第一个完成后新开的 task subagent）成了**孤儿**：存在于 `nodes` 但从 `root_id` 走不到，不出现在 `node_list()` / `active_node_ids()` / 状态栏选择器。

D7 之前，那个永不更新、永远 Running 的 "task:" 包装节点一直是 root，把状态栏"撑住"了（显示无用的 task: 条目）。D7 移除包装节点后，这个早就存在的孤儿问题暴露：第一个 subagent 完成后 / 多个 subagent 并行时，主 agent 窗口的状态栏选择器消失。

**已确认**：诊断测试 `node_list = ["sub1"]`（sub2 丢失），`active_count = 1`。

## Root Cause

`src/tui/components/subagent_tree.rs`:
```rust
pub fn upsert(&mut self, progress: SubagentProgress) {
    ...
    if parent_id.is_none() && self.root_id.is_none() {  // 只接受第一个！
        self.root_id = Some(node_id.clone());
    }
    ...
}

pub fn node_list(&self) -> Vec<String> {
    ...
    if let Some(ref root) = self.root_id { walk(self, root, &mut list); }  // 只走 root_id
    list
}
```

`root_id: Option<String>` 只能持有一个根，`node_list` 只遍历它。多顶层 subagent 场景下后续节点不可达。

## Fix Goal

`SubagentTree` 支持森林（多 root）：`node_list()` 在遍历 `root_id` 之后，继续遍历所有其它 `parent_id: None` 的节点（按 `started_at` 排序保证确定性），用 visited 集合防重复。这样每个顶层 subagent 都出现在 `node_list` / `active_node_ids` / 状态栏。

不改动 `upsert` 的 `root_id` 设置逻辑（保持 back-compat，`root_id` 仍是"第一个根"），不改动测试 fixture（visited 防重复使现有 fixture 仍正确）。改动范围：仅 `subagent_tree.rs`。

## Impact

- **代码**：`src/tui/components/subagent_tree.rs`（`node_list` 重构 + 新增森林测试）。
- **测试**：`subagent_tree.rs` 新增多 root 场景测试；现有测试不变（visited 兼容 fixture 的 `parent_id: None` 全设模式）。
- **spec**：无 delta。`subagent-status-display` 现有 spec 已要求"列出每个活跃 subagent"，本修复让实现匹配 spec。
- **回归风险**：极低。`node_list` 是只读遍历，visited 防重复保证不产生重复条目；DFS 顺序从 root_id 起保持稳定。
