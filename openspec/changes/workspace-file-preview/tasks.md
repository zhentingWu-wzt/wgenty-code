# Tasks: workspace-file-preview

## 1. Daemon — 路径边界与列表端点

- [x] 1.1 在 `src/daemon/fs.rs` 实现 `resolve_workspace_roots()`：收集已注册项目根 + 活跃 worktree 根（canonicalize），供两个端点共享
- [x] 1.2 实现 `GET /api/v1/fs/entries`：目录+文件混合列表（`name`/`is_dir`/`size`），目录在前、同级名称不区分大小写排序，忽略隐藏文件与符号链接，2000 条上限 + `truncated` 标记，路径越界 403、不存在 404
- [x] 1.3 注册路由（`src/daemon/routes.rs`）并更新 `docs/API.md`

## 2. Daemon — 文件读取端点

- [x] 2.1 实现 `GET /api/v1/fs/file`：8KB null-byte 探测 + 扩展名白名单分流——文本返回 JSON（行数组 + `version{mtime_ms,size}`，UTF-8 失败降级 octet-stream + `is_binary`），白名单二进制按 mime 返回原始字节
- [x] 2.2 大小上限：文本 1.5MB / 二进制 5MB，超限 413 JSON（`{size, limit}`）

## 3. Daemon — 测试

- [x] 3.1 集成测试：路径边界三类（注册项目根内 200 / worktree 根内 200 / `/etc/passwd` 与符号链接逃逸 403）
- [x] 3.2 集成测试：列表排序/隐藏文件忽略/截断标记；文本读取行结构与 version；二进制 mime 与 413 超限

## 4. Web — API 层与状态

- [x] 4.1 `api/types.ts` + `api/client.ts`：`FsEntry`/`FileContent`/`FileVersion` 类型与 `listEntries`/`fetchFile`（文本走 JSON，二进制按 blob 消费）
- [x] 4.2 `previewStore.ts`（zustand）：tab 集合（按 workspace+path 去重）、激活 tab、关闭/清空

## 5. Web — 文件树

- [x] 5.1 `FileTree.tsx`：挂载于 `ProjectTree` 的 task 节点下，懒加载逐级展开，目录可折叠、文件点击开预览；`target/`、`node_modules/` 置灰默认折叠；截断时显示"已截断"

## 6. Web — 预览面板

- [x] 6.1 `PreviewPanel.tsx` tab 框架：主区呈现、复用/去重、关闭交互，加载与错误态
- [x] 6.2 文本/代码预览：行号 + shiki 高亮（>256KB 降级纯文本）
- [x] 6.3 Markdown 渲染（marked + shiki 代码块，禁内嵌 HTML）与查看源码切换
- [x] 6.4 图片（含 SVG 以 `<img>` blob 呈现）与 PDF（iframe/blob）；不支持二进制与超限（413）友好提示

## 7. 收尾

- [x] 7.1 `cargo fmt`/`clippy`/`cargo test` 与 web `tsc`/`build`/`test` 全绿；`openspec validate` 通过
