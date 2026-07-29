//! Heuristic task-complexity scoring for automatic model routing.
//!
//! [`complexity_score`] returns a 0.0–1.0 score from structural analysis of the
//! task prompt — numbered steps, file references, dependency markers, and
//! length. The router thresholds this score (see [`ModelRoutingConfig`]) to
//! pick a [`ModelTier`], optionally consulting an LLM for borderline cases.
//!
//! [`ModelRoutingConfig`]: crate::config::models::ModelRoutingConfig
//! [`ModelTier`]: crate::config::models::ModelTier

/// Raw structural signals extracted from a prompt. Exposed so the router and
/// tests can inspect *why* a score came out the way it did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComplexitySignals {
    pub numbered_steps: u32,
    pub file_refs: u32,
    pub dependency_hits: u32,
    pub length: usize,
}

/// Extract structural complexity signals from `prompt`.
///
/// These are the same signals the legacy `is_complex_task` checked, now exposed
/// individually so [`complexity_score`] can weight them continuously.
pub fn extract_signals(prompt: &str) -> ComplexitySignals {
    let prompt = prompt.trim();
    let length = prompt.len();

    // Numbered steps: "1. Refactor auth\n2. Update callers\n3. Add tests"
    let numbered_steps = {
        let mut count = 0u32;
        for line in prompt.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(|c: char| c.is_ascii_digit())
                && trimmed.chars().find(|c| !c.is_ascii_digit()) == Some('.')
            {
                count += 1;
            }
        }
        count
    };

    // File path references: `src/auth.rs`, "path/to/file", extensions, etc.
    let file_refs = (prompt.matches('`').count() / 2 // paired backticks
        + prompt.matches("src/").count()
        + prompt.matches("tests/").count()
        + prompt.matches(".rs").count()
        + prompt.matches(".ts").count()
        + prompt.matches(".js").count()
        + prompt.matches(".py").count()) as u32;

    // Explicit dependency/sequencing markers — phrase-based to avoid matching
    // common words like "first" or "after" in isolation.
    let lower = prompt.to_lowercase();
    let dependency_signals = [
        "depends on",
        "must complete before",
        "after that",
        "then you should",
        "before you",
        "first you",
        "first, ",
        "second, ",
        "finally, ",
        "step by step",
        "one by one",
    ];
    let dependency_hits = dependency_signals
        .iter()
        .filter(|kw| lower.contains(*kw))
        .count() as u32;

    ComplexitySignals {
        numbered_steps,
        file_refs,
        dependency_hits,
        length,
    }
}

/// Continuous complexity score in `[0.0, 1.0]`.
///
/// Each signal contributes a sub-score capped at 1.0, then the dominant
/// structural signal carries 0.8 of the weight and prompt length is a mild
/// 0.2 amplifier. The final score is clamped to `[0.0, 1.0]`.
///
/// Intuition:
/// - A trivial prompt ("read this file") → near 0.0.
/// - A structured multi-step task with dependencies → near 1.0.
/// - The router's default thresholds (`boundary_low=0.3`, `boundary_high=0.7`)
///   mean scores in `[0.3, 0.7]` are "borderline" and may trigger LLM fallback.
pub fn complexity_score(prompt: &str) -> f64 {
    let s = extract_signals(prompt);

    // Each signal is normalized so its "clearly complex" threshold maps to ~1.0.
    // steps: 3+ numbered steps is a multi-step task; saturate at 3.
    let steps_score = (s.numbered_steps as f64 / 3.0).min(1.0);
    // file_refs: 3+ refs indicates cross-file work; saturate at 8 so that a
    // single incidental path mention (e.g. "read src/main.rs") doesn't register.
    let files_score = (s.file_refs as f64 / 8.0).min(1.0);
    // dependencies: 3+ markers indicates explicit sequencing; saturate at 4.
    let deps_score = (s.dependency_hits as f64 / 4.0).min(1.0);
    // length: only matters as a mild amplifier above 500 chars; saturate at 2000.
    let length_score = ((s.length as f64 - 500.0) / 1500.0).clamp(0.0, 1.0);

    // Any single strong structural signal should be enough to mark a task
    // complex, so the dominant signal carries most of the weight (0.8) and
    // length is a mild amplifier (0.2). This is an OR-leaning blend: a prompt
    // with 3 numbered steps but no file refs / deps still scores 0.8.
    let blended = steps_score.max(deps_score).max(files_score) * 0.8 + length_score * 0.2;
    blended.clamp(0.0, 1.0)
}

/// Legacy boolean classifier, retained for compatibility.
///
/// Returns `true` when [`complexity_score`] exceeds `0.7` (clearly complex) and
/// `use_small_model` was not explicitly requested. The router uses the
/// continuous score directly; this helper exists for callers that still want a
/// hard boolean (e.g. deciding whether to dispatch to the RLM pipeline).
#[allow(dead_code)]
pub fn is_complex_task(prompt: &str, use_small_model: bool) -> bool {
    if use_small_model {
        return false; // User explicitly asked for cheap model.
    }
    complexity_score(prompt) >= 0.7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_prompt_scores_low() {
        let score = complexity_score("read the file src/main.rs");
        assert!(
            score < 0.3,
            "trivial prompt should score < 0.3, got {score}"
        );
    }

    #[test]
    fn empty_prompt_scores_zero() {
        assert_eq!(complexity_score(""), 0.0);
        assert_eq!(complexity_score("   "), 0.0);
    }

    #[test]
    fn multi_step_numbered_scores_high() {
        let prompt = "1. Refactor auth module\n2. Update all callers\n3. Add tests\n4. Update docs\n5. Migrate config";
        let score = complexity_score(prompt);
        assert!(
            score > 0.7,
            "5 numbered steps should score > 0.7, got {score}"
        );
    }

    #[test]
    fn dependency_markers_boost_score() {
        let prompt = "First, refactor the auth module. After that, update the callers. Finally, depends on the config migration step by step.";
        let score = complexity_score(prompt);
        // Multiple dependency phrases should push this into at least medium.
        assert!(
            score > 0.4,
            "dependency markers should boost score > 0.4, got {score}"
        );
    }

    #[test]
    fn many_file_refs_boost_score() {
        let prompt = "Update `src/auth.rs`, `src/api/mod.rs`, `src/config/models.rs`, `tests/auth_test.rs`, and `src/lib.rs`";
        let score = complexity_score(prompt);
        assert!(
            score > 0.3,
            "many file refs should boost score > 0.3, got {score}"
        );
    }

    #[test]
    fn is_complex_task_legacy_bool() {
        // Explicit small-model request always returns false.
        assert!(!is_complex_task("1. a\n2. b\n3. c\n4. d\n5. e", true));
        // Clearly complex (5 steps) without explicit small → true.
        assert!(is_complex_task("1. a\n2. b\n3. c\n4. d\n5. e", false));
        // Trivial → false.
        assert!(!is_complex_task("read file", false));
    }

    #[test]
    fn extract_signals_counts_correctly() {
        let s = extract_signals("1. First step\n2. Second step\nupdate `src/a.rs`");
        assert_eq!(s.numbered_steps, 2);
        assert!(s.file_refs >= 2); // `src/a.rs` → 1 backtick-pair + "src/" + ".rs"
    }

    #[test]
    fn score_is_in_unit_range() {
        // A pathological prompt with every signal maxed should still clamp.
        let mut prompt = String::new();
        for i in 1..=20 {
            prompt.push_str(&format!(
                "{i}. step depends on first, after that, finally, step by step `src/file{i}.rs`\n"
            ));
        }
        let score = complexity_score(&prompt);
        assert!(score <= 1.0, "score must be clamped to <= 1.0, got {score}");
    }
}
