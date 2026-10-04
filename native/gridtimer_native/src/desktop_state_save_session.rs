// v1.1.0.3 Windows - Check a save once under the same journal reservation.
use super::*;

/// Internal phase boundaries for an explicitly requested save diagnostic.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopSavePhase {
    OpenAndFullIntegrity,
    SchemaForeignKeysAndOwners,
    HistoryAudit,
    IndependentEvidence,
    ParentSelection,
    ProtectedSync,
    PrivacyPrepare,
    IncomingAnalysis,
    DestructiveDropAndPrivacyCommit,
    InsertReadbackVerify,
    HistoryPrune,
    ParentMirrorRecheck,
    SqliteCommit,
    ConnectionClose,
}

/// The default implementation has no clock, allocation or dynamic dispatch.
#[doc(hidden)]
pub trait DesktopSaveObserver {
    fn measure<T>(&mut self, phase: DesktopSavePhase, operation: impl FnOnce() -> T) -> T;
}

#[doc(hidden)]
pub struct NoopDesktopSaveObserver;

impl DesktopSaveObserver for NoopDesktopSaveObserver {
    #[inline(always)]
    fn measure<T>(&mut self, _phase: DesktopSavePhase, operation: impl FnOnce() -> T) -> T {
        operation()
    }
}

/// Exact bytes verified and committed by this module. Public callers cannot
/// fabricate a successful commit or turn an arbitrary snapshot into authority.
/// Mirror preparation rechecks the database identity, sequence and exact row
/// before reusing the semantic analysis of these bytes.
pub struct DesktopCommittedSnapshot {
    pub(super) database_path: PathBuf,
    pub(super) evidence: DesktopStateJournalEvidence,
    pub(super) snapshot: DesktopStateSnapshot,
    pub(super) parent: Option<DesktopStateSnapshot>,
    policy: DesktopPrivacyPolicy,
}

impl DesktopCommittedSnapshot {
    pub fn snapshot(&self) -> &DesktopStateSnapshot {
        &self.snapshot
    }

    pub fn journal_evidence(&self) -> &DesktopStateJournalEvidence {
        &self.evidence
    }

    pub fn privacy_policy(&self) -> &DesktopPrivacyPolicy {
        &self.policy
    }

    /// Only a pure compatibility hint. Mirror publication still rechecks the
    /// corresponding database row under its own reserved transaction.
    pub fn contains_exact_snapshot(&self, owner: &str, raw: &str) -> bool {
        std::iter::once(&self.snapshot)
            .chain(self.parent.iter())
            .any(|snapshot| snapshot.owner == owner && snapshot.app_data_json == raw)
    }
}

impl DesktopStateStore {
    /// The fast path is deliberately limited to ordinary local saves in an
    /// existing current-format journal. Initialization, migrations and media
    /// declaration transactions retain their established entry points.
    /// All authority checks occur again for every call; no filesystem time or
    /// remembered verification flag grants permission for a later save.
    pub fn record_existing_save_session(
        database_path: &Path,
        owner: &str,
        app_data_json: &str,
        now_epoch_millis: i64,
        source: &str,
        protect_sync: impl FnOnce(Option<&DesktopStateSnapshot>) -> Result<Vec<u8>, String>,
        verify_evidence: impl FnOnce(&DesktopStateJournalEvidence) -> Result<(), String>,
        check_privacy_mirrors: impl FnOnce(&DesktopPrivacyPolicy) -> Result<(), String>,
    ) -> DesktopStateStoreResult<Option<(Self, DesktopCommittedSnapshot)>> {
        Self::record_existing_save_session_observed(
            database_path,
            owner,
            app_data_json,
            now_epoch_millis,
            source,
            protect_sync,
            verify_evidence,
            check_privacy_mirrors,
            &mut NoopDesktopSaveObserver,
        )
    }

    #[doc(hidden)]
    pub fn record_existing_save_session_observed(
        database_path: &Path,
        owner: &str,
        app_data_json: &str,
        now_epoch_millis: i64,
        source: &str,
        protect_sync: impl FnOnce(Option<&DesktopStateSnapshot>) -> Result<Vec<u8>, String>,
        verify_evidence: impl FnOnce(&DesktopStateJournalEvidence) -> Result<(), String>,
        check_privacy_mirrors: impl FnOnce(&DesktopPrivacyPolicy) -> Result<(), String>,
        observer: &mut impl DesktopSaveObserver,
    ) -> DesktopStateStoreResult<Option<(Self, DesktopCommittedSnapshot)>> {
        validate_owner(owner)?;
        let source = normalized_source(source)?;
        if source != "local_save" || !database_path.try_exists()? {
            return Ok(None);
        }
        if fs::metadata(database_path)?.len() == 0 {
            return Err(integrity("existing journal is empty"));
        }
        let canonical_database_path = fs::canonicalize(database_path)?;
        let store = Self {
            database_path: database_path.to_path_buf(),
        };
        let mut connection = observer.measure(DesktopSavePhase::OpenAndFullIntegrity, || {
            store.open_connection(false)
        })?;
        let version: i64 = observer.measure(DesktopSavePhase::OpenAndFullIntegrity, || {
            connection.pragma_query_value(None, "user_version", |row| row.get(0))
        })?;
        if version != STORE_SCHEMA_VERSION {
            return Ok(None);
        }
        let transaction = observer.measure(DesktopSavePhase::OpenAndFullIntegrity, || {
            connection.transaction_with_behavior(TransactionBehavior::Immediate)
        })?;
        observer.measure(DesktopSavePhase::OpenAndFullIntegrity, || {
            verify_sqlite_full_integrity(&transaction)
        })?;
        observer.measure(DesktopSavePhase::SchemaForeignKeysAndOwners, || {
            verify_required_schema(&transaction)?;
            verify_foreign_keys(&transaction)?;
            verify_all_owner_registries(&transaction)
        })?;
        let (quarantined, audited_head) =
            observer.measure(DesktopSavePhase::HistoryAudit, || {
                quarantine_invalid_rows_preserving_evidence_with_head_in_transaction(
                    &transaction,
                    None,
                    now_epoch_millis.max(0),
                    Some(owner),
                )
            })?;
        let changes_after_audit = transaction.total_changes();
        observer.measure(DesktopSavePhase::IndependentEvidence, || {
            let evidence = read_journal_evidence(&transaction)?;
            verify_evidence(&evidence).map_err(integrity)
        })?;
        let (owner_initialized, parent) =
            observer.measure(DesktopSavePhase::ParentSelection, || {
                let owner_initialized = owner_registry_initialized(&transaction, owner)?;
                // Consume the audit only inside this same reservation, before
                // any write. A quarantine or later write requires a fresh head
                // read, including effects of triggers on already visited rows.
                let parent =
                    if quarantined == 0 && transaction.total_changes() == changes_after_audit {
                        audited_head
                    } else {
                        latest_valid_in_transaction(&transaction, owner, now_epoch_millis.max(0))?
                    };
                Ok::<_, DesktopStateStoreError>((owner_initialized, parent))
            })?;
        let protected_sync_state = observer.measure(DesktopSavePhase::ProtectedSync, || {
            protect_sync(parent.as_ref()).map_err(integrity)
        })?;
        let (prepared_privacy, policy) =
            observer.measure(DesktopSavePhase::PrivacyPrepare, || {
                let prepared_privacy =
                    privacy::prepare_privacy_transition(&transaction, owner, app_data_json)?;
                let policy = prepared_privacy.policy().clone();
                check_privacy_mirrors(&policy).map_err(integrity)?;
                Ok::<_, DesktopStateStoreError>((prepared_privacy, policy))
            })?;
        let mut parent_for_mirror = None;
        let (analysis, raw_sha256, sync_state_sha256) =
            observer.measure(DesktopSavePhase::IncomingAnalysis, || {
                let (analysis, raw_sha256) =
                    analyze_app_data_json_with_raw_digest(app_data_json, 0)?;
                Ok::<_, DesktopStateStoreError>((
                    analysis,
                    raw_sha256,
                    sha256_hex(&protected_sync_state),
                ))
            })?;
        let result = record_snapshot_in_transaction_observed(
            &transaction,
            owner,
            app_data_json,
            &protected_sync_state,
            now_epoch_millis.max(0),
            &source,
            "",
            &[],
            analysis,
            raw_sha256,
            sync_state_sha256,
            Some(VerifiedSaveHistory {
                owner_initialized,
                parent,
                privacy: Some(prepared_privacy),
            }),
            Some(&mut parent_for_mirror),
            observer,
        );
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(
                error @ (DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot
                | DesktopStateStoreError::SuspiciousItemDrop { .. }),
            ) => {
                observer.measure(DesktopSavePhase::SqliteCommit, || transaction.commit())?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        // Privacy redaction can rewrite the prior row. Such a row must not be
        // authorized by its pre-redaction bytes when rotating a JSON backup.
        let parent = observer.measure(DesktopSavePhase::ParentMirrorRecheck, || {
            let parent = match parent_for_mirror {
                Some(parent) if snapshot_matches_row(&transaction, &parent)? => Some(parent),
                _ => None,
            };
            Ok::<_, DesktopStateStoreError>(parent)
        })?;
        let evidence = read_journal_evidence(&transaction)?;
        observer.measure(DesktopSavePhase::SqliteCommit, || transaction.commit())?;
        let committed = DesktopCommittedSnapshot {
            database_path: canonical_database_path,
            evidence,
            snapshot,
            parent,
            policy,
        };
        observer.measure(DesktopSavePhase::ConnectionClose, || drop(connection));
        Ok(Some((store, committed)))
    }

    /// Revalidate this precise commit while reserving the journal against a
    /// concurrent writer, then refresh independent evidence inside that scope.
    /// This is not a reusable authorization for another commit or database.
    pub fn with_committed_snapshot_evidence(
        &self,
        committed: &DesktopCommittedSnapshot,
        refresh: impl FnOnce(&DesktopStateJournalEvidence) -> Result<(), String>,
    ) -> DesktopStateStoreResult<()> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_required_schema(&transaction)?;
        verify_foreign_keys(&transaction)?;
        let owner = &committed.snapshot.owner;
        validate_owner(owner)?;
        if fs::canonicalize(self.database_path())? != committed.database_path
            || read_journal_evidence(&transaction)? != committed.evidence
            || !owner_registry_initialized(&transaction, owner)?
            || !snapshot_matches_row(&transaction, &committed.snapshot)?
        {
            return Err(integrity(
                "committed evidence receipt no longer matches this journal",
            ));
        }
        refresh(&committed.evidence).map_err(integrity)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str) -> (PathBuf, DesktopStateStore, String) {
        let directory = std::env::temp_dir().join(format!(
            "desktop-save-session-{label}-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        fs::create_dir_all(&directory).unwrap();
        let store = DesktopStateStore::open(directory.join("state.sqlite3")).unwrap();
        let state = app_data::default_app_data_json(100);
        store
            .record_with_sync_state("owner", &state, b"protected", 100, "local_save")
            .unwrap();
        (directory, store, state)
    }

    fn save(
        store: &DesktopStateStore,
        state: &str,
    ) -> DesktopStateStoreResult<DesktopCommittedSnapshot> {
        DesktopStateStore::record_existing_save_session(
            store.database_path(),
            "owner",
            state,
            200,
            "local_save",
            |parent| Ok(parent.unwrap().protected_sync_state.clone()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .map(|result| result.unwrap().1)
    }

    #[test]
    fn save_session_commits_exact_timer_state_and_reopens_with_sync_pair() {
        let (directory, store, state) = fixture("roundtrip");
        store
            .record_with_sync_state(
                "other-owner",
                &state,
                b"foreign-protected",
                150,
                "local_save",
            )
            .unwrap();
        let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
        SAVE_AUDIT_COUNTS.with(|counts| counts.set(SaveAuditCounts::default()));
        let committed = save(&store, &running).unwrap();
        assert_eq!(
            SAVE_AUDIT_COUNTS.with(std::cell::Cell::get),
            SaveAuditCounts {
                integrity: 1,
                history: 1,
                // One full check covers the quick-check subset in this reservation.
                quick: 0,
            }
        );
        assert_eq!(committed.snapshot.app_data_json, running);
        assert_eq!(committed.snapshot.protected_sync_state, b"protected");
        assert_eq!(committed.parent.as_ref().unwrap().app_data_json, state);
        assert_eq!(committed.parent.as_ref().unwrap().owner, "owner");
        let unchanged = save(&store, &running).unwrap();
        assert_eq!(unchanged.snapshot, committed.snapshot);
        assert_eq!(unchanged.parent.as_ref(), Some(&committed.snapshot));
        let reopened = DesktopStateStore::open(store.database_path()).unwrap();
        assert_eq!(
            reopened.latest_valid("owner", 200).unwrap().unwrap(),
            committed.snapshot
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_session_rechecks_head_after_quarantine_triggers_change_an_already_audited_row() {
        let (directory, store, state) = fixture("audit-trigger");
        store
            .record_with_sync_state("other-owner", &state, b"other", 150, "local_save")
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='other-owner';
             CREATE TRIGGER change_audited_head_after_quarantine
             AFTER DELETE ON desktop_state_snapshots WHEN OLD.owner='other-owner'
             BEGIN
                 UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='owner';
             END;",
            )
            .unwrap();
        drop(connection);
        let parent_was_rechecked = std::cell::Cell::new(false);
        let result = DesktopStateStore::record_existing_save_session(
            store.database_path(),
            "owner",
            &state,
            200,
            "local_save",
            |parent| {
                assert!(
                    parent.is_none(),
                    "pre-quarantine bytes must not authorize the altered head"
                );
                parent_was_rechecked.set(true);
                Ok(b"protected".to_vec())
            },
            |_| Ok(()),
            |_| Ok(()),
        );
        assert!(parent_was_rechecked.get());
        assert!(matches!(
            result,
            Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot)
        ));
        let connection = store.open_connection(false).unwrap();
        let (raw, quarantined): (String, i64) = connection
            .query_row(
                "SELECT app_data_json, (SELECT count(*) FROM desktop_state_snapshot_quarantine)
             FROM desktop_state_snapshots WHERE owner='owner'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(raw, "{}");
        assert_eq!(quarantined, 1);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_session_external_evidence_and_privacy_failures_prevent_commit() {
        let (directory, store, state) = fixture("evidence");
        let before = store.latest_valid("owner", 200).unwrap().unwrap();
        let evidence = store.journal_evidence().unwrap();
        let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
        for fail_evidence in [true, false] {
            let result = DesktopStateStore::record_existing_save_session(
                store.database_path(),
                "owner",
                &running,
                200,
                "local_save",
                |_| Ok(b"changed".to_vec()),
                |_| {
                    if fail_evidence {
                        Err("external sequence is newer".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    if fail_evidence {
                        Ok(())
                    } else {
                        Err("privacy mirror could not be verified".into())
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(store.latest_valid("owner", 200).unwrap().unwrap(), before);
            assert_eq!(store.journal_evidence().unwrap(), evidence);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn committed_evidence_refresh_reserves_the_exact_commit_and_propagates_failure() {
        let (directory, store, state) = fixture("evidence-reservation");
        let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
        let committed = save(&store, &running).unwrap();
        let expected = committed.evidence.clone();
        store
            .with_committed_snapshot_evidence(&committed, |evidence| {
                assert_eq!(evidence, &expected);
                let other = Connection::open(store.database_path()).unwrap();
                other.busy_timeout(Duration::ZERO).unwrap();
                let error = other
                    .execute(
                        "UPDATE desktop_state_journal_metadata SET commit_sequence=commit_sequence",
                        [],
                    )
                    .unwrap_err();
                assert_eq!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy)
                );
                Ok(())
            })
            .unwrap();
        assert!(store
            .with_committed_snapshot_evidence(&committed, |_| Err("marker write failed".into()))
            .is_err());
        assert_eq!(store.journal_evidence().unwrap(), expected);
        assert_eq!(
            store.latest_valid("owner", 200).unwrap().unwrap(),
            committed.snapshot
        );
        // A later successful save invalidates even a receipt that was already
        // used successfully; the refresh callback must never see its authority.
        let paused = app_data::pause_slots_app_data_json(&running, &[1], 250).unwrap();
        store.record("owner", &paused, 250, "local_save").unwrap();
        let called = std::cell::Cell::new(false);
        assert!(store
            .with_committed_snapshot_evidence(&committed, |_| {
                called.set(true);
                Ok(())
            })
            .is_err());
        assert!(!called.get());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_session_keeps_corruption_in_quarantine_and_refuses_reseed() {
        let (directory, store, state) = fixture("corrupt");
        let connection = store.open_connection(false).unwrap();
        connection
            .execute("UPDATE desktop_state_snapshots SET app_data_json='{}'", [])
            .unwrap();
        drop(connection);
        let result = DesktopStateStore::record_existing_save_session(
            store.database_path(),
            "owner",
            &state,
            200,
            "local_save",
            |_| Ok(Vec::new()),
            |_| Ok(()),
            |_| Ok(()),
        );
        assert!(matches!(
            result,
            Err(DesktopStateStoreError::InitializedOwnerWithoutValidSnapshot)
        ));
        assert!(store.owner_initialized("owner").unwrap());
        assert!(store.latest_valid("owner", 200).unwrap().is_none());
        let connection = store.open_connection(false).unwrap();
        let quarantined: i64 = connection
            .query_row(
                "SELECT count(*) FROM desktop_state_snapshot_quarantine",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quarantined, 1);
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn committed_mirror_receipt_tracks_its_own_metadata_commits() {
        let (directory, store, state) = fixture("mirror");
        let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
        let mut committed = save(&store, &running).unwrap();
        let writer = store.lock_recovery_mirror_writer().unwrap();
        let before = committed.evidence.commit_sequence;
        for (index, raw) in [&state, &running].into_iter().enumerate() {
            let target = directory.join(if index == 0 {
                "timer_state.json.bak"
            } else {
                "timer_state.json"
            });
            let temporary = directory.join(format!("timer_state.json.tmp-proof-{index}"));
            assert!(writer
                .prepare_committed("owner", &target, &temporary, raw, None, &mut committed)
                .unwrap());
            assert_eq!(committed.evidence, store.journal_evidence().unwrap());
            assert_eq!(
                committed.evidence.commit_sequence,
                before + index as i64 + 1
            );
            fs::write(&target, raw).unwrap();
            assert!(store
                .recovery_mirror_is_verified("owner", &target, raw)
                .unwrap());
        }
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn committed_mirror_receipt_rejects_owner_tampering_and_later_commits() {
        for corrupt_row in [true, false] {
            let (directory, store, state) = fixture("stale");
            let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
            let mut committed = save(&store, &running).unwrap();
            let writer = store.lock_recovery_mirror_writer().unwrap();
            let target = directory.join("timer_state.json");
            let temporary = directory.join("timer_state.json.tmp-proof");
            assert!(writer
                .prepare_committed(
                    "other-owner",
                    &target,
                    &temporary,
                    &running,
                    None,
                    &mut committed
                )
                .is_err());
            if corrupt_row {
                let connection = store.open_connection(false).unwrap();
                connection
                    .execute(
                        "UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE id=?1",
                        params![committed.snapshot.id],
                    )
                    .unwrap();
            } else {
                let paused = app_data::pause_slots_app_data_json(&running, &[1], 250).unwrap();
                store.record("owner", &paused, 250, "local_save").unwrap();
            }
            let refreshed = std::cell::Cell::new(false);
            assert!(store
                .with_committed_snapshot_evidence(&committed, |_| {
                    refreshed.set(true);
                    Ok(())
                })
                .is_err());
            assert!(!refreshed.get());
            assert!(writer
                .prepare_committed("owner", &target, &temporary, &running, None, &mut committed)
                .is_err());
            assert!(!target.exists());
            drop(writer);
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn save_session_privacy_redaction_rebinds_mirrors_and_rejects_plaintext_restore() {
        let (directory, store, state) = fixture("privacy");
        let plain = app_data::upsert_note_app_data_json(
            &state,
            r#"{"id":"private","title":"Private","content":"SAVE_SESSION_PRIVATE_MARKER","updatedAtEpochMillis":100}"#,
            100,
        ).unwrap();
        store
            .record_with_sync_state("owner", &plain, b"protected", 100, "local_save")
            .unwrap();
        let other = store.record("other", &plain, 100, "local_save").unwrap();
        let primary = directory.join("timer_state.json");
        let temporary = directory.join("timer_state.json.tmp-privacy");
        let writer = store.lock_recovery_mirror_writer().unwrap();
        assert!(writer
            .prepare("owner", &primary, &temporary, &plain, None)
            .unwrap());
        fs::write(&primary, &plain).unwrap();
        drop(writer);
        let deleted =
            app_data::delete_note_permanently_app_data_json(&plain, "private", 200).unwrap();
        let mut committed = save(&store, &deleted).unwrap();
        assert!(!committed.policy.is_empty());
        assert!(
            committed.parent.is_none(),
            "pre-redaction plaintext cannot remain an exact mirror authority"
        );
        let redacted = committed.policy.redact_json(&plain).unwrap().unwrap();
        assert!(!redacted.contains("SAVE_SESSION_PRIVATE_MARKER"));
        let writer = store.lock_recovery_mirror_writer().unwrap();
        assert!(writer
            .prepare_committed(
                "owner",
                &primary,
                &temporary,
                &redacted,
                Some((&primary, &plain)),
                &mut committed
            )
            .unwrap());
        assert_eq!(committed.evidence, store.journal_evidence().unwrap());
        fs::write(&primary, &redacted).unwrap();
        assert!(store
            .recovery_mirror_is_verified("owner", &primary, &redacted)
            .unwrap());
        drop(writer);
        assert!(save(&store, &plain).is_err());
        assert_eq!(store.latest_valid("other", 300).unwrap().unwrap(), other);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_session_refuses_substantial_unexplained_loss_and_future_schema() {
        let (directory, store, state) = fixture("loss");
        let mut populated: Value = serde_json::from_str(&state).unwrap();
        populated["sessions"] = Value::Array(
            (0..20)
                .map(|index| {
                    serde_json::json!({
                        "id": format!("session-{index}"), "slotId": 1, "slotTitle": "",
                        "categoryId": null, "startedAtEpochMillis": 101 + index,
                        "endedAtEpochMillis": 102 + index, "durationMillis": 1,
                        "updatedAtEpochMillis": 102 + index
                    })
                })
                .collect(),
        );
        let populated = app_data::sanitize_app_data_json(&populated.to_string(), 150).unwrap();
        let previous = store
            .record_with_sync_state("owner", &populated, b"protected", 150, "local_save")
            .unwrap();
        assert!(matches!(
            save(&store, &state),
            Err(DesktopStateStoreError::SuspiciousItemDrop { .. })
        ));
        let mut future: Value = serde_json::from_str(&populated).unwrap();
        future["schemaVersion"] = serde_json::json!(i32::MAX);
        assert!(save(&store, &future.to_string()).is_err());
        assert_eq!(store.latest_valid("owner", 300).unwrap().unwrap(), previous);
        fs::remove_dir_all(directory).unwrap();
    }
}
