/// Detect whether a prompt is complex enough to warrant RLM delegation.
///
/// Uses structural analysis instead of naive keyword matching:
/// 1. Multi-step structure (numbered steps, explicit sequencing)
/// 2. File references (paths in backticks/quotes)
/// 3. Dependency declarations ("depends on", "after X completes")
/// 4. Length as a secondary signal (>1000 chars, not 500)
///
/// This avoids routing simple tasks like "create a file" through the
/// expensive RLM pipeline.
#[allow(dead_code)]
pub(super) fn is_complex_task(prompt: &str, use_small_model: bool) -> bool {
    if use_small_model {
        return false; // User explicitly asked for cheap model
    }

    let prompt = prompt.trim();
    let len = prompt.len();

    // ── Structural signals (primary) ────────────────────────────────────

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
    if numbered_steps >= 3 {
        return true;
    }

    // File path references: `src/auth.rs`, "path/to/file", etc.
    let file_refs = prompt.matches('`').count() / 2  // paired backticks
        + prompt.matches("src/").count()
        + prompt.matches("tests/").count()
        + prompt.matches(".rs").count()
        + prompt.matches(".ts").count()
        + prompt.matches(".js").count()
        + prompt.matches(".py").count();
    if file_refs >= 3 {
        return true;
    }

    // Explicit dependency/sequencing markers — phrase-based to avoid
    // matching common words like "first" or "after" in isolation.
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
    let dep_hits = dependency_signals
        .iter()
        .filter(|kw| lower.contains(*kw))
        .count();
    if dep_hits >= 3 {
        return true;
    }

    // ── Length (secondary signal, raised from 500 to 1000) ──────────────
    if len > 1000 {
        // Only trigger if there are also structural indicators.
        return numbered_steps > 0 || file_refs > 0 || dep_hits > 0;
    }

    false
}
