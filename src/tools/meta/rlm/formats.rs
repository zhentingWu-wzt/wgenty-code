//! RLM structured output formats — Claims and UnifiedDiff schemas plus Aggregator.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ── Claims format ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimsOutput {
    pub format: String, // "structured-claims/1"
    pub claims: Vec<Claim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ClaimsMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub claim: String,
    pub evidence: String,
    pub confidence: f32, // 0.0 - 1.0
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    #[serde(default)]
    pub actionable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimsMetadata {
    pub parse_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_warning: Option<String>,
}

// ── Diff format ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOutput {
    pub format: String, // "unified-diff/1"
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub file: String,
    pub intent: String, // change intent description
    pub diff: String,   // unified diff string
    pub confidence: f32,
    #[serde(default)]
    pub depends_on: Vec<String>, // dependent file paths
}

// ── Aggregator types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictStatus {
    Unresolved,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub claim_a_id: String,
    pub claim_b_id: String,
    pub status: ConflictStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeStatus {
    Clean,
    PotentialWriteConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeResult {
    pub file: String,
    pub status: ChangeStatus,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorOutput {
    pub claims: Vec<Claim>,
    pub conflicts: Vec<ConflictEntry>,
    pub file_changes: Vec<FileChangeResult>,
    pub needs_llm_fallback: bool,
    pub unresolved_items: Vec<ConflictEntry>,
}

// ── Parse implementations ──────────────────────────────────────────────

impl ClaimsOutput {
    /// Parse a ClaimsOutput from text, trying multiple extraction strategies.
    pub fn parse(text: &str) -> Result<Self, String> {
        // Level 1: Direct JSON parse
        if let Ok(claims) = serde_json::from_str::<Self>(text) {
            return Ok(claims);
        }

        // Level 2: Extract from markdown code block
        let re = Regex::new(
            r#"```(?:json)?\s*(\{[\s\S]*?"format"\s*:\s*"structured-claims/1"[\s\S]*?\})\s*```"#,
        )
        .map_err(|e| format!("Regex error: {}", e))?;
        if let Some(caps) = re.captures(text) {
            if let Ok(claims) = serde_json::from_str::<Self>(&caps[1]) {
                return Ok(claims);
            }
        }

        // Level 2b: Loose extraction — find any JSON with "claims" array
        let re_loose = Regex::new(r#"\{[\s\S]*?"claims"\s*:\s*\[[\s\S]*?\][\s\S]*?\}"#)
            .map_err(|e| format!("Regex error: {}", e))?;
        if let Some(caps) = re_loose.captures(text) {
            if let Ok(claims) = serde_json::from_str::<Self>(&caps[0]) {
                return Ok(claims);
            }
        }

        // Level 3: Fallback — unstructured
        Ok(ClaimsOutput {
            format: "structured-claims/1".into(),
            claims: vec![Claim {
                id: "unstructured-1".into(),
                claim: text.to_string(),
                evidence: String::new(),
                confidence: 0.5,
                conflicts_with: vec![],
                actionable: false,
                recommendation: None,
            }],
            metadata: Some(ClaimsMetadata {
                parse_method: "unstructured-fallback".into(),
                parse_warning: Some(
                    "Failed to parse structured output; preserving raw text".into(),
                ),
            }),
        })
    }
}

impl DiffOutput {
    /// Parse a DiffOutput from text, trying multiple extraction strategies.
    pub fn parse(text: &str) -> Result<Self, String> {
        // Level 1: Direct JSON parse
        if let Ok(diff) = serde_json::from_str::<Self>(text) {
            return Ok(diff);
        }

        // Level 2: Extract from markdown code block
        let re = Regex::new(
            r#"```(?:json)?\s*(\{[\s\S]*?"format"\s*:\s*"unified-diff/1"[\s\S]*?\})\s*```"#,
        )
        .map_err(|e| format!("Regex error: {}", e))?;
        if let Some(caps) = re.captures(text) {
            if let Ok(diff) = serde_json::from_str::<Self>(&caps[1]) {
                return Ok(diff);
            }
        }

        // Level 3: Fallback
        Ok(DiffOutput {
            format: "unified-diff/1".into(),
            changes: vec![FileChange {
                file: "unknown".into(),
                intent: "Unstructured diff".into(),
                diff: text.to_string(),
                confidence: 0.5,
                depends_on: vec![],
            }],
        })
    }
}

// ── Jaccard similarity ─────────────────────────────────────────────────

/// Compute Jaccard similarity between two strings (token-level).
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let tokenize = |s: &str| -> HashSet<String> {
        s.to_lowercase()
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| t.len() > 1)
            .map(|t| t.to_string())
            .collect()
    };

    let set_a = tokenize(a);
    let set_b = tokenize(b);

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

// ── Aggregator ─────────────────────────────────────────────────────────

pub struct Aggregator;

impl Aggregator {
    /// Merge multiple sub-task results into a consolidated output.
    pub fn merge(results: Vec<(&str, &str)>) -> AggregatorOutput {
        let mut all_claims: Vec<(usize, Claim)> = Vec::new();
        let mut all_changes: Vec<(usize, FileChange)> = Vec::new();

        for (i, (_label, content)) in results.iter().enumerate() {
            // Try claims
            if let Ok(claims_output) = ClaimsOutput::parse(content) {
                for claim in claims_output.claims {
                    all_claims.push((i, claim));
                }
            }

            // Try diff
            if let Ok(diff_output) = DiffOutput::parse(content) {
                for change in diff_output.changes {
                    all_changes.push((i, change));
                }
            }
        }

        let claims: Vec<Claim> = all_claims.into_iter().map(|(_, c)| c).collect();
        let changes: Vec<FileChange> = all_changes.into_iter().map(|(_, c)| c).collect();

        // Deduplicate claims
        let deduped = Self::deduplicate_claims(claims, 0.8);

        // Detect conflicts
        let conflicts = Self::detect_conflicts(&deduped);

        // Merge file changes
        let file_changes = Self::merge_file_changes(changes);

        let unresolved: Vec<ConflictEntry> = conflicts
            .iter()
            .filter(|c| matches!(c.status, ConflictStatus::Unresolved))
            .cloned()
            .collect();

        AggregatorOutput {
            claims: deduped,
            conflicts,
            file_changes,
            needs_llm_fallback: !unresolved.is_empty(),
            unresolved_items: unresolved,
        }
    }

    /// Deduplicate claims using Jaccard similarity.
    fn deduplicate_claims(claims: Vec<Claim>, threshold: f64) -> Vec<Claim> {
        let mut result: Vec<Claim> = Vec::new();
        let mut merged_ids: HashSet<String> = HashSet::new();

        for i in 0..claims.len() {
            if merged_ids.contains(&claims[i].id) {
                continue;
            }
            let mut claim = claims[i].clone();

            for j in (i + 1)..claims.len() {
                if merged_ids.contains(&claims[j].id) {
                    continue;
                }
                if jaccard_similarity(&claim.claim, &claims[j].claim) > threshold {
                    // Merge: keep higher confidence, concatenate evidence
                    if claims[j].confidence > claim.confidence {
                        claim.confidence = claims[j].confidence;
                    }
                    claim.evidence = format!("{}; {}", claim.evidence, claims[j].evidence);
                    merged_ids.insert(claims[j].id.clone());
                }
            }
            result.push(claim);
        }
        result
    }

    /// Detect conflicts between claims based on conflicts_with references.
    fn detect_conflicts(claims: &[Claim]) -> Vec<ConflictEntry> {
        let mut conflicts = Vec::new();
        let claim_map: HashMap<&str, &Claim> = claims.iter().map(|c| (c.id.as_str(), c)).collect();

        for claim in claims {
            for conflict_id in &claim.conflicts_with {
                if let Some(target) = claim_map.get(conflict_id.as_str()) {
                    conflicts.push(ConflictEntry {
                        claim_a_id: claim.id.clone(),
                        claim_b_id: target.id.clone(),
                        status: ConflictStatus::Unresolved,
                    });
                }
            }
        }
        conflicts
    }

    /// Merge file changes, detecting write conflicts.
    fn merge_file_changes(changes: Vec<FileChange>) -> Vec<FileChangeResult> {
        let mut by_file: HashMap<String, Vec<FileChange>> = HashMap::new();
        for change in changes {
            by_file.entry(change.file.clone()).or_default().push(change);
        }

        by_file
            .into_iter()
            .map(|(file, file_changes)| {
                if file_changes.len() > 1 {
                    FileChangeResult {
                        file,
                        status: ChangeStatus::PotentialWriteConflict,
                        changes: file_changes,
                    }
                } else {
                    FileChangeResult {
                        file,
                        status: ChangeStatus::Clean,
                        changes: file_changes,
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claims_direct_json() {
        let json = r#"{"format":"structured-claims/1","claims":[{"id":"c1","claim":"Auth uses JWT","evidence":"Found in src/auth.rs","confidence":0.9,"conflicts_with":[],"actionable":true}]}"#;
        let parsed = ClaimsOutput::parse(json).unwrap();
        assert_eq!(parsed.claims.len(), 1);
        assert_eq!(parsed.claims[0].id, "c1");
        assert!((parsed.claims[0].confidence - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_parse_claims_code_block() {
        let text = "Here is my analysis:\n```json\n{\"format\":\"structured-claims/1\",\"claims\":[{\"id\":\"c1\",\"claim\":\"test\",\"evidence\":\"\",\"confidence\":0.8,\"conflicts_with\":[],\"actionable\":false}]}\n```\nDone.";
        let parsed = ClaimsOutput::parse(text).unwrap();
        assert_eq!(parsed.claims.len(), 1);
    }

    #[test]
    fn test_parse_claims_fallback() {
        let text = "Just plain text analysis without JSON structure.";
        let parsed = ClaimsOutput::parse(text).unwrap();
        assert_eq!(parsed.claims[0].id, "unstructured-1");
    }

    #[test]
    fn test_diff_output_parse() {
        let json = r#"{"format":"unified-diff/1","changes":[{"file":"src/main.rs","intent":"Add logging","diff":"@@ -1,3 +1,4 @@","confidence":0.9,"depends_on":[]}]}"#;
        let parsed = DiffOutput::parse(json).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].file, "src/main.rs");
    }

    #[test]
    fn test_jaccard_identical() {
        let sim = jaccard_similarity(
            "The quick brown fox jumps over the lazy dog",
            "The quick brown fox jumps over the lazy dog",
        );
        assert!((sim - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let sim = jaccard_similarity("aaaa bbbb cccc dddd eeee", "ffff gggg hhhh iiii jjjj");
        assert!((sim - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_jaccard_partial() {
        let sim = jaccard_similarity("The quick brown fox", "The slow brown dog");
        assert!(sim > 0.0 && sim < 1.0);
    }

    #[test]
    fn test_jaccard_empty() {
        let sim = jaccard_similarity("", "");
        assert!((sim - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_deduplicate_claims() {
        let claims = vec![
            Claim {
                id: "c1".into(),
                claim: "System uses JWT authentication for user login".into(),
                evidence: "file1".into(),
                confidence: 0.8,
                conflicts_with: vec![],
                actionable: false,
                recommendation: None,
            },
            Claim {
                id: "c2".into(),
                claim: "System uses JWT authentication for user login verification".into(),
                evidence: "file2".into(),
                confidence: 0.9,
                conflicts_with: vec![],
                actionable: false,
                recommendation: None,
            },
            Claim {
                id: "c3".into(),
                claim: "Database is PostgreSQL".into(),
                evidence: "file3".into(),
                confidence: 0.7,
                conflicts_with: vec![],
                actionable: false,
                recommendation: None,
            },
        ];
        let deduped = Aggregator::deduplicate_claims(claims, 0.8);
        assert_eq!(deduped.len(), 2); // c1 and c2 merged, c3 stays
    }

    #[test]
    fn test_detect_conflicts() {
        let claims = vec![
            Claim {
                id: "c1".into(),
                claim: "Use library A".into(),
                evidence: "".into(),
                confidence: 0.9,
                conflicts_with: vec!["c2".into()],
                actionable: true,
                recommendation: None,
            },
            Claim {
                id: "c2".into(),
                claim: "Use library B".into(),
                evidence: "".into(),
                confidence: 0.7,
                conflicts_with: vec!["c1".into()],
                actionable: true,
                recommendation: None,
            },
        ];
        let conflicts = Aggregator::detect_conflicts(&claims);
        assert_eq!(conflicts.len(), 2); // both directions
    }

    #[test]
    fn test_merge_file_changes_single() {
        let changes = vec![FileChange {
            file: "src/main.rs".into(),
            intent: "fix".into(),
            diff: "".into(),
            confidence: 0.9,
            depends_on: vec![],
        }];
        let merged = Aggregator::merge_file_changes(changes);
        assert_eq!(merged.len(), 1);
        assert!(matches!(merged[0].status, ChangeStatus::Clean));
    }

    #[test]
    fn test_merge_file_changes_conflict() {
        let changes = vec![
            FileChange {
                file: "src/main.rs".into(),
                intent: "fix".into(),
                diff: "diff1".into(),
                confidence: 0.9,
                depends_on: vec![],
            },
            FileChange {
                file: "src/main.rs".into(),
                intent: "refactor".into(),
                diff: "diff2".into(),
                confidence: 0.8,
                depends_on: vec![],
            },
        ];
        let merged = Aggregator::merge_file_changes(changes);
        assert_eq!(merged.len(), 1);
        assert!(matches!(
            merged[0].status,
            ChangeStatus::PotentialWriteConflict
        ));
    }
}
