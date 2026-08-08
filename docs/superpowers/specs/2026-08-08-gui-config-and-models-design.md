---
archived-with: 2026-08-08-gui-config-and-models
status: final
---
# Design Doc: GUI Config and Models

> 技术设计文档 for `gui-config-and-models` change。

## 概述

配置与模型管理界面：模型切换、配置编辑、MCP servers 管理、skills 查看、memory 浏览/搜索/删除。

## 设计决策

### 1. 后端逻辑全部复用（零底层改动）

探索发现三个缺口的底层逻辑都已存在：
- **配置写入**：`Settings::save()` + `switch_model` 模式可直接复用
- **MCP 管理**：`McpManager` 已有 add/remove/start/stop/restart 方法
- **memory 删除**：`MemoryManager::delete_memory(origin, id)` 已实现

工作量集中在 daemon HTTP handler 封装 + 前端面板。

### 2. 配置写入安全

PUT /config 只接受 transport 级字段（max_tokens/timeout/streaming/api_base）。
- `api_key`/`appkey` **永不接受、永不返回**
- 校验：max_tokens > 0, timeout > 0
- 写入模式镜像 switch_model：clone → validate → disk save → live handle → broadcast

### 3. MCP 管理：start/stop 代替 enabled 字段

不新增 `enabled: bool` 到 McpConfig。用 start/stop 满足运行时启用/禁用——更简单，不改配置结构体。

### 4. Memory 搜索：client-side 过滤

daemon 的 listMemory 已返回 content，前端按 substring 过滤即可，无需新 API。

## 影响范围

- daemon：3 个新 handler 文件（handlers.rs/models.rs/routes.rs）+ GlobalEventKind::ConfigChanged
- 前端：2 个新面板（ConfigPanel/McpPanel）+ MemoryPanel 增强 + RightRail 入口
- 不改：McpConfig 结构、Settings 结构、底层 manager 逻辑
