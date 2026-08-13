# wgenty-code Web 端 Orca 风格重设计

日期：2026-08-05
状态：已确认（路线 A：新壳渐进迁移）

## 背景与目标

wgenty-code 的 web 前端（`web/`，React 18 + Vite 5 + zustand，纯 HTTP REST + SSE 连接 Rust daemon）目前是"顶栏 + 左栏 + 单会话聊天"的简单布局。目标是让其**界面与交互**对齐 Orca 桌面端（`/Users/wuzhenting/workspace/project/orca`，Electron IDE）的三段式工作台风格。

**范围约束**（已与用户确认）：

- 只做界面与交互对齐；功能复用现有 daemon API（聊天/会话/worktree/权限/memory/skills/tasks/checkpoints），**不新增后端能力**（无终端 PTY、无文件浏览器、无编辑器、无 source control）
- Orca 仅作视觉/交互参考，**不复制其代码与资源**（GPL 商业产品）
- 通信层（`web/src/api/`、`web/src/agent/sessionRunner.ts`）与会话状态层（`state/sessionManager.ts`、`state/sessionStore.ts`）零改动

## 技术选型

- 引入 **Tailwind CSS v4**（`@tailwindcss/vite`，CSS-first 配置）+ **shadcn 风格组件**（vendored 在 `components/ui/`，底层 `radix-ui`）
- 新增依赖：`tailwindcss@4`、`@tailwindcss/vite`、`radix-ui`、`clsx`、`tailwind-merge`、`class-variance-authority`
- 复用已有：`lucide-react`、`sonner`、`zustand`、@fontsource 字体（不动）
- React 18 不升级

## 架构

```
web/src/
├── styles/globals.css        # Tailwind v4 入口 + shadcn token（:root 亮色 / .dark 暗色）
├── lib/utils.ts              # cn() = clsx + tailwind-merge
├── components/
│   ├── ui/                   # shadcn vendored 基础件（button/dialog/dropdown-menu/tooltip/
│   │                         #   scroll-area/context-menu 等，按需逐个添加）
│   └── layout/               # 新壳层：AppTopbar / LeftSidebar / SessionTabBar / RightRail
├── features/
│   ├── chat/                 # 现有 ChatView/Composer/ToolCallCard/DiffView/CodeBlock/SessionHeader 迁入
│   ├── sessions/             # ProjectTree 演进为会话侧边栏 + NewSessionModal
│   ├── panels/               # Skills/Sessions/Memory/Checkpoints/Tasks 右栏面板
│   └── permissions/          # PermissionModal / QuestionModal（保持模态框）
├── state/
│   ├── uiStore.ts            # 新增：tab/布局/主题状态（见下）
│   ├── sessionManager.ts     # 不动
│   └── sessionStore.ts       # 不动
├── api/                      # 不动
└── agent/sessionRunner.ts    # 不动
```

## 布局

```
┌──────────────────────────────────────────────┐
│ AppTopbar: 品牌/项目名 · 连接状态 · 主题/右栏开关 │  ← 应用栏，不仿 macOS 窗口按钮
├──────────┬───────────────────────┬───────────┤
│ Left     │ SessionTabBar         │ RightRail │
│ Sidebar  ├───────────────────────┤ activity  │
│ 会话树    │ 激活会话的 ChatView     │ bar +     │
│          │ + Composer            │ 面板       │
├──────────┴───────────────────────┴───────────┤
│ StatusBar（daemon 状态，保留现有功能，换样式）    │
└──────────────────────────────────────────────┘
```

- **LeftSidebar**：现有 ProjectTree（project → worktree → session 三级树）保留全部交互（新建/归档/删除会话与 worktree），换 Orca 侧边栏视觉；可折叠、宽度可拖拽；移动端保持抽屉式
- **SessionTabBar**：点击侧边栏会话 = 打开/激活 tab；可关闭、拖拽排序；tab 显示会话状态点（运行中/待权限）
- **RightRail**：36px activity bar（图标竖排）+ 面板区。面板：Sessions（搜索/归档浏览）、Skills、Memory、Checkpoints、Tasks。点图标切换、再点收起。取代原 `/sessions` `/memory` `/undo` 等模态框；对应斜杠命令保留，行为改为打开面板
- **Model 面板**保持模态框形态（快速切换操作，不适合常驻面板）
- **PermissionModal / QuestionModal** 保持模态框不变

## Tab 状态模型（uiStore）

```ts
{
  openTabs: sessionId[]        // 顺序即 tab 顺序
  activeTabId: sessionId | null
  rightPanel: 'sessions'|'skills'|'memory'|'checkpoints'|'tasks'|null
  leftCollapsed: boolean
  theme: 'light'|'dark'|'system'
}
```

- uiStore 只存 sessionId 引用，会话数据仍在 sessionManager；会话被归档/删除时同步移除对应 tab
- **不做分栏（split view）**，tab 模型预留扩展空间（YAGNI）

## 主题体系

- 全部收敛在 `styles/globals.css`（Tailwind v4 CSS-first）
- shadcn neutral 色板 token：`--background/--foreground/--card/--popover/--primary/--secondary/--muted/--accent/--destructive/--border/--input/--ring/--radius` + `--sidebar-*` 一组；`:root` 亮色 + `.dark` 暗色两套值
- 观感参照 Orca：灰阶 neutral、紧凑密度、13px 基础字号；数值自定义，不抄代码
- 主题切换：light / dark / system（`matchMedia`），持久化 `localStorage`，`documentElement` 挂 `.dark` class

## 迁移阶段

每阶段结束保持应用可用、`pnpm build` 与测试通过：

- **P0 基础设施**：装依赖、globals.css token、cn()、components/ui 基础件（按需）
- **P1 新壳层**：uiStore、AppTopbar、LeftSidebar（ProjectTree 换壳）、StatusBar 换装、主题切换生效
- **P2 tab 系统**：SessionTabBar + 会话即 tab（打开/关闭/排序/状态点）
- **P3 右栏**：RightRail + 5 个面板迁入，斜杠命令改为打开对应面板
- **P4 聊天区收尾**：ChatView/Composer/ToolCallCard/DiffView/各模态框 Tailwind 化，删除旧 CSS

## 测试

- 现有 vitest 测试随迁移同步更新，保持全绿（`pnpm --dir web test`）
- 新增：uiStore 单测（tab 增删/排序/与会话删除联动）、右栏面板切换、主题 class 切换
- 手动走查：`pnpm dev` 与 Orca 桌面端并排对比布局观感

## 非目标

- 终端（PTY/xterm）、文件浏览器、Monaco 编辑器、source control、设置中心、多 agent 编排 UI
- tab 分栏（split view）
- WebSocket 迁移（保持现有 REST + SSE）
- 移动端深度适配（仅保持 LeftSidebar 抽屉可用）
