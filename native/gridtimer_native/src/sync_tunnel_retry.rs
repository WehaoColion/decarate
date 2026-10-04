// v0.0.1 - Pace tunnel allocation and preserve provider cooldown across restarts.
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HEALTH_RESET: Duration = Duration::from_secs(5 * 60);
static JOURNAL: Mutex<Option<Journal>> = Mutex::new(None);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u8,
    attempts: u32,
    application_after: u64,
    provider_after: u64,
}

struct Journal {
    path: PathBuf,
    state: State,
    healthy_since: Option<Instant>,
}

impl Journal {
    fn load(path: PathBuf) -> io::Result<Self> {
        let state = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(io::Error::other("tunnel retry state is a reparse point"));
                    }
                }
                if !metadata.is_file() || metadata.len() > 4096 {
                    return Err(io::Error::other("invalid tunnel retry state file"));
                }
                let state: State = serde_json::from_slice(&fs::read(&path)?)?;
                if state.version != 1 || state.attempts > 32 {
                    return Err(io::Error::other("unsupported tunnel retry state"));
                }
                state
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => State {
                version: 1,
                attempts: 0,
                application_after: 0,
                provider_after: 0,
            },
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            state,
            healthy_since: None,
        })
    }

    fn remaining(&self, now: u64) -> u64 {
        self.state
            .application_after
            .max(self.state.provider_after)
            .saturating_sub(now)
    }

    fn save(&mut self, next: State) -> io::Result<()> {
        super::atomic_write_text(&self.path, &serde_json::to_string(&next)?)
            .map_err(io::Error::other)?;
        self.state = next;
        Ok(())
    }

    fn request<T>(
        &mut self,
        now: u64,
        send: impl FnOnce() -> (T, Option<u64>),
    ) -> io::Result<Result<T, u64>> {
        let wait = self.remaining(now);
        if wait > 0 {
            return Ok(Err(wait));
        }
        let mut next = self.state.clone();
        let delay = (60_u64 << next.attempts.min(4)).min(15 * 60);
        next.attempts = next.attempts.saturating_add(1).min(32);
        next.application_after = now.saturating_add(delay);
        self.save(next)?;
        self.healthy_since = None;
        // Commit the attempt before contacting the provider. Allocation success
        // alone does not prove that the resulting public endpoint is healthy.
        let (value, provider_deadline) = send();
        if let Some(deadline) = provider_deadline {
            let mut next = self.state.clone();
            next.provider_after = next.provider_after.max(deadline);
            // Retain this deadline in memory even if the durable write fails.
            self.state = next.clone();
            self.save(next)?;
        }
        Ok(Ok(value))
    }

    fn observe(&mut self, healthy: bool, now: Instant) -> io::Result<()> {
        if !healthy {
            self.healthy_since = None;
            return Ok(());
        }
        let started = *self.healthy_since.get_or_insert(now);
        if self.state.attempts > 0 && now.saturating_duration_since(started) >= HEALTH_RESET {
            let mut next = self.state.clone();
            next.attempts = 0;
            next.application_after = 0;
            // A successful endpoint probe must not shorten a provider deadline.
            self.save(next)?;
        }
        Ok(())
    }
}

fn with<T>(operation: impl FnOnce(&mut Journal) -> io::Result<T>) -> io::Result<T> {
    let mut journal = JOURNAL
        .lock()
        .map_err(|_| io::Error::other("tunnel retry state lock poisoned"))?;
    if journal.is_none() {
        *journal = Some(Journal::load(
            super::launcher_runtime_directory().join("tunnel_retry_v1.json"),
        )?);
    }
    operation(journal.as_mut().unwrap())
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn remaining() -> io::Result<u64> {
    with(|journal| Ok(journal.remaining(epoch_seconds())))
}

pub(super) fn deadline_after(seconds: u64) -> u64 {
    epoch_seconds().saturating_add(seconds.min(86_400))
}

pub(super) fn request<T>(send: impl FnOnce() -> (T, Option<u64>)) -> io::Result<Result<T, u64>> {
    with(|journal| journal.request(epoch_seconds(), send))
}

pub(super) fn observe(healthy: bool) -> io::Result<()> {
    with(|journal| journal.observe(healthy, Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tenrate_tunnel_retry_{}_{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn journal(&self) -> Journal {
            Journal::load(self.0.join("retry.json")).unwrap()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tunnel_retry_paces_actual_requests_across_restarts_and_allocation_success() {
        let directory = Directory::new();
        let mut now = 100;
        let mut calls = 0;
        for delay in [60, 120, 240, 480, 900, 900] {
            let mut journal = directory.journal();
            assert_eq!(
                Ok(200),
                journal
                    .request(now, || {
                        calls += 1;
                        (200, None)
                    })
                    .unwrap()
            );
            let mut reopened = directory.journal();
            assert_eq!(
                Err(delay),
                reopened
                    .request::<u16>(now, || panic!("duplicate provisioning escaped pacing"))
                    .unwrap()
            );
            assert_eq!(1, reopened.remaining(now + delay - 1));
            now += delay;
        }
        assert_eq!(6, calls);
    }

    #[test]
    fn tunnel_retry_retains_provider_deadline_through_restart_and_health_reset() {
        let directory = Directory::new();
        let mut journal = directory.journal();
        assert_eq!(Ok(429), journal.request(100, || (429, Some(3700))).unwrap());
        let mut journal = directory.journal();
        assert_eq!(3650, journal.remaining(50));
        let start = Instant::now();
        journal.observe(true, start).unwrap();
        journal
            .observe(true, start + HEALTH_RESET - Duration::from_secs(1))
            .unwrap();
        assert_eq!(1, journal.state.attempts);
        journal.observe(false, start + HEALTH_RESET).unwrap();
        journal.observe(true, start + HEALTH_RESET).unwrap();
        assert_eq!(1, journal.state.attempts);
        journal.observe(true, start + HEALTH_RESET * 2).unwrap();
        assert_eq!(0, journal.state.attempts);
        let mut journal = directory.journal();
        assert_eq!(
            Err(1),
            journal
                .request::<u16>(3699, || panic!("provider cooldown was shortened"))
                .unwrap()
        );
        assert_eq!(Ok(200), journal.request(3700, || (200, None)).unwrap());
    }

    #[test]
    fn tunnel_retry_must_persist_before_contacting_the_provider() {
        let directory = Directory::new();
        let blocked = directory.0.join("occupied");
        fs::write(&blocked, b"keep").unwrap();
        let mut journal = directory.journal();
        journal.path = blocked.join("retry.json");
        assert!(journal
            .request::<u16>(100, || panic!("request sent without a durable retry fence"))
            .is_err());
        assert_eq!(b"keep", fs::read(&blocked).unwrap().as_slice());
        assert_eq!(0, journal.state.attempts);
        fs::write(directory.0.join("retry.json"), b"invalid state").unwrap();
        assert!(Journal::load(directory.0.join("retry.json")).is_err());
    }
}
