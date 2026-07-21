## 1. Ops API 基础

- [ ] 1.1 在 `daemon/models.rs` 新增 `OverviewResponse`、`MemoryListResponse` / `MemoryItemResponse`（含 `origin`）、扩展 `ConfigResponse` → `OpsConfigResponse`（脱敏字段）
- [ ] 1.2 实现 `GET /api/v1/overview`（project_root、session_count、memory status 摘要、model、version）
- [ ] 1.3 实现 `GET /api/v1/memory/status`、`GET /api/v1/memory`（scope/min_importance/limit/offset/q）、`GET /api/v1/memory/:id`、`POST /api/v1/memory/prune`
- [ ] 1.4 扩展 `GET /api/v1/config` 为运维只读大盘 DTO；断言 api_key 不出现明文
- [ ] 1.5 在 `routes.rs` 注册上述路由（protected）；补充 handler 单元/集成测试

## 2. 静态控制台壳

- [ ] 2.1 新增 `src/daemon/web/`（或 `web/ops-console/`）静态入口：`index.html` + JS/CSS；hash 路由骨架
- [ ] 2.2 daemon 挂载静态资源与 `/`；API 与静态共存；保留 health 公开
- [ ] 2.3 前端 token 引导（sessionStorage）与统一 `fetch` 封装（Bearer、错误展示）

## 3. 页面实现（P0）

- [ ] 3.1 Overview 页：健康/项目根/计数/模型摘要
- [ ] 3.2 Sessions 列表 + 搜索 + 详情（消息只读）+ 删除确认
- [ ] 3.3 Memory 列表（scope/importance）+ 详情 + prune 确认
- [ ] 3.4 Config 只读分组展示（脱敏）

## 4. 文档与收尾

- [ ] 4.1 更新 `WGENTY.md` Daemon 小节：控制台 URL、token 路径、P0 能力范围
- [ ] 4.2 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + 相关 `cargo test` 通过
