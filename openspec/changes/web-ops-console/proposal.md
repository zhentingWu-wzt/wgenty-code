# Proposal: Web 项目运维控制台（P0）

## Why

CLI/TUI 已能驱动 agent，但项目级后台状态（会话、记忆、运行配置）缺少浏览器侧的长期可观察与可管理入口。运维与排障仍依赖命令行拼装，不利于持续挂着查看当前项目健康度。现有 `daemon` 已具备 token 鉴权与会话 CRUD 等 API 雏形，具备以较低增量补齐「项目运维台」的条件。

## What Changes

1. **Web 运维台（P0）**：在 daemon 上提供可浏览器访问的轻量控制台，面向**单项目、本机/受信网络**场景。
2. **概览页**：展示项目根、daemon health、会话数量、记忆 project/global 计数、当前模型摘要。
3. **会话管理 UI + API 复用**：列表 / 搜索 / 详情 / 删除；优先复用已有 `/api/v1/sessions*`。
4. **记忆管理 API + UI**：新增 memory HTTP（status / list 过滤 / 详情 / prune 需确认）；包装现有 `MemoryManager`。
5. **配置只读大盘**：扩展配置读取面（相对今日仅 model/base/max_tokens/timeout/streaming），**密钥脱敏**；P0 **不写回**。
6. **静态前端托管**：daemon 提供控制台静态资源（或嵌入资源），受与 API 一致的鉴权策略约束（细节见 design）。

**Non-goals（P0）**

- 控制台内完整聊天 / agent 驱动
- 配置写回与热更新
- 每轮原始 prompt/上下文全文溯源（P2）
- 多项目切换器、多用户 RBAC
- 公网暴露级安全加固

## Capabilities

### New Capabilities

- `web-ops-console`: 浏览器运维台信息架构、页面与静态托管、鉴权访问体验
- `ops-console-api`: 运维台所需 HTTP API（overview、memory、扩展只读 config）；与现有 sessions API 的契约对齐

### Modified Capabilities

- （无强制修改既有 capability 的 REQUIREMENTS；若实现中发现 sessions 响应字段不足以支撑详情页，可在 design/specs 中以 delta 补充 `daemon` 相关约定。当前 `openspec/specs/` 若无对应 daemon spec，则全部落在新 capability。）

## Impact

- **Code**: `src/daemon/`（routes/handlers/state）、可能新增 `src/daemon/ops/` 或 `web/` 静态资源目录；`MemoryManager` 只读/prune 包装；配置序列化与脱敏
- **API**: 新增 `/api/v1/overview`、`/api/v1/memory*`；扩展 `GET /api/v1/config` 只读字段；sessions 保持兼容
- **Deps**: 前端构建链（若采用 SPA）或极简静态 HTML/JS；daemon `tower-http` 静态文件服务（feature `daemon` 内）
- **Security**: API token；密钥永不回传明文；prune/delete 需显式确认
- **Docs**: WGENTY.md CLI/Daemon 小节补充控制台访问方式
