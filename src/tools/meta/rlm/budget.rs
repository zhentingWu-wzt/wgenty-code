//! Token budget allocation for RLM pipeline phases.

/// Minimum token budget per sub-task in thousands (800k tokens = 800 in budget units).
/// Each RLM sub-task receives at least this many tokens, regardless of how the
/// executor pool is subdivided.
pub const MIN_PER_TASK_BUDGET_K: u64 = 800;

/// Budget allocation across RLM pipeline phases.
#[derive(Debug, Clone)]
pub struct BudgetAllocation {
    pub total: u64,         // total budget in thousands
    pub planner: u64,       // 10%
    pub executor_pool: u64, // 80%
    pub aggregator: u64,    // 10%
}

impl BudgetAllocation {
    /// Create a new allocation from total budget in thousands.
    pub fn new(total_k: u64) -> Self {
        Self {
            total: total_k,
            planner: total_k / 10,
            executor_pool: total_k * 8 / 10,
            aggregator: total_k / 10,
        }
    }

    /// Distribute executor pool across individual tasks.
    ///
    /// Each sub-task receives at least [`MIN_PER_TASK_BUDGET_K`] tokens (800k).
    /// If the executor pool is too small to satisfy this minimum for all tasks,
    /// the per-task allocation is still boosted to the minimum — the intent is
    /// that sub-tasks always have enough tokens to be useful.
    pub fn distribute_to_tasks(&self, task_count: usize) -> Vec<u64> {
        if task_count == 0 {
            return vec![];
        }
        let fair_per_task = self.executor_pool / task_count as u64;
        // Ensure each sub-task gets at least MIN_PER_TASK_BUDGET_K tokens.
        let per_task = fair_per_task.max(MIN_PER_TASK_BUDGET_K);
        if per_task > fair_per_task {
            tracing::info!(
                target: "rlm",
                "RLM budget: per-task budget boosted from {}k to {}k (minimum = {}k)",
                fair_per_task,
                per_task,
                MIN_PER_TASK_BUDGET_K
            );
        }
        vec![per_task; task_count]
    }

    /// Roll over unused budget from one phase to the next.
    pub fn rollover_unused(&mut self, phase: &str, unused: u64) {
        match phase {
            "planner" => self.executor_pool += unused,
            "executor" => self.aggregator += unused,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_100k() {
        let a = BudgetAllocation::new(100);
        assert_eq!(a.total, 100);
        assert_eq!(a.planner, 10);
        assert_eq!(a.executor_pool, 80);
        assert_eq!(a.aggregator, 10);
    }

    #[test]
    fn test_distribute_to_tasks() {
        // With 100k total, executor_pool = 80k. 4 tasks: 80/4 = 20k each,
        // but MIN_PER_TASK_BUDGET_K = 800k, so each task gets 800k.
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(4);
        assert_eq!(dist.len(), 4);
        assert_eq!(dist[0], MIN_PER_TASK_BUDGET_K);
    }

    #[test]
    fn test_distribute_large_budget() {
        // With a large enough budget, division gives more than minimum.
        // total = 50_000k, executor_pool = 40_000k. 4 tasks: 40_000/4 = 10_000k each.
        let a = BudgetAllocation::new(50_000);
        let dist = a.distribute_to_tasks(4);
        assert_eq!(dist.len(), 4);
        assert_eq!(dist[0], 10_000); // 40_000 / 4 = 10_000 > 800
    }

    #[test]
    fn test_distribute_zero_tasks() {
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(0);
        assert!(dist.is_empty());
    }

    #[test]
    fn test_rollover_unused_planner() {
        let mut a = BudgetAllocation::new(100);
        a.rollover_unused("planner", 5);
        assert_eq!(a.executor_pool, 85); // 80 + 5 unused
        assert_eq!(a.aggregator, 10);
    }

    #[test]
    fn test_rollover_unused_executor() {
        let mut a = BudgetAllocation::new(100);
        a.rollover_unused("executor", 15);
        assert_eq!(a.aggregator, 25); // 10 + 15 unused
    }

    #[test]
    fn test_small_budget() {
        let a = BudgetAllocation::new(1); // 1k tokens
        assert_eq!(a.planner, 0); // integer division: 1/10 = 0
        assert_eq!(a.executor_pool, 0);
        // 0/1 = 0, but boosted to MIN_PER_TASK_BUDGET_K
        let dist = a.distribute_to_tasks(1);
        assert_eq!(dist[0], MIN_PER_TASK_BUDGET_K);
    }

    #[test]
    fn test_min_per_task_budget() {
        // With a budget of 100k total (80k executor), 5 tasks would normally
        // get 16k each. The minimum of 800k kicks in.
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(5);
        assert_eq!(dist.len(), 5);
        assert_eq!(dist[0], MIN_PER_TASK_BUDGET_K);
        assert_eq!(dist[4], MIN_PER_TASK_BUDGET_K);
    }
}
