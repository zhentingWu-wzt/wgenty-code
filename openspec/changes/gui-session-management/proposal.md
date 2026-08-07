# Proposal: gui-session-management

> Comet 批量拆分项 2/4（batch: `.comet/batches/gui-desktop.json`）。依赖 gui-desktop-foundation 提供的应用骨架与面板挂载点。

## Why

TUI 已具备会话持久化与恢复能力（daemon 提供 sessions CRUD/search、checkpoints、undo-turn 等端点），但终端形态下会话浏览与历史管理体验受限。GUI 桌面端需要以图形化方式提供同等能力，让用户直观管理大量历史会话。

## What Changes

- 新增 GUI 会话列表面板：展示全部会话（标题、时间、模型等元信息），支持新建、切换、删除
- 新增会话搜索：按关键词检索历史会话
- 新增会话恢复：切换会话后加载完整对话历史并继续对话
- 新增 checkpoint 与 undo-turn 界面：查看每轮 checkpoint，对指定 turn 执行撤销回滚

## Capabilities

### New Capabilities

- `gui-session-management`: GUI 会话管理——会话列表/新建/切换/删除/搜索、历史加载、checkpoint 查看与 undo-turn 回滚

### Modified Capabilities

（无——复用 daemon 现有 sessions/checkpoints/undo-turn API）

## Impact

- **新增代码**：`src/gui/` 下的会话管理面板与相关状态逻辑（挂在 foundation 的面板挂载点上）
- **依赖**：gui-desktop-foundation（骨架、会话编排客户端、对话界面）与 daemon-session-orchestration（版本化会话存储）；daemon 服务端按需使用版本化 API，本 change 不改服务端
- **不触碰**：core、daemon 服务端、TUI
