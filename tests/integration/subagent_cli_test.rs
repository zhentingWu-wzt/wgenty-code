//! `wgenty-code subagent` subcommand — offline, read-only inspection of
//! subagent transcripts (list / trace / health). Reads the SQLite transcript
//! store directly; does not start an agent.

#[cfg(test)]
mod tests {
    use clap::Parser;
    use wgenty_code::cli::Cli;

    // ─── 4.1 parse tests ────────────────────────────────────────────────

    #[test]
    fn parses_subagent_list_defaults() {
        let cli = Cli::try_parse_from(vec!["wgenty-code", "subagent", "list"]);
        assert!(cli.is_ok(), "list should parse: {:?}", cli.err());
    }

    #[test]
    fn parses_subagent_list_with_filters() {
        let cli = Cli::try_parse_from(vec![
            "wgenty-code",
            "subagent",
            "list",
            "--session",
            "sess-1",
            "--status",
            "failed",
            "--limit",
            "10",
        ]);
        assert!(cli.is_ok(), "list+filters should parse: {:?}", cli.err());
    }

    #[test]
    fn parses_subagent_trace_with_format_and_raw() {
        let cli = Cli::try_parse_from(vec![
            "wgenty-code",
            "subagent",
            "trace",
            "abc-123",
            "--format",
            "html",
        ]);
        assert!(cli.is_ok(), "trace --format html: {:?}", cli.err());

        let cli = Cli::try_parse_from(vec!["wgenty-code", "subagent", "trace", "abc-123", "--raw"]);
        assert!(cli.is_ok(), "trace --raw: {:?}", cli.err());
    }

    #[test]
    fn parses_subagent_health_with_period() {
        let cli = Cli::try_parse_from(vec!["wgenty-code", "subagent", "health", "--period", "7d"]);
        assert!(cli.is_ok(), "health --period 7d: {:?}", cli.err());
    }

    #[test]
    fn rejects_unknown_format() {
        let cli = Cli::try_parse_from(vec![
            "wgenty-code",
            "subagent",
            "trace",
            "abc",
            "--format",
            "bogus",
        ]);
        assert!(cli.is_err(), "unknown --format must be rejected");
    }

    #[test]
    fn rejects_unknown_period() {
        let cli = Cli::try_parse_from(vec![
            "wgenty-code",
            "subagent",
            "health",
            "--period",
            "bogus",
        ]);
        assert!(cli.is_err(), "unknown --period must be rejected");
    }
}
