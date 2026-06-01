//! Guardian Module — Security review for high-risk tool calls.
//!
//! Two-tier review:
//!   1. Rule-based: fast pattern matching for known-dangerous commands
//!   2. LLM review (optional): for medium/high-risk commands, call a review LLM

use serde::{Deserialize, Serialize};

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
const CRITICAL_PATTERNS: &[&str] = &[
    "rm -rf / ",
    "mkfs.",
    "dd if=",
    "> /dev/sda",
    ":(){ :|:& };:", // fork bomb
    "chmod 777 /",
];

const HIGH_PATTERNS: &[&str] = &[
    "sudo ",
    "rm -rf",
    "git push --force",
    "git reset --hard",
    "DROP TABLE",
    "DELETE FROM",
    "shutdown",
    "reboot",
    "chmod -R 777",
    "chown -R",
    "> /etc/",
    "curl | bash",
    "wget -O - | sh",
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

/// Classify the risk level of a shell command.
pub fn classify_risk(command: &str) -> RiskLevel {
    let cmd_lower = command.to_lowercase();

    for pattern in CRITICAL_PATTERNS {
        if cmd_lower.contains(&pattern.to_lowercase()) {
            return RiskLevel::Critical;
        }
    }

    for pattern in HIGH_PATTERNS {
        if cmd_lower.contains(&pattern.to_lowercase()) {
            return RiskLevel::High;
        }
    }

    for pattern in MEDIUM_PATTERNS {
        if cmd_lower.contains(&pattern.to_lowercase()) {
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
            decision.rationale = format!(
                "AUTO-DENIED: {}",
                decision.rationale
            );
        }

        decision
    }
}

impl Default for Guardian {
    fn default() -> Self {
        Self::new(GuardianConfig::default())
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
        assert_eq!(classify_risk("rm -rf / --no-preserve-root"), RiskLevel::Critical);
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
