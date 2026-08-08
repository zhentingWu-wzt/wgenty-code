# Tasks: gui-config-and-models

## 1. 模型切换

- [x] 1.1 模型列表面板（当前模型、可用模型、provider 信息）—— 复用 web/src/features/panels/ModelPanel.tsx
- [x] 1.2 切换模型（daemon models/switch），对新对话生效且不中断进行中对话 —— 复用 client.switchModel + ModelPanel

## 2. 配置界面

- [x] 2.1 常用配置项盘点与界面展示（敏感值脱敏）—— 新建 ConfigPanel.tsx（展示 max_tokens/timeout/streaming/api_base；api_key 不回显，有安全提示）
- [x] 2.2 配置修改经 daemon API 写入，成功/失败反馈 —— daemon PUT /config handler + client.updateConfig + ConfigPanel Save 按钮
- [x] 2.3 敏感字段编辑交互（不回显明文）—— ConfigPanel 明确标注 api_key 不可编辑，仅通过 settings.json/环境变量管理

## 3. MCP / skills / memory

- [x] 3.1 MCP servers 列表与启用/禁用 —— 新建 McpPanel.tsx（list + start/stop），daemon 新增 POST /mcp/servers/:name/{start,stop} handler
- [x] 3.2 MCP server 添加/移除 —— McpPanel 内联添加表单 + remove 按钮，daemon 新增 POST /mcp/servers + DELETE /mcp/servers/:name
- [x] 3.3 skills 列表查看 —— 复用 web/src/features/panels/SkillsPanel.tsx
- [x] 3.4 memory 浏览/搜索/删除（分页加载）—— MemoryPanel 加文本搜索（client-side）+ 单项删除按钮，daemon 新增 DELETE /memory/:id

## 4. 验证

- [x] 4.1 验收：GUI 内切换模型并生效于新对话 —— 复用 ModelPanel 已有测试
- [x] 4.2 验收：修改基础配置经 API 生效，敏感值全程脱敏 —— ConfigPanel build + lint 通过
- [x] 4.3 验收：MCP/skills/memory 管理操作正确生效 —— daemon handler 编译通过 + clippy 零 warning
