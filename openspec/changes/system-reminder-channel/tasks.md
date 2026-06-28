## 1. 数据结构与 readers

- [x] 1.1 在 `src/runtime/hooks/mod.rs` 新增 `InjectedFragment` 公共结构（`content`/`priority`/`visibility`/`source_label`），并新增 `collect_injections(&[HookOutcome]) -> Vec<InjectedFragment>` 辅助函数；增加单测覆盖空 outcomes、单 outcome、多 outcome 排序场景
- [x] 1.2 在 `src/utils/project.rs` 新增 `read_user_global_instructions() -> Option<(PathBuf, String)>` 读取 `~/.wgenty-code/WGENTY.md`,使用 `dirs::home_dir()`,无 home / 文件不存在均返回 `None`;增加单测覆盖存在、缺失、空文件三种情况
- [x] 1.3 在 `src/utils/project.rs` 新增 `read_user_global_rules() -> Vec<(PathBuf, String)>` 扫 `~/.wgenty-code/rules/*.md` 顶层非空 .md 文件，按字母序返回；忽略子目录与非 .md；增加单测覆盖空目录、多文件排序、子目录忽略
- [x] 1.4 扩展 `PromptContext` 增加 `project_root: Option<PathBuf>` 字段及 `with_project_root(path)` builder 方法；保持向后兼容（默认 `None`）

## 2. Reminder builder

- [x] 2.1 在 `src/prompts/mod.rs` 新增私有常量 `REMINDER_PREAMBLE_OPENING` 和 `REMINDER_PREAMBLE_CLOSING`，精确复刻 Claude Code 措辞（含 `# claudeMd` 与闭合 preamble 6 空格缩进）
- [x] 2.2 实现 `build_user_turn_reminder(ctx: &PromptContext, hook_injections: &[InjectedFragment]) -> Option<String>`：聚合 4 个文件源 + hook 注入按 priority 排序，按 D6 文本骨架渲染；4 源全缺且无 hook 注入返回 `None`
- [x] 2.3 实现来源标注辅助函数 `render_attribution_header(absolute_path: &Path, description: &str) -> String`，统一输出 `Contents of <absolute-path> (<description>):` 格式
- [ ] 2.4 单测：全 4 源齐全的完整 reminder 文本快照（含具体顺序、缩进、preamble）
- [ ] 2.5 单测：缺失各文件源时不出现空标题、不报错；4 源全缺且无 hook 返回 `None`
- [ ] 2.6 单测：来源标注路径是绝对路径
- [ ] 2.7 单测：rules/*.md 字母序
- [ ] 2.8 单测：hook 注入按 priority asc 排序，ties 保持调用方传入顺序

## 3. 请求构造层接入

- [ ] 3.1 查找 `src/tui/agent/` 下构造发送给模型的 user message 的位置（预计在 stream.rs 或 mod.rs 的请求装配处），把 reminder 注入路径插入：构造 user content 字符串时先拼 reminder，再拼原始 prompt
- [ ] 3.2 把 `tui/app/input.rs:181` 的 `tokio::spawn(async move { hm.fire(...) })` 改为 `await` 同步执行，并把 outcomes 通过 PendingInput 或等价通道传给请求构造层
- [ ] 3.3 在请求构造路径调用 `collect_injections(&outcomes)` 提取 `InjectedFragment`，传给 `build_user_turn_reminder`
- [ ] 3.4 集成测：模拟 user 输入，断言第一轮 user message content 头部包含 `<system-reminder>` 块
- [ ] 3.5 集成测：连续两轮 user 输入，第二轮 user message 再次包含 reminder（per-turn 验证）

## 4. 移除旧 Layer + 适配 builder

- [ ] 4.1 删除 `src/prompts/mod.rs` Layer 7（AGENTS.md）和 Layer 8（WGENTY.md）的 system message push 代码块
- [ ] 4.2 `PromptContextBuilder::with_wgenty_md` / `with_agents_md` 保持签名不变，仅在 `assemble_instructions` 内部确保数据被 reminder builder 而不是 system message push 使用
- [ ] 4.3 `src/tui/app/mod.rs` 在构造 PromptContext 时同时调用 `with_project_root(std::env::current_dir())`，让 reminder builder 能渲染绝对路径
- [ ] 4.4 单测：assembled system_messages 中**不再**出现 `# AGENTS.md` 或 `# WGENTY.md — 项目规则与约定` 文本（硬切验证）

## 5. Hook injection 接通

- [ ] 5.1 验证 `UserPromptSubmit` hook 的 `HookOutcome` 中 `injected_content` 已经被正确填充（如未填充则在 `run_inject_action` 路径补齐）；增加单测保证 `HookAction::InjectContext` 的 outcomes 包含 `injected_content`
- [ ] 5.2 在请求构造层把 hook 收集到的 `InjectedFragment` 与文件源一起传给 reminder builder，验证多个 hook 时优先级和顺序正确
- [ ] 5.3 集成测：在 `settings.json` 配置 `UserPromptSubmit` hook 返回 `"injected_content": "EXTRA"`，断言下一轮 user message 中可见 `EXTRA` 字符串
- [ ] 5.4 集成测：配置两个 hook（priority 不同），断言注入内容按 priority 排序

## 6. Token 预算警告

- [ ] 6.1 把 `src/tui/app/mod.rs` 现有"WGENTY+AGENTS 超阈值警告"改造为"完整 reminder 块超阈值警告"
- [ ] 6.2 警告触发位置：首次构造 reminder 时计算（不是 session 启动时），保留"每 session 仅一次"语义
- [ ] 6.3 hook 注入内容**不计入**预算（动态、每轮变）；只计入 4 个文件源 + preamble overhead
- [ ] 6.4 单测：超阈值触发警告，二次构造不重复触发
- [ ] 6.5 单测：未超阈值不发警告

## 7. Documentation & polish

- [ ] 7.1 在 `WGENTY.md`（项目根）新增一段 "Context injection channels" 说明 `~/.wgenty-code/WGENTY.md` + `~/.wgenty-code/rules/` 用法（注意：是文档说明，不是实际放规则）
- [ ] 7.2 CHANGELOG 标记 BREAKING："项目说明改走 system reminder 通道，不再出现在 system prompt 链路"
- [ ] 7.3 在 `~/.wgenty-code/rules/` 新建示例文件 `comet-phase-guard.md`（从 `~/.claude/rules/comet-phase-guard.md` 拷贝），用于 dogfood 本次实现
- [ ] 7.4 运行完整 `cargo test` 与 `cargo clippy -- -D warnings`，零 warning 通过
- [ ] 7.5 运行 `cargo fmt -- --check`，格式合规

## 8. 验证

- [ ] 8.1 验证 12 条验收场景全部覆盖至少 1 个测试用例
- [ ] 8.2 启动 `wgenty-code repl`，输入任意 prompt，用 logs / debug toggle 确认 user message 内容含 reminder 块
- [ ] 8.3 删除 `~/.wgenty-code/WGENTY.md`，再次输入 prompt，确认无报错、无空标题
- [ ] 8.4 配置 `UserPromptSubmit` hook 返回 inject content，重启 repl 验证 hook 注入端到端工作
- [ ] 8.5 用 `cargo run -- repl --prompt "X"` 单次查询模式同样验证 reminder 注入

## 9. 解决 design doc 的 Open Questions

- [ ] 9.1 O1: 决定 `# claudeMd` 标题保留 vs 改名（design 阶段加载 brainstorming 时定）
- [ ] 9.2 O2: 验证 `tui/app/input.rs` UserPromptSubmit fire 改 await 不引入死锁（design 阶段读 start_next_turn 并发模型）
- [ ] 9.3 O3: 定 `LayerVisibility::Internal` 在 TUI transcript 层的具体过滤实现路径
