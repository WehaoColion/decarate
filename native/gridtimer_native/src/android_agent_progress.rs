use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentStage {
    FirstPass,
    Review,
    Verifying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentActivity {
    Preparing,
    Model,
    Tool,
    Verifying,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentTerminal {
    Running,
    Ready,
    ReviewIncomplete,
    Failed,
    Cancelled,
}

/// Events contain only execution state and counts, never model or document data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentProgressEvent {
    StageStarted(AgentStage),
    RequestStarted,
    RequestFinished,
    ToolFinished { documents_read: usize },
    Verifying,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentProgressSnapshot {
    task_id: String,
    sequence: u64,
    stage: AgentStage,
    activity: AgentActivity,
    requests: usize,
    tool_calls: usize,
    documents_read: usize,
    request_in_flight: bool,
    cancel_requested: bool,
    terminal: AgentTerminal,
    elapsed_seconds: u64,
}

pub(crate) struct AgentProgress {
    state: Mutex<AgentProgressSnapshot>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

impl AgentProgress {
    pub(crate) fn new(task_id: &str, cancelled: Arc<AtomicBool>) -> Self {
        let cancel_requested = cancelled.load(Ordering::Acquire);
        Self {
            state: Mutex::new(AgentProgressSnapshot {
                task_id: task_id.to_string(),
                sequence: 0,
                stage: AgentStage::FirstPass,
                activity: AgentActivity::Preparing,
                requests: 0,
                tool_calls: 0,
                documents_read: 0,
                request_in_flight: false,
                cancel_requested,
                terminal: AgentTerminal::Running,
                elapsed_seconds: 0,
            }),
            cancelled,
            started: Instant::now(),
        }
    }

    /// The state lock linearizes cancellation, request admission and completion.
    /// An admitted request may finish normally; cancellation never pretends to
    /// interrupt an existing transport.
    pub(crate) fn request_cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.terminal != AgentTerminal::Running || state.cancel_requested {
            return;
        }
        self.cancelled.store(true, Ordering::Release);
        state.cancel_requested = true;
        state.sequence = state.sequence.saturating_add(1);
    }

    pub(crate) fn apply(&self, event: AgentProgressEvent) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.terminal != AgentTerminal::Running {
            return false;
        }
        state.cancel_requested |= self.cancelled.load(Ordering::Acquire);
        match event {
            AgentProgressEvent::StageStarted(stage) => {
                if state.cancel_requested || state.request_in_flight {
                    return false;
                }
                if !matches!(
                    (state.stage, stage),
                    (AgentStage::FirstPass, AgentStage::FirstPass)
                        | (AgentStage::FirstPass, AgentStage::Review)
                ) {
                    return false;
                }
                if stage == state.stage && (state.requests > 0 || state.tool_calls > 0) {
                    return false;
                }
                state.stage = stage;
                state.activity = AgentActivity::Preparing;
                state.documents_read = 0;
            }
            AgentProgressEvent::RequestStarted => {
                if state.cancel_requested
                    || state.request_in_flight
                    || state.stage == AgentStage::Verifying
                {
                    return false;
                }
                state.requests = state.requests.saturating_add(1);
                state.request_in_flight = true;
                state.activity = AgentActivity::Model;
            }
            AgentProgressEvent::RequestFinished => {
                if !state.request_in_flight {
                    return false;
                }
                state.request_in_flight = false;
                state.activity = AgentActivity::Preparing;
            }
            AgentProgressEvent::ToolFinished { documents_read } => {
                if state.request_in_flight || state.stage == AgentStage::Verifying {
                    return false;
                }
                state.tool_calls = state.tool_calls.saturating_add(1);
                state.documents_read = state.documents_read.max(documents_read);
                state.activity = AgentActivity::Tool;
            }
            AgentProgressEvent::Verifying => {
                if state.request_in_flight {
                    return false;
                }
                state.stage = AgentStage::Verifying;
                state.activity = AgentActivity::Verifying;
                state.documents_read = 0;
            }
        }
        state.sequence = state.sequence.saturating_add(1);
        true
    }

    pub(crate) fn finish(&self, terminal: AgentTerminal) -> AgentTerminal {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.terminal != AgentTerminal::Running
            || terminal == AgentTerminal::Running
            || state.request_in_flight
        {
            return state.terminal;
        }
        state.cancel_requested |= self.cancelled.load(Ordering::Acquire);
        state.terminal = if state.cancel_requested {
            AgentTerminal::Cancelled
        } else {
            terminal
        };
        state.activity = AgentActivity::Finished;
        state.elapsed_seconds = self.started.elapsed().as_secs();
        state.sequence = state.sequence.saturating_add(1);
        state.terminal
    }

    pub(crate) fn snapshot(&self) -> AgentProgressSnapshot {
        let mut snapshot = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if snapshot.terminal == AgentTerminal::Running {
            snapshot.elapsed_seconds = self.started.elapsed().as_secs();
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress() -> AgentProgress {
        AgentProgress::new("run", Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn progress_cancel_keeps_an_admitted_request_in_flight_until_it_returns() {
        let progress = progress();
        assert!(progress.apply(AgentProgressEvent::RequestStarted));
        progress.request_cancel();
        let waiting = progress.snapshot();
        assert!(waiting.cancel_requested && waiting.request_in_flight);
        assert_eq!(waiting.terminal, AgentTerminal::Running);
        assert_eq!(
            progress.finish(AgentTerminal::Ready),
            AgentTerminal::Running
        );
        assert!(!progress.apply(AgentProgressEvent::RequestStarted));
        assert!(progress.apply(AgentProgressEvent::RequestFinished));
        assert_eq!(
            progress.finish(AgentTerminal::Ready),
            AgentTerminal::Cancelled
        );
        let stopped = progress.snapshot();
        assert!(!stopped.request_in_flight);
        assert_eq!(stopped.requests, 1);
        assert_eq!(stopped.activity, AgentActivity::Finished);
    }

    #[test]
    fn progress_cancel_and_finish_have_a_single_stable_terminal_order() {
        let cancelled = progress();
        cancelled.request_cancel();
        assert_eq!(
            cancelled.finish(AgentTerminal::Ready),
            AgentTerminal::Cancelled
        );
        assert!(!cancelled.apply(AgentProgressEvent::StageStarted(AgentStage::Review)));
        assert_eq!(
            cancelled.finish(AgentTerminal::Failed),
            AgentTerminal::Cancelled
        );

        let completed = progress();
        assert_eq!(completed.finish(AgentTerminal::Ready), AgentTerminal::Ready);
        let terminal = completed.snapshot();
        completed.request_cancel();
        assert!(!completed.apply(AgentProgressEvent::RequestStarted));
        assert!(!completed.apply(AgentProgressEvent::ToolFinished { documents_read: 3 }));
        assert_eq!(
            completed.finish(AgentTerminal::Failed),
            AgentTerminal::Ready
        );
        assert_eq!(completed.snapshot(), terminal);
        assert!(!completed.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn progress_stage_counts_are_independent_and_cannot_go_backwards() {
        let progress = progress();
        progress.apply(AgentProgressEvent::ToolFinished { documents_read: 1 });
        progress.apply(AgentProgressEvent::ToolFinished { documents_read: 1 });
        assert_eq!(progress.snapshot().documents_read, 1);
        progress.apply(AgentProgressEvent::StageStarted(AgentStage::Review));
        assert_eq!(progress.snapshot().documents_read, 0);
        progress.apply(AgentProgressEvent::ToolFinished { documents_read: 1 });
        assert_eq!(progress.snapshot().documents_read, 1);
        assert_eq!(progress.snapshot().tool_calls, 3);
        assert!(!progress.apply(AgentProgressEvent::StageStarted(AgentStage::FirstPass)));
        progress.apply(AgentProgressEvent::Verifying);
        let verifying = progress.snapshot();
        assert_eq!(verifying.stage, AgentStage::Verifying);
        assert_eq!(verifying.documents_read, 0);
        assert!(!progress.apply(AgentProgressEvent::RequestStarted));
    }

    #[test]
    fn progress_snapshot_has_only_the_public_state_and_count_fields() {
        let progress = progress();
        let value = serde_json::to_value(progress.snapshot()).unwrap();
        let fields = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            fields,
            std::collections::HashSet::from([
                "taskId",
                "sequence",
                "stage",
                "activity",
                "requests",
                "toolCalls",
                "documentsRead",
                "requestInFlight",
                "cancelRequested",
                "terminal",
                "elapsedSeconds"
            ])
        );
        assert_eq!(value["stage"], "first_pass");
        assert_eq!(value["activity"], "preparing");
        assert_eq!(value["terminal"], "running");
    }
}
