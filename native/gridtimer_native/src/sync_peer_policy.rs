// v0.0.1 - Keep local HTTP peer verification bound to the requesting thread.
use std::cell::Cell;
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::net::TcpStream;
use std::rc::Rc;

pub type LocalHttpPeerVerifier = fn(&TcpStream) -> io::Result<File>;
thread_local! {
    static VERIFIER: Cell<Option<LocalHttpPeerVerifier>> = const { Cell::new(None) };
}

/// Scope this guard around desktop network work. Each HTTP connection is
/// checked before the first request byte, and its verified image stays pinned
/// until the complete response has been consumed. The guard cannot cross threads.
pub struct LocalHttpPeerVerification {
    previous: Option<LocalHttpPeerVerifier>,
    _same_thread: PhantomData<Rc<()>>,
}
impl LocalHttpPeerVerification {
    pub fn enforce(verifier: LocalHttpPeerVerifier) -> Self {
        Self {
            previous: VERIFIER.with(|slot| slot.replace(Some(verifier))),
            _same_thread: PhantomData,
        }
    }
}
impl Drop for LocalHttpPeerVerification {
    fn drop(&mut self) {
        VERIFIER.with(|slot| slot.set(self.previous));
    }
}

pub(super) fn verify(stream: &TcpStream) -> io::Result<Option<File>> {
    VERIFIER.with(|slot| match slot.get() {
        Some(verifier) => verifier(stream).map(Some),
        None => Ok(None),
    })
}
