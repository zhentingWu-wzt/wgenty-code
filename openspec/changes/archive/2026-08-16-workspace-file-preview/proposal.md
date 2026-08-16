# Proposal: workspace-file-preview

## Why

Web 端（浏览器薄客户端）目前只能管理会话与任务，无法浏览或查看工作区中的文件——用户看到 agent 修改了哪些文件，却必须切回终端/本地编辑器才能看内容。为文件预览补上 daemon 读取 API 与前端面板，是"浏览器里完成 IDE 闭环"的第一步（后续编辑能力将复用本次的读取端点与版本信息）。

## What Changes

- daemon 新增 `GET /api/v1/fs/entries`：返回指定目录下目录+文件混合列表（`name`/`is_dir`/`size`），供前端文件树懒加载
- daemon 新增 `GET /api/v1/fs/file`：读取文件内容——文本返回带行号的结构化内容与版本信息（mtime/size），二进制按 mime 返回原始字节
- 路径安全：两个端点均要求目标路径 canonicalize 后位于**已注册项目根或其 worktree 根**内，越界返回 403；不做全盘浏览（现有 `GET /fs/dirs` 保持不变，继续服务目录选择器场景）
- 大小上限：文本 1.5MB、二进制（图片/PDF 等）5MB，超限返回 413 与结构化错误
- web 前端新增工作区文件树（挂在 `ProjectTree` 的 task/workspace 节点下，懒加载）与主区预览 tab：文本/代码带行号与语法高亮（复用 shiki）、Markdown 渲染、图片/PDF 以 blob URL 加载、超限/二进制不支持时给出友好提示

非目标（明确不在本 change 内）：文件编辑与冲突处理——**产品决策：编辑从 web 端路线图移除**（本地部署下是降级体验且污染 per-turn checkpoint），替代承接为「预览选中行引用到对话」与 `vscode://` 跳转本地编辑器（后续候选，成本 < 编辑器的 1/10）；重新评估触发条件：远程/headless workspace 成为一等部署模式。完整决策记录见 `docs/superpowers/brainstorm-summary.md`。其余非目标：Office/CAD/压缩包预览、LSP 桥接、对外部目录的浏览授权。

## Capabilities

### New Capabilities
- `workspace-file-preview`: 工作区文件浏览与预览——daemon 文件列表/内容读取 API（路径约束、大小上限、版本信息）与 web 端文件树、多类型预览面板的行为契约

### Modified Capabilities

（无——`web-agent-frontend` spec 覆盖的是既有面板行为，本次新增面板为独立 capability，不修改其既有 Requirement。）

## Impact

- **daemon（Rust）**：`src/daemon/fs.rs` 新增两个 handler 与共享路径校验辅助；`src/daemon/routes.rs` 注册路由；需新增集成测试（路径越界、上限、二进制探测）
- **web（React/TS）**：`features/` 下新增文件树与预览面板组件；`api/client.ts` 增加对应方法与类型；`zustand` 状态扩展（打开的预览 tab 集合）
- **依赖**：前端不新增重型依赖（高亮用已有 shiki；PDF 用浏览器原生 `iframe`/blob，不引 pdf.js）
- **兼容性**：纯新增端点，无破坏性变更；`GET /fs/dirs` 行为不变
