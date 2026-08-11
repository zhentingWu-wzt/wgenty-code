# Brainstorm Summary

- Change: org-graph-contract-viewer
- Date: 2026-08-11

## 确认的技术方案

把内置 `NodeRegistry`（5 个 `NodeContract` × 5 维）渲染成 `table` / `dot` / `mermaid` / `json` 四种可读视图。

- 新增顶层 CLI 命令组：`wgenty-code org-graph contracts [--format table|dot|mermaid|json]`（默认 `table`）。handler 经 `AppState` 取 `settings.subagent`（`SubagentLimits`）→ `NodeRegistry::builtin()` → `render()` → stdout。
- 新增纯函数渲染模块 `src/org_graph/render.rs`：`Format` 枚举（`clap::ValueEnum`，默认 `Table`）+ `render(registry, format) -> String` + 四个独立格式函数。
- 为 `NodeRegistry` 新增只读 `iter()`：`const CANONICAL_ORDER: [NodeType; 5]`（枚举声明序）遍历 `HashMap`，返回 `Vec<&NodeContract>`，保证输出确定性。
- 三个人读格式（`table`/`dot`/`mermaid`）**省略 `system_prompt`**（唯一长字段）以保可读；`json` 全保真（含 `system_prompt`）作为无损事实源。

**dot/mermaid 结构（已确认 Option A）**：契约层扁平（5 节点、无边）；每个契约 = 一个丰富节点卡（record/label 列五维）。视觉编码：`can_spawn` → 节点形状（leaf=`box`、spawnable=`component`）；`can_mutate_fs` → 填充色（true=filled、false=white）。标签文字始终承载全部维度，视觉编码仅为速读辅助（非承重）。

## 关键取舍与风险

- **扁平 vs 人造树**：选诚实反映扁平（无边），真正 org-chart 留待关系层。人造根节点会误导。
- **`system_prompt` 仅出现在 json**：三视觉格式省略以保可读；json 为无损逃生口。`--help` 注明。
- **DOT/Mermaid 语法合法性风险**：手写格式化易出隐蔽错误。缓解：结构断言（`digraph` 开头、5 节点闭合、mermaid 合法图类型）+ CI 有 graphviz 时加 `dot -Tsvg` 冒烟解析（guarded）。
- **新顶层命令增长 CLI 表面**：接受（为 Org-Graph 多层子系统留扩展空间）。
- **视觉编码是装饰性的**：渲染器忽略样式时，标签文字仍承载全部信息。

## 测试策略

纯函数 → 无需 CLI 管线即可单测：
- `iter()`：5 契约齐全、顺序稳定、与逐个 `get()` 一致。
- `json`：serde 往返与 `NodeRegistry::builtin()` 逐字段相等。
- `explore_readonly` true/false：`can_mutate_fs` 在四种格式均如实反映。
- `dot`/`mermaid`：结构断言（图类型声明、5 节点）。
- 回归：现有 `org_graph` + `cli` 测试全绿（纯新增）。

## Spec Patch

无。delta spec 的验收场景（table/dot/mermaid/json/explore_readonly）已被设计满足；视觉编码是"每个契约渲染为节点"的实现细节，非 spec 级变更。
