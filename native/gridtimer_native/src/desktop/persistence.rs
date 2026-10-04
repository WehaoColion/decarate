// v1.1.0.3 Windows - Keep one timer intention across saves and acknowledge timer-only data.
// v1.0.3.17 Windows - Commit timer starts and pauses on the persistence worker.
// v1.0.3.1 Windows - Bind failed saves to their appearance revision and retry only a new choice.
// v1.0.3 Windows - Retain timer summaries only when saved sessions and cache generation still match.
// v1.0.2.2 Windows - Check timer phase identity without allocating knowledge history.
// v1.0.2.1 Windows - Build timer read models on the save worker and move owned snapshots.
// v1.0.2.0 Windows - Make appearance cards report actual selection changes.
// v1.0.1.9 Windows - Repaint the desktop immediately after an appearance selection.
// v1.0.1.8 Windows - Keep theme receipts lightweight on the GUI thread.
// v1.0.1.7 Windows - Persist appearance changes outside the GUI thread.
// v2.22.54 - Invalidate read caches when a saved document becomes current.
// v2.22.52 - Preserve structured knowledge pages through editing and persistence.
// v2.22.44 - Persist automatic timer phases with drafts outside the GUI thread.
use super::*;
use gridtimer_native::runtime::latest_worker::LatestWorker;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DraftRevisions {
    pub note: u64,
    pub slot: u64,
    pub finance: u64,
    pub theme: u64,
}

pub(super) struct NoteInput {
    knowledge: Option<gridtimer_native::knowledge::KnowledgePage>,
    id: String,
    kind: DesktopNoteKind,
    title: String,
    blocks: Vec<DesktopNoteBlock>,
    rich_document: Option<DesktopNoteDocument>,
    pinned: bool,
    folder: Option<String>,
    accent: String,
    existing: Option<DesktopNote>,
    crypto_session: zeroize::Zeroizing<String>,
}

pub(super) struct SnapshotInput {
    base: String,
    base_version: u64,
    state_path: PathBuf,
    root: PathBuf,
    owner: String,
    sync: DesktopSyncSession,
    drafts: DraftRevisions,
    note: Option<NoteInput>,
    slot: Option<(i32, String, String)>,
    finance: Option<DesktopFinanceProfile>,
    theme: Option<i32>,
    timer_phase: Option<PendingTimerPhaseSettlement>,
    timer_action: Option<PendingTimerAction>,
    now: i64,
    #[cfg(test)]
    timer_latency_probe: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TimerActionKind {
    Start,
    Pause,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingTimerAction {
    pub id: u64,
    pub workspace: String,
    pub base_version: u64,
    pub kind: TimerActionKind,
    pub slot_ids: Vec<i32>,
    pub requested_at_epoch_millis: i64,
    pub expected_slots: Vec<TimerActionSlotIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TimerActionSlotIdentity {
    slot_id: i32,
    running_since_epoch_millis: Option<i64>,
    active_run_id: Option<String>,
    running_revision: Option<i64>,
    accumulated_millis: i64,
}

impl TimerActionSlotIdentity {
    fn from_slot(slot: &DesktopSlot) -> Self {
        Self {
            slot_id: slot.id,
            running_since_epoch_millis: slot.running_since_epoch_millis,
            active_run_id: slot
                .extra
                .get("activeRunId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            running_revision: slot
                .extra
                .get("runningUpdatedAtEpochMillis")
                .and_then(Value::as_i64)
                .filter(|revision| *revision > 0),
            accumulated_millis: slot.accumulated_millis,
        }
    }

    fn matches_slot(&self, slot: &DesktopSlot) -> bool {
        let current = Self::from_slot(slot);
        self.slot_id == current.slot_id
            && self.running_since_epoch_millis == current.running_since_epoch_millis
            && self.active_run_id == current.active_run_id
            && self.accumulated_millis == current.accumulated_millis
            // Zero/missing is a legacy fallback, not a run generation. Saving
            // a name fills it from updatedAt without changing the timer. Once
            // an explicit generation exists, it must continue to match exactly.
            && self.running_revision.is_none_or(|revision| current.running_revision == Some(revision))
    }
}

enum TimerSessionUpdate {
    Unchanged,
    Prepend(Vec<DesktopSession>),
    Replace(Vec<DesktopSession>),
}

pub(super) struct TimerOnlyReceipt {
    slots: Vec<DesktopSlot>,
    sessions: TimerSessionUpdate,
}

enum SnapshotDataReceipt {
    Full(DesktopAppData),
    TimerOnly(TimerOnlyReceipt),
    ThemeOnly,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TimerSaveStageTimes {
    controlled_session_micros: u64,
    evidence_refresh_micros: u64,
    privacy_mirrors_micros: u64,
    readable_mirror_micros: u64,
    privacy_finish_micros: u64,
    total_micros: u64,
    // These mirror subtotals are included in the outer mirror stages above.
    mirror_prepare_micros: u64,
    mirror_evidence_micros: u64,
    mirror_file_micros: u64,
    // Disjoint subtotals inside controlled_session_micros. They are emitted
    // only by the explicit desktop timer diagnostic and never grant authority.
    controlled_open_and_full_integrity_micros: u64,
    controlled_schema_foreign_keys_and_owners_micros: u64,
    controlled_history_audit_micros: u64,
    controlled_independent_evidence_micros: u64,
    controlled_parent_selection_micros: u64,
    controlled_protected_sync_micros: u64,
    controlled_privacy_prepare_micros: u64,
    controlled_incoming_analysis_micros: u64,
    controlled_destructive_drop_and_privacy_commit_micros: u64,
    controlled_insert_readback_verify_micros: u64,
    controlled_history_prune_micros: u64,
    controlled_parent_mirror_recheck_micros: u64,
    controlled_sqlite_commit_micros: u64,
    controlled_connection_close_micros: u64,
}

#[cfg(test)]
thread_local! {
    static TIMER_SAVE_STAGES: std::cell::RefCell<Option<TimerSaveStageTimes>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
struct TimerSaveProbeCapture {
    started: Option<Instant>,
    previous: Option<TimerSaveStageTimes>,
}

#[cfg(test)]
impl TimerSaveProbeCapture {
    fn begin(enabled: bool) -> Self {
        Self {
            started: enabled.then(Instant::now),
            previous: if enabled {
                TIMER_SAVE_STAGES
                    .with(|stages| stages.replace(Some(TimerSaveStageTimes::default())))
            } else {
                None
            },
        }
    }

    fn finish(mut self) -> Option<TimerSaveStageTimes> {
        let started = self.started.take()?;
        let mut result = TIMER_SAVE_STAGES.with(|stages| stages.replace(self.previous.take()));
        if let Some(result) = result.as_mut() {
            result.total_micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        }
        result
    }
}

#[cfg(test)]
impl Drop for TimerSaveProbeCapture {
    fn drop(&mut self) {
        // Clear even when the save returns early or unwinds. A later operation
        // on the same writer thread must never inherit this measurement.
        if self.started.is_some() {
            let _ = TIMER_SAVE_STAGES.with(|stages| stages.replace(self.previous.take()));
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum TimerSavePhase {
    ControlledSession,
    EvidenceRefresh,
    PrivacyMirrors,
    ReadableMirror,
    PrivacyFinish,
    MirrorPrepare,
    MirrorEvidence,
    MirrorFile,
    ControlledOpenAndFullIntegrity,
    ControlledSchemaForeignKeysAndOwners,
    ControlledHistoryAudit,
    ControlledIndependentEvidence,
    ControlledParentSelection,
    ControlledProtectedSync,
    ControlledPrivacyPrepare,
    ControlledIncomingAnalysis,
    ControlledDestructiveDropAndPrivacyCommit,
    ControlledInsertReadbackVerify,
    ControlledHistoryPrune,
    ControlledParentMirrorRecheck,
    ControlledSqliteCommit,
    ControlledConnectionClose,
}

#[cfg(test)]
pub(super) struct TimerControlledSaveObserver;

#[cfg(test)]
impl gridtimer_native::desktop_state_store::DesktopSaveObserver for TimerControlledSaveObserver {
    fn measure<T>(
        &mut self,
        phase: gridtimer_native::desktop_state_store::DesktopSavePhase,
        operation: impl FnOnce() -> T,
    ) -> T {
        use gridtimer_native::desktop_state_store::DesktopSavePhase;
        let phase = match phase {
            DesktopSavePhase::OpenAndFullIntegrity => {
                TimerSavePhase::ControlledOpenAndFullIntegrity
            }
            DesktopSavePhase::SchemaForeignKeysAndOwners => {
                TimerSavePhase::ControlledSchemaForeignKeysAndOwners
            }
            DesktopSavePhase::HistoryAudit => TimerSavePhase::ControlledHistoryAudit,
            DesktopSavePhase::IndependentEvidence => TimerSavePhase::ControlledIndependentEvidence,
            DesktopSavePhase::ParentSelection => TimerSavePhase::ControlledParentSelection,
            DesktopSavePhase::ProtectedSync => TimerSavePhase::ControlledProtectedSync,
            DesktopSavePhase::PrivacyPrepare => TimerSavePhase::ControlledPrivacyPrepare,
            DesktopSavePhase::IncomingAnalysis => TimerSavePhase::ControlledIncomingAnalysis,
            DesktopSavePhase::DestructiveDropAndPrivacyCommit => {
                TimerSavePhase::ControlledDestructiveDropAndPrivacyCommit
            }
            DesktopSavePhase::InsertReadbackVerify => {
                TimerSavePhase::ControlledInsertReadbackVerify
            }
            DesktopSavePhase::HistoryPrune => TimerSavePhase::ControlledHistoryPrune,
            DesktopSavePhase::ParentMirrorRecheck => TimerSavePhase::ControlledParentMirrorRecheck,
            DesktopSavePhase::SqliteCommit => TimerSavePhase::ControlledSqliteCommit,
            DesktopSavePhase::ConnectionClose => TimerSavePhase::ControlledConnectionClose,
        };
        let _stage = TimerSaveStageGuard::begin(phase);
        operation()
    }
}

#[cfg(test)]
pub(super) struct TimerSaveStageGuard {
    phase: TimerSavePhase,
    started: Option<Instant>,
}

#[cfg(test)]
impl TimerSaveStageGuard {
    pub(super) fn begin(phase: TimerSavePhase) -> Self {
        Self {
            phase,
            started: TIMER_SAVE_STAGES.with(|stages| stages.borrow().is_some().then(Instant::now)),
        }
    }
}

#[cfg(test)]
impl Drop for TimerSaveStageGuard {
    fn drop(&mut self) {
        let Some(started) = self.started else { return };
        let elapsed = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        TIMER_SAVE_STAGES.with(|stages| {
            let mut stages = stages.borrow_mut();
            let Some(stages) = stages.as_mut() else {
                return;
            };
            let target = match self.phase {
                TimerSavePhase::ControlledSession => &mut stages.controlled_session_micros,
                TimerSavePhase::EvidenceRefresh => &mut stages.evidence_refresh_micros,
                TimerSavePhase::PrivacyMirrors => &mut stages.privacy_mirrors_micros,
                TimerSavePhase::ReadableMirror => &mut stages.readable_mirror_micros,
                TimerSavePhase::PrivacyFinish => &mut stages.privacy_finish_micros,
                TimerSavePhase::MirrorPrepare => &mut stages.mirror_prepare_micros,
                TimerSavePhase::MirrorEvidence => &mut stages.mirror_evidence_micros,
                TimerSavePhase::MirrorFile => &mut stages.mirror_file_micros,
                TimerSavePhase::ControlledOpenAndFullIntegrity => {
                    &mut stages.controlled_open_and_full_integrity_micros
                }
                TimerSavePhase::ControlledSchemaForeignKeysAndOwners => {
                    &mut stages.controlled_schema_foreign_keys_and_owners_micros
                }
                TimerSavePhase::ControlledHistoryAudit => {
                    &mut stages.controlled_history_audit_micros
                }
                TimerSavePhase::ControlledIndependentEvidence => {
                    &mut stages.controlled_independent_evidence_micros
                }
                TimerSavePhase::ControlledParentSelection => {
                    &mut stages.controlled_parent_selection_micros
                }
                TimerSavePhase::ControlledProtectedSync => {
                    &mut stages.controlled_protected_sync_micros
                }
                TimerSavePhase::ControlledPrivacyPrepare => {
                    &mut stages.controlled_privacy_prepare_micros
                }
                TimerSavePhase::ControlledIncomingAnalysis => {
                    &mut stages.controlled_incoming_analysis_micros
                }
                TimerSavePhase::ControlledDestructiveDropAndPrivacyCommit => {
                    &mut stages.controlled_destructive_drop_and_privacy_commit_micros
                }
                TimerSavePhase::ControlledInsertReadbackVerify => {
                    &mut stages.controlled_insert_readback_verify_micros
                }
                TimerSavePhase::ControlledHistoryPrune => {
                    &mut stages.controlled_history_prune_micros
                }
                TimerSavePhase::ControlledParentMirrorRecheck => {
                    &mut stages.controlled_parent_mirror_recheck_micros
                }
                TimerSavePhase::ControlledSqliteCommit => {
                    &mut stages.controlled_sqlite_commit_micros
                }
                TimerSavePhase::ControlledConnectionClose => {
                    &mut stages.controlled_connection_close_micros
                }
            };
            *target = target.saturating_add(elapsed);
        });
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct TimerActionPerfTimes {
    pub action_id: u64,
    pub accepted: Instant,
    pub submitted: Option<Instant>,
    pub worker_started: Option<Instant>,
    pub transform_completed: Option<Instant>,
    pub save_completed: Option<Instant>,
    pub receipt_ready: Option<Instant>,
    pub receipt_applied: Option<Instant>,
    pub save_stages: Option<TimerSaveStageTimes>,
}

fn non_timer_data_matches(before: &DesktopAppData, after: &DesktopAppData) -> bool {
    before.categories == after.categories
        && before.archived_tasks == after.archived_tasks
        && before.note_folders == after.note_folders
        && before.notes == after.notes
        && before.note_preferences == after.note_preferences
        && before.note_preferences_updated_at_epoch_millis
            == after.note_preferences_updated_at_epoch_millis
        && before.finance_profile == after.finance_profile
        && before.theme_mode == after.theme_mode
        && before.oled_theme_enabled == after.oled_theme_enabled
        && before.extra == after.extra
}

fn classify_snapshot_data(
    before: &DesktopAppData,
    mut after: DesktopAppData,
    timer_only_request: bool,
) -> SnapshotDataReceipt {
    // This comparison runs on the writer. Unknown top-level fields count too:
    // sanitization must never silently change a domain omitted by a small receipt.
    if !timer_only_request || !non_timer_data_matches(before, &after) {
        return SnapshotDataReceipt::Full(after);
    }
    let sessions = if before.sessions == after.sessions {
        TimerSessionUpdate::Unchanged
    } else if after.sessions.ends_with(&before.sessions) {
        // Retain the worker-allocated capacity for the existing records. The UI
        // moves their ownership into this buffer without cloning historical text.
        let added = after.sessions.len() - before.sessions.len();
        after.sessions.truncate(added);
        TimerSessionUpdate::Prepend(std::mem::take(&mut after.sessions))
    } else {
        TimerSessionUpdate::Replace(std::mem::take(&mut after.sessions))
    };
    SnapshotDataReceipt::TimerOnly(TimerOnlyReceipt {
        slots: std::mem::take(&mut after.slots),
        sessions,
    })
}

pub(super) struct SnapshotReceipt {
    base_version: u64,
    state_path: PathBuf,
    drafts: DraftRevisions,
    state: String,
    data: SnapshotDataReceipt,
    sessions_unchanged: bool,
    content_changes: DesktopContentChanges,
    timer_projector: app_data::TimerProjector,
    theme_code: Option<i32>,
    note_id: Option<String>,
    unlocked: Option<DesktopNote>,
    slot_saved: bool,
    finance_saved: bool,
    theme_saved: bool,
    timer_phase: Option<PendingTimerPhaseSettlement>,
    timer_action: Option<PendingTimerAction>,
    warning: Option<String>,
    elapsed: Duration,
    #[cfg(test)]
    save_completed: Option<Instant>,
    #[cfg(test)]
    worker_started: Option<Instant>,
    #[cfg(test)]
    transform_completed: Option<Instant>,
    #[cfg(test)]
    receipt_ready: Option<Instant>,
    #[cfg(test)]
    save_stages: Option<TimerSaveStageTimes>,
}

struct SnapshotFailure {
    state_path: PathBuf,
    theme_revision: Option<u64>,
    theme_only: bool,
    timer_action_id: Option<u64>,
    error: String,
}

enum SnapshotCompletion {
    Saved(SnapshotReceipt),
    Failed(SnapshotFailure),
}

pub(super) struct DraftPersistence {
    worker: Option<LatestWorker<SnapshotInput, SnapshotCompletion>>,
    last_submitted: Option<DraftRevisions>,
    failed: bool,
    failed_theme_revision: Option<(PathBuf, u64)>,
    phase_retry_due_millis: i64,
    pub pending_timer_action: Option<PendingTimerAction>,
    timer_action_waiting_for_predecessor: bool,
    timer_action_context: Option<egui::Context>,
    timer_priority_frame: bool,
    #[cfg(test)]
    pub timer_latency_probe_enabled: bool,
    #[cfg(test)]
    pub timer_latency_probe: Option<TimerActionPerfTimes>,
    pub next_timer_action_id: u64,
    pub revisions: DraftRevisions,
    pub waiting_to_close: bool,
    pub committed_revision: u64,
}

pub(super) fn retained_timer_summary_key(
    current_version: u64,
    base_version: u64,
    cache_key: Option<(u64, i64)>,
    current_sessions: &[DesktopSession],
    saved_sessions: &[DesktopSession],
) -> Option<(u64, i64)> {
    let (cached_version, cached_day) = cache_key?;
    // Compare the already decoded records, including each recent-record field.
    // A cache from an older generation must never be made current by a receipt.
    if current_version == base_version
        && cached_version == base_version
        && current_sessions == saved_sessions
    {
        // Preserve the day rather than replacing it with today: the normal
        // summary lookup must still rebuild across a local midnight rollover.
        Some((current_version.saturating_add(1), cached_day))
    } else {
        None
    }
}

impl Default for DraftPersistence {
    fn default() -> Self {
        Self {
            worker: None,
            last_submitted: None,
            failed: false,
            failed_theme_revision: None,
            phase_retry_due_millis: 0,
            pending_timer_action: None,
            timer_action_waiting_for_predecessor: false,
            timer_action_context: None,
            timer_priority_frame: false,
            #[cfg(test)]
            timer_latency_probe_enabled: false,
            #[cfg(test)]
            timer_latency_probe: None,
            next_timer_action_id: 0,
            revisions: DraftRevisions::default(),
            waiting_to_close: false,
            committed_revision: 0,
        }
    }
}

impl DraftPersistence {
    pub(super) fn consume_timer_priority_frame(&mut self) -> bool {
        std::mem::take(&mut self.timer_priority_frame)
    }

    fn clear_timer_action(&mut self) {
        self.pending_timer_action = None;
        self.timer_action_waiting_for_predecessor = false;
        self.timer_action_context = None;
    }
    pub(super) fn failed(&self) -> bool {
        self.failed
    }

    pub fn pending(&self) -> bool {
        self.worker.as_ref().is_some_and(LatestWorker::is_pending)
    }

    pub(super) fn clear_failure_after_workspace_change(&mut self) {
        // The prepared scope has crossed the write barrier. Never detach a
        // pending receipt or replace the worker's monotonic revision counters.
        if self.pending() {
            return;
        }
        self.failed = false;
        self.failed_theme_revision = None;
        self.phase_retry_due_millis = 0;
        self.last_submitted = None;
        self.clear_timer_action();
    }
}

#[derive(Deserialize)]
struct SnapshotTimerState {
    #[serde(default)]
    slots: Vec<DesktopSlot>,
}

pub(super) fn snapshot_has_active_timer_run(raw: &str, slot_id: i32, run_id: &str) -> bool {
    // This guard reads timer identity only. Other fields are skipped by serde;
    // the subsequent shared mutation and sanitization still validate the whole
    // workspace before any write. Keep the original identity semantics, which
    // reject paused slots and missing run IDs rather than inferring a new run.
    serde_json::from_str::<SnapshotTimerState>(raw)
        .ok()
        .is_some_and(|state| {
            state.slots.iter().any(|slot| {
                slot.id == slot_id && desktop_slot_running_identity(slot) == Some(run_id)
            })
        })
}

fn validate_timer_action_snapshot(state: &str, action: &PendingTimerAction) -> Result<(), String> {
    let slots = serde_json::from_str::<SnapshotTimerState>(state)
        .map_err(|_| "计时状态无法验证，原状态已保留".to_string())?
        .slots;
    let ids = action.slot_ids.iter().copied().collect::<HashSet<_>>();
    if ids.len() != action.slot_ids.len() || ids.is_empty() {
        return Err("计时目标无效，原状态已保留".into());
    }
    if action.expected_slots.len() != ids.len()
        || action
            .expected_slots
            .iter()
            .map(|expected| expected.slot_id)
            .collect::<HashSet<_>>()
            != ids
        || action.expected_slots.iter().any(|expected| {
            !ids.contains(&expected.slot_id)
                || !slots.iter().any(|slot| expected.matches_slot(slot))
        })
    {
        return Err("计时记录已变化，未提交过期操作".into());
    }
    let valid = match action.kind {
        TimerActionKind::Start => {
            ids.len() == 1
                && slots
                    .iter()
                    .any(|slot| ids.contains(&slot.id) && slot.running_since_epoch_millis.is_none())
        }
        TimerActionKind::Pause => {
            slots
                .iter()
                .filter(|slot| ids.contains(&slot.id) && slot.running_since_epoch_millis.is_some())
                .count()
                == ids.len()
        }
    };
    if !valid {
        return Err("计时状态已变化，未提交过期操作".into());
    }
    Ok(())
}

fn apply_timer_action_to_snapshot(
    state: &str,
    action: &PendingTimerAction,
) -> Result<String, String> {
    validate_timer_action_snapshot(state, action)?;
    match action.kind {
        TimerActionKind::Start => app_data::start_slot_app_data_json(
            state,
            action.slot_ids[0],
            action.requested_at_epoch_millis,
        ),
        TimerActionKind::Pause => app_data::pause_slots_app_data_json(
            state,
            &action.slot_ids,
            action.requested_at_epoch_millis,
        ),
    }
    .ok_or_else(|| "计时变换未通过验证，原状态已保留".into())
}

fn commit_drafts(input: SnapshotInput) -> Result<SnapshotCompletion, String> {
    let mut failure = SnapshotFailure {
        state_path: input.state_path.clone(),
        theme_revision: input.theme.map(|_| input.drafts.theme),
        theme_only: input.theme.is_some()
            && input.note.is_none()
            && input.slot.is_none()
            && input.finance.is_none()
            && input.timer_phase.is_none()
            && input.timer_action.is_none(),
        timer_action_id: input.timer_action.as_ref().map(|action| action.id),
        error: String::new(),
    };
    // Keep request identity even when the write fails. LatestWorker's outer
    // error remains reserved for a panic or disconnected worker with no proof.
    Ok(match write_drafts(input) {
        Ok(saved) => SnapshotCompletion::Saved(saved),
        Err(error) => {
            failure.error = error;
            SnapshotCompletion::Failed(failure)
        }
    })
}

fn write_drafts(input: SnapshotInput) -> Result<SnapshotReceipt, String> {
    let started = Instant::now();
    #[cfg(test)]
    let worker_started = input.timer_latency_probe.then(Instant::now);
    let theme_only = input.theme.is_some()
        && input.note.is_none()
        && input.slot.is_none()
        && input.finance.is_none()
        && input.timer_phase.is_none()
        && input.timer_action.is_none();
    let timer_only_request = (input.timer_action.is_some() || input.timer_phase.is_some())
        && input.note.is_none()
        && input.slot.is_none()
        && input.finance.is_none()
        && input.theme.is_none();
    // The request already owns its snapshot. Moving it avoids allocating and
    // copying the complete workspace a second time on every draft save.
    let previous_data = (!theme_only).then(|| decode_data(&input.base));
    let mut state = input.base;
    let invalid = || "草稿变换未通过验证，原状态已保留".to_string();
    let mut unlocked = None;
    let mut declaration = None;
    let note_id = input.note.as_ref().map(|note| note.id.clone());
    if let Some(note) = input.note {
        let mut value = desktop_note_canvas_save_value(
            &note.id,
            note.kind,
            &note.title,
            &note.blocks,
            note.pinned,
            note.folder.as_deref(),
            &note.accent,
            note.existing.as_ref(),
            input.now,
        );
        apply_rich_document_to_note_value(&mut value, note.rich_document.as_ref());
        apply_knowledge_to_note_value(&mut value, note.knowledge.as_ref());
        let mut note_json =
            zeroize::Zeroizing::new(serde_json::to_string(&value).map_err(|_| invalid())?);
        if note
            .existing
            .as_ref()
            .is_some_and(|note| note.encryption.is_some())
        {
            let sealed = gridtimer_native::seal_desktop_note_json(&note_json, &note.crypto_session)
                .ok_or_else(|| "加密草稿密封失败，未写入明文".to_string())?;
            declaration =
                gridtimer_native::desktop_state_store::DesktopSealedMediaDeclaration::from_session(
                    &sealed,
                    &note.crypto_session,
                );
            unlocked = serde_json::from_str::<DesktopNote>(&note_json).ok();
            *note_json = sealed;
        }
        state = app_data::upsert_note_app_data_json(&state, &note_json, input.now)
            .ok_or_else(invalid)?;
    }
    let slot_saved = input.slot.is_some();
    if let Some((id, title, note)) = input.slot {
        state = app_data::update_slot_title_app_data_json(&state, id, &title, input.now)
            .ok_or_else(invalid)?;
        state = app_data::update_slot_note_app_data_json(&state, id, &note, input.now)
            .ok_or_else(invalid)?;
    }
    let finance_saved = input.finance.is_some();
    if let Some(finance) = input.finance {
        let encoded = serde_json::to_string(&finance).map_err(|_| invalid())?;
        state = app_data::update_finance_profile_app_data_json(&state, &encoded, input.now)
            .ok_or_else(invalid)?;
    }
    let requested_theme = input.theme;
    let theme_saved = requested_theme.is_some();
    if let Some(theme) = requested_theme {
        state =
            app_data::set_theme_mode_app_data_json(&state, theme, input.now).ok_or_else(invalid)?;
    }
    let timer_phase = input.timer_phase;
    let timer_action = input.timer_action;
    let prepared_timer = if timer_only_request && timer_phase.is_none() {
        if let Some(action) = timer_action.as_ref() {
            if action.base_version != input.base_version {
                return Err("计时请求对应的工作区版本已变化，原状态已保留".into());
            }
            // Keep the original snapshot identity check before normalization;
            // preparing a faster read model never authorizes an old request.
            validate_timer_action_snapshot(&state, action)?;
            let mutation = match action.kind {
                TimerActionKind::Start => app_data::DesktopTimerMutation::Start(action.slot_ids[0]),
                TimerActionKind::Pause => app_data::DesktopTimerMutation::Pause(&action.slot_ids),
            };
            Some(
                app_data::prepare_desktop_timer_mutation(
                    &state,
                    mutation,
                    action.requested_at_epoch_millis,
                    input.now,
                )
                .ok_or_else(invalid)?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let (next_state, timer_projector) = if let Some(prepared) = prepared_timer {
        (prepared.state_json, prepared.timer_projector)
    } else {
        if let Some(phase) = timer_phase.as_ref() {
            if !snapshot_has_active_timer_run(&state, phase.slot_id, &phase.run_id) {
                return Err("计时状态已变化，未提交过期记录".into());
            }
            state = app_data::start_slot_app_data_json(&state, phase.slot_id, input.now)
                .ok_or_else(invalid)?;
        }
        if let Some(action) = timer_action.as_ref() {
            if action.base_version != input.base_version {
                return Err("计时请求对应的工作区版本已变化，原状态已保留".into());
            }
            state = apply_timer_action_to_snapshot(&state, action)?;
        }
        let state = app_data::sanitize_app_data_json(&state, input.now).ok_or_else(invalid)?;
        // Mixed drafts and phase settlement retain their existing mutation path.
        let timer_projector = app_data::TimerProjector::parse(&state).map_err(|_| invalid())?;
        (state, timer_projector)
    };
    // Both branches own their result. Release the superseded source on this
    // worker before persistence allocates its transaction and mirror buffers.
    drop(state);
    let state = next_state;
    #[cfg(test)]
    let transform_completed = input.timer_latency_probe.then(Instant::now);
    #[cfg(test)]
    let save_probe = TimerSaveProbeCapture::begin(input.timer_latency_probe);
    let saved = save_state_snapshot_with_store_and_media_declaration(
        &input.root,
        &input.state_path,
        &input.owner,
        &state,
        &input.sync,
        input.now,
        "local_save",
        declaration.as_ref(),
    )
    .map_err(|error| error.to_string())?;
    #[cfg(test)]
    let save_stages = save_probe.finish();
    #[cfg(test)]
    let save_completed = input.timer_latency_probe.then(Instant::now);
    // Decoding the complete workspace is unnecessary for a theme-only receipt.
    // Keep that allocation and all note/document drops on the worker thread;
    // the GUI only needs the small theme code to update its display model.
    let data = (!theme_only).then(|| decode_data(&state));
    let sessions_unchanged = previous_data
        .as_ref()
        .zip(data.as_ref())
        .is_some_and(|(before, after)| before.sessions == after.sessions);
    let content_changes = previous_data
        .as_ref()
        .zip(data.as_ref())
        .map(|(before, after)| DesktopContentChanges::between(before, after))
        .unwrap_or_default();
    let data = match (previous_data.as_ref(), data) {
        (Some(before), Some(after)) => classify_snapshot_data(before, after, timer_only_request),
        _ => SnapshotDataReceipt::ThemeOnly,
    };
    #[cfg(test)]
    let receipt_ready = input.timer_latency_probe.then(Instant::now);
    Ok(SnapshotReceipt {
        base_version: input.base_version,
        state_path: input.state_path,
        drafts: input.drafts,
        state,
        data,
        sessions_unchanged,
        content_changes,
        timer_projector,
        theme_code: theme_only.then_some(requested_theme).flatten(),
        note_id,
        unlocked,
        slot_saved,
        finance_saved,
        theme_saved,
        timer_phase,
        timer_action,
        warning: saved.warning().map(str::to_owned),
        elapsed: started.elapsed(),
        #[cfg(test)]
        save_completed,
        #[cfg(test)]
        worker_started,
        #[cfg(test)]
        transform_completed,
        #[cfg(test)]
        receipt_ready,
        #[cfg(test)]
        save_stages,
    })
}

impl TimerWindowsClient {
    pub(super) fn request_timer_action(
        &mut self,
        kind: TimerActionKind,
        slot_ids: Vec<i32>,
        ctx: &egui::Context,
    ) {
        if self.persistence.waiting_to_close
            || !matches!(self.shutdown_state, ClientShutdownState::Running)
        {
            // Tray events continue to arrive while shutdown drains. Only an
            // intention accepted before the close barrier may still be resumed.
            self.status = "正在关闭，未提交新的计时操作".into();
            return;
        }
        if !self.workspace_persistence_ready || self.workspace_edit_locked() {
            self.status = "工作区暂时不可写，计时未改变".into();
            return;
        }
        if self.persistence.pending_timer_action.is_some() {
            // An accepted intention is never replaced by a later click, even
            // while another draft owns the single writer.
            ctx.request_repaint();
            return;
        }
        let expected_slots = slot_ids
            .iter()
            .filter_map(|id| {
                self.data
                    .slots
                    .iter()
                    .find(|slot| slot.id == *id)
                    .map(TimerActionSlotIdentity::from_slot)
            })
            .collect::<Vec<_>>();
        if slot_ids.is_empty() || expected_slots.len() != slot_ids.len() {
            self.status = "计时目标已变化，请重新操作".into();
            return;
        }
        self.persistence.next_timer_action_id =
            self.persistence.next_timer_action_id.saturating_add(1);
        let action_id = self.persistence.next_timer_action_id;
        self.persistence.pending_timer_action = Some(PendingTimerAction {
            id: action_id,
            workspace: self.background_job_workspace_fingerprint(),
            base_version: self.data_version,
            kind,
            slot_ids,
            requested_at_epoch_millis: now_millis(),
            expected_slots,
        });
        self.persistence.timer_action_waiting_for_predecessor =
            self.persistence.pending() || self.external_workspace_save_pending();
        self.persistence.timer_action_context = Some(ctx.clone());
        self.persistence.timer_priority_frame = true;
        #[cfg(test)]
        if self.persistence.timer_latency_probe_enabled {
            self.persistence.timer_latency_probe = Some(TimerActionPerfTimes {
                action_id,
                accepted: Instant::now(),
                submitted: None,
                worker_started: None,
                transform_completed: None,
                save_completed: None,
                receipt_ready: None,
                receipt_applied: None,
                save_stages: None,
            });
        }
        self.status = match kind {
            TimerActionKind::Start => "正在开始计时",
            TimerActionKind::Pause => "正在暂停计时",
        }
        .into();
        ctx.request_repaint();
        if !self.persistence.timer_action_waiting_for_predecessor {
            self.submit_draft_snapshot(ctx, true);
        }
    }

    pub(super) fn resume_queued_timer_action(&mut self) {
        if !self.persistence.timer_action_waiting_for_predecessor
            || self.persistence.pending()
            || self.external_workspace_save_pending()
        {
            return;
        }
        let valid = self.workspace_persistence_ready
            && !self.workspace_edit_locked()
            && !self.persistence.failed
            && self
                .persistence
                .pending_timer_action
                .as_ref()
                .is_some_and(|action| {
                    action.workspace == self.background_job_workspace_fingerprint()
                        && action.expected_slots.iter().all(|expected| {
                            self.data
                                .slots
                                .iter()
                                .any(|slot| expected.matches_slot(slot))
                        })
                });
        if !valid {
            self.persistence.timer_priority_frame = true;
            self.persistence.clear_timer_action();
            self.status = "前项保存或计时状态已变化，计时未改变，请重新操作".into();
            return;
        }
        let Some(ctx) = self.persistence.timer_action_context.clone() else {
            self.persistence.clear_timer_action();
            self.status = "计时请求无法继续，请重新操作".into();
            return;
        };
        self.persistence
            .pending_timer_action
            .as_mut()
            .unwrap()
            .base_version = self.data_version;
        self.persistence.timer_action_waiting_for_predecessor = false;
        self.persistence.last_submitted = None;
        self.submit_draft_snapshot(&ctx, true);
        ctx.request_repaint();
    }

    pub(super) fn cancel_waiting_timer_action(&mut self, reason: &str) {
        if !self.persistence.timer_action_waiting_for_predecessor {
            return;
        }
        if let Some(ctx) = self.persistence.timer_action_context.as_ref() {
            ctx.request_repaint();
        }
        self.persistence.timer_priority_frame = true;
        self.persistence.clear_timer_action();
        self.status = format!("计时未改变：{reason}");
    }

    fn draft_save_failed(&mut self, error: String) {
        if self.persistence.timer_action_waiting_for_predecessor {
            self.persistence.timer_priority_frame = true;
            self.persistence.clear_timer_action();
        }
        self.persistence.failed = true;
        self.persistence.failed_theme_revision = None;
        self.persistence.last_submitted = None;
        self.persistence.phase_retry_due_millis =
            if self.desktop_ui.pending_phase_settlement.is_some()
                && !self.note_dirty
                && !self.slot_dirty
                && !self.finance_dirty
                && !self.theme_dirty
            {
                save_clock_millis() + 5_000
            } else {
                0
            };
        if self.note_dirty {
            self.note_save_state = DesktopDocumentSaveState::Failed;
        }
        self.status = format!("保存失败，待保存内容仍保留：{error}");
        if self.persistence.waiting_to_close {
            self.persistence.waiting_to_close = false;
            self.shutdown_state = ClientShutdownState::SaveFailed { message: error };
        }
    }

    fn apply_draft_failure(&mut self, failure: SnapshotFailure) {
        if failure.state_path == self.state_path
            && failure.timer_action_id.is_some_and(|id| {
                self.persistence
                    .pending_timer_action
                    .as_ref()
                    .map(|action| action.id)
                    == Some(id)
            })
        {
            self.persistence.timer_priority_frame = true;
            self.persistence.clear_timer_action();
        }
        self.draft_save_failed(failure.error);
        if failure.state_path != self.state_path {
            return;
        }
        if let Some(revision) = failure.theme_revision {
            if self.theme_dirty && revision == self.persistence.revisions.theme {
                // Roll back only the preview that actually failed. A later
                // appearance selection is still pending and belongs to the user.
                self.theme_dirty = false;
                self.theme_save_due_epoch_millis = 0;
                self.pending_theme_code = None;
            }
            if failure.theme_only {
                self.persistence.failed_theme_revision = Some((failure.state_path, revision));
            }
        }
    }

    fn current_draft_save_failed(&mut self, error: String) {
        self.apply_draft_failure(SnapshotFailure {
            state_path: self.state_path.clone(),
            theme_revision: self.theme_dirty.then_some(self.persistence.revisions.theme),
            theme_only: self.theme_dirty
                && !self.note_dirty
                && !self.slot_dirty
                && !self.finance_dirty
                && self.desktop_ui.pending_phase_settlement.is_none()
                && self.persistence.pending_timer_action.is_none(),
            timer_action_id: self
                .persistence
                .pending_timer_action
                .as_ref()
                .map(|action| action.id),
            error,
        });
    }

    pub(super) fn submit_draft_snapshot(&mut self, ctx: &egui::Context, force: bool) {
        if !self.workspace_persistence_ready || self.workspace_edit_locked() {
            if self.persistence.pending_timer_action.is_some() {
                self.persistence.timer_priority_frame = true;
                self.persistence.clear_timer_action();
                self.status = "当前工作区尚不能写入，计时状态未改变".into();
            }
            return;
        }
        if self.external_workspace_save_pending() {
            return;
        }
        if self.persistence.pending_timer_action.is_some() && self.persistence.pending() {
            return;
        }
        if self
            .persistence
            .pending_timer_action
            .as_ref()
            .is_some_and(|action| {
                action.workspace != self.background_job_workspace_fingerprint()
                    || action.base_version != self.data_version
            })
        {
            self.persistence.timer_priority_frame = true;
            self.persistence.clear_timer_action();
            self.status = "计时请求对应的工作区已变化，请重新操作".into();
            return;
        }
        // A timer click must be submitted exactly once. LatestWorker can replace
        // an unsent draft request, so leave subsequent drafts for the receipt.
        self.retry_pending_timer_phase_settlement();
        let timer_action = self.persistence.pending_timer_action.clone();
        // Pausing a run already settles its elapsed phase. Restarting that run
        // first would give it a start time later than the user's pause click.
        let timer_phase = self
            .desktop_ui
            .pending_phase_settlement
            .clone()
            .filter(|phase| {
                !timer_action.as_ref().is_some_and(|action| {
                    action.kind == TimerActionKind::Pause
                        && action.slot_ids.contains(&phase.slot_id)
                })
            });
        let finance_ready = self.finance_money_inputs_ready();
        let save_finance = self.finance_dirty && finance_ready;
        if !self.note_dirty
            && !self.slot_dirty
            && !save_finance
            && !self.theme_dirty
            && timer_phase.is_none()
            && timer_action.is_none()
        {
            if !finance_ready {
                self.status = "金额输入尚未完成，请先修正或恢复原金额".into();
            }
            return;
        }
        if self.persistence.last_submitted == Some(self.persistence.revisions)
            && self.persistence.pending()
        {
            return;
        }
        let retry_phase = timer_phase.is_some()
            && !self.note_dirty
            && !self.slot_dirty
            && !self.finance_dirty
            && !self.theme_dirty
            && timer_action.is_none()
            && self.persistence.phase_retry_due_millis > 0
            && save_clock_millis() >= self.persistence.phase_retry_due_millis;
        // A distinct user choice may retry a failed appearance-only request.
        // Repaints alone, content failures and unknown worker failures stay
        // blocked until an explicit save; no repeating I/O failure loop is added.
        let retry_theme =
            self.theme_dirty
                && !self.persistence.pending()
                && !self.note_dirty
                && !self.slot_dirty
                && !self.finance_dirty
                && timer_phase.is_none()
                && timer_action.is_none()
                && self.persistence.failed_theme_revision.as_ref().is_some_and(
                    |(path, revision)| {
                        *path == self.state_path && self.persistence.revisions.theme > *revision
                    },
                );
        if self.persistence.failed
            && !force
            && !retry_phase
            && !retry_theme
            && timer_action.is_none()
        {
            return;
        }
        if self.selected_note_is_locked() && self.note_dirty {
            self.current_draft_save_failed("请先解锁笔记，再保存修改".into());
            return;
        }
        if self.persistence.worker.is_none() {
            let ctx = ctx.clone();
            match LatestWorker::spawn("desktop-persistence", commit_drafts, move || {
                ctx.request_repaint()
            }) {
                Ok(worker) => self.persistence.worker = Some(worker),
                Err(error) => {
                    self.current_draft_save_failed(format!("无法启动保存线程：{error}"));
                    return;
                }
            }
        }
        let owner = match desktop_state_owner(
            &self.sync.server_instance_id,
            &self.sync.account_namespace,
            &self.sync.user_id,
        ) {
            Ok(owner) => owner,
            Err(error) => {
                self.current_draft_save_failed(format!("无法保存工作区：{error}"));
                return;
            }
        };
        if self.note_dirty && self.selected_note_id.is_empty() {
            self.selected_note_id = random_desktop_identifier("note");
        }
        let note = self.note_dirty.then(|| NoteInput {
            knowledge: self.desktop_ui.knowledge.page.clone(),
            id: self.selected_note_id.clone(),
            kind: self.note_kind_draft,
            title: self.note_title_draft.clone(),
            blocks: self.note_blocks_draft.clone(),
            rich_document: self.rich_document_for_save().cloned(),
            pinned: self.note_pinned_draft,
            folder: self.note_folder_draft.clone(),
            accent: self.note_accent_draft.clone(),
            existing: self.selected_note(),
            crypto_session: zeroize::Zeroizing::new(self.note_crypto_session_token.clone()),
        });
        let input = SnapshotInput {
            base: self.state_json.clone(),
            base_version: self.data_version,
            state_path: self.state_path.clone(),
            root: self
                .sync_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(app_dir),
            owner,
            sync: self.sync.clone(),
            drafts: self.persistence.revisions,
            note,
            slot: (self.slot_dirty && self.slot_draft_id > 0).then(|| {
                (
                    self.slot_draft_id,
                    self.slot_title_draft.clone(),
                    self.slot_note_draft.clone(),
                )
            }),
            finance: save_finance.then(|| self.finance_draft.clone()),
            theme: self.theme_dirty.then(|| {
                self.pending_theme_code
                    .unwrap_or_else(|| theme_preference_code(desktop_theme_preference(&self.data)))
            }),
            timer_phase,
            timer_action,
            now: now_millis(),
            #[cfg(test)]
            timer_latency_probe: self.persistence.timer_latency_probe_enabled
                && self.persistence.pending_timer_action.is_some(),
        };
        if (input.timer_phase.is_some() || input.timer_action.is_some()) && self.sync_task.is_some()
        {
            self.sync_state_changed_since_request = true;
        }
        #[cfg(test)]
        if let Some(timing) = self
            .persistence
            .timer_latency_probe
            .as_mut()
            .filter(|timing| {
                input
                    .timer_action
                    .as_ref()
                    .is_some_and(|action| action.id == timing.action_id)
            })
        {
            timing.submitted = Some(Instant::now());
        }
        match self
            .persistence
            .worker
            .as_mut()
            .unwrap()
            .submit_latest(input)
        {
            Ok(_) => {
                self.persistence.last_submitted = Some(self.persistence.revisions);
                self.persistence.failed = false;
                self.persistence.failed_theme_revision = None;
                self.persistence.phase_retry_due_millis = 0;
                if self.note_dirty {
                    self.note_save_state = DesktopDocumentSaveState::Saving;
                }
                self.status =
                    if let Some(action) = self.persistence.pending_timer_action.as_ref() {
                        match action.kind {
                            TimerActionKind::Start => "正在开始计时",
                            TimerActionKind::Pause => "正在暂停计时",
                        }
                    } else if self.note_dirty || self.slot_dirty || self.finance_dirty {
                        "保存中"
                    } else if self.theme_dirty {
                        "外观保存中"
                    } else {
                        "计时记录保存中"
                    }
                    .to_string();
            }
            Err(error) => self.current_draft_save_failed(format!("无法提交保存：{error}")),
        }
    }

    pub(super) fn poll_draft_persistence(&mut self) {
        loop {
            let completion = self
                .persistence
                .worker
                .as_mut()
                .and_then(LatestWorker::try_recv);
            let Some(completion) = completion else {
                break;
            };
            match completion.result {
                Ok(SnapshotCompletion::Saved(saved)) => {
                    // No unrelated state mutation may pass the barrier while
                    // writes are outstanding. Receipts from another scope are never applied.
                    if saved.state_path != self.state_path
                        || saved.base_version > self.data_version
                        || saved.timer_phase.as_ref().is_some_and(|phase| {
                            phase.workspace != self.background_job_workspace_fingerprint()
                        })
                        || saved.timer_action.as_ref().is_some_and(|action| {
                            action.workspace != self.background_job_workspace_fingerprint()
                                || action.base_version != saved.base_version
                                || action.base_version != self.data_version
                                || self.persistence.pending_timer_action.as_ref() != Some(action)
                        })
                    {
                        if saved.timer_action.as_ref().is_some_and(|action| {
                            self.persistence
                                .pending_timer_action
                                .as_ref()
                                .map(|pending| pending.id)
                                == Some(action.id)
                        }) {
                            self.persistence.timer_priority_frame = true;
                            self.persistence.clear_timer_action();
                        }
                        self.draft_save_failed("保存回执与当前工作区不匹配".into());
                        continue;
                    }
                    let timer_action = saved.timer_action.clone();
                    let retained_timer_stats_key = self
                        .desktop_ui
                        .parity
                        .timer_stats_key
                        .filter(|(version, _)| {
                            self.data_version == saved.base_version
                                && *version == self.timers_cache_version()
                                && saved.sessions_unchanged
                        })
                        .map(|(_, day)| day);
                    self.state_json = saved.state;
                    match saved.data {
                        SnapshotDataReceipt::Full(data) => self.data = data,
                        SnapshotDataReceipt::TimerOnly(mut data) => {
                            self.data.slots = data.slots;
                            match &mut data.sessions {
                                TimerSessionUpdate::Unchanged => {}
                                TimerSessionUpdate::Prepend(added) => {
                                    added.append(&mut self.data.sessions);
                                    self.data.sessions = std::mem::take(added);
                                }
                                TimerSessionUpdate::Replace(sessions) => {
                                    self.data.sessions = std::mem::take(sessions);
                                }
                            }
                        }
                        SnapshotDataReceipt::ThemeOnly => {
                            if let Some(theme_code) = saved.theme_code {
                                apply_theme_code_to_data(&mut self.data, theme_code);
                            }
                        }
                    }
                    self.data_version = self.data_version.saturating_add(1);
                    self.apply_content_changes(saved.content_changes);
                    if let Some(day) = retained_timer_stats_key {
                        self.desktop_ui.parity.timer_stats_key =
                            Some((self.timers_cache_version(), day));
                    }
                    // This model belongs to the exact snapshot adopted above.
                    // Apply it only after the scope/generation checks; live slot
                    // drafts remain separate and keep their revision guards.
                    self.desktop_ui.projection =
                        Some(Arc::new(saved.timer_projector.project(now_millis())));
                    self.desktop_ui.projector = Some(saved.timer_projector);
                    self.desktop_ui.projection_version = self.timers_cache_version();
                    self.persistence.committed_revision = completion.revision;
                    if timer_action.as_ref().is_some_and(|action| {
                        self.persistence.pending_timer_action.as_ref() == Some(action)
                    }) {
                        self.persistence.timer_priority_frame = true;
                        self.persistence.clear_timer_action();
                        self.reconcile_bell_markers(now_millis());
                    }
                    if let Some(phase) = saved.timer_phase.as_ref() {
                        if self.desktop_ui.pending_phase_settlement.as_ref() == Some(phase) {
                            self.desktop_ui.pending_phase_settlement = None;
                        }
                        self.refresh_timer_projection(now_millis());
                    }
                    if !self.persistence.pending() {
                        self.persistence.failed = false;
                        self.persistence.failed_theme_revision = None;
                    }
                    if let Some(id) = saved.note_id {
                        if id == self.selected_note_id
                            && saved.drafts.note == self.persistence.revisions.note
                        {
                            self.note_dirty = false;
                            self.note_save_due_epoch_millis = 0;
                            self.note_save_state = DesktopDocumentSaveState::LocalSaved;
                            if saved.unlocked.is_some() {
                                self.note_unlocked_record = saved.unlocked;
                            }
                        }
                    }
                    if saved.slot_saved && saved.drafts.slot == self.persistence.revisions.slot {
                        self.slot_dirty = false;
                        self.slot_save_due_epoch_millis = 0;
                    }
                    if saved.finance_saved
                        && saved.drafts.finance == self.persistence.revisions.finance
                    {
                        self.finance_dirty = false;
                        self.finance_save_due_epoch_millis = 0;
                    }
                    if saved.theme_saved && saved.drafts.theme == self.persistence.revisions.theme {
                        self.theme_dirty = false;
                        self.theme_save_due_epoch_millis = 0;
                        self.pending_theme_code = None;
                    }
                    self.last_state_mirror_warning = saved.warning.clone();
                    self.status = saved.warning.unwrap_or_else(|| {
                        if let Some(action) = timer_action.as_ref() {
                            match action.kind {
                                TimerActionKind::Start => "计时已开始".into(),
                                TimerActionKind::Pause => "计时已暂停".into(),
                            }
                        } else if !self.finance_money_inputs_ready() {
                            "金额输入尚未完成，请先修正或恢复原金额".into()
                        } else if self.note_dirty || self.slot_dirty || self.finance_dirty {
                            "保存中".into()
                        } else if self.theme_dirty {
                            "外观保存中".into()
                        } else {
                            "已保存".into()
                        }
                    });
                    // Only sizes/timing, never content or credentials.
                    self.last_save_millis = saved.elapsed.as_millis().min(u64::MAX as u128) as u64;
                    self.rebuild_view_cache();
                    if let Some(action) = timer_action.as_ref().filter(|action| {
                        action.kind == TimerActionKind::Start && self.settings.timer_bell_enabled
                    }) {
                        let kind = self
                            .desktop_ui
                            .projection
                            .as_ref()
                            .and_then(|projection| {
                                projection
                                    .slots
                                    .iter()
                                    .find(|slot| slot.id == action.slot_ids[0])
                            })
                            .map(|slot| {
                                if slot.micro_break_phase == app_data::TimerViewPhase::Break {
                                    DesktopTimerBellKind::Break
                                } else {
                                    DesktopTimerBellKind::Focus
                                }
                            });
                        if let Some(kind) = kind {
                            let _ = self.start_timer_bell(kind);
                        }
                    }
                    #[cfg(test)]
                    if let Some(timing) =
                        self.persistence
                            .timer_latency_probe
                            .as_mut()
                            .filter(|timing| {
                                timer_action
                                    .as_ref()
                                    .is_some_and(|action| action.id == timing.action_id)
                            })
                    {
                        timing.save_completed = saved.save_completed;
                        timing.worker_started = saved.worker_started;
                        timing.transform_completed = saved.transform_completed;
                        timing.receipt_ready = saved.receipt_ready;
                        timing.receipt_applied = Some(Instant::now());
                        timing.save_stages = saved.save_stages;
                    }
                }
                Ok(SnapshotCompletion::Failed(failure)) => self.apply_draft_failure(failure),
                Err(error) => {
                    if self.persistence.pending_timer_action.is_some() {
                        self.persistence.timer_priority_frame = true;
                        self.persistence.clear_timer_action();
                        // A disconnected writer has no durable receipt. Require
                        // journal recovery before another timer mutation.
                        self.workspace_persistence_ready = false;
                        self.draft_save_failed(format!(
                            "计时保存结果无法确认，请重新打开客户端核验工作区：{error}"
                        ));
                    } else {
                        self.draft_save_failed(error);
                    }
                }
            }
        }
        self.resume_queued_timer_action();
    }

    pub(super) fn draft_write_barrier(&mut self) -> io::Result<()> {
        self.poll_draft_persistence();
        if self.persistence.pending() || self.external_workspace_save_pending() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "草稿正在保存，完成后可继续此操作",
            ));
        }
        Ok(())
    }
}

pub(super) fn save_clock_millis() -> i64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min((i64::MAX - 10_000) as u128) as i64
        + 1
}

pub(super) fn random_desktop_identifier(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "{prefix}-{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod timer_action_worker_tests {
    use super::*;

    #[test]
    fn timer_action_rejects_replays_and_commits_one_pause_session() {
        let state = app_data::default_app_data_json(1_000);
        let start = PendingTimerAction {
            id: 1,
            workspace: "test-workspace".into(),
            base_version: 1,
            kind: TimerActionKind::Start,
            slot_ids: vec![1],
            requested_at_epoch_millis: 2_000,
            expected_slots: vec![TimerActionSlotIdentity::from_slot(
                &decode_data(&state).slots[0],
            )],
        };
        let running = apply_timer_action_to_snapshot(&state, &start).unwrap();
        assert!(apply_timer_action_to_snapshot(&running, &start).is_err());
        let pause = PendingTimerAction {
            id: 2,
            kind: TimerActionKind::Pause,
            requested_at_epoch_millis: 7_000,
            expected_slots: vec![TimerActionSlotIdentity::from_slot(
                &decode_data(&running).slots[0],
            )],
            ..start
        };
        let paused = apply_timer_action_to_snapshot(&running, &pause).unwrap();
        assert!(apply_timer_action_to_snapshot(&paused, &pause).is_err());
        let saved = decode_data(&paused);
        assert!(saved
            .slots
            .iter()
            .find(|slot| slot.id == 1)
            .unwrap()
            .running_since_epoch_millis
            .is_none());
        assert_eq!(saved.sessions.len(), 1);
        assert_eq!(saved.sessions[0].ended_at_epoch_millis, 7_000);
    }

    #[test]
    fn timer_action_checks_original_run_identity_even_when_running_state_matches() {
        let state =
            app_data::start_slot_app_data_json(&app_data::default_app_data_json(1_000), 1, 2_000)
                .unwrap();
        let original = decode_data(&state);
        let pause = PendingTimerAction {
            id: 1,
            workspace: "test".into(),
            base_version: 1,
            kind: TimerActionKind::Pause,
            slot_ids: vec![1],
            requested_at_epoch_millis: 7_000,
            expected_slots: vec![TimerActionSlotIdentity::from_slot(&original.slots[0])],
        };
        let mut changed: Value = serde_json::from_str(&state).unwrap();
        changed["slots"][0]["activeRunId"] = json!("another-run");
        assert!(apply_timer_action_to_snapshot(&changed.to_string(), &pause).is_err());
        let mut changed: Value = serde_json::from_str(&state).unwrap();
        changed["slots"][0]["runningUpdatedAtEpochMillis"] = json!(6_000);
        assert!(apply_timer_action_to_snapshot(&changed.to_string(), &pause).is_err());
        assert!(apply_timer_action_to_snapshot(&state, &pause).is_ok());
    }

    #[test]
    fn timer_only_receipt_falls_back_when_any_non_timer_domain_changes() {
        let before = decode_data(&app_data::default_app_data_json(1_000));
        assert!(matches!(
            classify_snapshot_data(&before, before.clone(), true),
            SnapshotDataReceipt::TimerOnly(_)
        ));
        let mut variants = Vec::new();
        let mut after = before.clone();
        after.categories[0].name.push('x');
        variants.push(after);
        let mut after = before.clone();
        after.archived_tasks.push(DesktopArchivedTask::default());
        variants.push(after);
        let mut after = before.clone();
        after.note_preferences_updated_at_epoch_millis += 1;
        variants.push(after);
        let mut after = before.clone();
        after.notes.push(DesktopNote::default());
        variants.push(after);
        let mut after = before.clone();
        after.extra.insert("futureHeader".into(), json!(1));
        variants.push(after);
        let mut after = before.clone();
        after.theme_mode.push('x');
        variants.push(after);
        for after in variants {
            assert!(matches!(
                classify_snapshot_data(&before, after, true),
                SnapshotDataReceipt::Full(_)
            ));
        }
        assert!(matches!(
            classify_snapshot_data(&before, before.clone(), false),
            SnapshotDataReceipt::Full(_)
        ));
    }

    #[test]
    fn timer_receipt_rejects_wrong_workspace_base_version_and_action_identity() {
        for mismatch in ["valid", "workspace", "base_version", "action_id"] {
            let root = std::env::temp_dir().join(random_desktop_identifier("timer-receipt-scope"));
            fs::create_dir_all(&root).unwrap();
            let mut client =
                TimerWindowsClient::load_from_root(root.clone(), Arc::new(AtomicBool::new(false)));
            assert!(client.workspace_persistence_ready, "{}", client.status);
            client.settings.timer_bell_enabled = false;
            client.save_state().unwrap();
            let before = client.state_json.clone();
            let before_version = client.data_version;
            // Produce a genuine durable receipt, then alter only the identity
            // carried by the worker. The production receiver must reject it.
            client.persistence.worker = Some(
                LatestWorker::spawn(
                    "timer-receipt-identity-test",
                    move |input: SnapshotInput| {
                        let mut receipt = write_drafts(input)?;
                        match mismatch {
                            "workspace" => receipt
                                .timer_action
                                .as_mut()
                                .unwrap()
                                .workspace
                                .push_str("-other"),
                            "base_version" => {
                                receipt.base_version = receipt.base_version.saturating_add(1)
                            }
                            "action_id" => receipt.timer_action.as_mut().unwrap().id += 1,
                            "valid" => {}
                            _ => unreachable!(),
                        }
                        Ok(SnapshotCompletion::Saved(receipt))
                    },
                    || {},
                )
                .unwrap(),
            );
            let ctx = egui::Context::default();
            let slot = client.data.slots[0].clone();
            client.toggle_slot(&slot, &ctx);
            assert!(client.persistence.pending());
            client.ensure_selected_slot_draft();
            client.slot_title_draft = "newer draft must remain".into();
            client.mark_slot_dirty();
            let deadline = Instant::now() + Duration::from_secs(30);
            while client.persistence.pending() && Instant::now() < deadline {
                client.poll_draft_persistence();
                thread::sleep(Duration::from_millis(5));
            }
            assert!(
                !client.persistence.pending(),
                "receipt timed out: {mismatch}"
            );
            assert_eq!(client.slot_title_draft, "newer draft must remain");
            assert!(client.slot_dirty);
            if mismatch == "valid" {
                assert!(!client.persistence.failed());
                assert!(client.data.slots[0].running_since_epoch_millis.is_some());
                assert_eq!(client.data_version, before_version + 1);
                assert!(client.persistence.pending_timer_action.is_none());
            } else {
                assert!(
                    client.persistence.failed(),
                    "invalid receipt accepted: {mismatch}"
                );
                assert_eq!(client.state_json, before, "{mismatch}");
                assert_eq!(client.data_version, before_version, "{mismatch}");
                assert!(client.data.slots[0].running_since_epoch_millis.is_none());
                assert!(client.data.sessions.is_empty());
            }
            // Refusing an untrusted receipt must not roll back a completed
            // journal transaction; recovery can still recover the actual run.
            let persisted = decode_data(&fs::read_to_string(&client.state_path).unwrap());
            assert!(persisted.slots[0].running_since_epoch_millis.is_some());
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
