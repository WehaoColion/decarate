// v1.1.0.2 Windows - Invalidate read models only when their content domain changes.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DesktopContentChanges {
    notes: bool,
    finance: bool,
    timers: bool,
}

impl DesktopContentChanges {
    fn between(before: &DesktopAppData, after: &DesktopAppData) -> Self {
        Self {
            notes: before.notes != after.notes
                || before.note_folders != after.note_folders
                || before.note_preferences != after.note_preferences,
            finance: before.finance_profile != after.finance_profile,
            timers: before.slots != after.slots
                || before.sessions != after.sessions
                || before.archived_tasks != after.archived_tasks
                || before.categories != after.categories
                // Timer order/schema and future projection metadata live in
                // flattened fields. Treat unknown fields conservatively.
                || before.extra != after.extra,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DesktopContentVersions {
    applied: u64,
    notes: u64,
    finance: u64,
    timers: u64,
}

impl DesktopContentVersions {
    fn record(&mut self, revision: u64, changes: DesktopContentChanges) {
        if self.applied != revision.saturating_sub(1) {
            self.reset(revision.saturating_sub(1));
        }
        self.applied = revision;
        if changes.notes {
            self.notes = revision;
        }
        if changes.finance {
            self.finance = revision;
        }
        if changes.timers {
            self.timers = revision;
        }
    }

    fn reset(&mut self, revision: u64) {
        *self = Self {
            applied: revision,
            notes: revision,
            finance: revision,
            timers: revision,
        };
    }

    fn domain(&self, revision: u64, domain: u64) -> u64 {
        // Conservatively invalidate direct state changes that did not cross a
        // classified receipt. Domain generations are global revision numbers,
        // so a later unclassified revision cannot alias an earlier cache key.
        if self.applied == revision {
            domain
        } else {
            revision
        }
    }
}

impl TimerWindowsClient {
    fn notes_cache_version(&self) -> u64 {
        self.content_versions
            .domain(self.data_version, self.content_versions.notes)
    }

    fn finance_cache_version(&self) -> u64 {
        self.content_versions
            .domain(self.data_version, self.content_versions.finance)
    }

    fn timers_cache_version(&self) -> u64 {
        self.content_versions
            .domain(self.data_version, self.content_versions.timers)
    }

    fn apply_content_changes(&mut self, changes: DesktopContentChanges) {
        self.content_versions.record(self.data_version, changes);
        if changes.notes {
            self.invalidate_knowledge_read_cache();
        }
    }
}
