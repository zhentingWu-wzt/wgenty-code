# Brainstorm Summary

- Change: codegraph-tool-auto-registration
- Date: 2026-06-14

## 确认的技术方案

OnceLock<Arc<QueryEngine>> 懒加载 + 索引器边界守卫

- tools.rs: 移除 struct 级 Arc<QueryEngine>，改用 `static ENGINE: OnceLock<Arc<QueryEngine>>`
- get_engine(): 首次调用时从 cwd/.codegraph/index.db 打开 IndexStore，缓存到 OnceLock
- indexer.rs: 跳过未解析引用（instead of panic）、跳过负 ID 关系、跳过跨文件部分解析

## 关键取舍与风险

- OnceLock 单进程单索引 → 适合 daemon-per-session
- 静态不可重置 → daemon 重启即可重置

## 测试策略

- cargo build + test --lib + clippy
- Manual: 验证有/无索引场景

## Spec Patch

无
