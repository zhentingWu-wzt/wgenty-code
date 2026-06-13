//! Lenient JSON parser for tool call arguments.
//!
//! When the LLM generates tool calls with very long arguments (e.g., writing
//! a large file), the JSON string may be truncated due to max_tokens limits
//! or stream interruptions. Normal `serde_json::from_str` fails entirely on
//! truncated JSON, but we can recover partial data — at minimum the `path`
//! for file operations — to provide actionable error messages.

use regex::Regex;

/// Try to parse tool arguments, falling back to field extraction on failure.
///
/// Strategy (in order):
/// 1. Direct `serde_json::from_str`
/// 2. Pre-process to fix common regex escape errors, then retry `serde_json::from_str`
/// 3. Regex-based partial field extraction as last resort
///
/// Returns `(parsed_value, parse_error)` — if any step succeeds,
/// `parse_error` is `None`. Otherwise, extracts known fields via regex and
/// returns a partial JSON object with an `_parse_error` key.
pub fn parse_tool_args_lenient(raw: &str, _tool_name: &str) -> (serde_json::Value, Option<String>) {
    // Step 1: direct parse
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        return (v, None);
    }

    // Step 2: pre-process to fix common regex escape errors, retry
    let preprocessed = preprocess_json_args(raw);
    if preprocessed != raw {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&preprocessed) {
            return (v, None);
        }
    }

    // Step 3: partial field extraction as last resort
    let e = serde_json::from_str::<serde_json::Value>(raw).unwrap_err();
    let partial = extract_partial_fields(raw);
    let mut obj = if let serde_json::Value::Object(m) = partial {
        m
    } else {
        serde_json::Map::new()
    };
    // Collect field names before mutating obj
    let field_names: Vec<String> = obj
        .keys()
        .filter(|k| !k.starts_with('_'))
        .cloned()
        .collect();
    obj.insert(
        "_parse_error".to_string(),
        serde_json::json!({
            "message": e.to_string(),
            "raw_length": raw.len(),
            "raw_preview": truncate_for_display(raw, 200),
            "is_truncated": looks_truncated(raw),
        }),
    );
    obj.insert("_raw_arguments".to_string(), serde_json::json!(raw));
    (
        serde_json::Value::Object(obj),
        Some(format!(
            "JSON parse error: {}. Extracted partial fields: {}",
            e,
            field_names.join(", ")
        )),
    )
}

/// Pre-process raw tool-call arguments to fix common JSON escaping errors
/// that LLMs make when generating regex patterns.
///
/// Specifically, regex escape sequences like `\d`, `\w`, `\s`, `\D`, `\W`,
/// `\S`, `\B`, `\A`, `\Z` etc. are NOT valid JSON escape sequences.
/// When the LLM generates a grep pattern containing these, `serde_json` fails.
///
/// This function detects backslash-char pairs where `char` is not a valid
/// JSON escape target and inserts an additional backslash to produce valid JSON.
///
/// # Valid JSON escape sequences (left untouched):
/// `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, `\uXXXX`
///
/// # Invalid JSON escapes that are auto-fixed (common in regex):
/// `\d`, `\D`, `\w`, `\W`, `\s`, `\S`, `\B`, `\A`, `\Z`, `\xNN`,
/// backreferences `\1`-`\9`, and any other `\X` where X is not a JSON-valid target.
fn preprocess_json_args(raw: &str) -> String {
    // Fast path: no backslashes means nothing to fix
    if !raw.contains('\\') {
        return raw.to_string();
    }

    let chars: Vec<char> = raw.chars().collect();
    let mut result = String::with_capacity(raw.len() + 16);
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if is_valid_json_escape_target(next) {
                // Valid JSON escape — keep as-is
                result.push('\\');
                result.push(next);
                i += 2;
            } else if next == '\\' {
                // Already escaped backslash — keep as-is
                result.push_str("\\\\");
                i += 2;
            } else {
                // Invalid JSON escape (e.g. \d, \w, \s) — add extra backslash
                result.push_str("\\\\");
                result.push(next);
                i += 2;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Check if a character is a valid target for a JSON backslash escape.
///
/// Valid per RFC 7159: `"` `\` `/` `b` `f` `n` `r` `t` `u`
fn is_valid_json_escape_target(c: char) -> bool {
    matches!(c, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u')
}

/// Extract known fields from a potentially truncated JSON string using regex.
fn extract_partial_fields(raw: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Extract string fields: "key": "value" (with proper escape handling)
    for field in &[
        "path",
        "content",
        "description",
        "prompt",
        "subagent_type",
        "command",
        "pattern",
        "query",
        "url",
        "operation",
        "question",
        "subject",
        "title",
        "note_id",
        "old_string",
        "new_string",
        "file_path",
    ] {
        if let Some(val) = extract_string_field(raw, field) {
            map.insert(field.to_string(), serde_json::Value::String(val));
        }
    }

    // Extract bool fields
    for field in &[
        "background",
        "use_small_model",
        "replace_all",
        "multi_select",
    ] {
        if let Some(val) = extract_bool_field(raw, field) {
            map.insert(field.to_string(), serde_json::Value::Bool(val));
        }
    }

    serde_json::Value::Object(map)
}

/// Extract a string field value from JSON-like text using regex.
fn extract_string_field(raw: &str, field_name: &str) -> Option<String> {
    let pattern = format!(
        r#""{}"\s*:\s*"((?:[^"\\]|\\.)*)""#,
        regex::escape(field_name)
    );
    let re = Regex::new(&pattern).ok()?;
    re.captures(raw)
        .and_then(|caps| caps.get(1))
        .map(|m| unescape_json_string(m.as_str()))
}

/// Extract a boolean field value.
fn extract_bool_field(raw: &str, field_name: &str) -> Option<bool> {
    let pattern = format!(r#""{}"\s*:\s*(true|false)"#, regex::escape(field_name));
    let re = Regex::new(&pattern).ok()?;
    re.captures(raw)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str() == "true")
}

/// Check if a JSON string looks truncated (missing closing braces/brackets).
fn looks_truncated(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Count braces and brackets
    let open_braces = trimmed.chars().filter(|&c| c == '{').count();
    let close_braces = trimmed.chars().filter(|&c| c == '}').count();
    let open_brackets = trimmed.chars().filter(|&c| c == '[').count();
    let close_brackets = trimmed.chars().filter(|&c| c == ']').count();
    // Also check if the string ends mid-value
    let ends_abruptly = trimmed.ends_with(',') || trimmed.ends_with(':') || trimmed.ends_with('"');
    open_braces != close_braces || open_brackets != close_brackets || ends_abruptly
}

/// Truncate a string for display, showing head + tail.
fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let head_len = max_len * 2 / 3;
    let tail_len = max_len - head_len - 3; // 3 for "..."
    format!("{}...{}", &s[..head_len], &s[s.len() - tail_len..])
}

/// Basic JSON string unescaping: \" → ", \\ → \, \n → newline, etc.
fn unescape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('/') => result.push('/'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_json() {
        let (val, err) =
            parse_tool_args_lenient(r#"{"path": "test.md", "content": "hello"}"#, "file_write");
        assert!(err.is_none());
        assert_eq!(val["path"].as_str(), Some("test.md"));
        assert_eq!(val["content"].as_str(), Some("hello"));
    }

    #[test]
    fn test_parse_truncated_json_extracts_path() {
        // Simulate truncated JSON — content string is cut off
        let raw = r#"{"path": "WGENTY.md", "content": "#;
        let (val, err) = parse_tool_args_lenient(raw, "file_write");
        assert!(err.is_some());
        assert_eq!(val["path"].as_str(), Some("WGENTY.md"));
        assert!(val["_parse_error"].is_object());
        assert_eq!(val["_parse_error"]["is_truncated"].as_bool(), Some(true));
    }

    #[test]
    fn test_parse_truncated_json_mid_content() {
        let raw = r#"{"path": "README.md", "content": "very long content that gets truncated"#;
        let (val, err) = parse_tool_args_lenient(raw, "file_write");
        assert!(err.is_some());
        assert_eq!(val["path"].as_str(), Some("README.md"));
        // content is not extracted because the closing quote is missing
    }

    #[test]
    fn test_extract_multiple_fields() {
        let raw = r#"{"path": "/tmp/test", "command": "cargo build", "background": true"#;
        let (val, err) = parse_tool_args_lenient(raw, "exec_command");
        // This looks truncated (missing closing brace)
        assert!(err.is_some());
        assert_eq!(val["path"].as_str(), Some("/tmp/test"));
        assert_eq!(val["command"].as_str(), Some("cargo build"));
        assert_eq!(val["background"].as_bool(), Some(true));
    }

    #[test]
    fn test_looks_truncated() {
        assert!(looks_truncated(r#"{"path": "x", "content": ""#));
        assert!(looks_truncated(r#"{"path": "x","#));
        assert!(!looks_truncated(r#"{"path": "x"}"#));
        assert!(looks_truncated(r#"{"path": "x", "content": "y""#)); // missing closing brace
    }

    #[test]
    fn test_empty_input() {
        let (val, err) = parse_tool_args_lenient("", "file_write");
        assert!(err.is_some());
        assert!(val["_parse_error"].is_object());
    }
}
