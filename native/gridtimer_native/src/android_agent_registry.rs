use crate::android_agent_progress::{AgentProgress, AgentTerminal};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;
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
    progress: Arc<AgentProgress>,
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
                progress: Arc::new(AgentProgress::new(task_id, Arc::clone(&cancelled))),
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
            record.progress.request_cancel();
            return Ok(());
        }
        if self.records.len() >= self.capacity || self.cancellation_overflow {
            // A missing tombstone must never allow a cancelled task to start later.
            // Fail closed for this process rather than evicting cancellation records.
            self.cancellation_overflow = true;
            return Err(AgentRegistryError::CapacityReached);
        }
        let cancelled = Arc::new(AtomicBool::new(true));
        self.records.insert(
            task_id.to_string(),
            AgentTaskRecord {
                cancelled: Arc::clone(&cancelled),
                progress: Arc::new(AgentProgress::new(task_id, cancelled)),
                registered: false,
                running: false,
            },
        );
        Ok(())
    }

    pub(crate) fn progress(&self, task_id: &str) -> Option<Arc<AgentProgress>> {
        self.records
            .get(task_id)
            .map(|record| Arc::clone(&record.progress))
    }

    pub(crate) fn progress_for_run(
        &self,
        task_id: &str,
        expected: &Arc<AtomicBool>,
    ) -> Option<Arc<AgentProgress>> {
        let record = self.records.get(task_id)?;
        Arc::ptr_eq(&record.cancelled, expected).then(|| Arc::clone(&record.progress))
    }

    pub(crate) fn complete(&mut self, task_id: &str, expected: &Arc<AtomicBool>) -> bool {
        let Some(record) = self.records.get_mut(task_id) else {
            return false;
        };
        if !Arc::ptr_eq(&record.cancelled, expected) {
            return false;
        }
        record.progress.finish(AgentTerminal::Failed);
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
    #[test]
    fn progress_queries_do_not_register_tasks_and_old_instances_cannot_update_new_runs() {
        use crate::android_agent_progress::AgentProgressEvent;
        let mut registry = AgentRunRegistry::with_capacity(1);
        assert!(registry.progress("missing").is_none());
        assert!(registry.records.is_empty());
        registry.cancel("task").unwrap();
        let old_flag = registry.register("task").unwrap();
        let old = registry.progress_for_run("task", &old_flag).unwrap();
        let waiting = serde_json::to_value(old.snapshot()).unwrap();
        assert_eq!(waiting["cancelRequested"], true);
        assert_eq!(waiting["requests"], 0);
        assert!(registry.complete("task", &old_flag));
        assert!(registry.finish("task"));
        let current_flag = registry.register("task").unwrap();
        let current = registry.progress_for_run("task", &current_flag).unwrap();
        assert!(registry.progress_for_run("task", &old_flag).is_none());
        assert!(!Arc::ptr_eq(&old, &current));
        assert!(!old.apply(AgentProgressEvent::RequestStarted));
        assert!(!registry.complete("task", &old_flag));
        let current_snapshot = serde_json::to_value(current.snapshot()).unwrap();
        assert_eq!(current_snapshot["requests"], 0);
        assert_eq!(current_snapshot["cancelRequested"], false);
        assert_eq!(current_snapshot["terminal"], "running");
    }

    #[test]
    fn progress_late_cancel_does_not_overwrite_a_completed_terminal() {
        let mut registry = AgentRunRegistry::with_capacity(1);
        let flag = registry.register("task").unwrap();
        let progress = registry.progress_for_run("task", &flag).unwrap();
        assert_eq!(progress.finish(AgentTerminal::Ready), AgentTerminal::Ready);
        registry.cancel("task").unwrap();
        assert!(!flag.load(Ordering::Acquire));
        assert!(registry.complete("task", &flag));
        let snapshot = serde_json::to_value(registry.progress("task").unwrap().snapshot()).unwrap();
        assert_eq!(snapshot["terminal"], "ready");
        assert_eq!(snapshot["requestInFlight"], false);
        assert_eq!(snapshot["cancelRequested"], false);
    }
}
