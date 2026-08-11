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

/// budget 的紧凑表示：全 None -> "all None"，否则 key=val 逗号连接。
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

fn render_table(registry: &NodeRegistry) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<18} {:<20} {:<6} {:<11} {:<5} {:<18} {:<10} {}\n",
        "NODE-TYPE", "NAME", "SPAWN", "MUTATE-FS", "EXEC", "IO", "BUDGET", "TOOLS"
    ));
    out.push_str(&format!("{}\n", "-".repeat(100)));
    for c in registry.iter() {
        let io = format!("{:?}->{:?}", c.input_type, c.output_type);
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

fn render_dot(registry: &NodeRegistry) -> String {
    let mut out = String::new();
    out.push_str("digraph org_graph_contract {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [fontname=\"Helvetica\"];\n");
    for c in registry.iter() {
        // 视觉编码：can_spawn -> 形状；can_mutate_fs -> 填充色。
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
        "name={}\\ltype={:?}\\lspawn={}\\lmutate_fs={}\\lexec={}\\lIO={:?}->{:?}\\lbudget={}\\ltools={}\\l",
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

/// 优先级：can_spawn -> spawn；否则 can_mutate_fs -> mutate；否则 readonly。
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
        "{}<br/>type={:?}<br/>spawn={}<br/>mutate_fs={}<br/>exec={}<br/>IO={:?}->{:?}<br/>budget={}<br/>tools={}",
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
            r.iter().into_iter().map(|c| c.clone()).collect();
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
        // readonly=true -> explore can_mutate_fs=false -> 行内含 false（在 MUTATE-FS 列）。
        // 非 readonly -> true。两行其余列相同，故差异即 mutate_fs 反映。
    }

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
        // readonly=true -> can_mutate_fs=false -> fillcolor=white
        assert!(ro_explore.contains("fillcolor=white"), "ro explore should be white");
        // readonly=false -> can_mutate_fs=true -> lightyellow
        assert!(
            rw_explore.contains("fillcolor=lightyellow"),
            "rw explore should be lightyellow"
        );
    }

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
        // readonly=true -> can_mutate_fs=false 且 can_spawn=false -> readonly 类
        assert!(ro_explore.contains(":::readonly"), "ro explore should be readonly class");
        // readonly=false -> can_mutate_fs=true（仍 can_spawn=false）-> mutate 类
        assert!(rw_explore.contains(":::mutate"), "rw explore should be mutate class");
    }
}
