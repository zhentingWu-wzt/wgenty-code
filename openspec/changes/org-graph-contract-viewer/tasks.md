# Tasks

## 1. NodeRegistry 遍历入口

- [x] 1.1 为 `NodeRegistry` 新增 `iter()`（或 `all()`）只读遍历入口，按 `NodeType` 枚举稳定顺序（Explore, Plan, GeneralPurpose, Verification, WgentyCodeGuide）返回全部契约
- [x] 1.2 为遍历入口加单测：五个内置契约全部出现、顺序稳定、与逐个 `get()` 结果一致

## 2. 渲染模块（`src/org_graph/render.rs`）

- [x] 2.1 定义 `Format` 枚举（`Table` / `Dot` / `Mermaid` / `Json`），派生 `clap::ValueEnum`，默认 `Table`
- [x] 2.2 实现 `render_json`：复用 `NodeContract` 的 `Serialize`，输出 `NodeContract` JSON 数组
- [x] 2.3 实现 `render_table`：终端表格，覆盖五维字段（`system_prompt` 过长，默认截断/省略）
- [x] 2.4 实现 `render_dot`：合法 Graphviz DOT，每个契约渲染为节点
- [x] 2.5 实现 `render_mermaid`：合法 mermaid 图定义，每个契约渲染为节点
- [x] 2.6 实现统一入口 `render(registry, format) -> String`，按 `Format` 分派到上述函数

## 3. 渲染输出单测

- [ ] 3.1 `json` 输出可被 `NodeContract` serde 反序列化，且与 `NodeRegistry::builtin()` 逐字段相等
- [x] 3.2 `explore_readonly=true` / `false` 时，Explore 与 Plan 的 `can_mutate_fs` 在所有四种格式中如实反映
- [x] 3.3 `dot` 输出以 `digraph` 声明开头、节点闭合（结构断言；CI 有 graphviz 则加 `dot` 解析冒烟测试）
- [x] 3.4 `mermaid` 输出以合法图类型声明开头，且五个契约均成为图中节点

## 4. CLI 命令接线

- [ ] 4.1 在 `src/cli/mod.rs` 新增顶层 `OrgGraph` 命令组与 `Contracts` 子命令，带 `--format` value_enum（默认 `table`）
- [ ] 4.2 命令处理逻辑：加载 `SubagentLimits` 配置 → 构造 `NodeRegistry::builtin()` → `render()` → 打印到 stdout
- [ ] 4.3 在 `main.rs` 命令分派处接线 `Commands::OrgGraph { action: OrgGraphCommands::Contracts { format } }`

## 5. 集成验证

- [ ] 5.1 `cargo build` 通过；`cargo test` 全绿（新增测试通过 + 已有测试零回归）
- [ ] 5.2 手动验证四种格式输出（`table` / `dot` / `mermaid` / `json`）符合预期，`--format` 缺省为 `table`
