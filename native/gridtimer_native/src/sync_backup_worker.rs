// v0.0.2 - Include authenticated legacy sources in maintenance status.
// v0.0.1 - Wake bounded, serial backup maintenance after committed privacy changes.
use super::*;
use crate::server_store::{BackupPrivacySubscription, BackupPrivacyWake};

pub(super) struct Worker {
    subscription: BackupPrivacySubscription,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.subscription.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn start(
    store: Arc<SqliteServerStore>,
    directory: PathBuf,
    info: Arc<Mutex<ServerRuntimeInfo>>,
    interval: Duration,
) -> io::Result<Worker> {
    start_with_timing(
        store,
        directory,
        info,
        interval,
        Duration::from_millis(500),
        Duration::from_secs(30),
    )
}

pub(super) fn start_with_timing(
    store: Arc<SqliteServerStore>,
    directory: PathBuf,
    info: Arc<Mutex<ServerRuntimeInfo>>,
    interval: Duration,
    coalesce: Duration,
    retry: Duration,
) -> io::Result<Worker> {
    let subscription = store.subscribe_backup_privacy().map_err(store_io_error)?;
    if let Ok(mut info) = info.lock() {
        info.backup_privacy_wake = Some(subscription.clone());
    }
    let wake = subscription.clone();
    let thread = thread::Builder::new()
        .name("gridtimer-runtime-backup".into())
        .spawn(move || {
            run(
                &wake,
                interval,
                coalesce,
                retry,
                || {
                    perform_scheduled_runtime_backup(&store, &directory, &info);
                },
                || {
                    let requested = wake.requested();
                    let result = maintain_privacy(&store, &directory, &info);
                    if result.is_ok() {
                        wake.complete(requested);
                    }
                    result
                },
                || {
                    if let Ok(mut info) = info.lock() {
                        // Preserve a failure until a real successful retry. Incoming
                        // requests cannot erase an error by merely scheduling work.
                        if info.backup_privacy.status != "error" {
                            info.backup_privacy.status = "pending".into();
                        }
                    }
                },
            )
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not start runtime recovery backup worker: {error}"),
            )
        })?;
    Ok(Worker {
        subscription,
        thread: Some(thread),
    })
}

pub(super) fn visible_status(info: &ServerRuntimeInfo) -> BackupPrivacyStatus {
    let mut status = info.backup_privacy.clone();
    if info
        .backup_privacy_wake
        .as_ref()
        .is_some_and(BackupPrivacySubscription::outstanding)
        && !matches!(status.status.as_str(), "running" | "error")
    {
        status.status = "pending".into();
    }
    status
}

fn run(
    wake: &BackupPrivacySubscription,
    interval: Duration,
    coalesce: Duration,
    retry: Duration,
    mut backup: impl FnMut(),
    mut privacy: impl FnMut() -> io::Result<()>,
    mut pending: impl FnMut(),
) {
    let mut next_backup = Instant::now() + interval;
    let mut next_privacy: Option<Instant> = None;
    loop {
        let deadline = next_privacy.map_or(next_backup, |next| next.min(next_backup));
        match wake.wait(deadline.saturating_duration_since(Instant::now())) {
            BackupPrivacyWake::Stopped => return,
            BackupPrivacyWake::Changed => {
                // Only the first event sets this deadline: continuous writes
                // cannot defer cleanup forever, or bypass a failure backoff.
                next_privacy.get_or_insert_with(|| Instant::now() + coalesce);
                pending();
            }
            BackupPrivacyWake::Timeout => {}
        }
        if next_privacy.is_some_and(|deadline| Instant::now() >= deadline) {
            next_privacy = if privacy().is_err() {
                Some(Instant::now() + retry)
            } else {
                None
            };
        }
        // A fresh event arriving during either operation stays latched and is
        // consumed next iteration; completion never clears that new event.
        if Instant::now() >= next_backup {
            backup();
            next_backup = Instant::now() + interval;
        }
    }
}

fn maintain_privacy(
    store: &SqliteServerStore,
    directory: &Path,
    runtime: &Mutex<ServerRuntimeInfo>,
) -> io::Result<()> {
    if let Ok(mut info) = runtime.lock() {
        info.backup_privacy.status = "running".into();
    }
    let result = backup_privacy::clean(store, directory);
    if let Ok(mut info) = runtime.lock() {
        match &result {
            Ok(count) => {
                info.backup_privacy.status = "ok".into();
                info.backup_privacy.last_success_at_epoch_millis = now_millis();
                info.backup_privacy.message = format!(
                    "Managed startup, runtime, schema-14 and imported legacy archives checked; {count} updated."
                );
            }
            Err(error) => {
                info.backup_privacy.status = "error".into();
                info.backup_privacy.last_failure_at_epoch_millis = now_millis();
                info.backup_privacy.message = error.to_string().chars().take(384).collect();
            }
        }
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_privacy_worker_keeps_events_during_work_and_retries_without_another_event() {
        let directory = std::env::temp_dir().join(format!("privacy-wakeup-{}", random_token(12)));
        fs::create_dir_all(&directory).unwrap();
        let store = SqliteServerStore::open(directory.join("server.sqlite3"), None).unwrap();
        let subscription = store.subscribe_backup_privacy().unwrap();
        let key = crate::server_store::privacy_notification_test_key(&store);
        let (send, receive) = std::sync::mpsc::channel();
        let wake = subscription.clone();
        let started = Instant::now();
        crate::server_store::privacy_notification_test_notify(&key);
        crate::server_store::privacy_notification_test_notify(&key);
        let thread = thread::spawn(move || {
            let mut attempts = 0;
            run(
                &wake,
                Duration::from_secs(3600),
                Duration::from_millis(20),
                Duration::from_millis(40),
                || panic!("privacy activity must not create a full periodic backup"),
                || {
                    attempts += 1;
                    send.send((attempts, started.elapsed())).unwrap();
                    if attempts == 1 {
                        crate::server_store::privacy_notification_test_notify(&key);
                        Ok(())
                    } else if attempts == 2 {
                        Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "synthetic file lock",
                        ))
                    } else {
                        Ok(())
                    }
                },
                || {},
            );
        });
        let worker = Worker {
            subscription,
            thread: Some(thread),
        };
        let first = receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let second = receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let third = receive.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!((1, 2, 3), (first.0, second.0, third.0));
        assert!(
            third.1 - second.1 >= Duration::from_millis(35),
            "failure retry spun without backoff"
        );
        assert!(
            receive.recv_timeout(Duration::from_millis(80)).is_err(),
            "coalesced events ran redundant cleanup"
        );
        drop(worker);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }
}
