/// Compute Levenshtein (edit) distance between two strings.
/// Returns the minimum number of single-character edits (insertions,
/// deletions, substitutions) required to change `a` into `b`.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (a_len, b_len) = (a_chars.len(), b_chars.len());

    // Optimise for empty-string edge cases
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Single-row DP — only need previous row
    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0usize; b_len + 1];

    for i in 1..=a_len {
        curr[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1) // deletion
                .min(curr[j - 1] + 1) // insertion
                .min(prev[j - 1] + cost); // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

/// A fuzzy match result with score.
#[derive(Debug, PartialEq)]
pub struct ScoredMatch {
    pub name: String,
    pub distance: usize,
}

/// Search `candidates` for strings within `max_dist` Levenshtein distance
/// of `query`, excluding candidates whose length differs by >50%.
/// Returns top `max_results`, sorted by distance ascending.
pub fn fuzzy_search(
    query: &str,
    candidates: &[String],
    max_dist: usize,
    max_results: usize,
) -> Vec<ScoredMatch> {
    let mut matches: Vec<ScoredMatch> = candidates
        .iter()
        .filter(|c| {
            let len_diff = (c.len() as f64 - query.len() as f64).abs();
            let max_len = query.len().max(c.len()) as f64;
            len_diff / max_len <= 0.5
        })
        .map(|c| {
            let d = levenshtein_distance(query, c);
            ScoredMatch {
                name: c.clone(),
                distance: d,
            }
        })
        .filter(|m| m.distance <= max_dist)
        .collect();
    matches.sort_by_key(|m| m.distance);
    matches.truncate(max_results);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
    }

    #[test]
    fn test_levenshtein_same() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert_eq!(levenshtein_distance("rust", "rust"), 0);
    }

    #[test]
    fn test_levenshtein_substitution() {
        assert_eq!(levenshtein_distance("kitten", "sitten"), 1); // k→s
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3); // k→s, e→i, +g
    }

    #[test]
    fn test_levenshtein_insertion_deletion() {
        assert_eq!(levenshtein_distance("abc", "abcd"), 1);
        assert_eq!(levenshtein_distance("abcd", "abc"), 1);
    }

    #[test]
    fn test_levenshtein_completely_different() {
        // "rust" → "python": r→p, u→y, s→t, t→h, +o, +n = 6
        assert_eq!(levenshtein_distance("rust", "python"), 6);
    }

    #[test]
    fn test_levenshtein_unicode() {
        assert_eq!(levenshtein_distance("café", "cafe"), 1); // é→e
        assert_eq!(levenshtein_distance("你好", "你好吗"), 1); // +吗
    }

    #[test]
    fn test_fuzzy_search_returns_matches() {
        let candidates: Vec<String> = vec![
            "run_async",
            "run_sync",
            "run_task",
            "runner",
            "ToolRegistry",
            "completely_different",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

        let results = fuzzy_search("run_async", &candidates, 3, 5);
        // run_async (0), run_sync (2), run_task (3), runner (3)
        assert_eq!(results[0].name, "run_async");
        assert_eq!(results[0].distance, 0);
        assert!(!results.iter().any(|m| m.name == "ToolRegistry"));
    }

    #[test]
    fn test_fuzzy_search_respects_length_filter() {
        let candidates: Vec<String> = vec!["ru".to_string()];
        let results = fuzzy_search("run_async_long_name", &candidates, 10, 5);
        // Length diff: |2 - 18|/18 = 88% > 50% → filtered out
        assert!(results.is_empty());
    }

    #[test]
    fn test_fuzzy_search_max_distance() {
        let candidates: Vec<String> = vec!["abc".to_string()];
        let results = fuzzy_search("xyz", &candidates, 2, 5);
        // "abc" → "xyz" distance = 3, max_dist = 2 → filtered out
        assert!(results.is_empty());
    }

    #[test]
    fn test_fuzzy_search_truncates_to_max_results() {
        let candidates: Vec<String> = (0..10).map(|i| format!("run_{i:02}")).collect();
        let results = fuzzy_search("run_05", &candidates, 10, 3);
        assert_eq!(results.len(), 3);
    }
}
