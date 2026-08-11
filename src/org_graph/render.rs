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
}
