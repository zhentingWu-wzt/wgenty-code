# 代码风格

遵循 Rust 标准命名约定（`snake_case` 变量/函数，`CamelCase` 类型/trait，`SCREAMING_SNAKE_CASE` 常量）。

- 使用 `cargo fmt` 统一格式（CI 强制执行 `cargo fmt -- --check`）。
- 使用 `cargo clippy -- -D warnings` 保持零 warning（CI 强制执行）。
- 公开 API 优先添加 `///` 文档注释；复杂内部逻辑用 `//` 行注释说明意图。
- 遵循 [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) 设计公开接口。
- 模块组织：使用 `mod.rs` 风格或新式同名文件均可，保持模块内一致。

---

# 错误处理

- 库代码使用 `thiserror` 派生自定义错误枚举，提供明确的错误信息和 `#[error("...")]` 注解。
- 应用层使用 `anyhow::Result` + `.context("描述")` 添加上下文信息。
- 不要直接 `unwrap()` 或在无上下文的 `?` 中吞掉错误信息——每个 `?` 应通过 `.context()` 提供人类可读的失败描述。
- 对可能 panic 的代码（如数组索引）添加注释说明为何不会越界。

```rust
// ✅ 推荐
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("配置项无效: {0}")]
    Invalid(String),
}

pub fn load(path: &str) -> Result<Config> {
    std::fs::read_to_string(path)
        .context("读取配置文件失败")?
        .parse()
        .context("解析配置文件失败")
}

// ❌ 避免
pub fn load(path: &str) -> Result<Config> {
    Ok(std::fs::read_to_string(path)?.parse()?)
}
```

---

# 异步编程

- 使用 `tokio` 作为异步运行时（full features）。
- 共享可变状态通过 `Arc<RwLock<T>>` 实现，避免不必要的锁竞争。
- 批量并发操作使用 `futures::future::join_all` 或 `tokio::join!`。
- 长时间运行的后台任务使用 `tokio::spawn`，确保正确处理 JoinHandle。
- 异步 trait 使用 `#[async_trait]` 宏。

---

# 提交规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范，使用英文编写。

格式：
```
<type>(<scope>): <简短描述>

<body>

<footer>
```

类型（type）：
- `feat` — 新功能
- `fix` — 错误修复
- `docs` — 仅文档变更
- `style` — 代码风格（不影响功能，如格式化）
- `refactor` — 重构（不改变功能也不修 bug）
- `perf` — 性能优化
- `test` — 添加或修改测试
- `chore` — 构建、CI、依赖更新等杂务

scope 可选，为受影响的模块名（如 `cli`、`api`、`tools`、`sandbox`）。

示例：
```
feat(cli): 添加 config reset 命令

- 支持重置配置到默认值
- 添加 --force 标志

Closes #123
```

---

# 分支与 PR 流程

- **分支命名**：`feature/<描述>`、`fix/<描述>`、`refactor/<描述>`。
- 从 `develop` 创建功能分支，完成后向 `develop` 提交 PR。
- `main` 为稳定分支，仅通过 tag（`v*`）触发 Release。
- PR 标题遵循与 commit 相同的 Conventional Commits 格式。

PR 提交前自检：
1. 运行 `cargo clippy --all-targets -- -D warnings` 零 warning
2. 运行 `cargo fmt` 确保格式一致
3. 运行 `cargo test --all` 所有测试通过
4. 复杂变更添加相关注释和文档
5. 更新 CHANGELOG.md 记录变更

---

# 性能约束

新增代码不得显著影响基础性能指标：

- **启动时间**：增量 ≤ 5%
- **内存占用**：基础内存增量 ≤ 2%
- **二进制大小**：增量 ≤ 500KB

验证命令：
```bash
# 构建 release
cargo build --release

# 测试启动速度
time ./target/release/wgenty_code --version

# 检查二进制大小
ls -lh ./target/release/wgenty_code
```

---

# 工作流约定

- **复杂变更先规划**：涉及多模块的重构或新功能，先理清架构变更范围和影响面。
- **重构时解释权衡**：在 PR 描述中说明为什么选择方案 A 而非 B。
- **特性开关（feature flags）**：新功能若只适用特定场景，通过 Cargo feature flag 控制编译，保持默认构建的精简。
- **安全敏感变更**：涉及 `guardian/`、`sandbox/`、`permissions/` 的变更需额外审慎，说明安全影响。
- **跨平台兼容**：代码需在 linux/macos/windows 三平台均可编译运行，避免平台特定假设。
- **国际化**：面向用户的字符串应通过 `i18n/` 模块管理（使用 Fluent 格式），避免硬编码。
- **计划同步**：使用 `TodoWrite` 更新任务状态后，同步调用 `update_plan` 更新 UI 面板，保持两端状态一致。

---

# 模块依赖原则

- `tools/` 不应依赖 `agent/`（工具是独立的执行单元）。
- `api/` 不应依赖 `cli/` 或 `tui/`（API 客户端是底层基础设施）。
- `config/` 不应依赖任何业务模块。
- 跨层依赖通过 trait 抽象（如 `SandboxBackend`、`Tool`），避免具体类型耦合。

---

# 代码审查注意事项

审查时应关注：
- 错误处理是否充分（context 信息是否可操作）。
- 是否存在未处理的 `unwrap()` 或裸 `?`。
- 异步代码中锁持有的时间是否最小化。
- 工具执行（`tools/`）是否声明 `is_read_only()` 正确，影响权限审查。
- Feature flag 的 `required-features` 是否正确配置。
- 是否需要在 WGENTY.md 中更新架构或命令文档。

---

# 工具开发规范

- 所有内置工具实现 `Tool` trait（`name()`、`description()`、`input_schema()`、`execute()`、`is_read_only()`）。
- **`is_read_only()` 默认为 `false`**——任何只读工具（如 file_read、grep、glob）必须显式返回 `true`，否则会被 guardian 视为需要写权限。
- 新工具在 `ToolRegistry` 构造时注册。若仅特定 provider 支持（如 `apply_patch` 的 Anthropic 特有格式），在 `with_settings()` 中按 provider 动态移除。
- 工具执行结果应返回结构化 `ToolResult`，包含 `success`、`output`、可选的 `metadata`。
- 执行类工具（`exec_command` 等）需经过 guardian 安全审查，修改系统状态的工具需声明 `is_read_only() = false`。
