# Design: workspace-file-preview

## Context

web 端是 React + TS 薄客户端，所有能力来自 daemon 的 axum HTTP API。现状：

- `src/daemon/fs.rs` 仅有 `GET /api/v1/fs/dirs`——只列目录、刻意不返回文件（目录选择器专用），且默认从 home 开始浏览（无边界约束，这是该端点的既定契约，不在本次范围）
- 已注册项目与 worktree 的根路径可从 daemon 的 project registry / worktree 信息获得（`src/mcp/resources.rs` 的 `workspace_root` 已示范 canonicalize 校验模式）
- 前端 `features/sessions/ProjectTree.tsx` 是「项目→任务→会话」树；主区域已有 tab 化面板（diff 查看器等），shiki 已集成
- 依赖现状：daemon 有 `notify`/`tokio`/`axum`；web 有 `shiki`、`zustand`、`diff`，无编辑器/预览重型依赖

## Goals / Non-Goals

Goals（设计层）：

- 两个只读端点复用同一套路径边界校验，为后续编辑能力（PUT + 乐观并发）预留 `version` 语义
- 文件树与预览面板全部走现有 daemon client 封装与 zustand 模式，不引入新状态管理

Non-Goals（设计层）：

- 不做编辑、不做 watcher 推送——编辑已从路线图移除（产品决策，完整记录见 `docs/superpowers/brainstorm-summary.md`「编辑能力决策记录」；重开条件：远程/headless 成为一等部署模式）
- 不做分页/虚拟滚动大目录优化（截断标记已满足契约；虚拟列表留待真实需求）
- 不引入 pdf.js/monaco 等重型前端依赖

## Decisions

### D1: 新端点而非扩展 `/fs/dirs`

`/fs/dirs` 的契约是"全盘、仅目录、给选择器"，扩展它会破坏既有调用方。新增 `entries`（混合列表）与 `file`（内容），与它并立。

- 备选：给 `/fs/dirs` 加 `include_files` 参数——否决，语义耦合且无法施加路径边界

### D2: 路径校验——canonicalize 后前缀匹配已注册根集合

`resolve_workspace_roots()` 收集所有已注册项目根 + 活跃 worktree 根（canonicalize），请求路径 canonicalize 后检查 `starts_with` 任一根。符号链接经 canonicalize 自然解析，指向边界外即拒绝。

- 上限值（1.5MB/5MB/2000 条）做成 `const`，不进配置文件——YAGNI，等有真实诉求再外置

### D3: 文本/二进制判定与响应形态

读取前 8KB 探测 null-byte：含 `\0` 判二进制；再以扩展名白名单（png/jpg/jpeg/gif/webp/svg/pdf）走"二进制原始字节"路径，其余按 UTF-8 文本处理（`String::from_utf8` 失败则降级为二进制 `application/octet-stream` 响应并标记 `is_binary`）。文本响应为 JSON（行数组 + version），二进制响应为原始字节流（前端 blob 包装）。

- 备选：全走 base64 JSON——否决，二进制体积膨胀 33% 且白白解码

### D4: version = `{mtime_ms, size}`

mtime+size 组合作为弱版本标识，足以支撑后续编辑的 409 冲突判定（内容 hash 成本高且大文件不友好）。两个端点都返回它。

### D5: 前端结构（经 brainstorming 修订）

- `features/files/FileTree.tsx`：复用 `ProjectTree` 的 TreeNode 交互模式，挂在 task 节点下按需挂载
- 预览接入**现有 uiStore tab 体系**：tab id 规范 `preview:<absPath>`（照 `subagent:<nodeId>` 前缀模式），复用 `openTab/closeTab/moveTab/pruneTabs`，按 path 幂等去重——不新建 previewStore
- Markdown 渲染**复用既有 `react-markdown + remark-gfm`**（会话消息已在用），代码块委托现有 shiki `CodeBlock`——零新依赖
- `dangerouslySetInnerHTML` 仅用于自产 HTML（shiki 输出）
- SVG 一律 `<img src=blob>` 呈现，不 innerHTML 注入——消除脚本执行面

### D6: 超限与截断的响应语义

文本/二进制超限 → `413` JSON（`{size, limit}`）；目录条目超 2000 → 200 但带 `truncated: true` 标记，前端在树底显示"已截断"。

## Risks / Trade-offs

- [路径校验遗漏某个根来源（如新建 worktree 未入集合）导致误 403] → 校验函数以 project registry 为单一事实来源，集成测试覆盖"注册项目/其 worktree/未注册路径"三类
- [mtime 粒度问题导致 version 误判（编辑能力落地时）] → 后续 change 若引入 watcher，可用 inotify 事件计数替代；本 change 只透传事实
- [目录里含大量条目（如 target/）拖慢列表] → 条目上限截断 + 隐藏文件忽略已缓解；`target/`、`node_modules/` 等 IDE 噪音目录在**前端**置灰/默认折叠而非 daemon 过滤（保留 API 中立性）
- [SVG 经 `<img>` 呈现在外层样式下表现受限] → 接受；安全优先
- [大文本（接近 1.5MB）高亮卡顿] → 超过 256KB 的文本降级为无高亮纯文本渲染

## Migration Plan

纯新增端点与前端面板，无数据/配置迁移。部署即生效；回滚 = 还原代码，无持久化状态。

## Open Questions

（无——UI 细节如文件树入口图标、tab 关闭交互在 build 阶段按现有组件风格取齐即可。）
