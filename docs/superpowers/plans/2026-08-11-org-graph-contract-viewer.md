---
change: org-graph-contract-viewer
design-doc: docs/superpowers/specs/2026-08-11-org-graph-contract-viewer-design.md
base-ref: 45e31838d271a4cb3617a89a42df50a691f0487c
---

# Org-Graph Contract Viewer 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: 使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现本计划。步骤用复选框（`- [ ]`）语法跟踪。

**目标：** 把内置 `NodeRegistry`（5 个 `NodeContract` × 五维：capability / permission / budget / IO shape / identity）渲染成 `table` / `dot` / `mermaid` / `json` 四种可读视图，经新顶层命令 `wgenty-code org-graph contracts [--format]` 输出到 stdout。

**架构：** 三处 additive 改动——(1) `NodeRegistry::iter()` 只读方法（确定性顺序遍历 HashMap）；(2) 新纯函数模块 `src/org_graph/render.rs`（四个 `render_*` + 一个 `render` 派发，无 async/IO/状态）；(3) CLI 侧 `src/cli/org_graph.rs` handler + `Commands::OrgGraph` 变体 + `run_async` 接线。`org_graph` 模块保持 **零 clap 依赖**：`Format` 在 `render.rs` 里只派生 `Copy/Clone/Debug/PartialEq/Eq`，CLI 侧用一个独立的 `clap::ValueEnum`（`OrgGraphFormatArg`）映射到它。

**技术栈：** Rust，clap derive（`Subcommand` / `ValueEnum`），serde / serde_json（`NodeContract` 已派生 `Serialize`）。无新外部 crate。

## Global Constraints

- **纯函数模块**：`src/org_graph/render.rs` 无 async、无 I/O、无状态；每个 `render_*` 输入 `&NodeRegistry`、输出 `String`。
- **`org_graph` 零 clap 依赖**：`render::Format` 只派生 `Copy/Clone/Debug/PartialEq/Eq`；clap 的 `ValueEnum` 派生只出现在 `src/cli/` 侧的 `OrgGraphFormatArg`。
- **确定性顺序**：所有渲染通过 `NodeRegistry::iter()` 走 `CANONICAL_ORDER`（枚举声明序：Explore → Plan → GeneralPurpose → Verification → WgentyCodeGuide），不直接遍历 `HashMap`。
- **`system_prompt` 取舍**：仅 `json` 全保真（含 `system_prompt`）；`table` / `dot` / `mermaid` 省略该长字段，保留其余全部维度。`--format` 的 `--help` 文案注明此取舍。
- **无新外部 crate**：仅用已是 workspace 依赖的 clap / serde / serde_json。
- **零回归**：纯新增。现有 `org_graph`（`registry.rs` / `contract.rs`）与 `cli` 测试必须保持全绿。
- **数据信任边界**：渲染的 `name` 等字段来自内置 `NodeRegistry`（受控 ASCII），本计划不在渲染器内做 HTML/DOT 转义；视觉格式的语法合法性由结构断言保障（见各任务测试）。

---

## 文件结构

| 文件 | 责任 | 本计划动作 |
|------|------|-----------|
| `src/org_graph/registry.rs` | `NodeRegistry` 定义；新增 `iter()` + `CANONICAL_ORDER` | 修改 |
| `src/org_graph/mod.rs` | 模块导出；新增 `pub mod render;` | 修改 |
| `src/org_graph/render.rs` | 纯函数渲染：`Format` + `render` + 四个 `render_*` | 新建 |
| `src/cli/mod.rs` | `Commands` 枚举；新增 `OrgGraph` 变体 + `OrgGraphCommands`；声明 `pub mod org_graph;` | 修改 |
| `src/cli/org_graph.rs` | `org-graph` 命令 handler：`OrgGraphFormatArg`（clap）+ `run()` | 新建 |
| `src/cli/args.rs` | `run_async` 命令派发；新增 `OrgGraph` match arm | 修改 |

---

## Task 1: `NodeRegistry::iter()` 与确定性顺序

**Files:**
- Modify: `src/org_graph/registry.rs`（在 `impl NodeRegistry` 内新增 `iter()`，在文件内新增 `CANONICAL_ORDER` 常量；在 `#[cfg(test)] mod tests` 内新增两个测试）

**Interfaces:**
- Produces: `NodeRegistry::iter(&self) -> Vec<&NodeContract>`（按 `CANONICAL_ORDER` 稳定顺序返回全部契约）。后续所有渲染任务消费此方法。

- [x] **Step 1: 写失败测试**

在 `src/org_graph/registry.rs` 的 `#[cfg(test)] mod tests` 块内（现有测试之后）追加：

```rust
    #[test]
    fn iter_returns_all_five_in_canonical_order() {
        let r = registry(true);
        let ordered: Vec<NodeType> = r.iter().map(|c| c.node_type.clone()).collect();
        assert_eq!(
            ordered,
            vec![
                NodeType::Explore,
                NodeType::Plan,
                NodeType::GeneralPurpose,
                NodeType::Verification,
                NodeType::WgentyCodeGuide,
            ]
        );
    }

    #[test]
    fn iter_consistent_with_get() {
        let r = registry(true);
        let collected: Vec<&NodeContract> = r.iter();
        assert_eq!(collected.len(), 5, "iter returns all five builtins");
        for c in r.iter() {
            assert_eq!(
                c, r.get(&c.node_type).unwrap(),
                "iter entry must match get() for {:?}",
                c.node_type
            );
        }
    }
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib org_graph::registry::tests::iter_`
预期：编译失败（`method iter not found for NodeRegistry` 或 `CANONICAL_ORDER not found`）。

- [x] **Step 3: 写最小实现**

在 `src/org_graph/registry.rs` 的 `impl NodeRegistry {` 块内（紧接 `get` 方法之后）新增：

```rust
    /// 按稳定顺序（CANONICAL_ORDER）返回全部契约，用于确定性渲染。
    /// 未来若有缺项（自定义契约未注册），自动跳过。
    pub fn iter(&self) -> Vec<&NodeContract> {
        CANONICAL_ORDER
            .iter()
            .filter_map(|nt| self.contracts.get(nt))
            .collect()
    }
```

在 `impl NodeRegistry` 之前（`use` 语句之后、`pub struct NodeRegistry` 之前均可）新增常量：

```rust
/// 渲染用的稳定枚举顺序（枚举声明序）。HashMap 遍历无序，渲染/测试要求确定性。
const CANONICAL_ORDER: [NodeType; 5] = [
    NodeType::Explore,
    NodeType::Plan,
    NodeType::GeneralPurpose,
    NodeType::Verification,
    NodeType::WgentyCodeGuide,
];
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib org_graph::registry::tests`
预期：全部 PASS（含原有测试与两个新测试）。

- [x] **Step 5: 提交**

```bash
git add src/org_graph/registry.rs
git commit -m "feat(org-graph): add NodeRegistry::iter() with canonical order"
```

---

## Task 2: `render.rs` 脚手架 + `render_json`（serde 往返）

**Files:**
- Create: `src/org_graph/render.rs`
- Modify: `src/org_graph/mod.rs`（新增 `pub mod render;`）

**Interfaces:**
- Consumes: `NodeRegistry::iter()`（Task 1）、`NodeContract` 的 `Serialize`（已存在）。
- Produces: `pub enum Format { Table, Dot, Mermaid, Json }`（仅 `Copy/Clone/Debug/PartialEq/Eq`，**无 clap**）；`pub fn render(&NodeRegistry, Format) -> String`；私有 `render_json`。`render_table` / `render_dot` / `render_mermaid` 本任务先放编译期桩（返回 `String::new()`），由 Task 3/4/5 替换为真实实现。

- [x] **Step 1: 写失败测试**

新建 `src/org_graph/render.rs`，先只写测试部分（实现部分 Step 3 补全）。为避免循环，本任务的测试文件即源文件本身——先写完整文件骨架含测试，但 `render_json` 主体留到 Step 3。实际操作：直接在 Step 3 一次性写完整文件（含测试），然后 Step 2 跑测试验证其在补 `render_json` 前会失败。

> 说明：本任务采用「先建文件骨架（含桩 + 测试）→ 跑测试见失败 → 填 `render_json` → 跑测试见通过」的 TDD 节奏。Step 1 描述测试意图，Step 3 给出完整文件内容。

测试意图（将落入文件的 `#[cfg(test)] mod tests`）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::agent::SubagentLimits;
    use crate::org_graph::NodeRegistry;

    fn registry(readonly: bool) -> NodeRegistry {
        NodeRegistry::builtin(&SubagentLimits {
            explore_readonly: readonly,
            ..Default::default()
        })
    }

    #[test]
    fn render_json_via_dispatch() {
        let r = registry(true);
        let out = render(&r, Format::Json);
        assert!(out.starts_with('['), "json output is an array");
        let parsed: Vec<crate::org_graph::NodeContract> =
            serde_json::from_str(&out).expect("valid json");
        assert_eq!(parsed.len(), 5);
    }

    #[test]
    fn render_json_roundtrips_to_identical_contracts() {
        let r = registry(false);
        let parsed: Vec<crate::org_graph::NodeContract> =
            serde_json::from_str(&render_json(&r)).expect("valid json");
        let original: Vec<crate::org_graph::NodeContract> =
            r.iter().map(|c| c.clone()).collect();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed, original, "serde roundtrip must be field-exact");
    }

    #[test]
    fn render_json_includes_system_prompt() {
        // json 是无损格式；system_prompt 必须保留。
        let out = render_json(&registry(true));
        assert!(out.contains("system_prompt"));
        assert!(out.contains("code exploration subagent"));
    }
}
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib org_graph::render`
预期：失败（`error[E0433]: failed to resolve: could not find render in org_graph`，因为 `mod.rs` 尚未声明 `pub mod render;`，且文件尚未建）。

- [x] **Step 3: 写最小实现**

修改 `src/org_graph/mod.rs`，在 `pub mod registry;` 之后新增：

```rust
pub mod render;
```

新建 `src/org_graph/render.rs`，完整内容：

```rust
//! 纯函数渲染：把 NodeRegistry 渲染成 table / dot / mermaid / json。
//! 无 async、无 I/O、无状态。Format 故意**不**派生 clap::ValueEnum，
//! 保持 org_graph 模块零 clap 依赖；CLI 层（src/cli/org_graph.rs）做 ValueEnum 映射。

use crate::org_graph::registry::NodeRegistry;

/// 渲染格式。org_graph 侧无 clap 依赖（仅 Copy/Clone/Debug/PartialEq/Eq）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Table,
    Dot,
    Mermaid,
    Json,
}

/// 按 format 渲染整个注册表。
pub fn render(registry: &NodeRegistry, format: Format) -> String {
    match format {
        Format::Table => render_table(registry),
        Format::Dot => render_dot(registry),
        Format::Mermaid => render_mermaid(registry),
        Format::Json => render_json(registry),
    }
}

/// JSON 全保真（含 system_prompt）。序列化 5 契约数组。
fn render_json(registry: &NodeRegistry) -> String {
    let contracts = registry.iter();
    serde_json::to_string_pretty(&contracts).unwrap_or_else(|_| "[]".to_string())
}

// 以下三个格式在 Task 3/4/5 实现真实逻辑；此处为编译期桩，保证每一步都编译通过。
fn render_table(_registry: &NodeRegistry) -> String {
    String::new()
}

fn render_dot(_registry: &NodeRegistry) -> String {
    String::new()
}

fn render_mermaid(_registry: &NodeRegistry) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::agent::SubagentLimits;
    use crate::org_graph::NodeRegistry;

    fn registry(readonly: bool) -> NodeRegistry {
        NodeRegistry::builtin(&SubagentLimits {
            explore_readonly: readonly,
            ..Default::default()
        })
    }

    #[test]
    fn render_json_via_dispatch() {
        let r = registry(true);
        let out = render(&r, Format::Json);
        assert!(out.starts_with('['), "json output is an array");
        let parsed: Vec<crate::org_graph::NodeContract> =
            serde_json::from_str(&out).expect("valid json");
        assert_eq!(parsed.len(), 5);
    }

    #[test]
    fn render_json_roundtrips_to_identical_contracts() {
        let r = registry(false);
        let parsed: Vec<crate::org_graph::NodeContract> =
            serde_json::from_str(&render_json(&r)).expect("valid json");
        let original: Vec<crate::org_graph::NodeContract> =
            r.iter().map(|c| c.clone()).collect();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed, original, "serde roundtrip must be field-exact");
    }

    #[test]
    fn render_json_includes_system_prompt() {
        // json 是无损格式；system_prompt 必须保留。
        let out = render_json(&registry(true));
        assert!(out.contains("system_prompt"));
        assert!(out.contains("code exploration subagent"));
    }
}
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib org_graph::render`
预期：3 个测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add src/org_graph/render.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): add render module scaffold + render_json (serde roundtrip)"
```

---

## Task 3: `render_table`（手写表格）

**Files:**
- Modify: `src/org_graph/render.rs`（替换 `render_table` 桩为真实实现；新增两个辅助函数 `fmt_budget` / `truncate_str`；在 `tests` 内新增两个测试）

**Interfaces:**
- Consumes: `registry.iter()`、`NodeContract` 各字段。
- Produces: `render_table` 真实输出（供 `render(_, Format::Table)` 调用）。

- [x] **Step 1: 写失败测试**

在 `src/org_graph/render.rs` 的 `#[cfg(test)] mod tests` 块内追加：

```rust
    #[test]
    fn render_table_has_header_and_five_rows() {
        let out = render_table(&registry(true));
        assert!(out.contains("NODE-TYPE"), "header present");
        assert!(out.contains("NAME"));
        for nt in [
            "Explore",
            "Plan",
            "GeneralPurpose",
            "Verification",
            "WgentyCodeGuide",
        ] {
            assert!(out.contains(nt), "table missing {:?}", nt);
        }
        // 1 表头 + 1 分隔线 + 5 数据行 = 7 行
        assert_eq!(out.lines().count(), 7);
    }

    #[test]
    fn render_table_reflects_explore_readonly() {
        let ro = render_table(&registry(true));
        let rw = render_table(&registry(false));
        let ro_explore = ro.lines().find(|l| l.starts_with("Explore")).unwrap_or("");
        let rw_explore = rw.lines().find(|l| l.starts_with("Explore")).unwrap_or("");
        assert_ne!(ro_explore, rw_explore, "explore row must differ when explore_readonly flips");
        // readonly=true → explore can_mutate_fs=false → 行内含 false（在 MUTATE-FS 列）。
        // 非 readonly → true。两行其余列相同，故差异即 mutate_fs 反映。
    }
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib org_graph::render::tests::render_table`
预期：失败（`render_table` 桩返回空串，断言 `contains("NODE-TYPE")` 与 `lines().count() == 7` 不成立）。

- [x] **Step 3: 写最小实现**

在 `src/org_graph/render.rs` 内：

(a) 替换 `render_table` 桩为真实实现：

```rust
fn render_table(registry: &NodeRegistry) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<18} {:<20} {:<6} {:<11} {:<5} {:<18} {:<10} {}\n",
        "NODE-TYPE", "NAME", "SPAWN", "MUTATE-FS", "EXEC", "IO", "BUDGET", "TOOLS"
    ));
    out.push_str(&format!("{}\n", "-".repeat(100)));
    for c in registry.iter() {
        let io = format!("{:?}→{:?}", c.input_type, c.output_type);
        let tools = if c.capabilities.allowed_tools.is_empty() {
            "*".to_string()
        } else {
            c.capabilities.allowed_tools.join(",")
        };
        out.push_str(&format!(
            "{:<18} {:<20} {:<6} {:<11} {:<5} {:<18} {:<10} {}\n",
            format!("{:?}", c.node_type),
            truncate_str(&c.name, 20),
            c.permissions.can_spawn,
            c.permissions.can_mutate_fs,
            c.permissions.can_exec,
            truncate_str(&io, 18),
            fmt_budget(&c.budget),
            tools
        ));
    }
    out
}
```

(b) 在 `render_table` 之前（紧跟 `render_json` 之后）新增辅助函数：

```rust
/// budget 的紧凑表示：全 None → "all None"，否则 key=val 逗号连接。
fn fmt_budget(b: &crate::org_graph::ResourceBudget) -> String {
    let mut parts = Vec::new();
    if let Some(d) = b.max_depth {
        parts.push(format!("depth={}", d));
    }
    if let Some(c) = b.max_concurrent {
        parts.push(format!("conc={}", c));
    }
    if let Some(t) = b.token_budget_k {
        parts.push(format!("tok={}", t));
    }
    if let Some(r) = b.max_rounds {
        parts.push(format!("rounds={}", r));
    }
    if parts.is_empty() {
        "all None".to_string()
    } else {
        parts.join(",")
    }
}

/// 截断到 max 个字符（内置 name 均为 ASCII，char 计数 ≈ 显示宽度）。
fn truncate_str(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib org_graph::render`
预期：Task 2 的 3 个 + Task 3 的 2 个，共 5 个测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add src/org_graph/render.rs
git commit -m "feat(org-graph): implement render_table with manual column layout"
```

---

## Task 4: `render_dot`（Graphviz，扁平 + 视觉编码）

**Files:**
- Modify: `src/org_graph/render.rs`（替换 `render_dot` 桩；新增 `dot_node_id` 辅助；在 `tests` 内新增两个测试）

**Interfaces:**
- Produces: `render_dot` 真实输出。

- [x] **Step 1: 写失败测试**

在 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn render_dot_is_well_formed_with_five_nodes() {
        let out = render_dot(&registry(true));
        assert!(
            out.starts_with("digraph org_graph_contract {"),
            "must start with digraph header"
        );
        assert!(out.trim_end().ends_with('}'), "must close brace");
        // 5 个节点声明（每行一个 `];`）。
        let node_lines = out
            .lines()
            .filter(|l| l.contains("[label=") && l.contains("];"))
            .count();
        assert_eq!(node_lines, 5, "expected 5 node declarations");
    }

    #[test]
    fn render_dot_encodes_explore_readonly_as_fillcolor() {
        let ro = render_dot(&registry(true));
        let rw = render_dot(&registry(false));
        let ro_explore = ro.lines().find(|l| l.contains("explore [")).unwrap_or("");
        let rw_explore = rw.lines().find(|l| l.contains("explore [")).unwrap_or("");
        // readonly=true → can_mutate_fs=false → fillcolor=white
        assert!(ro_explore.contains("fillcolor=white"), "ro explore should be white");
        // readonly=false → can_mutate_fs=true → lightyellow
        assert!(
            rw_explore.contains("fillcolor=lightyellow"),
            "rw explore should be lightyellow"
        );
    }
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib org_graph::render::tests::render_dot`
预期：失败（桩返回空串，`starts_with("digraph ...")` 不成立）。

- [x] **Step 3: 写最小实现**

(a) 替换 `render_dot` 桩：

```rust
fn render_dot(registry: &NodeRegistry) -> String {
    let mut out = String::new();
    out.push_str("digraph org_graph_contract {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [fontname=\"Helvetica\"];\n");
    for c in registry.iter() {
        // 视觉编码：can_spawn → 形状；can_mutate_fs → 填充色。
        let shape = if c.permissions.can_spawn {
            "component"
        } else {
            "box"
        };
        let fill = if c.permissions.can_mutate_fs {
            "lightyellow"
        } else {
            "white"
        };
        let label = format_dot_label(c);
        out.push_str(&format!(
            "    {} [label=\"{}\", shape={}, style=filled, fillcolor={}];\n",
            dot_node_id(c),
            label,
            shape,
            fill
        ));
    }
    out.push_str("}\n");
    out
}

/// DOT 标识符不允许连字符；用 NodeType 的 Debug 小写形式（纯字母）。
fn dot_node_id(c: &crate::org_graph::NodeContract) -> String {
    format!("{:?}", c.node_type).to_lowercase()
}

/// record 式多行 label（\\l = 左对齐换行）。含全部五维，省略 system_prompt。
fn format_dot_label(c: &crate::org_graph::NodeContract) -> String {
    let tools = if c.capabilities.allowed_tools.is_empty() {
        "*".to_string()
    } else {
        c.capabilities.allowed_tools.join(",")
    };
    format!(
        "name={}\\ltype={:?}\\lspawn={}\\lmutate_fs={}\\lexec={}\\lIO={:?}→{:?}\\lbudget={}\\ltools={}\\l",
        c.name,
        c.node_type,
        c.permissions.can_spawn,
        c.permissions.can_mutate_fs,
        c.permissions.can_exec,
        c.input_type,
        c.output_type,
        fmt_budget(&c.budget),
        tools
    )
}
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib org_graph::render`
预期：7 个测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add src/org_graph/render.rs
git commit -m "feat(org-graph): implement render_dot (flat nodes + permission visual encoding)"
```

---

## Task 5: `render_mermaid`（flowchart + classDef）

**Files:**
- Modify: `src/org_graph/render.rs`（替换 `render_mermaid` 桩；新增 `mermaid_node_id` / `mermaid_class` / `format_mermaid_label` 辅助；在 `tests` 内新增两个测试）

**Interfaces:**
- Produces: `render_mermaid` 真实输出。

- [x] **Step 1: 写失败测试**

在 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn render_mermaid_is_well_formed_with_five_nodes() {
        let out = render_mermaid(&registry(true));
        assert!(
            out.starts_with("flowchart LR"),
            "must start with flowchart declaration"
        );
        // 5 个节点定义（每行一个 `:::`）。
        let node_lines = out
            .lines()
            .filter(|l| l.contains("[\"") && l.contains(":::"))
            .count();
        assert_eq!(node_lines, 5, "expected 5 node definitions");
        // classDef 声明存在。
        assert!(out.contains("classDef readonly"));
        assert!(out.contains("classDef spawn"));
        assert!(out.contains("classDef mutate"));
    }

    #[test]
    fn render_mermaid_encodes_explore_readonly_as_class() {
        let ro = render_mermaid(&registry(true));
        let rw = render_mermaid(&registry(false));
        let ro_explore = ro.lines().find(|l| l.contains("explore[")).unwrap_or("");
        let rw_explore = rw.lines().find(|l| l.contains("explore[")).unwrap_or("");
        // readonly=true → can_mutate_fs=false 且 can_spawn=false → readonly 类
        assert!(ro_explore.contains(":::readonly"), "ro explore should be readonly class");
        // readonly=false → can_mutate_fs=true（仍 can_spawn=false）→ mutate 类
        assert!(rw_explore.contains(":::mutate"), "rw explore should be mutate class");
    }
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib org_graph::render::tests::render_mermaid`
预期：失败（桩返回空串，`starts_with("flowchart LR")` 不成立）。

- [x] **Step 3: 写最小实现**

(a) 替换 `render_mermaid` 桩：

```rust
fn render_mermaid(registry: &NodeRegistry) -> String {
    let mut out = String::new();
    out.push_str("flowchart LR\n");
    for c in registry.iter() {
        let label = format_mermaid_label(c);
        out.push_str(&format!(
            "    {}[\"{}\"]:::{}\n",
            mermaid_node_id(c),
            label,
            mermaid_class(c)
        ));
    }
    out.push_str("    classDef readonly fill:#fff,stroke:#999;\n");
    out.push_str("    classDef spawn fill:#e8f5e9,stroke:#2e7d32;\n");
    out.push_str("    classDef mutate fill:#fff8e1,stroke:#f57c00;\n");
    out
}

/// mermaid 节点 ID（与 dot_node_id 一致，纯字母）。
fn mermaid_node_id(c: &crate::org_graph::NodeContract) -> String {
    format!("{:?}", c.node_type).to_lowercase()
}

/// 优先级：can_spawn → spawn；否则 can_mutate_fs → mutate；否则 readonly。
fn mermaid_class(c: &crate::org_graph::NodeContract) -> &'static str {
    if c.permissions.can_spawn {
        "spawn"
    } else if c.permissions.can_mutate_fs {
        "mutate"
    } else {
        "readonly"
    }
}

/// `<br/>` 分行的节点卡标签。含全部五维，省略 system_prompt。
fn format_mermaid_label(c: &crate::org_graph::NodeContract) -> String {
    let tools = if c.capabilities.allowed_tools.is_empty() {
        "*".to_string()
    } else {
        c.capabilities.allowed_tools.join(",")
    };
    format!(
        "{}<br/>type={:?}<br/>spawn={}<br/>mutate_fs={}<br/>exec={}<br/>IO={:?}→{:?}<br/>budget={}<br/>tools={}",
        c.name,
        c.node_type,
        c.permissions.can_spawn,
        c.permissions.can_mutate_fs,
        c.permissions.can_exec,
        c.input_type,
        c.output_type,
        fmt_budget(&c.budget),
        tools
    )
}
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib org_graph::render`
预期：9 个测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add src/org_graph/render.rs
git commit -m "feat(org-graph): implement render_mermaid (flowchart + classDef encoding)"
```

---

## Task 6: CLI 接线（`org-graph contracts` 命令）

**Files:**
- Create: `src/cli/org_graph.rs`
- Modify: `src/cli/mod.rs`（声明 `pub mod org_graph;`；`Commands` 新增 `OrgGraph` 变体；新增 `OrgGraphCommands` 枚举）
- Modify: `src/cli/args.rs`（`run_async` 新增 `OrgGraph` match arm）

**Interfaces:**
- Consumes: `crate::org_graph::render::{Format, render}`、`crate::org_graph::NodeRegistry::builtin`、`state.settings.agent.subagent`（`SubagentLimits`）。
- Produces: 顶层命令 `wgenty-code org-graph contracts [--format table|dot|mermaid|json]`，默认 `table`。

- [x] **Step 1: 写失败测试**

新建 `src/cli/org_graph.rs`，先写含测试的完整文件（Step 3）。测试意图：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_arg_maps_to_render_format() {
        use crate::org_graph::render::Format;
        assert_eq!(Format::from(OrgGraphFormatArg::Table), Format::Table);
        assert_eq!(Format::from(OrgGraphFormatArg::Dot), Format::Dot);
        assert_eq!(Format::from(OrgGraphFormatArg::Mermaid), Format::Mermaid);
        assert_eq!(Format::from(OrgGraphFormatArg::Json), Format::Json);
    }
}
```

- [x] **Step 2: 运行测试确认失败**

运行：`cargo test --lib cli::org_graph`
预期：失败（`error[E0432|E0433]: unresolved import / could not find org_graph in cli`）。

- [x] **Step 3: 写最小实现**

(a) 新建 `src/cli/org_graph.rs`：

```rust
//! `wgenty-code org-graph` — 审计内置 node-contract 注册表（纯只读）。

use clap::ValueEnum;

use crate::org_graph::NodeRegistry;
use crate::org_graph::render::Format;

/// CLI 侧格式参数（clap ValueEnum）。映射到 org_graph::render::Format，
/// 使 org_graph 模块本身保持零 clap 依赖。
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OrgGraphFormatArg {
    /// 人类可读表格（默认）
    Table,
    /// Graphviz DOT
    Dot,
    /// Mermaid flowchart
    Mermaid,
    /// 无损 JSON（含 system_prompt）
    Json,
}

impl From<OrgGraphFormatArg> for Format {
    fn from(arg: OrgGraphFormatArg) -> Self {
        match arg {
            OrgGraphFormatArg::Table => Format::Table,
            OrgGraphFormatArg::Dot => Format::Dot,
            OrgGraphFormatArg::Mermaid => Format::Mermaid,
            OrgGraphFormatArg::Json => Format::Json,
        }
    }
}

/// `org-graph` 子命令派发入口。
pub async fn run(
    state: &crate::state::AppState,
    action: &super::OrgGraphCommands,
) -> anyhow::Result<()> {
    match action {
        super::OrgGraphCommands::Contracts { format } => {
            // SubagentLimits 驱动 explore/plan 的 can_mutate_fs（explore_readonly）。
            let registry = NodeRegistry::builtin(&state.settings.agent.subagent);
            let out = crate::org_graph::render::render(&registry, (*format).into());
            print!("{}", out);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_arg_maps_to_render_format() {
        use crate::org_graph::render::Format;
        assert_eq!(Format::from(OrgGraphFormatArg::Table), Format::Table);
        assert_eq!(Format::from(OrgGraphFormatArg::Dot), Format::Dot);
        assert_eq!(Format::from(OrgGraphFormatArg::Mermaid), Format::Mermaid);
        assert_eq!(Format::from(OrgGraphFormatArg::Json), Format::Json);
    }
}
```

(b) 修改 `src/cli/mod.rs`：在顶部 `pub mod subagent;` 之后新增：

```rust
pub mod org_graph;
```

在 `pub enum Commands {` 内，最后一个变体 `Subagent { ... }` 之后、闭花括号之前，新增变体：

```rust

    /// Inspect the Org-Graph node-contract registry
    OrgGraph {
        #[command(subcommand)]
        action: OrgGraphCommands,
    },
```

在 `Commands` 枚举之后（`SubagentCommands` 枚举之后即可）新增枚举：

```rust
#[derive(Subcommand, Debug)]
pub enum OrgGraphCommands {
    /// Render the built-in node contracts.
    ///
    /// Visual formats (table / dot / mermaid) omit the long `system_prompt`
    /// field for readability; use `--format json` for the lossless source.
    Contracts {
        /// Output format (table | dot | mermaid | json)
        #[arg(
            long,
            value_enum,
            default_value_t = crate::cli::org_graph::OrgGraphFormatArg::Table
        )]
        format: crate::cli::org_graph::OrgGraphFormatArg,
    },
}
```

(c) 修改 `src/cli/args.rs`：在 `run_async` 的 `match &self.command {` 内，`Some(super::Commands::Subagent { action })` 分支之后，新增：

```rust
            Some(super::Commands::OrgGraph { action }) => {
                super::org_graph::run(&state, action).await?;
            }
```

- [x] **Step 4: 运行测试确认通过**

运行：`cargo test --lib cli::org_graph`
预期：`format_arg_maps_to_render_format` PASS。

运行：`cargo build`
预期：编译成功（验证 `Commands::OrgGraph` 变体与 `run_async` 接线无误）。

- [x] **Step 5: 提交**

```bash
git add src/cli/org_graph.rs src/cli/mod.rs src/cli/args.rs
git commit -m "feat(cli): add `org-graph contracts` command wiring render module"
```

---

## Task 7: 全量构建 / 测试 + 四格式手动验证

**Files:**
- 无源码改动；仅验证。

**Interfaces:**
- Consumes: Task 1–6 全部产出。

- [x] **Step 1: 全量测试**

运行：`cargo test`
预期：全部 PASS（含 `org_graph::registry`、`org_graph::render`、`org_graph::contract`、`cli::org_graph`、以及既有全部测试零回归）。

- [x] **Step 2: 全量构建**

运行：`cargo build`
预期：编译成功，无 warning 与本 change 相关。

- [x] **Step 3: 手动验证 —— table（默认）**

运行：`cargo run -- org-graph contracts`
预期：stdout 打印 7 行（表头 + 分隔线 + 5 契约行），列含 NODE-TYPE / NAME / SPAWN / MUTATE-FS / EXEC / IO / BUDGET / TOOLS；GeneralPurpose 的 SPAWN=true，其余=false。

- [x] **Step 4: 手动验证 —— json（无损）**

运行：`cargo run -- org-graph contracts --format json`
预期：stdout 打印合法 JSON 数组（5 元素），每个对象含 `system_prompt` 字段；explore 对象的 `system_prompt` 含 "code exploration subagent"。

- [x] **Step 5: 手动验证 —— dot / mermaid**

运行：`cargo run -- org-graph contracts --format dot`
预期：以 `digraph org_graph_contract {` 开头、`}` 结尾，含 5 个 `[label=..., shape=..., style=filled, fillcolor=...]` 节点声明。

运行：`cargo run -- org-graph contracts --format mermaid`
预期：以 `flowchart LR` 开头，含 5 个 `["..."]:::<class>` 节点定义与 3 个 `classDef` 声明。

- [x] **Step 6（可选，CI 有 graphviz 时）: dot 冒烟解析** — 已验证（VERIFIED）：graphviz 15.1.1 安装后补跑，`cargo run --quiet -- org-graph contracts --format dot | dot -Tsvg -o /tmp/org-graph-contracts.svg` 退出码 0，生成 7520 字节合法 SVG（`<?xml ...?>` 头），SVG 内含全部 5 个节点 id（explore/plan/generalpurpose/verification/wgentycodeguide）。证明 render_dot 输出不仅结构合法，且能被真实 Graphviz 引擎渲染。

运行：`cargo run -- org-graph contracts --format dot | dot -Tsvg -o /tmp/org-graph-contracts.svg`
预期：若系统装有 graphviz，`dot` 成功解析生成 SVG，退出码 0；无 graphviz 时跳过（不阻塞本任务，仅作冒烟）。

- [x] **Step 7: 验证 explore_readonly 配置驱动**

运行：`cargo run -- org-graph contracts --format dot`（默认配置）观察 explore 节点 `fillcolor=`，与修改 `settings.json` 的 `agent.subagent.explore_readonly` 后的输出对比（若当前环境不便改配置，则依赖 Task 4 单测已覆盖该维度）。

- [x] **Step 8: 提交验证证据（无源码改动则跳过 commit）**

本任务无源码改动；若手动验证中发现问题需修补，回到对应 Task 的 TDD 循环。验证全过后，本 change 的 build 阶段产物完成，转入 build 阶段退出条件（guard）。

---

## Self-Review

**1. Spec 覆盖：** 对照 OpenSpec delta spec 的 5 个验收场景——(a) 默认 table 列 5 维 → Task 3 + Task 7 Step 3；(b) `--format dot` 合法 DOT → Task 4 + Task 7 Step 5；(c) `--format mermaid` 合法 mermaid → Task 5 + Task 7 Step 5；(d) `--format json` 可反序列化 → Task 2 + Task 7 Step 4；(e) explore_readonly 驱动 can_mutate_fs → Task 3/4/5 各格式测试 + Task 7 Step 7。全部覆盖。

**2. 占位符扫描：** Task 2/3/4/5 的 `render_table/dot/mermaid` 桩是显式编译期过渡（Task 2 标注并由 Task 3/4/5 替换为真实代码），非 "TBD/TODO"。所有步骤均含可执行代码或命令。

**3. 类型一致性：** `NodeRegistry::iter()` 签名（Task 1）在 Task 2–6 一致使用；`Format` 四变体在 `render` 派发（Task 2）、`OrgGraphFormatArg`（Task 6）映射中名称一致；`state.settings.agent.subagent` 路径与 `config/mod.rs:33` + `config/agent.rs:252` 一致；`dot_node_id` / `mermaid_node_id` 均用 `format!("{:?}", c.node_type).to_lowercase()`。

---

## 执行交接

计划已完成并保存至 `docs/superpowers/plans/2026-08-11-org-graph-contract-viewer.md`。执行方式按 Comet build 阶段 Step 2 的联合决策确定（executing-plans 或 subagent-driven-development）。
