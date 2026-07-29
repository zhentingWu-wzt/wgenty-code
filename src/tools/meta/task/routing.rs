//! Subagent model routing decision logic.
//!
//! [`decide`] takes the caller's explicit `use_small_model` hint (if any), the
//! task prompt, and the routing config, and returns a [`ModelChoice`]. The
//! decision follows a strict priority chain (see [`ModelChoice`] docs) so that
//! explicit user/LLM intent always wins over heuristic auto-routing.
//!
//! Borderline heuristic scores (between `boundary_low` and `boundary_high`) are
//! surfaced as [`ModelChoice::NeedsLlm`] so the caller can decide whether to
//! spend an LLM call (via [`crate::tools::meta::rlm::classifier`]) or fall back
//! to a default tier.

use crate::config::models::{ModelRoutingConfig, ModelTier};
use crate::tools::meta::task::heuristic::complexity_score;

/// The routing decision for a subagent task.
///
/// Priority order (first match wins):
/// 1. [`Explicit`](Self::Explicit) — the caller passed `use_small_model`, so
///    honor it (backward-compatible with pre-routing behavior).
/// 2. [`Auto`](Self::Auto) — the heuristic score was clearly low or high, so a
///    tier is selected without an LLM call.
/// 3. [`NeedsLlm`](Self::NeedsLlm) — the score is borderline; the caller may
///    invoke the LLM classifier for a sharper decision.
/// 4. [`Auto`](Self::Auto) with `Medium` — routing disabled or no tiers
///    available, so the main model is used.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelChoice {
    /// Explicit request: `use_small_model = Some(true)` → Light,
    /// `Some(false)` → Heavy. Preserves the pre-routing binary behavior.
    Explicit(ModelTier),
    /// Auto-selected tier from a decisive heuristic score.
    Auto(ModelTier),
    /// Borderline heuristic score (between `boundary_low` and `boundary_high`).
    /// The caller decides whether to call the LLM classifier. `score` is the
    /// raw heuristic score for fallback use.
    NeedsLlm { score: f64 },
}

impl ModelChoice {
    /// Resolve to a concrete [`ModelTier`], for callers that don't want the LLM
    /// path. [`NeedsLlm`] resolves to [`ModelTier::Medium`].
    pub fn tier_or_medium(&self) -> ModelTier {
        match self {
            ModelChoice::Explicit(t) | ModelChoice::Auto(t) => *t,
            ModelChoice::NeedsLlm { .. } => ModelTier::Medium,
        }
    }
}

/// Decide the model routing for a subagent task.
///
/// - `use_small_model`: `None` = field absent (eligible for auto-routing);
///   `Some(b)` = explicit hint that always wins.
/// - `prompt`: the task prompt, scored by [`complexity_score`] when auto-routing.
/// - `routing`: the routing config (`enabled`, thresholds, llm_fallback).
/// - `has_light_tier` / `has_heavy_tier`: whether the configured profiles
///   actually declare those tiers (so we don't auto-route to a tier with no
///   matching profile — that would just pick `main` anyway, defeating the
///   purpose and wasting an LLM call).
pub fn decide(
    use_small_model: Option<bool>,
    prompt: &str,
    routing: &ModelRoutingConfig,
    has_light_tier: bool,
    has_heavy_tier: bool,
) -> ModelChoice {
    // 1. Explicit hint always wins (backward compatibility).
    match use_small_model {
        Some(true) => return ModelChoice::Explicit(ModelTier::Light),
        Some(false) => return ModelChoice::Explicit(ModelTier::Heavy),
        None => {}
    }

    // 2. Routing disabled → main model.
    if !routing.enabled {
        return ModelChoice::Auto(ModelTier::Medium);
    }

    // 3. If neither tier has a profile, auto-routing can't do better than main.
    if !has_light_tier && !has_heavy_tier {
        return ModelChoice::Auto(ModelTier::Medium);
    }

    let score = complexity_score(prompt);

    // 4. Decisive low score → Light (only if a Light profile exists).
    if score < routing.boundary_low && has_light_tier {
        return ModelChoice::Auto(ModelTier::Light);
    }
    // 5. Decisive high score → Heavy (only if a Heavy profile exists).
    if score > routing.boundary_high && has_heavy_tier {
        return ModelChoice::Auto(ModelTier::Heavy);
    }

    // 6. Borderline — caller may invoke LLM classifier.
    ModelChoice::NeedsLlm { score }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing() -> ModelRoutingConfig {
        ModelRoutingConfig::default()
    }

    #[test]
    fn explicit_true_wins_over_everything() {
        // Even a trivially simple prompt with explicit true → Light.
        let choice = decide(Some(true), "read file", &routing(), true, true);
        assert_eq!(choice, ModelChoice::Explicit(ModelTier::Light));
    }

    #[test]
    fn explicit_false_wins_over_everything() {
        // Even a complex prompt with explicit false → Heavy.
        let complex = "1. a\n2. b\n3. c\n4. d\n5. e";
        let choice = decide(Some(false), complex, &routing(), true, true);
        assert_eq!(choice, ModelChoice::Explicit(ModelTier::Heavy));
    }

    #[test]
    fn absent_hint_plus_disabled_routing_falls_to_medium() {
        let mut cfg = routing();
        cfg.enabled = false;
        let choice = decide(None, "1. a\n2. b\n3. c", &cfg, true, true);
        assert_eq!(choice, ModelChoice::Auto(ModelTier::Medium));
    }

    #[test]
    fn absent_hint_no_tiers_falls_to_medium() {
        // No profiles declare any tier → no point routing.
        let choice = decide(
            None,
            "1. a\n2. b\n3. c\n4. d\n5. e",
            &routing(),
            false,
            false,
        );
        assert_eq!(choice, ModelChoice::Auto(ModelTier::Medium));
    }

    #[test]
    fn simple_prompt_routes_to_light() {
        let choice = decide(None, "read the file", &routing(), true, true);
        assert_eq!(choice, ModelChoice::Auto(ModelTier::Light));
    }

    #[test]
    fn complex_prompt_routes_to_heavy() {
        let complex =
            "1. Refactor auth\n2. Update callers\n3. Add tests\n4. Update docs\n5. Migrate config";
        let choice = decide(None, complex, &routing(), true, true);
        assert_eq!(choice, ModelChoice::Auto(ModelTier::Heavy));
    }

    #[test]
    fn medium_complexity_is_borderline() {
        // A prompt with moderate signals — one numbered step plus a few file
        // refs — that lands in the borderline band rather than a decisive tier.
        let prompt = "1. Fix the bug in src/auth.rs. Then check src/api/mod.rs";
        let choice = decide(None, prompt, &routing(), true, true);
        match choice {
            ModelChoice::NeedsLlm { score } => {
                assert!(
                    (0.3..=0.7).contains(&score),
                    "borderline score should be in [0.3, 0.7], got {score}"
                );
            }
            other => panic!("expected NeedsLlm, got {other:?}"),
        }
    }

    #[test]
    fn light_only_available_no_heavy_profile() {
        // Complex prompt but no Heavy profile → not Heavy, and not Light (score
        // too high), so borderline/NeedsLlm.
        let complex = "1. a\n2. b\n3. c\n4. d\n5. e";
        let choice = decide(None, complex, &routing(), true, false);
        // Score is high (>0.7) but has_heavy_tier=false → neither branch fires,
        // so it falls through to NeedsLlm. Acceptable: caller resolves to Medium.
        assert!(matches!(choice, ModelChoice::NeedsLlm { .. }));
    }

    #[test]
    fn tier_or_medium_resolves_needs_llm_to_medium() {
        let choice = ModelChoice::NeedsLlm { score: 0.5 };
        assert_eq!(choice.tier_or_medium(), ModelTier::Medium);
    }
}
