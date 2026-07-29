//! LLM-based task-complexity classifier — the fallback for borderline
//! heuristic scores.
//!
//! When the structural heuristic ([`complexity_score`]) returns a borderline
//! score, the router may invoke [`classify_complexity`] for a sharper
//! assessment. The call uses the small model (cost-efficient), sends a minimal
//! prompt, and parses a single `{"score": 0.0-1.0}` JSON response. Any failure
//! returns `Err` so the caller falls back to the heuristic score or `Medium`.
//!
//! [`complexity_score`]: crate::tools::meta::task::heuristic::complexity_score

use crate::api::{ApiClient, ChatMessage};
use crate::tools::meta::rlm::pipeline::extract_json;

/// Ask the model to score the task complexity in `[0.0, 1.0]`.
///
/// Designed to be cheap: one short exchange, no tools, small model. The prompt
/// is explicit about the scale so the score is comparable to the heuristic.
pub async fn classify_complexity(client: &ApiClient, prompt: &str) -> Result<f64, String> {
    // Truncate very long prompts so the classification call stays small. The
    // first ~2000 chars carry enough signal for a complexity judgment.
    let snippet: String = prompt.chars().take(2000).collect();

    let sys = "You are a task-complexity classifier for an AI coding agent. \
               Given a task prompt, output ONLY a JSON object with a single field \
               \"score\" in [0.0, 1.0], where 0.0 = trivial (read/search/one step) \
               and 1.0 = very complex (multi-step, cross-file, with dependencies). \
               No explanation, no markdown.";

    let user = format!("Rate the complexity of this task:\n\n{snippet}\n\nJSON:");

    let messages = vec![ChatMessage::system(sys), ChatMessage::user(&user)];

    let response = client.chat(messages, None).await.map_err(|e| {
        tracing::debug!(target: "routing", error = %e, "complexity classifier API call failed");
        format!("complexity classifier call failed: {e}")
    })?;

    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("");

    let json_str = extract_json(content);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
        tracing::debug!(
            target: "routing",
            parse_error = %e,
            raw = %content,
            "complexity classifier returned non-JSON"
        );
        format!("complexity classifier parse failed: {e}")
    })?;

    let score = parsed
        .get("score")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "complexity classifier: missing 'score' field".to_string())?;

    // Clamp to [0, 1] — models occasionally emit slight overshoots.
    Ok(score.clamp(0.0, 1.0))
}
