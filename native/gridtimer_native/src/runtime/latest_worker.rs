// v2.22.27 - Bounded single-writer work queue with revision-tagged completion.
//! A worker owns blocking resources. Submissions only replace the waiting value;
//! they never wait for I/O. At most one operation runs and one waits.

use std::io;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

struct Pending<I> {
    next: Option<(u64, I)>,
    closed: bool,
}

pub struct Completion<O> {
    pub revision: u64,
    pub result: Result<O, String>,
}

pub struct LatestWorker<I, O> {
    shared: Arc<(Mutex<Pending<I>>, Condvar)>,
    results: mpsc::Receiver<Completion<O>>,
    handle: Option<JoinHandle<()>>,
    next_revision: u64,
    submitted: u64,
    completed: u64,
}

impl<I: Send + 'static, O: Send + 'static> LatestWorker<I, O> {
    pub fn spawn(
        name: &str,
        mut operation: impl FnMut(I) -> Result<O, String> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let shared = Arc::new((
            Mutex::new(Pending {
                next: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker_shared = Arc::clone(&shared);
        // Bound receipts too: an unpolled UI must not accumulate whole snapshots.
        // Backpressure only stalls this worker, never the submitting GUI thread.
        let (sender, results) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || loop {
                let work = {
                    let (mutex, ready) = &*worker_shared;
                    let mut pending = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
                    while pending.next.is_none() && !pending.closed {
                        pending = ready
                            .wait(pending)
                            .unwrap_or_else(|poison| poison.into_inner());
                    }
                    match pending.next.take() {
                        Some(work) => work,
                        None => break,
                    }
                };
                let (revision, input) = work;
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(input)))
                        .unwrap_or_else(
                            |_| Err("保存工作线程异常，草稿仍保留在窗口中".to_string()),
                        );
                if sender.send(Completion { revision, result }).is_err() {
                    break;
                }
                wake();
            })?;
        Ok(Self {
            shared,
            results,
            handle: Some(handle),
            next_revision: 1,
            submitted: 0,
            completed: 0,
        })
    }

    pub fn submit_latest(&mut self, input: I) -> io::Result<u64> {
        let revision = self.next_revision;
        self.next_revision = revision
            .checked_add(1)
            .ok_or_else(|| io::Error::other("保存版本计数已用尽"))?;
        let (mutex, ready) = &*self.shared;
        let mut pending = mutex
            .lock()
            .map_err(|_| io::Error::other("保存队列不可用"))?;
        if pending.closed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "保存队列已关闭"));
        }
        pending.next = Some((revision, input));
        self.submitted = revision;
        ready.notify_one();
        Ok(revision)
    }

    pub fn try_recv(&mut self) -> Option<Completion<O>> {
        match self.results.try_recv() {
            Ok(result) => {
                self.completed = self.completed.max(result.revision);
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) if self.is_pending() => {
                self.completed = self.submitted;
                Some(Completion {
                    revision: self.submitted,
                    result: Err("保存工作线程已停止".into()),
                })
            }
            _ => None,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.completed < self.submitted
    }
    pub fn submitted_revision(&self) -> u64 {
        self.submitted
    }
}

impl<I, O> Drop for LatestWorker<I, O> {
    fn drop(&mut self) {
        let (mutex, ready) = &*self.shared;
        let mut pending = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
        pending.closed = true;
        ready.notify_one();
        // Normal shutdown drains via receipts first. Never block a GUI thread
        // here; an explicitly forced process exit may abandon outstanding work.
        self.handle.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn slow_writer_keeps_only_latest_waiter_and_submission_does_not_block() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut worker = LatestWorker::spawn(
            "bounded-writer-test",
            move |n| {
                if n == 0 {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                Ok(n)
            },
            || {},
        )
        .unwrap();
        worker.submit_latest(0).unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        for n in 1..=1000 {
            worker.submit_latest(n).unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        release_tx.send(()).unwrap();
        let first = worker.results.recv_timeout(Duration::from_secs(2)).unwrap();
        let last = worker.results.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(first.result.unwrap(), 0);
        assert_eq!(last.result.unwrap(), 1000);
        assert_eq!(last.revision, 1001);
        assert!(worker.results.try_recv().is_err());
    }

    #[test]
    fn failed_receipt_is_not_success_and_worker_accepts_retry() {
        let mut worker = LatestWorker::spawn(
            "writer-failure-test",
            |n| {
                if n == 1 {
                    Err("disk unavailable".into())
                } else {
                    Ok(n)
                }
            },
            || {},
        )
        .unwrap();
        worker.submit_latest(1).unwrap();
        let failure = worker.results.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(failure.result.is_err());
        worker.submit_latest(2).unwrap();
        let success = worker.results.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(success.result.unwrap(), 2);
    }
}
