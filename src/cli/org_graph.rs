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
