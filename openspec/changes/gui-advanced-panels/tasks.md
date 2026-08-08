# Tasks: gui-advanced-panels

## 1. subagent 进度树

- [ ] 1.1 subagent 树形面板：层级、状态、工具执行进度（对接 daemon subagent API）
- [ ] 1.2 节点展开详情与状态节流刷新

## 2. todos 面板

- [ ] 2.1 todos 面板实时同步任务清单状态（新增/进行中/完成）

## 3. 透视面板

- [ ] 3.1 透视面板框架：tab + 分栏布局，挂接面板挂载点
- [ ] 3.2 五类数据展示：system prompt 分层、召回记忆、messages、hook 注入、token 统计（复用 tui-inspector 采集层）
- [ ] 3.3 数据不可用提示与敏感内容默认折叠

## 4. 验证

- [ ] 4.1 验收：subagent 执行过程树形可视化且状态实时更新
- [ ] 4.2 验收：todos 变更实时同步
- [ ] 4.3 验收：透视面板五类数据正确展示，无数据时明确提示
