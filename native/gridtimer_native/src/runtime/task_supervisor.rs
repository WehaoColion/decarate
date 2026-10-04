// v2.22.56 - Propagate scoped cancellation to active network operations.
// v2.22.25 - Supervise desktop worker threads and bounded application shutdown.

pub use super::cancellation::CancellationToken;
use std::collections::BTreeMap;
use std::io;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskKind {
    Sync,
    MediaSync,
    TokenRevoke,
    AiRequest,
    KnowledgeAiRequest,
    SpeechTranscription,
    HealthCheck,
    ServiceLaunch,
    TimerBell,
    LegalPreparation,
    LegalScan,
    LegalStore,
    LegalSync,
}

impl TaskKind {
    fn thread_name(self) -> &'static str {
        match self {
            Self::Sync => "desktop-sync",
            Self::MediaSync => "desktop-media-sync",
            Self::TokenRevoke => "desktop-token-revoke",
            Self::AiRequest => "desktop-ai",
            Self::KnowledgeAiRequest => "desktop-knowledge-ai",
            Self::SpeechTranscription => "desktop-speech-transcription",
            Self::HealthCheck => "desktop-health",
            Self::ServiceLaunch => "desktop-service-launch",
            Self::TimerBell => "desktop-timer-bell",
            Self::LegalPreparation => "desktop-legal-prepare",
            Self::LegalScan => "desktop-legal-scan",
            Self::LegalStore => "desktop-legal-store",
            Self::LegalSync => "desktop-legal-sync",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskDurability {
    Ephemeral,
    Recoverable,
    CommitSensitive,
}

struct ManagedTask {
    kind: TaskKind,
    durability: TaskDurability,
    cancellation: CancellationToken,
    handle: JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinishedTask {
    pub id: TaskId,
    pub kind: TaskKind,
    pub durability: TaskDurability,
    pub panicked: bool,
}

pub struct TaskSupervisor {
    next_id: u64,
    accepting: bool,
    tasks: BTreeMap<TaskId, ManagedTask>,
}

impl Default for TaskSupervisor {
    fn default() -> Self {
        Self {
            next_id: 1,
            accepting: true,
            tasks: BTreeMap::new(),
        }
    }
}

impl TaskSupervisor {
    pub fn spawn(
        &mut self,
        kind: TaskKind,
        durability: TaskDurability,
        operation: impl FnOnce(CancellationToken) + Send + 'static,
    ) -> io::Result<TaskId> {
        if !self.accepting {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "application shutdown is already in progress",
            ));
        }
        let id = TaskId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let handle = thread::Builder::new()
            .name(format!("{}-{}", kind.thread_name(), id.0))
            .spawn(move || {
                let _scope = worker_cancellation.enter();
                operation(worker_cancellation)
            })?;
        self.tasks.insert(
            id,
            ManagedTask {
                kind,
                durability,
                cancellation,
                handle,
            },
        );
        Ok(id)
    }

    pub fn begin_shutdown(&mut self) {
        self.accepting = false;
        for task in self.tasks.values() {
            task.cancellation.cancel();
        }
    }

    pub fn cancel_kinds(&self, kinds: &[TaskKind]) {
        for task in self
            .tasks
            .values()
            .filter(|task| kinds.contains(&task.kind))
        {
            task.cancellation.cancel();
        }
    }

    pub fn is_accepting(&self) -> bool {
        self.accepting
    }

    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn has_recoverable_work(&self) -> bool {
        self.tasks.values().any(|task| {
            matches!(
                task.durability,
                TaskDurability::Recoverable | TaskDurability::CommitSensitive
            )
        })
    }

    pub fn reap_finished(&mut self) -> Vec<FinishedTask> {
        let finished_ids = self
            .tasks
            .iter()
            .filter_map(|(id, task)| task.handle.is_finished().then_some(*id))
            .collect::<Vec<_>>();
        finished_ids
            .into_iter()
            .filter_map(|id| {
                self.tasks.remove(&id).map(|task| FinishedTask {
                    id,
                    kind: task.kind,
                    durability: task.durability,
                    panicked: task.handle.join().is_err(),
                })
            })
            .collect()
    }

    pub fn drain_for(&mut self, timeout: Duration) -> Vec<FinishedTask> {
        let started = Instant::now();
        let mut finished = Vec::new();
        loop {
            finished.extend(self.reap_finished());
            if self.tasks.is_empty() || started.elapsed() >= timeout {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn managed_tasks_are_reaped_after_completion() {
        let mut supervisor = TaskSupervisor::default();
        let (sender, receiver) = mpsc::channel();
        supervisor
            .spawn(TaskKind::Sync, TaskDurability::Recoverable, move |_| {
                sender.send(41).unwrap();
            })
            .unwrap();
        assert_eq!(41, receiver.recv_timeout(Duration::from_secs(1)).unwrap());
        let finished = supervisor.drain_for(Duration::from_secs(1));
        assert_eq!(1, finished.len());
        assert_eq!(0, supervisor.active_count());
        assert!(!finished[0].panicked);
    }

    #[test]
    fn shutdown_stops_accepting_and_cancels_workers() {
        let mut supervisor = TaskSupervisor::default();
        let (sender, receiver) = mpsc::channel();
        supervisor
            .spawn(
                TaskKind::AiRequest,
                TaskDurability::Ephemeral,
                move |token| {
                    while !token.is_cancelled() {
                        thread::yield_now();
                    }
                    sender.send(()).unwrap();
                },
            )
            .unwrap();
        supervisor.begin_shutdown();
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!supervisor.is_accepting());
        assert!(supervisor
            .spawn(TaskKind::HealthCheck, TaskDurability::Ephemeral, |_| {})
            .is_err());
        assert_eq!(1, supervisor.drain_for(Duration::from_secs(1)).len());
    }
}
