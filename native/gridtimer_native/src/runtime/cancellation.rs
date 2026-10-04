// v0.0.1 - Propagate worker cancellation to scoped blocking I/O without retaining completed requests.
use std::cell::RefCell;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};

type Action = Arc<dyn Fn() + Send + Sync>;
#[derive(Default)]
struct State {
    cancelled: AtomicBool,
    actions: Mutex<Vec<Action>>,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<State>,
}
impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let actions =
            std::mem::take(&mut *self.state.actions.lock().unwrap_or_else(|e| e.into_inner()));
        // Call outside the registry lock: an interrupted worker can immediately drop its lease.
        for action in actions {
            action();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
    pub fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }
    pub(crate) fn on_cancel(&self, action: impl Fn() + Send + Sync + 'static) -> Registration {
        let action: Action = Arc::new(action);
        let mut actions = self.state.actions.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() {
            drop(actions);
            action();
            Registration {
                state: Weak::new(),
                action,
            }
        } else {
            actions.push(action.clone());
            Registration {
                state: Arc::downgrade(&self.state),
                action,
            }
        }
    }
    pub fn enter(&self) -> Scope {
        Scope {
            previous: CURRENT.with(|current| current.replace(Some(self.clone()))),
            _same_thread: PhantomData,
        }
    }
}

pub(crate) struct Registration {
    state: Weak<State>,
    action: Action,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state
                .actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|action| !Arc::ptr_eq(action, &self.action));
        }
    }
}
thread_local! { static CURRENT: RefCell<Option<CancellationToken>> = const { RefCell::new(None) }; }
pub struct Scope {
    previous: Option<CancellationToken>,
    _same_thread: PhantomData<Rc<()>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}
pub(crate) fn current() -> Option<CancellationToken> {
    CURRENT.with(|current| current.borrow().clone())
}
pub fn check_current() -> io::Result<()> {
    current().map_or(Ok(()), |token| token.check())
}
pub(crate) fn cancelled_error() -> io::Error {
    // Interrupted is retried by read_exact/write_all; cancellation must terminate those loops.
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "sync request was cancelled",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn sync_cancel_registration_is_race_safe_and_releases_completed_io() {
        let count = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let bump = count.clone();
        let completed = token.on_cancel(move || {
            bump.fetch_add(1, Ordering::SeqCst);
        });
        drop(completed);
        let bump = count.clone();
        let active = token.on_cancel(move || {
            bump.fetch_add(10, Ordering::SeqCst);
        });
        token.cancel();
        token.cancel();
        drop(active);
        let bump = count.clone();
        let _late = token.on_cancel(move || {
            bump.fetch_add(100, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 110);
        assert!(token.state.actions.lock().unwrap().is_empty());
        for _ in 0..32 {
            let token = CancellationToken::new();
            let race = token.clone();
            let thread = std::thread::spawn(move || race.cancel());
            let count = Arc::new(AtomicUsize::new(0));
            let bump = count.clone();
            let _lease = token.on_cancel(move || {
                bump.fetch_add(1, Ordering::SeqCst);
            });
            thread.join().unwrap();
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn sync_cancel_scope_does_not_cancel_the_next_or_another_thread() {
        let outer = CancellationToken::new();
        let inner = CancellationToken::new();
        let _outer = outer.enter();
        {
            let _inner = inner.enter();
            inner.cancel();
            assert!(check_current().is_err());
            std::thread::spawn(|| assert!(check_current().is_ok()))
                .join()
                .unwrap();
        }
        assert!(check_current().is_ok());
        outer.cancel();
        assert!(check_current().is_err());
    }
}
