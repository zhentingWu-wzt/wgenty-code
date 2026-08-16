---
role: technical-design
canonical_spec: openspec
comet_change: workspace-file-preview
---

# Design Doc: workspace-file-preview

> 深度技术设计（Superpowers 产出）。行为契约见 `specs/workspace-file-preview/spec.md`，决策背景见 `design.md` 与 `docs/superpowers/brainstorm-summary.md`。

## 1. 实现蓝图

### 1.1 Daemon — `src/daemon/workspace_files.rs`（新模块）

```
workspace_files.rs
├── resolve_workspace_roots(state) -> Vec<PathBuf>      // 注册根 + worktree 根，canonicalize
├── ensure_within_roots(raw, roots) -> Result<PathBuf, FsError>  // 403 / 404 共享校验
├── GET /api/v1/fs/entries?path=<dir>
│   └── FsEntries { current, entries: Vec<FsEntry{name,is_dir,size}>, truncated }
├── GET /api/v1/fs/file?path=<file>
│   ├── 文本 → Json(FileTextResponse { lines: Vec<String>, version: FileVersion, encoding })
│   └── 二进制 → (StatusCode, [(header::CONTENT_TYPE, mime)], Vec<u8>)
└── FsError -> (StatusCode, Json(ErrorBody))            // 400/403/404/413 统一形态
```

要点：

- **根收集**：`state.projects.main_root()` + `registered_roots()` + 各项目 `worktrees::list` 的 worktree 路径；逐个 `canonicalize()`（失败跳过该根——磁盘上已不存在的 worktree 不应让端点 500）
- **校验顺序**：先 canonicalize 请求路径（不存在 → 404），再做 roots 前缀匹配（不匹配 → 403，错误体不区分"存在与否"避免探测）
- **entries**：`read_dir` 单遍收集；`file_type()` 判目录/文件，symlink（`is_symlink()`）整体跳过；目录在前、同级 `to_lowercase()` 排序；>2000 条置 `truncated: true` 并停止收集；子项 `metadata()` 失败按 size 0 处理
- **file 探测**：读前 8192 字节，含 `\0` → 二进制；再按扩展名白名单（png/jpg/jpeg/gif/webp/svg/pdf）分流；其余尝试 `String::from_utf8`，失败降级 `application/octet-stream` 并附 `is_binary: true` 的 JSON 头响应（见 1.2 例外）
- **上限**：文本 1.5MB / 二进制 5MB，在读内容**之前**用 `metadata().len()` 判定，超限直接 413 `{size, limit}`，不读文件体
- **路由注册**：`routes.rs` 挂入现有 protected router（`auth::require_auth` 之后），与 `/fs/dirs` 并立

### 1.2 响应形态例外：非白名单二进制

未进 mime 白名单、又不是合法 UTF-8 的文件（如 `.exe`、无扩展名二进制）：返回 JSON `FileTextResponse` 的变体 `{ is_binary: true, version, encoding: null }`（无 `lines`），前端据此显示"二进制文件，暂不支持预览"。白名单内二进制才走原始字节流。这样前端只需一个 `fetchFile`，凭响应形态分流。

### 1.3 Web — API 层

```ts
// api/types.ts
interface FsEntry { name: string; is_dir: boolean; size: number }
interface FsEntries { current: string; entries: FsEntry[]; truncated: boolean }
interface FileVersion { mtime_ms: u64; size: u64 }
type FileContent =
  | { kind: "text"; lines: string[]; version: FileVersion }
  | { kind: "binary-unsupported"; version: FileVersion }
  | { kind: "blob"; mime: string; blob: Blob }

// api/client.ts
listEntries(path: string): Promise<FsEntries>
fetchFile(path: string): Promise<FileContent>   // 按 Content-Type 分流
```

### 1.4 Web — 状态与 UI

- **tab 接入 uiStore**：id 规范 `preview:<absPath>`；新增 `PreviewTabMeta { workspaceRoot, relPath, kind }` 挂在现有 tab meta 通道（照 `subagent:` 模式）；重复打开 = `openTab` 幂等激活；project 卸载/切换时经现有 `pruneTabs` 清理归属 tab
- **FileTree.tsx**（`features/files/`）：挂在 `ProjectTree` task 节点下的「文件」分组节点（默认折叠，展开才发首个 `listEntries`）；子目录逐级懒加载；`target/`、`node_modules/`、`dist/` 置灰且默认折叠；`truncated` 时尾部显示"已截断"
- **PreviewPanel.tsx**（`features/files/`）：由 tab id 解析 meta 后分流渲染——
  - 代码：复用 `chat/CodeBlock` 的 highlighter 单例，新增 `showLineNumbers` 模式（shiki `transformers` 注入行号 DOM）；>256KB 跳过高亮纯文本渲染
  - Markdown：`react-markdown + remark-gfm`，`components.code` 委托 CodeBlock；工具栏含"源码"切换
  - 图片：`URL.createObjectURL(blob)` + `<img>`（SVG 同路径，天然禁脚本）；卸载时 `revokeObjectURL`
  - PDF：blob URL 塞 `<iframe>`
  - fallback/超限（413/`binary-unsupported`）：图标 + 实际大小提示

## 2. 测试策略

- **daemon 集成测试**（`tests/`）：临时目录注册为伪项目根——边界三类（根内 200 / worktree 内 200 / `/etc/passwd`+symlink 逃逸 403）；排序与隐藏忽略与截断；文本行结构与 version 字段；png mime；413；`is_binary` 降级
- **web 单测**：uiStore preview tab 幂等/去重/prune；FileTree 截断提示渲染；PreviewPanel 分流（text/md/fallback）
- **验收**：`cargo test` + `clippy` + `fmt`；web `tsc && vite build && vitest`

## 3. 边界条件清单

- worktree 磁盘已删 → 根收集时跳过，不 500
- 路径不存在 vs 越界 → 404 先于 403（canonicalize 失败即 404）；403 响应统一文案"outside registered workspaces"
- 目录列表中混入 socket/fifo → `file_type()` 非 dir/file 一律跳过
- 空文件（0 字节）→ 文本路径返回空 `lines` + version
- 超大文件**先判后读**，避免读入再丢弃
- blob URL 生命周期：tab 关闭/切换内容时 revoke，防泄漏
- 并发：端点无共享可变状态，天然并发安全

## 4. 明确不做

文件编辑与冲突处理（产品决策：已从路线图移除，完整记录见 `brainstorm-summary.md`「编辑能力决策记录」）、Office/CAD 预览、目录分页/虚拟滚动、LSP 桥。
