# Web UI 多会话指挥中心设计

日期：2026-08-02
状态：已确认（方案 A）

## 背景与目标

现有 web 前端（`web/`）是 daemon 的单会话瘦客户端：一个全局 `chatStore`、一个内存会话，所有功能（Sessions/Todos/Tasks/Model/Memory/Config）挤在左侧 tab 面板里。视觉上纯灰度无品牌色、零图标。

目标：重设计为 **Agent 指挥中心**（对标 Codex 桌面版 / Cursor 3.x）——多会话并行管理 + 三栏布局 + 视觉系统升级。

### 已确认的决策

| 决策点 | 结论 |
|:--|:--|
| 定位 | Agent 指挥中心（管进度、审批、多会话，不做代码编辑） |
| 并行模型 | 多会话并行，纯前端实现（不动 daemon 核心） |
| 布局 | 三栏式：左栏 = 会话 + Worktrees + Skills + 设置入口 |
| 数据源 | daemon 补少量轻量端点（worktrees、skills） |
| 前端架构 | 方案 A：每会话独立运行时（store 工厂 + 独立 loop） |

### 已接受的范围限制

- agent loop 跑在浏览器：关闭页面会中断运行中的会话（设计用 `beforeunload` 提示缓解）。
- daemon 的审批规则 / todos / tasks / 模型切换是进程级全局状态，并行会话会互相影响。UI 如实标注，不在本期修复。
- 所有会话共享 daemon 的单一 working_dir，无 per-session worktree 隔离（Worktree 面板仅为管理入口，不与会话绑定）。

## 方案对比记录

- **方案 A（采纳）**：每会话独立运行时。`chatStore` 从全局单例改为工厂函数，每会话一个 store 实例 + 独立 agent loop + 独立 AbortController。切换会话只切换渲染目标，后台会话继续跑。
- 方案 B（否决）：单 store + 快照切换。loop 回调绑死全局 store，切走的会话无法继续更新，实质仍是单会话。
- 方案 C（否决）：多浏览器 tab。无统一指挥中心体验，审批散落各 tab。

## 架构设计

### 1. 多会话状态层（核心重构）

```
web/src/state/
  sessionManager.ts   ← 新增：会话注册表（全局单例）
  sessionStore.ts     ← 现 chatStore.ts 改造为工厂函数
  sidebarStore.ts     ← 保留（全局 UI 偏好）
```

- `createSessionStore()`：现 `chatStore.ts` 全部状态（messages / isRunning / lastError / pendingPermission / AbortController 注册）原样搬进工厂，每个会话一个实例。组件从 `useChatStore(selector)` 改为按会话实例订阅。
- `sessionManager` 持有：
  - `sessions: Map<sessionId, { store, meta }>`
  - `meta: { id, name, status: "running" | "awaiting_approval" | "idle" | "error", lastPreview, updatedAt }`
  - `activeSessionId`
  - status 由 loop 事件推导：onStreamEvent → running；onPermissionRequired → awaiting_approval；loop 结束 → idle / error。左栏直接消费，无需 daemon 改会话模型。
- `agent/loop.ts` 已是回调注入、不碰 React 的设计，基本不改：每会话调一次 `runAgentLoop`，回调闭包绑各自 store 实例。
- 会话与 daemon session 关联：`POST /api/v1/sessions` 创建拿真实 id；sessionId 传给 `/tools/execute`；每轮结束后自动 `PUT /sessions/:id` 保存快照。加载历史会话 = `GET /sessions/:id` 灌入新 store 实例。

### 2. daemon 新增轻量端点

包装现有能力，不动 agent 核心：

| 端点 | 说明 |
|:--|:--|
| `GET /api/v1/worktrees` | 列出 git worktree（path / branch / HEAD / 是否主仓），包装 `git worktree list` |
| `POST /api/v1/worktrees` | 创建 worktree（branch + path），包装 `git worktree add` |
| `DELETE /api/v1/worktrees/:path` | 删除，包装 `git worktree remove` |
| `GET /api/v1/skills` | 列出 skill（name / 描述 / 来源 / 启用状态），复用 CLI `skills` 子命令背后的 manager |
| `POST /api/v1/skills/:name/toggle` | 启停 skill |

### 3. 权限流（纯前端适配）

- 每会话的 `onPermissionRequired` 写自己的 store 实例。
- `PermissionModal` 读取 activeSession 的 pendingPermission；StatusBar 角标汇总所有会话待审批数。
- 审批文案注明"此批准对所有会话生效"（因 daemon 审批规则全局共享）。

## 布局与组件

```
App
├── StatusBar              ← 保留，加全局待审批角标
└── app-body
    ├── LeftRail           ← 新增（替代现 Sidebar）
    │   ├── SessionList        会话卡片：状态点 / 名称 / 最后消息摘要 / 待审批角标 / 新建按钮
    │   ├── WorktreePanel      列表 + 创建 / 删除（新端点）
    │   ├── SkillPanel         列表 + 启停开关（新端点）
    │   └── RailFooter         模型切换、权限模式、Config（复用现 ModelPanel / ConfigPanel）
    ├── CenterPane         ← 当前会话
    │   ├── SessionHeader      会话名、状态、undo-turn 入口（现成端点）
    │   ├── ChatView           现有，改为按 sessionId 注入 store
    │   └── Composer           现有，绑当前会话的 loop
    └── ContextPanel       ← 新增：可开合右栏
        ├── TodosPanel         现有（UI 标注"全局共享"）
        ├── TasksPanel         现有（UI 标注"全局共享"）
        ├── MemoryPanel        现有（全局记忆浏览，与会话无关）
        ├── SubagentPanel      新增：子 agent 进度（/agents/children + trace SSE，端点现成）
        └── CheckpointsPanel   新增：checkpoint 列表 + undo（端点现成）
```

复用：`ChatView` / `ToolCallCard` / `DiffView` / `PermissionModal` / `MemoryPanel` / `TasksPanel` / `ModelPanel` / `ConfigPanel` 基本原样，仅数据来源改为按会话注入。
删除：现 `Sidebar` 的 tab 结构（被 LeftRail 分区替代）。

## 视觉系统

- 单一 accent 色（蓝紫 `#6e8efb`），仅用于 CTA、选中态、链接、流式光标；灰度骨架保留。
- `lucide-react` 图标替换全部文本符号按钮；LeftRail 分区配图标。
- `@fontsource` 自托管 Inter + JetBrains Mono。
- `sonner` toast：连接断开、审批结果、worktree/skill 操作反馈、会话完成。
- 消息流细节：assistant 流式光标、工具卡片左侧状态色条、用户消息浅底区分角色。

## 错误处理

- 单会话错误隔离：某会话 loop 抛错只将该会话 meta 置 `error` + 会话内错误条（现有 D7.3 transport/upstream 分类与重试逻辑搬进各 store 实例），不影响其他会话。
- daemon 断连：全局 toast + StatusBar 状态点（现有 heartbeat 保留），运行中会话全部置 `error`。
- worktree / skill 端点失败：面板内联错误 + 重试按钮，不弹 toast。
- `beforeunload`：有会话仍在运行时提示"关闭将中断 N 个运行中的会话"。

## 测试

- `sessionManager`：多会话隔离（两会话各自流式更新互不串扰）。
- `sessionStore` 工厂：实例间状态独立。
- 组件测试：`SessionList` 状态渲染、`Composer`（已有）、`PermissionModal` 多会话路由。
- 验收：`npm run lint && npm test && npm run typecheck && npm run build` 全绿。

## 新增依赖

- `lucide-react`、`sonner`（运行时）
- `@fontsource-variable/inter`、`@fontsource/jetbrains-mono`（自托管字体）
