//! Provider — Isolate provider-specific API differences behind a unified interface.
//!
//! Each provider (OpenAI, DeepSeek, Anthropic, etc.) has its own quirks:
//! - DeepSeek requires `reasoning_content` to be echoed back
//! - OpenAI is standard chat/completions
//! - Anthropic uses a different API format entirely
//!
//! The provider pattern keeps these differences out of the REPL/TUI layer.

use super::{ChatMessage, Delta, StreamToolCall};

/// The result of processing a single SSE delta chunk through the provider.
/// This is provider-neutral — the REPL only sees these fields.
#[derive(Debug, Clone, Default)]
pub struct ProcessedDelta {
    /// Text content delta (may be None if this chunk has no text)
    pub content: Option<String>,
    /// Thinking/reasoning content that must be echoed back to the API
    pub reasoning_content: Option<String>,
    /// Tool call fragments (incremental)
    pub tool_calls: Option<Vec<StreamToolCall>>,
    /// Finish reason if the stream is complete
    pub finish_reason: Option<String>,
}

/// Provider isolates provider-specific API behaviour.
pub trait Provider: Send + Sync {
    /// Provider identifier
    fn name(&self) -> &str;

    /// Resolve the model ID sent to the API (maps user-friendly names to API IDs)
    fn resolve_model_id(&self, model: &str) -> String;

    /// Process a raw SSE delta into a provider-neutral ProcessedDelta
    fn process_delta(&self, delta: &Delta) -> ProcessedDelta;

    /// Enrich a ChatMessage with provider-specific fields before sending to the API.
    /// Returns a serde_json::Value with any extra top-level fields to merge.
    fn enrich_message(&self, _msg: &ChatMessage) -> Option<serde_json::Value> {
        None
    }

    /// Whether this provider uses the OpenAI-compatible chat/completions format
    fn is_openai_compat(&self) -> bool {
        true
    }
}

// ── OpenAI Provider ──────────────────────────────────────────────────────────

pub struct OpenAIProvider;

impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn resolve_model_id(&self, model: &str) -> String {
        model.to_string()
    }

    fn process_delta(&self, delta: &Delta) -> ProcessedDelta {
        ProcessedDelta {
            content: delta.content.clone(),
            reasoning_content: None,
            tool_calls: delta.tool_calls.clone(),
            finish_reason: None,
        }
    }
}

// ── DeepSeek Provider ────────────────────────────────────────────────────────

pub struct DeepSeekProvider;

impl Provider for DeepSeekProvider {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn resolve_model_id(&self, model: &str) -> String {
        // Map friendly names to actual DeepSeek model IDs
        match model.to_lowercase().as_str() {
            "v3" | "deepseek-v3" => "deepseek-chat".to_string(),
            "r1" | "deepseek-r1" | "reasoner" => "deepseek-reasoner".to_string(),
            other => other.to_string(),
        }
    }

    fn process_delta(&self, delta: &Delta) -> ProcessedDelta {
        ProcessedDelta {
            content: delta.content.clone(),
            reasoning_content: delta.reasoning_content.clone(),
            tool_calls: delta.tool_calls.clone(),
            finish_reason: None,
        }
    }

    /// DeepSeek requires reasoning_content to be passed back in subsequent requests
    fn enrich_message(&self, msg: &ChatMessage) -> Option<serde_json::Value> {
        if let Some(ref rc) = msg.reasoning_content {
            if !rc.is_empty() {
                return Some(serde_json::json!({
                    "reasoning_content": rc
                }));
            }
        }
        None
    }
}

// ── Anthropic Provider ──────────────────────────────────────────────────────

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn resolve_model_id(&self, model: &str) -> String {
        match model.to_lowercase().as_str() {
            "sonnet" | "claude-sonnet" | "claude-sonnet-4-6" => {
                "claude-sonnet-4-6-20250514".to_string()
            }
            "opus" | "claude-opus" | "claude-opus-4-7" => "claude-opus-4-7-20250514".to_string(),
            "haiku" | "claude-haiku" | "claude-haiku-4-5" => {
                "claude-haiku-4-5-20251001".to_string()
            }
            other => other.to_string(),
        }
    }

    fn process_delta(&self, delta: &Delta) -> ProcessedDelta {
        ProcessedDelta {
            content: delta.content.clone(),
            reasoning_content: None,
            tool_calls: delta.tool_calls.clone(),
            finish_reason: None,
        }
    }

    fn is_openai_compat(&self) -> bool {
        false
    }
}

// ── Provider Factory ─────────────────────────────────────────────────────────

/// Detect which provider to use based on the API base URL.
pub fn detect_provider(base_url: &str) -> std::sync::Arc<dyn Provider> {
    let url_lower = base_url.to_lowercase();
    let provider: Box<dyn Provider> = if url_lower.contains("deepseek") {
        Box::new(DeepSeekProvider)
    } else if url_lower.contains("anthropic") {
        Box::new(AnthropicProvider)
    } else if url_lower.contains("openai") {
        Box::new(OpenAIProvider)
    } else {
        // Default to OpenAI-compatible for unknown endpoints (Ollama, vLLM, etc.)
        Box::new(OpenAIProvider)
    };
    provider.into()
}

/// Resolve the provider, honoring an explicit override from settings before
/// falling back to base-url auto-detection. Override values: `"anthropic"`,
/// `"openai"` (or `"openai-compat"`), `"deepseek"`. Lets a user point at a
/// relay whose URL doesn't reveal its format (e.g. packyapi — Anthropic-native
/// but no "anthropic" in the URL) and force the correct path.
pub fn resolve_provider(
    base_url: &str,
    provider_override: Option<&str>,
) -> std::sync::Arc<dyn Provider> {
    if let Some(p) = provider_override {
        match p.to_lowercase().as_str() {
            "anthropic" => return std::sync::Arc::new(AnthropicProvider),
            "openai" | "openai-compat" | "openai_compatible" => {
                return std::sync::Arc::new(OpenAIProvider)
            }
            "deepseek" => return std::sync::Arc::new(DeepSeekProvider),
            _ => {}
        }
    }
    detect_provider(base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_anthropic() {
        let provider = detect_provider("https://api.anthropic.com");
        assert_eq!(provider.name(), "anthropic");
        assert!(!provider.is_openai_compat());
    }

    #[test]
    fn test_detect_deepseek() {
        let provider = detect_provider("https://api.deepseek.com");
        assert_eq!(provider.name(), "deepseek");
    }

    #[test]
    fn test_detect_openai() {
        let provider = detect_provider("https://api.openai.com");
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_resolve_provider_override_anthropic() {
        // packyapi URL has no "anthropic" — auto-detect would pick OpenAI and
        // hit the flaky /v1/chat/completions compat layer. The override forces
        // the stable /v1/messages Anthropic path (matches Claude Code).
        let provider = resolve_provider("https://api-slb.packyapi.com", Some("anthropic"));
        assert_eq!(provider.name(), "anthropic");
        assert!(!provider.is_openai_compat());
    }

    #[test]
    fn test_resolve_provider_override_openai() {
        let provider = resolve_provider("https://api.anthropic.com", Some("openai"));
        assert_eq!(provider.name(), "openai");
        assert!(provider.is_openai_compat());
    }

    #[test]
    fn test_resolve_provider_override_case_insensitive() {
        let provider = resolve_provider("https://example.com", Some("Anthropic"));
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_resolve_provider_unknown_override_falls_back() {
        // Unknown override value → fall back to base-url auto-detection.
        let provider = resolve_provider("https://api.deepseek.com", Some("nonsense"));
        assert_eq!(provider.name(), "deepseek");
    }

    #[test]
    fn test_resolve_provider_no_override_uses_detect() {
        let provider = resolve_provider("https://api.anthropic.com", None);
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_anthropic_model_mapping() {
        let provider = AnthropicProvider;
        assert_eq!(
            provider.resolve_model_id("sonnet"),
            "claude-sonnet-4-6-20250514"
        );
        assert_eq!(
            provider.resolve_model_id("opus"),
            "claude-opus-4-7-20250514"
        );
        assert_eq!(
            provider.resolve_model_id("haiku"),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(
            provider.resolve_model_id("claude-sonnet-4-6"),
            "claude-sonnet-4-6-20250514"
        );
        assert_eq!(provider.resolve_model_id("custom-model"), "custom-model");
    }

    #[test]
    fn test_deepseek_model_mapping() {
        let provider = DeepSeekProvider;
        assert_eq!(provider.resolve_model_id("v3"), "deepseek-chat");
        assert_eq!(provider.resolve_model_id("r1"), "deepseek-reasoner");
        assert_eq!(provider.resolve_model_id("custom-model"), "custom-model");
    }

    #[test]
    fn test_deepseek_reasoning_content() {
        let provider = DeepSeekProvider;
        let delta = Delta {
            role: None,
            content: None,
            reasoning_content: Some("thinking...".to_string()),
            tool_calls: None,
        };
        let processed = provider.process_delta(&delta);
        assert_eq!(processed.reasoning_content, Some("thinking...".to_string()));
    }

    #[test]
    fn test_openai_no_reasoning() {
        let provider = OpenAIProvider;
        let delta = Delta {
            role: None,
            content: Some("hello".to_string()),
            reasoning_content: None,
            tool_calls: None,
        };
        let processed = provider.process_delta(&delta);
        assert_eq!(processed.content, Some("hello".to_string()));
        assert_eq!(processed.reasoning_content, None);
    }
}
