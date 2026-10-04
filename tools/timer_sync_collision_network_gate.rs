// Supplemental test instrumentation only. Included only in isolated test copies.
#[cfg(test)]
mod timer_sync_collision_network_gate {
    use std::cell::Cell;

    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static COUNTS: Cell<[u32; 4]> = const { Cell::new([0; 4]) };
    }

    pub(crate) struct Scope;

    impl Scope {
        pub(crate) fn enter() -> Self {
            ACTIVE.with(|active| assert!(!active.replace(true)));
            COUNTS.with(|counts| counts.set([0; 4]));
            Self
        }

        pub(crate) fn counts(&self) -> [u32; 4] {
            COUNTS.with(Cell::get)
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(false));
        }
    }

    // Called only immediately before dispatching a NEW outbound network task.
    // Sync response reception, merge, commit and receipt adoption are untouched.
    pub(crate) fn block(dispatch: usize) -> bool {
        assert!(dispatch < 4);
        if !ACTIVE.with(Cell::get) {
            return false;
        }
        COUNTS.with(|counts| {
            let mut current = counts.get();
            current[dispatch] += 1;
            counts.set(current);
        });
        true
    }
}
