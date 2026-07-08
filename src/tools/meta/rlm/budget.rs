//! Token budget allocation for RLM pipeline phases.

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
    /// Each sub-task receives an equal share of the executor pool
    /// (`executor_pool / task_count`). Returns an empty vec when `task_count`
    /// is zero.
    pub fn distribute_to_tasks(&self, task_count: usize) -> Vec<u64> {
        if task_count == 0 {
            return vec![];
        }
        let per_task = self.executor_pool / task_count as u64;
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
        // With 100k total, executor_pool = 80k. 4 tasks: 80/4 = 20k each.
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(4);
        assert_eq!(dist.len(), 4);
        assert_eq!(dist[0], 20);
    }

    #[test]
    fn test_distribute_large_budget() {
        // total = 50_000k, executor_pool = 40_000k. 4 tasks: 40_000/4 = 10_000k each.
        let a = BudgetAllocation::new(50_000);
        let dist = a.distribute_to_tasks(4);
        assert_eq!(dist.len(), 4);
        assert_eq!(dist[0], 10_000);
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
        // 1k total -> executor_pool = 0 (integer division). 1 task: 0/1 = 0.
        let a = BudgetAllocation::new(1);
        assert_eq!(a.planner, 0);
        assert_eq!(a.executor_pool, 0);
        let dist = a.distribute_to_tasks(1);
        assert_eq!(dist[0], 0);
    }

    #[test]
    fn test_distribute_even_split() {
        // 100k total (80k executor), 5 tasks: 80/5 = 16k each.
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(5);
        assert_eq!(dist.len(), 5);
        assert_eq!(dist[0], 16);
        assert_eq!(dist[4], 16);
    }
}
