//! Per-project [`MemoryManager`] routing (multi-project).
//!
//! The daemon's main project keeps the `MemoryManager` built at startup;
//! every registered project lazily gets its own manager rooted at its own
//! directory (`<project>/.wgenty-code/memory/`). The tier-2 review LLM is
//! shared across all instances.

use crate::context::consolidation::MemoryReviewLlm;
use crate::context::{MemoryManager, MemoryResolver};
use crate::daemon::projects::ProjectRegistry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MemoryRouter {
    settings: crate::config::Settings,
    registry: ProjectRegistry,
    main: Arc<MemoryManager>,
    managers: RwLock<HashMap<PathBuf, Arc<MemoryManager>>>,
    review_llm: RwLock<Option<Arc<dyn MemoryReviewLlm>>>,
}

impl MemoryRouter {
    pub fn new(
        settings: crate::config::Settings,
        registry: ProjectRegistry,
        main: Arc<MemoryManager>,
    ) -> Self {
        Self {
            settings,
            registry,
            main,
            managers: RwLock::new(HashMap::new()),
            review_llm: RwLock::new(None),
        }
    }

    /// The main project's manager (the one built at daemon startup).
    pub fn main(&self) -> Arc<MemoryManager> {
        self.main.clone()
    }

    /// Share the tier-2 review LLM with the main manager, every cached
    /// project manager, and all future lazily-created ones.
    pub async fn set_review_llm(&self, llm: Option<Arc<dyn MemoryReviewLlm>>) {
        self.main.set_review_llm(llm.clone()).await;
        let cached: Vec<Arc<MemoryManager>> =
            self.managers.read().await.values().cloned().collect();
        for m in cached {
            m.set_review_llm(llm.clone()).await;
        }
        *self.review_llm.write().await = llm;
    }

    /// Manager for an exact project root (get-or-create).
    pub async fn for_project(&self, root: &Path) -> Arc<MemoryManager> {
        if *root == self.registry.main_root() {
            return self.main.clone();
        }
        if let Some(m) = self.managers.read().await.get(root) {
            return m.clone();
        }
        let mgr = Arc::new(MemoryManager::with_settings(
            &self.settings,
            root.to_path_buf(),
        ));
        if let Some(llm) = self.review_llm.read().await.clone() {
            mgr.set_review_llm(Some(llm)).await;
        }
        let mut map = self.managers.write().await;
        // Double-check: a concurrent creator may have won the race.
        map.entry(root.to_path_buf()).or_insert(mgr).clone()
    }

    /// Main manager plus one per registered project (creating as needed) —
    /// used by startup fan-out tasks (AutoDream).
    pub async fn all(&self) -> Vec<Arc<MemoryManager>> {
        let mut out = vec![self.main.clone()];
        for root in self.registry.registered_roots() {
            out.push(self.for_project(&root).await);
        }
        out
    }
}

#[async_trait::async_trait]
impl MemoryResolver for MemoryRouter {
    /// Route a tool invocation to its project's pool: the longest known
    /// project root containing the workdir wins (so a session bound to a
    /// worktree writes to the worktree's project, not a worktree-local pool);
    /// unknown roots fall back to the main project.
    async fn resolve(&self, workdir: Option<&Path>) -> Arc<MemoryManager> {
        let Some(wd) = workdir else {
            return self.main.clone();
        };
        let wd = wd.canonicalize().unwrap_or_else(|_| wd.to_path_buf());
        let mut best: Option<PathBuf> = None;
        for root in
            std::iter::once(self.registry.main_root()).chain(self.registry.registered_roots())
        {
            let longer = best
                .as_ref()
                .is_none_or(|b| root.as_os_str().len() > b.as_os_str().len());
            if wd.starts_with(&root) && longer {
                best = Some(root);
            }
        }
        match best {
            Some(root) => self.for_project(&root).await,
            None => self.main.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, MemoryRouter) {
        let main_dir = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let registry = ProjectRegistry::load(
            main_dir.path().to_path_buf(),
            store.path().join("projects.json"),
        );
        let settings = crate::config::Settings::default();
        let main = Arc::new(MemoryManager::new(main_dir.path().to_path_buf()));
        let router = MemoryRouter::new(settings, registry, main);
        (main_dir, store, router)
    }

    #[tokio::test]
    async fn routes_to_owning_project_and_shares_instances() {
        let (main_dir, _store, router) = setup();
        let proj = tempfile::tempdir().unwrap();
        let proj_canon = proj.path().canonicalize().unwrap();
        router.registry.add(proj.path().to_str().unwrap()).unwrap();

        // Workdir inside the project (e.g. a worktree) routes to the project.
        let nested = proj_canon.join(".worktrees").join("feat");
        std::fs::create_dir_all(&nested).unwrap();
        let a = router.resolve(Some(&nested)).await;
        let b = router.for_project(&proj_canon).await;
        assert!(Arc::ptr_eq(&a, &b), "same project must share one manager");

        // Main root and unknown roots use the main manager.
        let main = router.resolve(Some(main_dir.path())).await;
        assert!(Arc::ptr_eq(&main, &router.main()));
        let outside = router.resolve(Some(Path::new("/tmp"))).await;
        assert!(Arc::ptr_eq(&outside, &router.main()));

        // No workdir → main.
        let none = router.resolve(None).await;
        assert!(Arc::ptr_eq(&none, &router.main()));
    }
}
