# Design: gui-session-management

## Context

daemon 已提供完整会话 API：sessions CRUD/search、checkpoints、undo-turn（见 `src/daemon/routes.rs`）。foundation change 将提供面板挂载点、daemon client 与对话界面。本 change 在其上新增会话管理面板。

## Goals / Non-Goals

**Goals:**
- 会话列表/搜索/新建/切换/删除的 GUI 面板
- 切换会话后完整加载历史并可继续对话
- checkpoint 列表与 undo-turn 回滚的图形化操作

**Non-Goals:**
- 对话渲染本身（foundation）、配置界面（gui-config-and-models）
- daemon API 变更；跨设备会话同步

## Decisions

1. **纯客户端实现**：全部能力通过 daemon 现有 API 组合实现，不新增服务端端点；若发现 API 缺口，在 build 阶段升级为范围决策点。
2. **面板化集成**：会话列表作为侧边导航下的一级面板，切换会话通过共享应用状态通知对话面板刷新，不直接耦合对话组件内部。
3. **undo-turn 二次确认**：回滚操作不可逆性高，GUI 必须提供明确的确认交互与结果反馈。

## Risks / Trade-offs

- [长历史会话加载慢] → 分页/懒加载历史消息，先渲染最近 N 轮
- [daemon 会话 API 与 GUI 需求存在缺口（如列表排序/过滤参数）] → build 阶段先 spike 验证 API 完备性，缺口升级为范围决策点而非私自改 daemon
