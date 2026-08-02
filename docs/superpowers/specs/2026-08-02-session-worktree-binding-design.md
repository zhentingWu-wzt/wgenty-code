# Project → Worktree → Session（N:1 绑定）设计

日期：2026-08-02
状态：已确认（方案 A + 模型 2）

## 背景与目标

指挥中心的多会话目前全部跑在 daemon 的单一 working_dir 里，并行会话改代码会互相踩踏。本设计引入 project → worktree → session 模型：**worktree 扮演"任务"的角色（分支 + 目录，不新增实体），会话是工作区里的对话记录（N:1——多个会话可绑定同一个 worktree）**。

核心语义（与 TUI 现状兼容）：session 是廉价的对话记录，保存完整聊天历史；主检出就是"默认工作区"，今天的所有会话等价于绑在主检出上。绑定 worktree 只是给会话换一个隔离的工作区，不改变 session 的本质。

### 已确认的决策

| 决策点 | 结论 |
|:--|:--|
| 概念模型 | worktree = 任务（不新增 Task 实体）；session = 工作区里的对话，N:1 绑定 |
| 生命周期 | 删会话 = 仅删对话记录，不动 worktree；删 worktree 时处理名下会话（确认后自动解绑） |
| 会话归档 | 会话可归档（daemon `Session.metadata` 持久化），归档后不显示在会话列表和 `/sessions` 浏览器默认视图；提供取消归档入口 |
| worktree 创建时机 | 新建会话时三选一：主检出（默认）/ 绑定现有 worktree / 新建 worktree |
| 主检出会话 | 允许。不绑 worktree = 绑主检出，行为与现状完全一致 |
| project 范围 | v1 单仓库：project = daemon 启动所在仓库（`settings.storage.working_dir`），不做多仓库注册表 |
| 实现路线 | 方案 A：daemon 持有 session→worktree 绑定（服务端权威，可持久化） |

### v1 明确不做

- 多仓库 project 注册表（v2）
- checkpoint 按会话/工作区隔离（checkpoint store 全局、跨会话 `keep_n` prune 是已知遗留）
- 仓库外路径的 worktree（guardian 会对每个路径弹 Ask，体验差；绑定端点直接拒绝）
- TUI 侧的绑定 UI（绑定存于 daemon，TUI 可后续接入）
- worktree 的合并/删除自动化（合并回主分支是用户手动操作）

## 关键探索结论（已核实）

- `ToolContext.workdir: Option<&Path>`（`src/agent/identity.rs:140`）已存在，subagent 的 worktree 隔离是现成先例（`src/teams/guarding_tool_port.rs:86`）。
- 已支持 workdir 的工具：file_read / file_edit / file_write / list_files / execute_command / exec_command / run_test / background / checkpoint pre-edit 捕获（`src/tools/mod.rs:29-38` 的 `resolve_path`）。
- **忽略 workdir 的工具**：grep（`src/tools/search/grep.rs:33`）、glob（`glob_search.rs:64`）、apply_patch（只吃模型入参里的 workdir）。
- `execute_tool` 在两处硬编码 `workdir: None`（`src/daemon/handlers.rs:516, 576`）。
- guardian 路径策略以 canonicalized working_dir 为 workspace_root（`src/permissions/policy.rs:32-35`）——**仓库内 `.worktrees/<name>` 零改动通过**；仓库外每路径弹 Ask。
- sandbox 的 writable workspace 由 workdir 派生（`src/sandbox/policy.rs:94-124`）——绑定后沙箱命令只能写该工作区内（符合预期；绑同一 worktree 的多个会话共享该区）。
- `Session.metadata: HashMap<String, Value>`（`src/context/memory_session.rs:25`）已存在于磁盘格式，但 HTTP 层不透传。
- checkpoint store 全局、按 turn_id 键控，仓库内 worktree 路径可正确捕获/回滚，v1 不动。

## Daemon 设计

### 1. 绑定状态

`DaemonState` 新增：

```rust
pub session_workdirs: Arc<RwLock<HashMap<String, PathBuf>>>
```

- `bind_session_worktree(session_id, path)`：校验 path 必须位于 canonicalized `working_dir` 之内（拒绝仓库外路径），写入 map。**多个 session_id 可映射到同一路径（N:1 天然成立，无需额外机制）**。
- `unbind_session_worktree(session_id)`：移除映射（不删磁盘 worktree）。
- `session_workdir(session_id) -> Option<PathBuf>`：供 execute_tool 查询；`None` = 主检出。
- `worktree_sessions(path) -> Vec<String>`：反查某 worktree 名下的会话（删 worktree 时解绑用）。

### 2. 身份模型（关键）

绑定会话使用**单一身份**：创建绑定会话时，web 先 `POST /api/v1/sessions` 创建 daemon 会话拿到 id，并把该 id 同时作为运行时 session_id（`/tools/execute` 的参数）和持久化 id。这样绑定只需按一个 id 键控。未绑定会话维持现状（本地 `web-*` id + autosave 时才注册 daemon session，语义上等价于绑主检出）。

### 3. 绑定端点

```
PUT /api/v1/sessions/:id/worktree
  body: { "path": ".worktrees/feat-x", "branch": "feat-x" }    # path 相对 working_dir 或绝对路径
  200 → { "session_id": "...", "worktree": { "path": "...", "branch": "..." } }
  400 → 路径不在 working_dir 内 / worktree 不存在
  404 → session 不存在
DELETE /api/v1/sessions/:id/worktree   # 解除绑定（会话回到主检出，不删磁盘 worktree）
```

- 路径校验：两侧都 canonicalize 后做前缀检查，拒绝逃逸（`../`、仓库外绝对路径）。
- 绑定写入 `session_workdirs` map，**同时**持久化到 `Session.metadata["worktree"] = { path, branch }`（daemon 重启后从 session 文件恢复 map——`MemorySessionManager` 加载时回填）。
- `SessionResponse` / `SessionInfoResponse` 增加 `worktree: Option<{ path, branch }>` 字段（web 分组/标注用）。

### 4. 归档端点

```
PUT /api/v1/sessions/:id/archive
  body: { "archived": true }    # 或 false 取消归档
  200 → { "session_id": "...", "archived": true }
  404 → session 不存在
```

- 归档使用存储层已有的 `SessionStatus::Archived`（`SessionManager.archive()/unarchive()`），不引入 metadata 标志——单一事实来源。
- `SessionInfoResponse`（`GET /sessions` 列表项）的 `status` 字段透出归档状态，由客户端过滤——**daemon 不做列表过滤**（`/sessions` 始终返回全部，浏览器默认视图隐藏归档项）。
- 归档不影响绑定关系和消息内容，纯粹是列表可见性标志。

### 5. execute_tool 注入

`src/daemon/handlers.rs` 两处 ToolContext 构建点（Allow 路径 :516、auto-approve 路径 :576）：

```rust
let session_wd = state.session_workdir(&session_id);
// ...
workdir: session_wd.as_deref(),
```

`session_id` 查不到映射（包括缺省值 `"default"`）→ `workdir: None` → 行为与现状一致（向后兼容 TUI/旧客户端）。

### 6. grep / glob 补 workdir

`grep.rs:33`、`glob_search.rs:64` 的相对 `path` 改走 `resolve_path(path, context.workdir)`（与 list_files 同模式，一处一行）。apply_patch 本期不改（模型入参可带 workdir）。

## Web 设计

### 1. 新建会话弹窗（NewSessionModal）

点左栏 "+ New session" 弹出：

- 会话名输入框（可空，沿用默认命名）
- 工作区三选一（radio）：
  - **主检出**（默认）— 现状行为（本地会话，首轮后 autosave）
  - **现有 worktree** — 下拉列出 `GET /api/v1/worktrees` 的非主 worktree → 依次：`POST /api/v1/sessions` 建 daemon 会话（其 id 作为统一 id）→ `PUT /sessions/:id/worktree` 绑定
  - **新建 worktree** — 输入分支名 → 依次：`POST /api/v1/sessions` → `POST /api/v1/worktrees`（path 推导为 `.worktrees/<branch 规范化>`）→ `PUT /sessions/:id/worktree` 绑定
- 任一步失败 → 弹窗内联报错，不建本地会话（先建 worktree 成功但绑定失败时，worktree 留在磁盘，UI 提示可手动删）

### 2. 会话列表按工作区分组

Sessions 区内按工作区分组渲染：

- **主检出**组：所有未绑定会话（现状的全部会话）
- 每个有绑定会话的 worktree 一个组，组标题为分支名（mono 字体）
- 组标题小字显示会话数；组内会话卡片与现状一致（状态点/名称/摘要/审批角标）

### 3. 删除与归档流程（补全目前缺失的删除会话入口）

- **删会话**（卡片 hover 删除按钮）：确认后 `removeSession` + daemon `DELETE /sessions/:daemonId`。不问 worktree 的事——它只是删一条对话记录。
- **归档会话**（卡片 hover 归档按钮 / `/sessions` 浏览器行内归档按钮）：daemon `PUT /sessions/:id/archive { archived: true }`；若归档的是当前打开的本地会话，同时关闭本地 entry。归档会话从 Sessions 列表和 `/sessions` 浏览器默认视图消失。
- **取消归档**：`/sessions` 浏览器底部有 "Archived (N)" 折叠区（默认折叠，复用 RailSection 折叠交互），列出归档会话，行内提供 Unarchive 按钮（`archived: false` → `SessionStatus::Active`）→ 会话回到默认视图。
- **删 worktree**（Worktrees 面板现有 Remove）：先反查名下会话——
  - 无绑定会话：现状确认后直接删
  - 有绑定会话：确认框列出 N 个会话，文案"这些会话将解绑并回到主检出"→ 确认后逐一对存活会话调 `DELETE /sessions/:id/worktree` 解绑（本地 entry 同步清 worktree 标记），再删 worktree

## 错误处理

- worktree 创建/绑定失败：NewSessionModal 内联错误，不创建半边状态
- daemon 重启：从 `Session.metadata` 恢复绑定 map；session 文件丢失则绑定丢失（会话降级为主检出行为，不报错）
- 绑定的 worktree 被外部删除（手动 `git worktree remove`）：`resolve_path` 落到不存在目录，工具调用自然报错并在会话内显示——不在 v1 做主动检测
- 同一 worktree 的多个会话并行改同一文件：可能冲突——这是用户选择共享工作区的预期行为，不做锁

## 测试

**daemon（cargo test）**

- 绑定端点：路径在 working_dir 内 → 200；路径逃逸 → 400；worktree 不存在 → 400；session 不存在 → 404
- N:1：两个 session 绑定同一路径，`session_workdir` 各自返回该路径；`worktree_sessions` 反查返回两者
- `session_workdir` 注入：`execute_tool` 在绑定会话上以 workdir 执行（临时 git 仓库 + 真实 file_write 验证文件落在 worktree）
- metadata 往返：绑定 → `GET /sessions/:id` 返回 worktree 字段；解绑 → 字段消失且 map 清除
- 归档端点：`PUT /sessions/:id/archive` 往返（归档后 `GET /sessions` 列表项带 `archived: true`，取消后恢复）；session 不存在 → 404
- grep/glob 相对路径在绑定 workdir 下解析

**web（vitest）**

- NewSessionModal：主检出直接建会话；选现有 worktree → createSession + bind；选新建 → createSession + createWorktree + bind；任一步失败显示内联错误且不建本地会话
- sessionManager：`SessionEntry.worktree` 存取；`createLocalSession` 支持显式 id
- 会话列表按工作区分组渲染（主检出组 + worktree 组）
- 归档：归档操作调用 archive 端点并（对打开的会话）关闭本地 entry；`/sessions` 浏览器默认隐藏归档项，Archived 折叠区可展开并取消归档
- 删 worktree 流程：有绑定会话时确认后先解绑再删除

**验收**：`cargo test --all` + `cargo clippy --all-targets -- -D warnings` + web `lint/test/typecheck/build` 全绿；手动冒烟：建一个绑定 worktree 的会话让 agent 改文件，确认改动落在 `.worktrees/<name>` 而主检出干净；再建第二个会话绑同一 worktree，确认两个会话都在该目录工作。

## 新增依赖

无。
