/// Default helper for serde: returns true.
pub(super) fn default_rlm_max_replan() -> usize {
    2
}

pub(super) fn default_rlm_jaccard_threshold() -> f64 {
    0.8
}

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_max_transcript_age_days() -> u32 {
    30
}

pub(super) fn default_transcript_db_path() -> String {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    format!("{}/.wgenty-code/subagent_transcripts.db", home)
}
