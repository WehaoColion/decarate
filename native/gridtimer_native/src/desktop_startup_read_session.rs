// v1.1.0.2 Windows - Reuse pure compatibility checks only during one loading session.
//! This scope never caches journal authority, owner/envelope validation, media
//! declarations, SQLite integrity, or write permission. Keys hash the exact
//! bytes read on this thread and include the sanitizer's time input.
use crate::app_data::AppDataJsonCompatibility;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;

const MAX_ENTRIES: usize = 128;

#[derive(Default)]
struct Session {
    depth: usize,
    classifications: VecDeque<(([u8; 32], i64), AppDataJsonCompatibility)>,
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// Stack-bound and deliberately not Send: a completed, cancelled or panicked
/// startup discards all entries. No cache is persisted to disk.
pub struct DesktopStartupReadSession(PhantomData<Rc<()>>);

impl DesktopStartupReadSession {
    pub fn enter() -> Self {
        SESSION.with(|session| session.borrow_mut().depth += 1);
        Self(PhantomData)
    }
}

impl Drop for DesktopStartupReadSession {
    fn drop(&mut self) {
        SESSION.with(|session| {
            let mut session = session.borrow_mut();
            session.depth -= 1;
            if session.depth == 0 {
                session.classifications.clear();
            }
        });
    }
}

pub(crate) fn classify(
    raw: &str,
    now: i64,
    classify: impl FnOnce() -> AppDataJsonCompatibility,
) -> AppDataJsonCompatibility {
    if !SESSION.with(|session| session.borrow().depth > 0) {
        return classify();
    }
    let key = (Sha256::digest(raw.as_bytes()).into(), now);
    if let Some(result) = SESSION.with(|session| {
        session
            .borrow()
            .classifications
            .iter()
            .find(|(candidate, _)| candidate == &key)
            .map(|(_, result)| *result)
    }) {
        return result;
    }
    let result = classify();
    SESSION.with(|session| {
        let mut session = session.borrow_mut();
        if session.classifications.len() == MAX_ENTRIES {
            session.classifications.pop_front();
        }
        session.classifications.push_back((key, result));
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn session_reuses_exact_bytes_and_time_but_never_survives_loading() {
        let calls = Cell::new(0);
        let classify_once = |raw, now| {
            classify(raw, now, || {
                calls.set(calls.get() + 1);
                AppDataJsonCompatibility::CurrentKnown
            })
        };
        {
            let _scope = DesktopStartupReadSession::enter();
            classify_once("one", 0);
            classify_once("one", 0);
            assert_eq!(calls.get(), 1);
            classify_once("changed", 0);
            classify_once("one", 1);
            assert_eq!(calls.get(), 3);
        }
        classify_once("one", 0);
        assert_eq!(calls.get(), 4);
        let _next = DesktopStartupReadSession::enter();
        classify_once("one", 0);
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn invalid_and_future_inputs_are_not_inherited_from_valid_inputs() {
        let _scope = DesktopStartupReadSession::enter();
        let valid = crate::app_data::default_app_data_json(100);
        assert_eq!(
            crate::app_data::app_data_json_compatibility(&valid, 0),
            AppDataJsonCompatibility::CurrentKnown
        );
        assert_eq!(
            crate::app_data::app_data_json_compatibility("invalid", 0),
            AppDataJsonCompatibility::Invalid
        );
        assert_eq!(
            crate::app_data::app_data_json_compatibility("{\"schemaVersion\":2147483647}", 0),
            AppDataJsonCompatibility::Future
        );
    }
}
