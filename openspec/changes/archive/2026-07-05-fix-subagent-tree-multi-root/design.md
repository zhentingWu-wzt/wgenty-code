## Context

`SubagentTree` 用 `root_id: Option<String>` 跟踪树根。`upsert` 把第一个 `parent_id: None` 的节点设为 `root_id`，`node_list` 从 `root_id` DFS。这只能表达单根树。

D7（`fix-subagent-focus-nav`）移除 `task` 包装节点后，每个 task subagent 直接 `parent_id: None`。多个顶层 subagent 时，第二个起成为孤儿（在 `nodes` 里但不可达），状态栏选择器丢失。

## Fix

重写 `node_list()` 为森林遍历：

1. 先 DFS 走 `root_id`（back-compat，保持原有 DFS 起点与顺序）。
2. 再遍历 `nodes` 中所有 `parent_id.is_none()` 且未被访问的节点，按 `started_at`（升序）+ `node_id`（tie-break）排序后依次 DFS。
3. visited 集合防重复（兼容测试 fixture：`make_node` 对所有节点设 `parent_id: None`，但 root_id 先走 → 其 children 被访问 → 后续 loop 跳过，不产生重复）。

```rust
pub fn node_list(&self) -> Vec<String> {
    let mut list = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    fn walk(tree: &SubagentTree, node_id: &str, list: &mut Vec<String>, visited: &mut HashSet<String>) {
        if !visited.insert(node_id.to_string()) { return; }
        list.push(node_id.to_string());
        if let Some(node) = tree.nodes.get(node_id) {
            for child in &node.children { walk(tree, child, list, visited); }
        }
    }
    if let Some(ref root) = self.root_id { walk(self, root, &mut list, &mut visited); }
    // Forest: walk any other top-level nodes (parent_id None) not yet visited.
    // Deterministic order by started_at, then node_id.
    let mut extra: Vec<&String> = self.nodes.iter()
        .filter(|(id, n)| n.progress.parent_id.is_none() && !visited.contains(*id))
        .map(|(id, _)| id)
        .collect();
    extra.sort_by(|a, b| {
        let sa = self.nodes[a].progress.started_at;
        let sb = self.nodes[b].progress.started_at;
        sa.cmp(&sb).then_with(|| a.cmp(b))
    });
    for root in extra { walk(self, root, &mut list, &mut visited); }
    list
}
```

`upsert` / `clear` / `root_id` 字段不变。`real_node_list` / `active_count` / `active_node_ids` 等基于 `node_list` 的方法自动继承森林支持。

## Non-Goals

- 不改 `root_id` 字段类型（保持 `Option<String>`，作为"第一个根"的 back-compat）。
- 不改测试 fixture（visited 兼容现有 `parent_id: None` 全设模式）。
- 不改 `upsert` 的 `root_id` 设置逻辑。
- 不动 delegate 1:N 分组（D9 `is_grouping_node` 过滤仍生效）。

## Risks

- **顺序非确定性** → 用 `started_at` + `node_id` 排序 extra roots 保证稳定。
- **visited 防重复** → 测试 fixture 的 `make_node`（parent_id 全 None）不会产生重复条目（root_id 先走标记 children 已访问）。
- **回归** → `node_list` 只读，DFS 从 root_id 起的顺序不变；现有测试应全过。
