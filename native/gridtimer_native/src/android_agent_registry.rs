use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_TASK_RECORDS: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AgentRegistryError {
    InvalidTaskId,
    DuplicateTask,
    CapacityReached,
}

struct AgentTaskRecord {
    cancelled: Arc<AtomicBool>,
    registered: bool,
    running: bool,
}

pub(crate) struct AgentRunRegistry {
    records: HashMap<String, AgentTaskRecord>,
    capacity: usize,
    cancellation_overflow: bool,
}

impl Default for AgentRunRegistry {
    fn default() -> Self {
        Self::with_capacity(MAX_TASK_RECORDS)
    }
}

impl AgentRunRegistry {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            records: HashMap::new(),
            capacity,
            cancellation_overflow: false,
        }
    }

    pub(crate) fn register(
        &mut self,
        task_id: &str,
    ) -> Result<Arc<AtomicBool>, AgentRegistryError> {
        if !valid_task_id(task_id) {
            return Err(AgentRegistryError::InvalidTaskId);
        }
        if self.cancellation_overflow {
            return Err(AgentRegistryError::CapacityReached);
        }
        if let Some(record) = self.records.get_mut(task_id) {
            if record.registered {
                return Err(AgentRegistryError::DuplicateTask);
            }
            record.registered = true;
            record.running = true;
            return Ok(Arc::clone(&record.cancelled));
        }
        if self.records.len() >= self.capacity {
            return Err(AgentRegistryError::CapacityReached);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        self.records.insert(
            task_id.to_string(),
            AgentTaskRecord {
                cancelled: Arc::clone(&cancelled),
                registered: true,
                running: true,
            },
        );
        Ok(cancelled)
    }

    pub(crate) fn cancel(&mut self, task_id: &str) -> Result<(), AgentRegistryError> {
        if !valid_task_id(task_id) {
            return Err(AgentRegistryError::InvalidTaskId);
        }
        if let Some(record) = self.records.get(task_id) {
            record.cancelled.store(true, Ordering::Release);
            return Ok(());
        }
        if self.records.len() >= self.capacity || self.cancellation_overflow {
            // A missing tombstone must never allow a cancelled task to start later.
            // Fail closed for this process rather than evicting cancellation records.
            self.cancellation_overflow = true;
            return Err(AgentRegistryError::CapacityReached);
        }
        self.records.insert(
            task_id.to_string(),
            AgentTaskRecord {
                cancelled: Arc::new(AtomicBool::new(true)),
                registered: false,
                running: false,
            },
        );
        Ok(())
    }

    pub(crate) fn complete(&mut self, task_id: &str, expected: &Arc<AtomicBool>) -> bool {
        let Some(record) = self.records.get_mut(task_id) else {
            return false;
        };
        if !Arc::ptr_eq(&record.cancelled, expected) {
            return false;
        }
        record.running = false;
        true
    }

    pub(crate) fn finish(&mut self, task_id: &str) -> bool {
        if !valid_task_id(task_id) {
            return false;
        }
        if self
            .records
            .get(task_id)
            .is_some_and(|record| record.running)
        {
            return false;
        }
        self.records.remove(task_id);
        true
    }
}

fn valid_task_id(task_id: &str) -> bool {
    !task_id.trim().is_empty() && task_id.len() <= 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_before_registration_is_preserved_until_the_task_finishes() {
        let mut registry = AgentRunRegistry::with_capacity(2);
        registry.cancel("task").unwrap();
        let flag = registry.register("task").unwrap();
        assert!(flag.load(Ordering::Acquire));
        assert!(!registry.finish("task"));
        assert!(registry.complete("task", &flag));
        assert!(registry.finish("task"));
        assert!(registry.records.is_empty());
    }

    #[test]
    fn cancel_after_registration_updates_the_existing_run_flag() {
        let mut registry = AgentRunRegistry::with_capacity(2);
        let flag = registry.register("task").unwrap();
        registry.cancel("task").unwrap();
        assert!(flag.load(Ordering::Acquire));
    }

    #[test]
    fn duplicate_registration_is_rejected_until_explicit_finish() {
        let mut registry = AgentRunRegistry::with_capacity(2);
        let flag = registry.register("task").unwrap();
        assert!(matches!(
            registry.register("task"),
            Err(AgentRegistryError::DuplicateTask)
        ));
        registry.complete("task", &flag);
        assert!(matches!(
            registry.register("task"),
            Err(AgentRegistryError::DuplicateTask)
        ));
        registry.finish("task");
        assert!(!registry.register("task").unwrap().load(Ordering::Acquire));
    }

    #[test]
    fn old_guard_cannot_complete_or_release_a_new_run_with_the_same_id() {
        let mut registry = AgentRunRegistry::with_capacity(2);
        let old = registry.register("task").unwrap();
        registry.complete("task", &old);
        registry.finish("task");
        let current = registry.register("task").unwrap();
        assert!(!registry.complete("task", &old));
        assert!(!registry.finish("task"));
        registry.cancel("task").unwrap();
        assert!(current.load(Ordering::Acquire));
    }

    #[test]
    fn full_registry_rejects_new_runs_and_preserves_existing_cancelled_tasks() {
        let mut registry = AgentRunRegistry::with_capacity(1);
        registry.cancel("cancelled").unwrap();
        assert!(matches!(
            registry.register("other"),
            Err(AgentRegistryError::CapacityReached)
        ));
        assert_eq!(registry.records.len(), 1);
        assert!(registry
            .register("cancelled")
            .unwrap()
            .load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_overflow_stays_bounded_and_cannot_start_after_capacity_is_freed() {
        let mut registry = AgentRunRegistry::with_capacity(1);
        registry.cancel("first").unwrap();
        assert_eq!(
            registry.cancel("overflow"),
            Err(AgentRegistryError::CapacityReached)
        );
        assert_eq!(registry.records.len(), 1);
        registry.finish("first");
        assert!(matches!(
            registry.register("overflow"),
            Err(AgentRegistryError::CapacityReached)
        ));
        assert!(registry.records.is_empty());
    }

    #[test]
    fn unstarted_and_invalid_tasks_are_finished_without_dropping_active_runs() {
        let mut registry = AgentRunRegistry::with_capacity(1);
        assert_eq!(registry.cancel(" "), Err(AgentRegistryError::InvalidTaskId));
        assert!(matches!(
            registry.register(&"x".repeat(101)),
            Err(AgentRegistryError::InvalidTaskId)
        ));
        assert!(registry.records.is_empty());
        registry.cancel("unstarted").unwrap();
        assert!(registry.finish("unstarted"));
        assert!(registry.finish("missing"));
        assert!(registry.register("active").is_ok());
        assert!(!registry.finish("active"));
    }
}
