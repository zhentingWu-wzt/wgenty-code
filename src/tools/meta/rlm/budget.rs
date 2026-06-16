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
        let a = BudgetAllocation::new(100);
        let dist = a.distribute_to_tasks(4);
        assert_eq!(dist.len(), 4);
        assert_eq!(dist[0], 20); // 80/4 = 20
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
        assert_eq!(a.aggregator, 0);
    }
}
