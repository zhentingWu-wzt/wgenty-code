---
change: workspace-file-preview
design-doc: openspec/changes/workspace-file-preview/docs/superpowers/design-doc.md
base-ref: 9d7ec4535260f919b676f28130a8592f7557c6a1
---

# 实施计划：workspace-file-preview

> 权威依据：design-doc.md（技术设计）+ specs/workspace-file-preview/spec.md（行为契约）。
> 对应 tasks.md 任务组 1-7。每步独立可验证；daemon 步骤（1-3）与 web 步骤（4-6）可并行，收尾（7）串行。

## 集成点事实（已核对源码）

- uiStore（`web/src/state/uiStore.ts`）：`openTabs/openTab/closeTab/moveTab/pruneTabs/activeTabId`；subagent 模式 = `subagentTabs: Record<string, SubagentTabMeta>` + `openSubagentTab(meta)`（开+激活+幂等）；`closeTab` 内联清理 subagent meta
- Tab 渲染：`web/src/components/layout/SessionTabBar.tsx:16` `SUBAGENT_PREFIX` 分支（标签、关闭后激活逻辑）
- 主区分发：`web/src/App.tsx:66-69` 按 `activeTabId` 前缀取 meta，`:299` 渲染 SubagentDetailPanel——PreviewPanel 同样接入
- daemon 路由：`src/daemon/routes.rs` 现有 `GET /api/v1/fs/dirs`（`fs.rs::list_dirs`）挂 protected router，bearer 认证统一施加
- 项目注册表：`DaemonState.projects`（`main_root()` + `registered_roots()`），`src/daemon/state.rs:1678` 附近有遍历模式；worktree 列表经 worktrees 模块
- shiki：`web/src/features/chat/CodeBlock.tsx` highlighter 单例 + `codeToHtml`
- react-markdown + remark-gfm 已在依赖（chat 消息渲染在用）

---

## Step 1 — daemon 路径边界与 entries 端点（tasks 1.1, 1.2, 1.3）

**做什么**：新建 `src/daemon/workspace_files.rs`：

1. `resolve_workspace_roots(state: &DaemonState) -> Vec<PathBuf>`：`projects.main_root()` + `registered_roots()` + 各项目 worktree 根；逐个 canonicalize，失败跳过；去重
2. `ensure_within_roots(raw: &str, state) -> Result<PathBuf, WorkspaceFsError>`：canonicalize 失败/不存在 → 404；不在任一根内 → 403（统一文案 `outside registered workspaces`，不泄露存在性）
3. `GET /api/v1/fs/entries` handler：read_dir 单遍收集 `FsEntry{name, is_dir, size}`——`file_type()` 判定，symlink 与 socket/fifo 跳过；忽略 `.` 开头；目录在前、同级 `to_lowercase()` 排序；2000 条上限置 `truncated: true` 停止；子项 metadata 失败按 size 0。响应 `FsEntries{current, entries, truncated}`
4. `WorkspaceFsError` 枚举 → `(StatusCode, Json)` 统一映射（404/403/400/413）
5. `routes.rs` 挂 `GET /fs/entries`（与 `/fs/dirs` 并立）；`docs/API.md` 补文档

**改哪些文件**：`src/daemon/workspace_files.rs`（新）、`src/daemon/routes.rs`、`src/daemon/mod.rs`（模块声明）、`docs/API.md`

**验收**：
```bash
cargo test -p wgenty-code workspace_files            # 单元测试：排序/隐藏忽略/截断/错误映射
cargo clippy --all-targets -- -D warnings
```

## Step 2 — daemon file 端点（tasks 2.1, 2.2）

**做什么**：同模块新增 `GET /api/v1/fs/file`：

1. `ensure_within_roots` 校验后 `metadata().len()` **先判后读**：文本限 1.5MB、白名单二进制限 5MB，超限 413 JSON `{size, limit}` 不读文件体（上限值 `const`）
2. 读前 8192 字节探测 `\0` → 二进制；扩展名白名单 `png/jpg/jpeg/gif/webp/svg/pdf` → 原始字节流响应（`Content-Type` 按扩展名映射，白名单内不依赖探测结果）
3. 其余尝试 `String::from_utf8`：成功 → JSON `FileTextResponse{lines: Vec<String>, version: FileVersion{mtime_ms, size}}`（version 取自 metadata，mtime 毫秒）；失败 → JSON 变体 `{is_binary: true, version}`（无 lines）
4. 空文件 → 空 `lines` + version；目录而非文件 → 400

**改哪些文件**：`src/daemon/workspace_files.rs`、`src/daemon/routes.rs`、`docs/API.md`

**验收**：
```bash
cargo test -p wgenty-code workspace_files   # 探测分流/UTF-8 降级/413/空文件/mime 映射
```

## Step 3 — daemon 集成测试（tasks 3.1, 3.2）

**做什么**：`tests/` 下新增集成测试，tempfile 建伪项目根 + 伪 worktree，注入 DaemonState（沿用现有集成测试的 state 构造模式，先读 `tests/integration/` 任一 fs 相关测试取齐）：

- 边界三类：注册根内 200 / worktree 根内 200 / `/etc/passwd` 403 + 边界内 symlink 指向边界外 403
- entries：排序（目录前/大小写不敏感）、隐藏忽略、symlink 跳过、2000 截断标记
- file：文本行结构与 version 字段、png mime 字节流、413 `{size,limit}`、`is_binary` 降级变体

**改哪些文件**：`tests/integration/workspace_files_test.rs`（新）

**验收**：
```bash
cargo test --test workspace_files_test
```

## Step 4 — web API 层与 tab 状态（tasks 4.1, 4.2）

**做什么**：

1. `api/types.ts`：`FsEntry/FsEntries/FileVersion/FileContent`（text | binary-unsupported | blob 三态联合类型，对齐 Step 2 响应形态）
2. `api/client.ts`：`listEntries(path)`；`fetchFile(path)`——按 `Content-Type` 分流（`application/json` → 解析 text/binary-unsupported；其余 → blob + mime）
3. uiStore 照 subagent 模式扩展：`PreviewTabMeta{workspaceRoot, relPath, kind}`、`previewTabs: Record<string, PreviewTabMeta>`、`openPreviewTab(meta)`（id = `preview:<absPath>`，开+激活+幂等）；`closeTab` 增加 `preview:` 前缀 meta 清理（镜像现有 subagent 分支）

**改哪些文件**：`web/src/api/types.ts`、`web/src/api/client.ts`、`web/src/state/uiStore.ts`

**验收**：
```bash
cd web && npx tsc --noEmit && npx vitest run src/state/uiStore.test.ts 2>/dev/null || true
```

## Step 5 — FileTree（tasks 5.1）

**做什么**：`web/src/features/files/FileTree.tsx`：

- 挂在 `ProjectTree` task（workspace）节点下作为「文件」分组节点（默认折叠；首展才调 `listEntries(workspaceRoot)`）
- 子目录逐级懒加载（本地 state 记展开态与缓存）；目录可折叠、文件点击 → `openPreviewTab`
- `target/ node_modules/ dist/` 置灰样式 + 默认折叠；`truncated` 时列表尾显示「已截断」；错误态 toast（复用 sonner）

**改哪些文件**：`web/src/features/files/FileTree.tsx`（新）、`web/src/features/sessions/ProjectTree.tsx`（挂载点）

**验收**：
```bash
cd web && npx tsc --noEmit
```

## Step 6 — PreviewPanel + tab 集成（tasks 6.1-6.4）

**做什么**：

1. `web/src/features/files/PreviewPanel.tsx`：接收 `PreviewTabMeta`，`fetchFile` 后按 `FileContent` 三态分流：
   - text + 代码扩展名 → 行号 + shiki 高亮（复用 CodeBlock highlighter 单例；行号经 shiki transformers 或独立行号列实现）；>256KB 跳过高亮纯文本
   - text + `.md` → `react-markdown + remark-gfm`，`components.code` 委托 CodeBlock；工具栏「源码」切换
   - blob + `image/*` → `URL.createObjectURL` + `<img>`（SVG 同路径）；卸载/切换时 `revokeObjectURL`
   - blob + `application/pdf` → `<iframe src=blobUrl>`
   - binary-unsupported / 413 / 其他错误 → 图标 + 实际大小/原因提示（不白屏）
2. `App.tsx:66-69` 同模式取 `previewTabs[activeTabId]`，渲染 `<PreviewPanel>`；`SessionTabBar.tsx` 增加 `PREVIEW_PREFIX` 分支（标签 = relPath 文件名，关闭逻辑镜像 subagent）
3. 加载态与错误态

**改哪些文件**：`web/src/features/files/PreviewPanel.tsx`（新）、`web/src/App.tsx`、`web/src/components/layout/SessionTabBar.tsx`

**验收**：
```bash
cd web && npx tsc --noEmit && npx vitest run
```

## Step 7 — 收尾全绿（tasks 7.1）

**做什么**：勾选 tasks.md 全部任务；全量验证。

**验收**：
```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
cd web && npx tsc --noEmit && npx vitest run && npm run build
openspec validate workspace-file-preview --type change --strict
```

---

## 风险与回退

- 集成测试的 state 构造若与现有模式差异大 → 参考 `tests/integration/` 中最接近的 fs/worktree 测试，必要时抽公共 fixture
- shiki 行号方案若 transformers 不满足 → 退化为左侧行号列 + 滚动同步（纯 CSS）
- 回退：纯新增端点/组件，revert 提交即可，无持久化状态
