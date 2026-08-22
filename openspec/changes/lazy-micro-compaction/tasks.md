# Tasks: lazy-micro-compaction

## 1. 纯函数层（compaction.rs）

- [x] 1.1 新增 `sniff_tool_error(content: &str) -> Option<String>`：JSON `success=false`（对象/字符串 error 形态）→ 错误消息；`Tool '…' blocked by hook: ` 纯文本 → 首行；其余 `None`。char 边界安全截断 200 chars
- [x] 1.2 新增 `error_head_marker(tool_name, content) -> String`：失败结果产 `[Previous: used X; error: <摘要>]`；成功结果产裸 `[Previous: used X]`（格式向后兼容）
- [x] 1.3 新增 `advance_frontier(messages: &[ChatMessage], keep: usize) -> usize`：返回「覆盖最新 keep 条 tool 消息」的最小索引（不足 keep 条则 0）
- [x] 1.4 新增 `build_request_tail(raw: &[ChatMessage], boundary: usize, frontier: usize) -> Vec<ChatMessage>`：`micro_compact(raw[boundary..frontier])`（带错误摘要 marker、file_read 豁免）`++ raw[frontier..]`
- [x] 1.5 单元测试：嗅探三种错误形态 + 成功/非 JSON 返回 None；marker 截断边界；advance_frontier 边界（0 条 / 恰好 keep 条 / 多于 keep 条）；视图确定性（同输入两次构建逐字节一致）与 append-only 性质（frontier 不变、追加消息时旧视图是前缀）；file_read 在压实段内豁免、段外逐字

<!-- tasks 1.1-1.5: 压实段用独立 compact_frontier_segment（段内全压实——keep-3 由
     frontier 本身保证，设计伪码中的 micro_compact 仅示意）。marker 截断 200 chars
     含 ellipsis；file_read/read_file 豁免；边界 clamp（frontier<boundary →
     boundary；>len → len；boundary>len → 空）。10 个新单测全绿。 -->

## 2. 触发与接线（loop_.rs）

- [x] 2.1 `LoopTurnState` 增加 `micro_compact_frontier: usize`（默认 0，非持久化）
- [x] 2.2 请求视图构建替换：`build_request_tail(raw, boundary, frontier)`；用 `estimate_prompt_tokens_calibrated(视图+fixed_overhead)` 对比水位线 `(context_window*3/5).saturating_sub(max_tokens)`，超线则 `frontier = advance_frontier(raw, 3)`（clamp ≥ boundary）后重建视图
- [x] 2.3 全量压缩评估保持在实际发送视图上（`needs_compaction` 调用点不变，确认入参是构建后的 `messages`）；压缩成功推进 `compaction_boundary` 后 clamp `frontier = max(frontier, boundary)`
- [x] 2.4 排查并适配受影响的现有测试（grep `micro_compact` / `Previous: used` 断言）；确认 413 应急 `fallback_micro_compact` 与 `prepare_compaction_transcript` 路径未受影响

<!-- task 2.1: TUI 侧构造点（tui/agent/core.rs）补 micro_compact_frontier: 0——
     每轮重置为逐字，符合 D6（恢复/新轮从 verbatim 起步，超水位时立即重新推进）。
     task 2.2: fixed_tool_def_chars + build_calibration 提前到视图构建前（水位
     估算需要）；closure 捕获 summary clone 避免与 state 写入的借用冲突。
     task 2.3: 压缩成功块内的估算视图同样换用 build_request_tail（防乐观估算
     导致的无限压缩循环）。task 2.4: 残留 micro_compact_messages 调用仅剩
     compactor.rs 两处（413 应急 + 摘要器输入，设计要求不动）与
     tui/agent/mod.rs 手动 /compact 的 UI token 读数刷新（不构建请求，无行为
     影响）；mod.rs re-export 保留。既有测试零适配即全绿。 -->

## 3. 验证

- [x] 3.1 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` 通过
- [x] 3.2 `cargo test` 全绿（新增单测 + runtime/compaction 既有套件零回归）
- [x] 3.3 `openspec validate lazy-micro-compaction` 通过

<!-- task 3.2: lib 1815（+10 新测试）+ integration 218，0 failed。clippy/fmt 干净。
     openspec validate 通过。改动面：compaction.rs +375 / loop_.rs ~81 / tui core +1。 -->
