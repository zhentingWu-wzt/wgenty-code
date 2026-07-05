# Tasks — fix-subagent-tree-multi-root

修复 SubagentTree 多 root 孤儿回归，让 `node_list` 支持森林遍历。参考 `design.md`。

## 1. node_list 森林遍历

- [x] 1.1 `src/tui/components/subagent_tree.rs`：重写 `node_list()`——先 DFS `root_id`，再遍历其它 `parent_id: None` 节点（按 `started_at` + `node_id` 排序），visited 集合防重复
- [x] 1.2 新增测试：两个顶层 subagent（均 `parent_id: None`）都出现在 `node_list` / `real_node_list` / `active_count`
- [x] 1.3 新增测试：delegate wrapper（root）+ 独立 task subagent（parent_id None）共存，两者都可达
- [x] 1.4 验证现有测试全过（visited 兼容 fixture 的 parent_id 全 None 模式，不产生重复）

## 2. 构建与验收

- [x] 2.1 `cargo build` 通过
- [x] 2.2 `cargo test --lib` 通过
- [x] 2.3 手动验收：主 agent 开多个 task subagent，状态栏选择器显示全部；第一个完成后状态栏仍显示后续活跃 subagent — 自动化测试（503 passed，含 2 个森林测试）+ build 通过；交互式 TUI 手动验收 deferred 到 verify 阶段
