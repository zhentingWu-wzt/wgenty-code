# Design Doc: GUI Session Management

> 技术设计文档 for `gui-session-management` change。

## 概述

会话管理界面：列表、搜索、切换、历史恢复、checkpoint undo。基于 gui-desktop-foundation 的 Tauri 壳，**绝大部分复用 web/ 已有组件**。

## 设计决策

### 1. 复用 web/ 组件（零重写）

web/ 在 web-ops-console 阶段已实现完整的会话管理 UI：
- `SessionsPanel` — 列表、归档、删除
- `NewSessionModal` — 新建会话（main checkout / worktree）
- `CheckpointsPanel` — checkpoint 列表 + undo-turn
- `sessionLoad.ts` — 无损历史恢复

Tauri webview 直接渲染这些组件，无需改动。

### 2. 搜索功能（唯一新增）

**Daemon API**（已存在）：`GET /api/v1/sessions/search?q=<query>` — 匹配会话名和消息内容，跨所有注册项目根。

**Frontend 新增**：
- `DaemonClient.searchSessions(query)` — 封装 search API
- `SessionsPanel` 搜索输入框 — 300ms debounce，搜索时显示扁平结果（不分 active/archived），清空回退全量列表

### 3. 无需 daemon 改动

所有 API（list、search、load、delete、checkpoints、undo-turn）已在 daemon 实现。

## 影响范围

- 新增：`DaemonClient.searchSessions`（1 个方法）
- 修改：`SessionsPanel`（加搜索输入框 + debounce）
- 不改：daemon 服务端、Tauri 壳、其他 web/ 组件
