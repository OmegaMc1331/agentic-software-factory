use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::RuntimeError;

pub struct RuntimeBudget {
    pub(crate) tasks: Arc<factory_core::TaskBudget>,
    operations: Arc<Counter>,
    terminals: Arc<Counter>,
    checkpoints_deferred: AtomicBool,
}

struct Counter {
    name: &'static str,
    maximum: usize,
    used: AtomicUsize,
    closed: AtomicBool,
}

pub struct ResourcePermit {
    counter: Arc<Counter>,
}

impl RuntimeBudget {
    pub fn new(operations: usize, terminals: usize) -> Self {
        Self::with_task_limit(operations, terminals, 8)
    }

    pub fn with_task_limit(operations: usize, terminals: usize, tasks: usize) -> Self {
        Self {
            tasks: Arc::new(factory_core::TaskBudget::new(tasks.clamp(1, 32))),
            operations: Arc::new(Counter::new("operations", operations)),
            terminals: Arc::new(Counter::new("terminals", terminals)),
            checkpoints_deferred: AtomicBool::new(false),
        }
    }

    pub fn operation(&self) -> Result<ResourcePermit, RuntimeError> {
        self.operations.acquire()
    }

    pub(crate) fn terminal(&self) -> Result<ResourcePermit, RuntimeError> {
        self.terminals.acquire()
    }

    pub fn usage(&self) -> (usize, usize) {
        (
            self.operations.used.load(Ordering::Acquire),
            self.terminals.used.load(Ordering::Acquire),
        )
    }

    pub fn close(&self) {
        self.operations.closed.store(true, Ordering::Release);
        self.terminals.closed.store(true, Ordering::Release);
    }

    pub fn begin_shutdown(&self) {
        self.checkpoints_deferred.store(true, Ordering::Release);
        self.close();
    }

    pub fn allow_shutdown_checkpoints(&self) {
        self.checkpoints_deferred.store(false, Ordering::Release);
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.operations.closed.load(Ordering::Acquire)
    }

    pub(crate) fn checkpoints_deferred(&self) -> bool {
        self.checkpoints_deferred.load(Ordering::Acquire)
    }
}

impl Default for RuntimeBudget {
    fn default() -> Self {
        Self::new(8, 8)
    }
}

impl Counter {
    fn new(name: &'static str, maximum: usize) -> Self {
        Self {
            name,
            maximum,
            used: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        }
    }

    fn acquire(self: &Arc<Self>) -> Result<ResourcePermit, RuntimeError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RuntimeError::ResourceLimit("application shutting down"));
        }
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < self.maximum).then_some(used + 1)
            })
            .map_err(|_| RuntimeError::ResourceLimit(self.name))?;
        let permit = ResourcePermit {
            counter: self.clone(),
        };
        if self.closed.load(Ordering::Acquire) {
            return Err(RuntimeError::ResourceLimit("application shutting down"));
        }
        Ok(permit)
    }
}

impl Drop for ResourcePermit {
    fn drop(&mut self) {
        self.counter.used.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ResourcePermit {
    pub fn cancellation(&self) -> &AtomicBool {
        &self.counter.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_closes_new_admission_without_releasing_owned_permits() {
        let budget = RuntimeBudget::new(2, 2);
        let operation = budget.operation().unwrap();
        let terminal = budget.terminal().unwrap();
        budget.close();
        budget.close();
        assert!(budget.operation().is_err());
        assert!(budget.terminal().is_err());
        assert_eq!(budget.usage(), (1, 1));
        drop(operation);
        drop(terminal);
        assert_eq!(budget.usage(), (0, 0));
        assert!(budget.operation().is_err());
    }

    #[test]
    fn shared_budget_is_atomic_and_released_by_ownership() {
        let budget = Arc::new(RuntimeBudget::new(2, 1));
        let first = budget.operation().unwrap();
        let second = budget.operation().unwrap();
        assert!(matches!(
            budget.operation(),
            Err(RuntimeError::ResourceLimit("operations"))
        ));
        let terminal = budget.terminal().unwrap();
        assert!(budget.terminal().is_err());
        drop(first);
        drop(second);
        drop(terminal);
        assert_eq!(budget.usage(), (0, 0));
    }
}
