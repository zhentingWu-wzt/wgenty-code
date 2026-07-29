//! Unified tokenizer for memory recall, similarity, and TF-IDF indexing.
//!
//! Historically every call site (`MemoryIndex`, `ConsolidationEngine`,
//! `extract_keywords`) did its own `split_whitespace()` + `is_meaningful_token()`.
//! That worked for English but left Chinese recall broken: `split_whitespace`
//! does not split CJK text (no spaces), so a whole Chinese sentence became one
//! giant token that never matched anything.
//!
//! This module centralizes tokenization behind a [`Tokenizer`] trait with two
//! implementations:
//!
//! - [`DefaultTokenizer`] — zero-dependency CJK bigram fallback plus the
//!   existing English whitespace split. Always available.
//! - [`JiebaTokenizer`] — precise Chinese word segmentation via `jieba-rs`.
//!   Compiled in only when the `zh-segmentation` feature is enabled, because
//!   the embedded dictionary adds ~2MB to the binary.
//!
//! Call sites should obtain a tokenizer via [`build_tokenizer`] and never call
//! `split_whitespace` directly on memory content.

/// Unified tokenization contract. All similarity / indexing / keyword
/// extraction code goes through this so CJK and English are handled
/// consistently.
pub trait Tokenizer: Send + Sync {
    /// Split `text` into lowercase meaningful tokens: stop words and trivially
    /// short tokens are removed. The exact definition of "meaningful" is
    /// implementation-specific (see [`DefaultTokenizer`] / [`JiebaTokenizer`]).
    fn meaningful_tokens(&self, text: &str) -> Vec<String>;
}

/// English stop words + trivial tokens filtered out by every tokenizer.
///
/// Shared between [`DefaultTokenizer`] and [`JiebaTokenizer`] so both
/// implementations agree on which tokens are noise.
const STOP_WORDS_EN: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "being", "to",
    "of", "in", "on", "at", "by", "for", "with", "from", "as", "into", "than", "then", "this",
    "that", "these", "those", "it", "its", "i", "you", "he", "she", "we", "they", "not", "no",
    "do", "does", "did", "has", "have", "had", "will", "would", "can", "could", "should", "may",
    "might", "must", "if", "so", "up", "out", "about",
];

/// High-frequency Chinese function words / particles that carry no topical
/// signal for memory recall. Single-char particles are filtered by the general
/// `min_chars` rule; two-char entries here catch bigrams like "我们" / "这个".
const STOP_WORDS_ZH: &[&str] = &[
    "我们", "你们", "他们", "这个", "那个", "这些", "那些", "什么", "怎么", "可以", "应该", "已经",
    "现在", "的话", "以为",
];

/// Default tokenizer: English whitespace split + CJK bigram fallback.
///
/// This is the zero-dependency path (only `unicode-segmentation`, already an
/// indirect dep). CJK characters are grouped into maximal runs and each run is
/// converted to overlapping 2-grams ("用户登录" → ["用户", "户登", "登录"]).
/// Non-CJK segments keep the original whitespace-split behavior.
pub struct DefaultTokenizer;

impl DefaultTokenizer {
    /// Minimum token length in **Unicode chars** (not bytes). Previously this
    /// used `str::len()` (bytes), which let a single 3-byte CJK char slip past
    /// the `< 3` guard while semantically being one character. For CJK bigrams
    /// this rule is applied per-bigram (2 chars < 3, so bigrams rely on the
    /// dedicated CJK path that bypasses the ASCII min-length gate — see
    /// [`Self::cjk_bigrams`]).
    const MIN_CHARS: usize = 3;

    fn is_stop_word(lower: &str) -> bool {
        Self::is_stop_word_pub(lower)
    }

    /// Public CJK-aware stop-word check shared with [`JiebaTokenizer`].
    pub(crate) fn is_stop_word_pub(lower: &str) -> bool {
        STOP_WORDS_EN.contains(&lower) || STOP_WORDS_ZH.contains(&lower)
    }

    /// Push `word` lowercased into `out` if it clears the stop-word / length
    /// filters. Returns whether it was pushed. Used for non-CJK (ASCII) tokens.
    fn push_if_meaningful(out: &mut Vec<String>, word: &str) -> bool {
        let lower = word.to_lowercase();
        if lower.chars().count() < Self::MIN_CHARS {
            return false;
        }
        if Self::is_stop_word(&lower) {
            return false;
        }
        out.push(lower);
        true
    }

    /// True when `c` is a CJK ideograph (CJK Unified, compat, or extension A).
    /// Used to detect runs that need bigram segmentation.
    fn is_cjk(c: char) -> bool {
        matches!(c,
            '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
            | '\u{3400}'..='\u{4DBF}' // CJK Extension A
            | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        )
    }

    /// Segment a maximal CJK run into overlapping bigrams, each emitted via the
    /// CJK-aware filter (stop-word check only, no ASCII min-length gate since a
    /// CJK bigram is already the smallest meaningful unit).
    fn cjk_bigrams(run: &str, out: &mut Vec<String>) {
        let chars: Vec<char> = run.chars().collect();
        if chars.is_empty() {
            return;
        }
        if chars.len() == 1 {
            // Single CJK char: too short to be a meaningful bigram; drop.
            return;
        }
        for window in chars.windows(2) {
            let bigram: String = window.iter().collect();
            if Self::is_stop_word(&bigram) {
                continue;
            }
            out.push(bigram);
        }
    }

    /// Split `text` by scanning characters directly: CJK chars accumulate into
    /// runs (flushed via bigram segmentation when interrupted), ASCII
    /// alphanumeric chars accumulate into words (flushed when a separator
    /// appears). Everything else (whitespace, punctuation) is a separator.
    ///
    /// This intentionally does **not** use `unicode_words()`: UAX#29 does not
    /// segment CJK text into words, so a pure-Chinese string yields no
    /// `UnicodeWord` items at all.
    fn tokenize_segments(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut cjk_run = String::new();
        let mut ascii_word = String::new();

        let flush_ascii = |word: &mut String, out: &mut Vec<String>| {
            if !word.is_empty() {
                Self::push_if_meaningful(out, word);
                word.clear();
            }
        };
        let flush_cjk = |run: &mut String, out: &mut Vec<String>| {
            if !run.is_empty() {
                Self::cjk_bigrams(run, out);
                run.clear();
            }
        };

        for ch in text.chars() {
            if Self::is_cjk(ch) {
                flush_ascii(&mut ascii_word, &mut tokens);
                cjk_run.push(ch);
            } else if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                flush_cjk(&mut cjk_run, &mut tokens);
                ascii_word.push(ch);
            } else {
                // Separator: flush both accumulators.
                flush_cjk(&mut cjk_run, &mut tokens);
                flush_ascii(&mut ascii_word, &mut tokens);
            }
        }
        // Tail flush.
        flush_cjk(&mut cjk_run, &mut tokens);
        flush_ascii(&mut ascii_word, &mut tokens);

        tokens
    }
}

impl Tokenizer for DefaultTokenizer {
    fn meaningful_tokens(&self, text: &str) -> Vec<String> {
        self.tokenize_segments(text)
    }
}

/// Precise Chinese segmentation via jieba-rs. Compiled in only under the
/// `zh-segmentation` feature; otherwise [`build_tokenizer`] returns a
/// [`DefaultTokenizer`].
#[cfg(feature = "zh-segmentation")]
pub struct JiebaTokenizer {
    jieba: jieba_rs::Jieba,
}

#[cfg(feature = "zh-segmentation")]
impl JiebaTokenizer {
    pub fn new() -> Self {
        Self {
            jieba: jieba_rs::Jieba::new(),
        }
    }
}

#[cfg(feature = "zh-segmentation")]
impl Default for JiebaTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "zh-segmentation")]
impl Tokenizer for JiebaTokenizer {
    fn meaningful_tokens(&self, text: &str) -> Vec<String> {
        use crate::context::tokenizer::DefaultTokenizer;
        let mut tokens = Vec::new();
        // jieba's cut() handles CJK + ASCII mix in one pass. In jieba-rs 0.10
        // it returns Vec<Token> where Token { word: &str, start: usize }; we
        // operate on `.word`. Apply the same stop-word / length filtering as
        // DefaultTokenizer so both paths agree on what counts as noise.
        for token in self.jieba.cut(text, true) {
            let segment = token.word;
            if segment.chars().any(DefaultTokenizer::is_cjk) {
                // Chinese segment: keep it as-is (jieba already split to words).
                // Apply CJK stop-word filter; skip the ASCII min-length gate.
                let lower = segment.to_lowercase();
                if DefaultTokenizer::is_stop_word_pub(&lower) {
                    continue;
                }
                tokens.push(lower);
            } else {
                // ASCII / numeric segment: split on non-alphanumeric and apply
                // the full English filter (stop words + min length).
                for piece in segment.split(|c: char| !c.is_alphanumeric()) {
                    DefaultTokenizer::push_if_meaningful(&mut tokens, piece);
                }
            }
        }
        tokens
    }
}

/// Construct the best available tokenizer for this build.
///
/// Returns [`JiebaTokenizer`] when the `zh-segmentation` feature is enabled,
/// otherwise [`DefaultTokenizer`] (CJK bigram fallback).
pub fn build_tokenizer() -> Box<dyn Tokenizer> {
    #[cfg(feature = "zh-segmentation")]
    {
        Box::new(JiebaTokenizer::new())
    }
    #[cfg(not(feature = "zh-segmentation"))]
    {
        Box::new(DefaultTokenizer)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(text: &str) -> Vec<String> {
        DefaultTokenizer.meaningful_tokens(text)
    }

    #[test]
    fn english_whitespace_split_unchanged() {
        let toks = tokens_of("rust async programming patterns");
        assert!(toks.contains(&"rust".into()));
        assert!(toks.contains(&"async".into()));
        assert!(toks.contains(&"programming".into()));
    }

    #[test]
    fn english_stop_words_filtered() {
        let toks = tokens_of("the rust is a language");
        assert!(!toks.iter().any(|t| t == "the" || t == "is" || t == "a"));
        assert!(toks.iter().any(|t| t == "rust"));
    }

    #[test]
    fn short_tokens_filtered_by_char_count() {
        // MIN_CHARS = 3 (counted by Unicode chars, not bytes): 1- and 2-char
        // ASCII tokens are dropped. This matches the legacy byte-based `< 3`
        // guard for ASCII (where 1 byte == 1 char).
        let toks = tokens_of("a go run");
        assert!(!toks.iter().any(|t| t == "a"), "1-char dropped");
        assert!(!toks.iter().any(|t| t == "go"), "2-char dropped");
        assert!(toks.iter().any(|t| t == "run"), "3-char passes");
    }

    #[test]
    fn cjk_text_bigram_segmented() {
        // The core fix: Chinese text must produce tokens, not one giant blob.
        let toks = tokens_of("用户登录流程");
        assert!(!toks.is_empty(), "CJK must be segmented, got empty");
        assert!(toks.contains(&"用户".into()), "bigram 用户: {toks:?}");
        assert!(toks.contains(&"登录".into()), "bigram 登录: {toks:?}");
        assert!(toks.contains(&"流程".into()), "bigram 流程: {toks:?}");
    }

    #[test]
    fn cjk_recall_overlap_via_bigram() {
        // The recall payoff: a query "登录" and content "用户登录流程" share
        // the "登录" bigram, so TF-IDF / Jaccard can now match them.
        let query_toks = tokens_of("登录怎么做");
        let content_toks = tokens_of("用户登录流程说明");
        let overlap: Vec<&String> = query_toks
            .iter()
            .filter(|q| content_toks.contains(q))
            .collect();
        assert!(
            overlap.iter().any(|t| t.as_str() == "登录"),
            "expected 登录 overlap, query={query_toks:?} content={content_toks:?}"
        );
    }

    #[test]
    fn mixed_cjk_and_english() {
        let toks = tokens_of("使用 rust 开发 backend");
        assert!(toks.contains(&"rust".into()));
        assert!(toks.contains(&"backend".into()));
        // CJK part "使用开发" bigrams should be present.
        assert!(toks
            .iter()
            .any(|t| t.chars().count() == 2 && t.contains('用')));
    }

    #[test]
    fn cjk_single_char_run_filtered() {
        // A lone CJK char is 1 char → below MIN_CHARS → filtered.
        let toks = tokens_of("啊");
        assert!(toks.is_empty(), "single CJK char below min: {toks:?}");
    }

    #[test]
    fn empty_and_punctuation_input() {
        assert!(tokens_of("").is_empty());
        assert!(tokens_of("!!! ??? ...").is_empty());
    }

    #[test]
    fn build_tokenizer_returns_a_working_instance() {
        let tz = build_tokenizer();
        let toks = tz.meaningful_tokens("rust 登录");
        assert!(toks.iter().any(|t| t == "rust"));
        // 登录 may be a bigram (default) or precise word (jieba) — either way
        // the CJK half must yield something.
        assert!(toks.iter().any(|t| t.contains('登') || t.contains('录')));
    }
}
