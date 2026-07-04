## 1. src/api 模块拆分(纯重构,代码已在工作区)

- [x] 1.1 创建 `src/api/error.rs`,移动 `wrap_network_error` / `format_api_error` 及其单元测试
- [x] 1.2 创建 `src/api/types.rs`,移动 wire-format 类型(`ToolDefinition`/`ChatMessage`/`ChatRequest`/`ChatResponse`/`StreamChunk` 等)及其单元测试
- [x] 1.3 更新 `src/api/mod.rs`:`pub mod error/types` + `pub use types::*` + `pub(crate) use error::*`,保持 `crate::api::*` 旧路径
- [x] 1.4 `cargo build && cargo test --lib && cargo clippy --lib` 验证行为零变化、旧 `crate::api::*` 引用仍编译

## 2. subagent 大结果交付重新设计(行为变更)

- [x] 2.1 comet-design 阶段 brainstorming 选定缓解方案(按需加载 B1 / compaction 降级 B2 / 混合 B3)
- [x] 2.2 查清 `to_compact()` 是否 dead code、是否应接入 compaction 作为降级入口
- [x] 2.3 按 design 定案调整 `src/teams/subagent_mailbox.rs`:`SubagentResponse` 变体、`to_content()`、`to_compact()`、`offload_if_large()`
- [x] 2.4 同步 `src/tools/meta/task.rs` 调用方(若 `to_content` 返回语义变化)
- [x] 2.5 若选 B2:扩展 `src/tui/agent/compaction.rs` 识别上下文中的大 subagent 结果并替换为磁盘引用
- [x] 2.6 更新/新增测试覆盖选定方案的 spec scenarios(不丢细节 + token 可控 + 磁盘恢复 + 持久化失败降级)
- [x] 2.7 `cargo test` 验证 `subagent-result-delivery` spec 全部 scenarios 通过

## Final Review (whole-branch)

- **结果**: APPROVED(无 Critical/Important)
- **验证**: cargo build + test --lib 462 passed + clippy 无 warning;调用方 `src/tools/meta/task.rs` + `src/tui/agent/compaction.rs` 零改动(git diff 确认);footer em-dash U+2014 字节序列 E2 80 94 验证;`to_compact` 完全删除(grep 无残留);Change A glob re-export 生效(58 处 `crate::api::*` 引用 + `daemon/handlers.rs:98` 的 `crate::api::format_api_error` 经 `pub(crate) use error::*;` 再导出)
- **接受的 Minor findings**(不阻塞 merge):
  1. doc comment `>` hack(line 85):`clippy::doc_lazy_continuation` fix 的必要方式(显式 blockquote continuation),非装饰性,接受
  2. 磁盘失败测试环境依赖:`test_disk_persistence_failure_degrades_to_inline` 用不可写目录注入 store 失败,非 root 环境稳定;plan 已备 fallback(已存在文件),接受
  3. `test_summarized_summary_is_head_prefix` 略同义反复:测试验证 summary 生成契约(`content.chars().take(SUMMARY_HEAD_LEN)`),断言方式直接但覆盖语义,接受
