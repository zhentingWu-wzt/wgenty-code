use serde::{Deserialize, Serialize};

/// 结构化失败根因分类，在捕获时从结构化信号确定。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureRootCause {
    TokenBudgetExceeded,
    GuardianRejected { reason: String },
    SandboxFailed,
    ApiError,
    ToolPanic,
    Timeout,
    UserCancelled,
    #[default]
    Unknown,
}

/// 失败轮次的单个工具调用步骤（参数已脱敏）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCallStep {
    pub tool_name: String,
    pub params_summary: serde_json::Value,
    pub elapsed_ms: u64,
}

/// 失败轮次完整上下文（截断存储）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FailedRoundContext {
    pub assistant_text: String,
    pub final_tool_output: String,
}

/// 单次重试的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome {
    Succeeded,
    Failed,
}

/// 单次重试记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub error: String,
    pub root_cause: FailureRootCause,
    pub strategy: String,
    pub outcome: RetryOutcome,
}

const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "token",
    "secret",
    "password",
    "apikey",
    "access_token",
    "refresh_token",
];

/// 递归脱敏敏感键的值。
pub fn redact_params(params: serde_json::Value) -> serde_json::Value {
    match params {
        serde_json::Value::Object(mut map) => {
            for (k, v) in map.iter_mut() {
                let lower = k.to_lowercase();
                if SENSITIVE_KEYS.iter().any(|s| lower.contains(s)) {
                    *v = serde_json::Value::String("***REDACTED***".into());
                } else {
                    *v = redact_params(v.clone());
                }
            }
            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact_params).collect())
        }
        other => other,
    }
}

/// char-boundary 安全截断（不 panic on multi-byte UTF-8）。
pub fn truncate_char_safe(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_keys() {
        let params = json!({"api_key": "sk-123", "token": "abc", "query": "hello", "password": "pw"});
        let redacted = redact_params(params);
        assert_eq!(redacted["api_key"], "***REDACTED***");
        assert_eq!(redacted["token"], "***REDACTED***");
        assert_eq!(redacted["password"], "***REDACTED***");
        assert_eq!(redacted["query"], "hello");
    }

    #[test]
    fn redacts_nested_sensitive_keys() {
        let params = json!({"outer": {"secret": "s", "ok": 1}});
        let redacted = redact_params(params);
        assert_eq!(redacted["outer"]["secret"], "***REDACTED***");
        assert_eq!(redacted["outer"]["ok"], 1);
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        let s = "中文测试─字符";
        let t = truncate_char_safe(s, 4);
        assert!(t.chars().count() <= 4);
        assert!(s.starts_with(&t));
    }

    #[test]
    fn truncate_empty_and_short_unchanged() {
        assert_eq!(truncate_char_safe("", 10), "");
        assert_eq!(truncate_char_safe("abc", 10), "abc");
    }

    #[test]
    fn root_cause_default_is_unknown() {
        assert_eq!(FailureRootCause::default(), FailureRootCause::Unknown);
    }
}
