//! Test Output Parser — multi-framework test result parsing.
//!
//! Parses raw stdout/stderr from test runners into a unified TestOutput struct.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default)]
pub struct TestOutput {
    pub success: bool,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub summary: String,
    pub failures: Vec<String>,
}

impl TestOutput {
    /// Parse test output based on framework name.
    pub fn parse(framework: &str, stdout: &str, stderr: &str, exit_code: i32) -> Self {
        let combined = format!("{}\n{}", stdout, stderr);

        match framework {
            "rust-cargo" => Self::parse_cargo(&combined, exit_code),
            "node-jest" | "node-vitest" => Self::parse_jest(&combined, exit_code),
            "node-npm" => Self::parse_jest(&combined, exit_code),
            "python-pytest" => Self::parse_pytest(&combined, exit_code),
            "python-unittest" => Self::parse_unittest(&combined, exit_code),
            "go" => Self::parse_go(&combined, exit_code),
            _ => Self::parse_generic(&combined, exit_code),
        }
    }

    /// Parse `cargo test` output.
    fn parse_cargo(output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;

        // Extract "test result: ok. 12 passed; 0 failed; 2 ignored"
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"test result:\s*(\w+)\.\s*(\d+) passed;\s*(\d+) failed;\s*(\d+) ignored")
                .unwrap()
        });

        let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
        let summary;

        if let Some(caps) = RE.captures(output) {
            passed = caps[2].parse().unwrap_or(0);
            failed = caps[3].parse().unwrap_or(0);
            skipped = caps[4].parse().unwrap_or(0);
            summary = caps[0].to_string();
        } else if !success {
            summary = "Test compilation or execution failed.".to_string();
            failed = 1; // Mark as failed if we can't parse
        } else {
            summary = "Tests completed.".to_string();
            passed = 1;
        }

        // Extract failure messages
        let failures = Self::extract_cargo_failures(output);

        // Extract duration
        static DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"finished in ([\d.]+)s").unwrap()
        });
        let duration_ms = DURATION_RE
            .captures(output)
            .and_then(|c| c[1].parse::<f64>().ok())
            .map(|s| (s * 1000.0) as u64)
            .unwrap_or(0);

        TestOutput {
            success,
            passed,
            failed,
            skipped,
            timed_out: false,
            duration_ms,
            summary,
            failures,
        }
    }

    /// Parse jest/vitest/npm test output.
    fn parse_jest(output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;

        // "Tests: 12 passed, 3 failed, 1 skipped, 16 total"
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"Tests:\s*(\d+)\s*failed,\s*(\d+)\s*passed.*?(\d+)\s*total")
                .unwrap()
        });

        let (mut passed, mut failed) = (0usize, 0usize);
        let skipped = 0usize;
        let summary;

        if let Some(caps) = RE.captures(output) {
            failed = caps[1].parse().unwrap_or(0);
            let total: usize = caps[3].parse().unwrap_or(0);
            // Try to find passed count
            if let Some(pass_caps) = Regex::new(r"(\d+)\s*passed").unwrap().captures(output) {
                passed = pass_caps[1].parse().unwrap_or(0);
            } else {
                passed = total.saturating_sub(failed);
            }
            summary = format!("Tests: {} failed, {} passed, {} total", failed, passed, total);
        } else if !success {
            summary = "Test execution failed.".to_string();
            failed = 1;
        } else {
            summary = "Tests completed.".to_string();
            passed = 1;
        }

        // Extract failure messages (lines containing ● or FAIL)
        let failures: Vec<String> = output
            .lines()
            .filter(|l| l.contains("●") || l.contains("FAIL"))
            .map(|l| l.trim().to_string())
            .take(10)
            .collect();

        // Duration: "Time: 3.4 s"
        static DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"Time:\s*([\d.]+)\s*s").unwrap()
        });
        let duration_ms = DURATION_RE
            .captures(output)
            .and_then(|c| c[1].parse::<f64>().ok())
            .map(|s| (s * 1000.0) as u64)
            .unwrap_or(0);

        TestOutput {
            success,
            passed,
            failed,
            skipped,
            timed_out: false,
            duration_ms,
            summary,
            failures,
        }
    }

    /// Parse pytest output.
    fn parse_pytest(output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;

        // "12 passed, 3 failed, 1 skipped in 3.45s"
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(\d+)\s*passed.*?(\d+)\s*failed.*?(\d+)\s*skipped").unwrap()
        });

        let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
        let summary;

        if let Some(caps) = RE.captures(output) {
            passed = caps[1].parse().unwrap_or(0);
            failed = caps[2].parse().unwrap_or(0);
            skipped = caps[3].parse().unwrap_or(0);
            summary = format!(
                "{} passed, {} failed, {} skipped",
                passed, failed, skipped
            );
        } else if !success {
            summary = "Test execution failed.".to_string();
            failed = 1;
        } else {
            summary = "Tests completed.".to_string();
            passed = 1;
        }

        // Extract failure lines (FAILED or ERROR)
        let failures: Vec<String> = output
            .lines()
            .filter(|l| l.contains("FAILED") || l.contains("ERROR"))
            .map(|l| l.trim().to_string())
            .take(10)
            .collect();

        // Duration: "in 3.45s"
        static DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"in\s*([\d.]+)\s*s").unwrap()
        });
        let duration_ms = DURATION_RE
            .captures(output)
            .and_then(|c| c[1].parse::<f64>().ok())
            .map(|s| (s * 1000.0) as u64)
            .unwrap_or(0);

        TestOutput {
            success,
            passed,
            failed,
            skipped,
            timed_out: false,
            duration_ms,
            summary,
            failures,
        }
    }

    /// Parse python unittest output.
    fn parse_unittest(output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;

        // "Ran 12 tests in 0.123s" and "OK" or "FAILED (failures=3)"
        static RAN_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"Ran\s*(\d+)\s*tests?\s*in\s*([\d.]+)\s*s").unwrap()
        });

        let (mut passed, mut failed) = (0usize, 0usize);
        let summary;
        let mut duration_ms = 0u64;

        if let Some(caps) = RAN_RE.captures(output) {
            let total: usize = caps[1].parse().unwrap_or(0);
            duration_ms = caps[2]
                .parse::<f64>()
                .map(|s| (s * 1000.0) as u64)
                .unwrap_or(0);
            if success {
                passed = total;
                summary = format!("Ran {} tests in {:.3}s — OK", total, duration_ms as f64 / 1000.0);
            } else {
                // Try to extract failure count
                if let Some(fail_caps) =
                    Regex::new(r"failures=(\d+)").unwrap().captures(output)
                {
                    failed = fail_caps[1].parse().unwrap_or(0);
                    passed = total.saturating_sub(failed);
                }
                summary = format!("Ran {} tests — FAILED", total);
            }
        } else if !success {
            summary = "Test execution failed.".to_string();
            failed = 1;
        } else {
            summary = "Tests completed.".to_string();
            passed = 1;
        }

        let failures: Vec<String> = output
            .lines()
            .filter(|l| l.contains("FAIL:") || l.contains("ERROR:"))
            .map(|l| l.trim().to_string())
            .take(10)
            .collect();

        TestOutput {
            success,
            passed,
            failed,
            skipped: 0,
            timed_out: false,
            duration_ms,
            summary,
            failures,
        }
    }

    /// Parse `go test` output.
    fn parse_go(output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;

        // "ok   package/name  0.123s" or "FAIL  package/name  0.123s"
        let lines: Vec<&str> = output.lines().collect();
        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut summary_parts: Vec<String> = Vec::new();
        let mut duration_ms = 0u64;

        for line in &lines {
            if let Some(rest) = line.strip_prefix("ok ") {
                passed += 1;
                summary_parts.push(line.to_string());
                // Parse duration
                if let Some(last) = rest.split_whitespace().last() {
                    if let Ok(s) = last.trim_end_matches('s').parse::<f64>() {
                        duration_ms += (s * 1000.0) as u64;
                    }
                }
            } else if line.starts_with("FAIL\t") || line.starts_with("FAIL ") {
                failed += 1;
                summary_parts.push(line.to_string());
            }
        }

        let summary = if summary_parts.is_empty() {
            if success {
                "All tests passed.".to_string()
            } else {
                "Test execution failed.".to_string()
            }
        } else {
            summary_parts.join("\n")
        };

        if !success && failed == 0 {
            failed = 1;
        }

        let failures: Vec<String> = output
            .lines()
            .filter(|l| l.starts_with("--- FAIL"))
            .map(|l| l.trim().to_string())
            .take(10)
            .collect();

        TestOutput {
            success,
            passed,
            failed,
            skipped: 0,
            timed_out: false,
            duration_ms,
            summary,
            failures,
        }
    }

    /// Generic fallback parser.
    fn parse_generic(_output: &str, exit_code: i32) -> Self {
        let success = exit_code == 0;
        TestOutput {
            success,
            passed: if success { 1 } else { 0 },
            failed: if success { 0 } else { 1 },
            skipped: 0,
            timed_out: false,
            duration_ms: 0,
            summary: if success {
                "Tests completed.".to_string()
            } else {
                "Tests failed.".to_string()
            },
            failures: Vec::new(),
        }
    }

    /// Extract failure details from cargo test output.
    fn extract_cargo_failures(output: &str) -> Vec<String> {
        let mut failures = Vec::new();
        let mut _in_failure = false;

        for line in output.lines() {
            if line.starts_with("---- ") && line.contains("stdout") {
                _in_failure = true;
                if let Some(name) = line.strip_prefix("---- ").and_then(|s| s.strip_suffix(" stdout ----")) {
                    failures.push(name.trim().to_string());
                }
            } else if line.starts_with("failures:") {
                _in_failure = false;
            }
        }

        failures.truncate(10);
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cargo_success() {
        let output = "running 12 tests\ntest result: ok. 10 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 2.50s\n";
        let result = TestOutput::parse("rust-cargo", output, "", 0);
        assert!(result.success);
        assert_eq!(result.passed, 10);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 2);
        assert_eq!(result.duration_ms, 2500);
    }

    #[test]
    fn test_parse_cargo_failure() {
        let output = "running 5 tests\ntest result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.20s\n";
        let result = TestOutput::parse("rust-cargo", output, "", 101);
        assert!(!result.success);
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 2);
        assert_eq!(result.duration_ms, 1200);
    }

    #[test]
    fn test_parse_pytest() {
        let output = "test_module.py::test_foo PASSED\ntest_module.py::test_bar FAILED\n\n12 passed, 1 failed, 3 skipped in 3.45s\n";
        let result = TestOutput::parse("python-pytest", output, "", 1);
        assert!(!result.success);
        assert_eq!(result.passed, 12);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 3);
        assert_eq!(result.duration_ms, 3450);
    }

    #[test]
    fn test_parse_go_success() {
        let output = "ok  \texample.com/pkg\t0.123s\nok  \texample.com/pkg2\t0.456s\n";
        let result = TestOutput::parse("go", output, "", 0);
        assert!(result.success);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn test_parse_go_failure() {
        let output = "ok  \texample.com/pkg\t0.123s\nFAIL\texample.com/pkg2\t0.456s\n";
        let result = TestOutput::parse("go", output, "", 1);
        assert!(!result.success);
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
    }
}
