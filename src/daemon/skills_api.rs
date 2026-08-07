//! Skills list endpoint for the web command center's SkillPanel (read-only).

use crate::daemon::state::DaemonState;
use crate::knowledge::loader::SkillLoader;
use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub source_path: String,
}

/// Map the loader's skills into response entries, sorted by name for a stable
/// panel display. (`skill_names` + `load_skill` are the loader's only public
/// accessors; every name resolves, so the filter_map never drops in practice.)
pub(crate) fn collect_skills(loader: &SkillLoader) -> Vec<SkillEntry> {
    let mut out: Vec<SkillEntry> = loader
        .skill_names()
        .into_iter()
        .filter_map(|n| loader.load_skill(&n))
        .map(|s| SkillEntry {
            name: s.name.clone(),
            description: s.description.clone(),
            source_path: s.source_path.display().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// GET /api/v1/skills — list skills visible to the daemon (read-only).
pub async fn list_skills(State(state): State<Arc<DaemonState>>) -> Json<Vec<SkillEntry>> {
    Json(collect_skills(&state.skill_loader))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_and_sorts_skills() {
        let dir = tempfile::tempdir().unwrap();
        for (name, desc) in [("zeta", "last"), ("alpha", "first")] {
            let skill_dir = dir.path().join("skills").join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
            )
            .unwrap();
        }
        let loader = SkillLoader::load_from_dir(dir.path());
        let entries = collect_skills(&loader);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[0].description, "first");
        assert!(entries[0].source_path.ends_with("SKILL.md"));
    }
}
