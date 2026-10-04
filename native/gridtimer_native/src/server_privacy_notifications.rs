// v0.0.1 - Coalesce committed privacy changes without polling stored note content.
use super::*;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

#[derive(Debug, Default)]
struct State {
    changed: bool,
    stopped: bool,
    requested: u64,
    completed: u64,
}

#[derive(Debug, Default)]
struct Wake {
    state: Mutex<State>,
    ready: Condvar,
}

type Subscribers = std::collections::HashMap<String, Vec<Weak<Wake>>>;
static SUBSCRIBERS: OnceLock<Mutex<Subscribers>> = OnceLock::new();

#[derive(Clone, Debug)]
pub(crate) struct BackupPrivacySubscription {
    wake: Arc<Wake>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BackupPrivacyWake {
    Changed,
    Timeout,
    Stopped,
}

impl BackupPrivacySubscription {
    pub(crate) fn requested(&self) -> u64 {
        self.wake
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested
    }

    pub(crate) fn complete(&self, requested: u64) {
        self.wake
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .completed = requested;
    }

    pub(crate) fn outstanding(&self) -> bool {
        let state = self
            .wake
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.requested != state.completed
    }

    pub(crate) fn wait(&self, timeout: Duration) -> BackupPrivacyWake {
        let state = self
            .wake
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (mut state, _) = self
            .wake
            .ready
            .wait_timeout_while(state, timeout, |state| !state.changed && !state.stopped)
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            BackupPrivacyWake::Stopped
        } else if std::mem::take(&mut state.changed) {
            BackupPrivacyWake::Changed
        } else {
            BackupPrivacyWake::Timeout
        }
    }

    pub(crate) fn stop(&self) {
        self.wake
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
        self.wake.ready.notify_all();
    }
}

impl SqliteServerStore {
    pub(crate) fn subscribe_backup_privacy(&self) -> StoreResult<BackupPrivacySubscription> {
        let key = privacy_journal::target_fingerprint(self.database_path())?;
        let wake = Arc::new(Wake::default());
        let mut subscribers = SUBSCRIBERS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        subscribers.retain(|_, entries| {
            entries.retain(|entry| entry.strong_count() > 0);
            !entries.is_empty()
        });
        subscribers
            .entry(key)
            .or_default()
            .push(Arc::downgrade(&wake));
        Ok(BackupPrivacySubscription { wake })
    }
}

// A notification is only a scheduling hint. The durable journal and verified
// backup certificates remain the authority, including after a process restart.
pub(super) fn notify(key: &str) {
    let Some(subscribers) = SUBSCRIBERS.get() else {
        return;
    };
    let mut subscribers = subscribers
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(entries) = subscribers.get_mut(key) else {
        return;
    };
    entries.retain(|entry| {
        let Some(wake) = entry.upgrade() else {
            return false;
        };
        let mut state = wake.state.lock().unwrap_or_else(|error| error.into_inner());
        state.changed = true;
        state.requested = state.requested.wrapping_add(1);
        drop(state);
        wake.ready.notify_one();
        true
    });
}

#[cfg(test)]
pub(crate) fn privacy_notification_test_key(store: &SqliteServerStore) -> String {
    privacy_journal::target_fingerprint(store.database_path()).unwrap()
}
#[cfg(test)]
pub(crate) fn privacy_notification_test_notify(key: &str) {
    notify(key);
}
