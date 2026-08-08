# Tasks: gui-config-and-models

## 1. 模型切换

- [ ] 1.1 模型列表面板（当前模型、可用模型、provider 信息）
- [ ] 1.2 切换模型（daemon models/switch），对新对话生效且不中断进行中对话

## 2. 配置界面

- [ ] 2.1 常用配置项盘点与界面展示（敏感值脱敏）
- [ ] 2.2 配置修改经 daemon API 写入，成功/失败反馈
- [ ] 2.3 敏感字段编辑交互（不回显明文）

## 3. MCP / skills / memory

- [ ] 3.1 MCP servers 列表与启用/禁用
- [ ] 3.2 MCP server 添加/移除
- [ ] 3.3 skills 列表查看
- [ ] 3.4 memory 浏览/搜索/删除（分页加载）

## 4. 验证

- [ ] 4.1 验收：GUI 内切换模型并生效于新对话
- [ ] 4.2 验收：修改基础配置经 API 生效，敏感值全程脱敏
- [ ] 4.3 验收：MCP/skills/memory 管理操作正确生效
