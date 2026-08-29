//! Guardian Module — Security review for high-risk tool calls.
//!
//! Two-tier review:
//!   1. Rule-based: fast pattern matching for known-dangerous commands
//!   2. LLM review (optional): for medium/high-risk commands, call a review LLM

use serde::{Deserialize, Serialize};

/// Extract the shell text a command-execution tool will run, so every
/// guardian integration point reviews the same set of tools:
/// `command`-style tools (`execute_command`/`exec_command`/`background`) and
/// stdin injection into an already-running session (`write_stdin`).
pub fn shell_text_for_tool<'a>(tool_name: &str, args: &'a serde_json::Value) -> Option<&'a str> {
    match tool_name {
        "execute_command" | "exec_command" | "background" => args.get("command")?.as_str(),
        "write_stdin" => {
            let chars = args.get("chars")?.as_str()?;
            (!chars.is_empty()).then_some(chars)
        }
        _ => None,
    }
}

/// Risk level for a proposed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Outcome of a security review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianDecision {
    pub allowed: bool,
    pub risk_level: RiskLevel,
    pub rationale: String,
    pub requires_approval: bool,
}

/// Patterns that indicate elevated risk.
/// Note: critical patterns must be specific to avoid false positives on
/// legitimate operations like `rm -rf /tmp`.
const CRITICAL_PATTERNS: &[&str] = &[
    "rm -rf / ", // rm -rf / (with trailing space)
    "rm -rf /*", // rm -rf /* (glob)
    "rm -rf /$", // rm -rf / at end of string
    "rm -rf --no-preserve-root",
    "mkfs.",
    "dd if=",
    "> /dev/sda",
    ":(){ :|:& };:", // fork bomb
    "chmod 777 /",
];

const HIGH_PATTERNS: &[&str] = &[
    "sudo ",
    "sudo\t",
    "rm -rf",
    "rm -fr",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "drop table",
    "delete from",
    "shutdown",
    "reboot",
    "chmod -r 777",
    "chown -r",
    "> /etc/",
    "curl | bash",
    "curl | sh",
    "wget -o - | sh",
    "wget | bash",
];

const MEDIUM_PATTERNS: &[&str] = &[
    "npm publish",
    "cargo publish",
    "docker rm",
    "docker rmi",
    "kubectl delete",
    "git push",
    "pip uninstall",
    "npm uninstall",
];

/// Detect command substitution patterns that could hide dangerous commands.
const SUBSTITUTION_INDICATORS: &[&str] = &["$(", "`"];

/// Check if `command` contains `pattern` as a word-boundary match.
/// This prevents "rm -rf /" from matching "rm -rf /tmp".
fn matches_pattern(command: &str, pattern: &str) -> bool {
    let pat_lower = pattern.to_lowercase();
    let cmd_lower = command.to_lowercase();

    if !cmd_lower.contains(&pat_lower) {
        return false;
    }

    // For patterns ending with '/' (like "rm -rf /"), ensure the next
    // character is whitespace, end-of-string, or a flag (starts with '-'),
    // NOT a path component (which would mean a subdirectory).
    if pat_lower.ends_with('/') {
        if let Some(pos) = cmd_lower.find(&pat_lower) {
            let after_pos = pos + pat_lower.len();
            if after_pos < cmd_lower.len() {
                let next_char = cmd_lower.as_bytes()[after_pos];
                // Allow: space, tab, newline, '-', end of string
                // Reject: alphanumeric (indicates a subpath like /tmp)
                if next_char.is_ascii_alphanumeric() || next_char == b'_' {
                    return false;
                }
            }
        }
    }

    true
}

/// Fork bombs appear with and without spaces (`:(){ :|:& };:` vs
/// `:(){:|:&};:`); collapse whitespace before matching either form.
fn is_fork_bomb(command: &str) -> bool {
    let collapsed: String = command
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    collapsed.contains(":(){:|:&};:")
}

/// Detect `rm` with recursive flags (in any order or combined form) whose
/// target is the filesystem root. Catches variants the literal patterns miss,
/// e.g. `rm -r -f /`, `rm --recursive -f /*`, `rm -fr / --no-preserve-root`.
fn is_destructive_rm(command: &str) -> bool {
    let lowered = command.to_lowercase();
    let tokens: Vec<&str> = lowered.split_whitespace().collect();

    for (i, token) in tokens.iter().enumerate() {
        // Basename only for the command token (`/usr/bin/rm` → `rm`);
        // target tokens must stay intact so `/` and `/*` are recognisable.
        let base = token.rsplit('/').next().unwrap_or(token);
        if base != "rm" {
            continue;
        }
        let mut recursive = false;
        let mut root_target = false;
        let mut preserve_root = false;
        for t in &tokens[i + 1..] {
            match *t {
                "--no-preserve-root" => preserve_root = true,
                "--recursive" => recursive = true,
                other if other.starts_with("--") => {}
                other if other.starts_with('-') && other.len() > 1 => {
                    // Short-flag cluster: -r, -f, -rf, -fr, -R (lowercased).
                    if other.contains('r') {
                        recursive = true;
                    }
                }
                // First non-flag token is the (only) target examined.
                "/" | "/*" | "/*/" => {
                    root_target = true;
                    break;
                }
                _ => break,
            }
        }
        if (recursive && root_target) || preserve_root {
            return true;
        }
    }
    false
}

/// Classify the risk level of a shell command.
pub fn classify_risk(command: &str) -> RiskLevel {
    let cmd_lower = command.to_lowercase();

    if is_fork_bomb(command) || is_destructive_rm(command) {
        return RiskLevel::Critical;
    }

    // Check for command substitution that could hide dangerous operations
    for indicator in SUBSTITUTION_INDICATORS {
        if cmd_lower.contains(indicator) {
            for pattern in HIGH_PATTERNS.iter().chain(CRITICAL_PATTERNS.iter()) {
                let pattern_trimmed = pattern.trim();
                if matches_pattern(&cmd_lower, pattern_trimmed) {
                    return RiskLevel::High;
                }
            }
        }
    }

    for pattern in CRITICAL_PATTERNS {
        if matches_pattern(&cmd_lower, pattern) {
            return RiskLevel::Critical;
        }
    }

    for pattern in HIGH_PATTERNS {
        if matches_pattern(&cmd_lower, pattern) {
            return RiskLevel::High;
        }
    }

    for pattern in MEDIUM_PATTERNS {
        if matches_pattern(&cmd_lower, pattern) {
            return RiskLevel::Medium;
        }
    }

    RiskLevel::Low
}

/// Perform a rule-based security review of a command.
/// Returns a GuardianDecision without calling an LLM.
pub fn review_command(command: &str, tool_name: &str) -> GuardianDecision {
    let risk = classify_risk(command);

    match risk {
        RiskLevel::Low => GuardianDecision {
            allowed: true,
            risk_level: risk,
            rationale: "Low-risk operation, no dangerous patterns detected.".to_string(),
            requires_approval: false,
        },
        RiskLevel::Medium => GuardianDecision {
            allowed: true,
            risk_level: risk,
            rationale: format!(
                "Medium-risk operation detected in {tool_name}. Pattern matches known caution patterns. Proceeding with approval flag."
            ),
            requires_approval: true,
        },
        RiskLevel::High => GuardianDecision {
            allowed: true,
            risk_level: risk,
            rationale: format!(
                "HIGH-RISK operation in {tool_name}: '{command}'. This command matches destructive patterns. Strongly recommend manual review."
            ),
            requires_approval: true,
        },
        RiskLevel::Critical => GuardianDecision {
            allowed: false,
            risk_level: risk,
            rationale: format!(
                "CRITICAL-RISK operation blocked in {tool_name}: '{command}'. This command matches potentially irreversible destructive patterns."
            ),
            requires_approval: true,
        },
    }
}

/// Guardian configuration.
#[derive(Debug, Clone)]
pub struct GuardianConfig {
    /// Enable the guardian module.
    pub enabled: bool,
    /// Enable LLM-based review for medium+ risk commands.
    pub llm_review: bool,
    /// Auto-deny critical-risk commands.
    pub auto_deny_critical: bool,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            llm_review: false,
            auto_deny_critical: true,
        }
    }
}

/// Guardian instance for checking tool executions.
#[derive(Debug, Clone)]
pub struct Guardian {
    config: GuardianConfig,
}

impl Guardian {
    pub fn new(config: GuardianConfig) -> Self {
        Self { config }
    }

    /// Review a tool execution. Returns a decision.
    pub fn check(&self, tool_name: &str, command: &str) -> GuardianDecision {
        if !self.config.enabled {
            return GuardianDecision {
                allowed: true,
                risk_level: RiskLevel::Low,
                rationale: "Guardian is disabled.".to_string(),
                requires_approval: false,
            };
        }

        let mut decision = review_command(command, tool_name);

        // Override for critical-risk: auto-deny if configured
        if self.config.auto_deny_critical && decision.risk_level >= RiskLevel::Critical {
            decision.allowed = false;
            decision.rationale = format!("AUTO-DENIED: {}", decision.rationale);
        }

        decision
    }
}

impl Default for Guardian {
    fn default() -> Self {
        Self::new(GuardianConfig::default())
    }
}

impl From<&crate::config::GuardianSettings> for GuardianConfig {
    fn from(settings: &crate::config::GuardianSettings) -> Self {
        Self {
            enabled: settings.enabled,
            llm_review: settings.llm_review,
            auto_deny_critical: settings.auto_deny_critical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_safe_command() {
        assert_eq!(classify_risk("ls -la"), RiskLevel::Low);
        assert_eq!(classify_risk("cargo build"), RiskLevel::Low);
        assert_eq!(classify_risk("rg pattern src/"), RiskLevel::Low);
    }

    #[test]
    fn test_classify_medium_risk() {
        assert_eq!(classify_risk("git push"), RiskLevel::Medium);
        assert_eq!(classify_risk("npm publish"), RiskLevel::Medium);
    }

    #[test]
    fn test_classify_high_risk() {
        assert_eq!(classify_risk("sudo rm -rf /tmp"), RiskLevel::High);
        assert_eq!(classify_risk("rm -rf node_modules"), RiskLevel::High);
    }

    #[test]
    fn test_classify_critical_risk() {
        assert_eq!(
            classify_risk("rm -rf / --no-preserve-root"),
            RiskLevel::Critical
        );
    }

    #[test]
    fn test_classify_critical_rm_flag_order_variants() {
        // Literal patterns miss these; the token scanner must catch them.
        assert_eq!(classify_risk("rm -r -f /"), RiskLevel::Critical);
        assert_eq!(classify_risk("rm -fr /"), RiskLevel::Critical);
        assert_eq!(classify_risk("rm --recursive -f /*"), RiskLevel::Critical);
        assert_eq!(classify_risk("/usr/bin/rm -r -f /"), RiskLevel::Critical);
        assert_eq!(
            classify_risk("rm -rf / --no-preserve-root"),
            RiskLevel::Critical
        );
        // Recursive rm on a real subpath stays High (destructive but targeted).
        assert_eq!(classify_risk("rm -rf /tmp/x"), RiskLevel::High);
    }

    #[test]
    fn test_classify_fork_bomb_spacing_variants() {
        assert_eq!(classify_risk(":(){ :|:& };:"), RiskLevel::Critical);
        assert_eq!(classify_risk(":(){:|:&};:"), RiskLevel::Critical);
        assert_eq!(classify_risk(":(){  :|:&  };:"), RiskLevel::Critical);
    }

    #[test]
    fn test_guardian_settings_wired_into_config() {
        let settings = crate::config::GuardianSettings {
            enabled: false,
            llm_review: false,
            auto_deny_critical: false,
        };
        let guardian = Guardian::new(GuardianConfig::from(&settings));
        let decision = guardian.check("execute_command", "rm -r -f /");
        assert!(decision.allowed, "disabled guardian must allow");
    }

    #[test]
    fn test_guardian_auto_deny_critical() {
        let guardian = Guardian::new(GuardianConfig {
            enabled: true,
            llm_review: false,
            auto_deny_critical: true,
        });
        let decision = guardian.check("execute_command", "rm -rf / --no-preserve-root");
        assert!(!decision.allowed);
        assert_eq!(decision.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_guardian_disabled() {
        let guardian = Guardian::new(GuardianConfig {
            enabled: false,
            ..Default::default()
        });
        let decision = guardian.check("execute_command", "rm -rf / --no-preserve-root");
        assert!(decision.allowed);
    }
}
