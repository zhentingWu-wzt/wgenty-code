## 1. TokenCounter 扩展

- [x] 1.1 为 `TokenCounter` 新增 `last_prompt_tokens: Arc<AtomicUsize>` 字段
- [x] 1.2 实现 `set_prompt_tokens(tokens: usize)` 方法，更新 `last_prompt_tokens`
- [x] 1.3 实现 `last_prompt_tokens(&self) -> usize` 读取方法

## 2. API 用量记录

- [x] 2.1 在 `AgentLoop::run_agent_loop` 的 token accounting 处，将 `usage.prompt_tokens` 存入 `token_counter.set_prompt_tokens()`

## 3. 上下文窗口配置

- [ ] 3.1 在 `Settings`/`ModelConfig` 中新增 `context_window: usize` 字段，默认 200000
- [ ] 3.2 确保 `settings.json` 序列化/反序列化兼容（可选字段，缺失时用默认值）

## 4. 进度条组件

- [ ] 4.1 新建 `src/tui/components/context_bar.rs`，实现 `render(f, area, used, max)` 函数
- [ ] 4.2 渲染 Unicode 进度条（8 格 `▓`/`░`）+ 百分比文字
- [ ] 4.3 实现颜色阈值逻辑：绿 <50%，黄 50-80%，红 >80%
- [ ] 4.4 在 `src/tui/components/mod.rs` 注册 `context_bar` 模块

## 5. 模式标签栏集成

- [ ] 5.1 修改 `render_mode_label`，在模式标签右侧渲染上下文进度条
- [ ] 5.2 从 `App` 传入 `token_counter.last_prompt_tokens()` 和 `settings.models.context_window`
- [ ] 5.3 窄终端（宽度 <40）自动隐藏进度条

## 6. 测试

- [ ] 6.1 `TokenCounter` 的 `set_prompt_tokens`/`last_prompt_tokens` 单元测试
- [ ] 6.2 进度条颜色阈值边界测试（49%/50%/80%/81%）
- [ ] 6.3 `context_window` 配置默认值与自定义值测试
