// Windows startup - Keep one owner head through a read-only audited recovery session.
// v1.1.0.2 Windows - Read the startup recovery head under one verified journal reservation.
use super::*;

pub struct DesktopStartupWorkspaceHead {
    pub owner_initialized: bool,
    pub privacy_policy: DesktopPrivacyPolicy,
    pub latest: Option<DesktopStateSnapshot>,
}

impl DesktopStateStore {
    /// The caller checks independent evidence while the journal is reserved.
    /// Only an existing current-format journal uses this path; initialization
    /// and format migrations retain the established recovery implementation.
    /// No authority receipt escapes: the result is an owned snapshot and policy.
    pub fn open_existing_startup_workspace(
        database_path: &Path,
        owner: &str,
        now_epoch_millis: i64,
        mut verify_independent_evidence: impl FnMut(&DesktopStateJournalEvidence) -> Result<(), String>,
    ) -> DesktopStateStoreResult<Option<(Self, DesktopStartupWorkspaceHead)>> {
        validate_owner(owner)?;
        if !database_path.try_exists()? {
            return Ok(None);
        }
        if fs::metadata(database_path)?.len() == 0 {
            return Err(integrity("existing journal is empty"));
        }
        let store = Self {
            database_path: database_path.to_path_buf(),
        };
        let mut connection = store.open_connection(false)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != STORE_SCHEMA_VERSION {
            return Ok(None);
        }
        // Full integrity includes the quick-check subset; keep the separate
        // post-audit quick check after schema or quarantine writes.
        verify_sqlite_full_integrity(&connection)?;
        apply_schema(&mut connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_required_schema(&transaction)?;
        verify_foreign_keys(&transaction)?;
        verify_all_owner_registries(&transaction)?;
        let (quarantined, audited_head) =
            quarantine_invalid_rows_preserving_evidence_with_head_in_transaction(
                &transaction,
                None,
                now_epoch_millis.max(0),
                Some(owner),
            )?;
        let changes_after_audit = transaction.total_changes();
        verify_quick_check(&transaction)?;
        let evidence = read_journal_evidence(&transaction)?;
        verify_independent_evidence(&evidence).map_err(integrity)?;
        // The audit head is reusable only before a write in this same journal
        // reservation. Quarantine triggers can alter already visited rows.
        let latest_before_repair =
            if quarantined == 0 && transaction.total_changes() == changes_after_audit {
                audited_head
            } else {
                latest_valid_in_transaction(&transaction, owner, 0)?
            };
        let repaired_head = privacy::repair_privacy_history_from_head_in_transaction(
            &transaction,
            owner,
            latest_before_repair,
        )?;
        let owner_initialized = owner_registry_initialized(&transaction, owner)?;
        let privacy_policy = privacy::read_privacy_policy(&transaction, owner)?.0;
        let latest = if owner_initialized {
            repaired_head
        } else {
            None
        };
        transaction.commit()?;
        Ok(Some((
            store,
            DesktopStartupWorkspaceHead {
                owner_initialized,
                privacy_policy,
                latest,
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn startup_session_rejects_external_evidence_before_returning_any_head() {
        let root = std::env::temp_dir().join(format!(
            "startup-session-evidence-{}-{}",
            std::process::id(),
            system_time_epoch_millis()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.sqlite3");
        let store = DesktopStateStore::open(&path).unwrap();
        store
            .record(
                "owner",
                &app_data::default_app_data_json(100),
                100,
                "local_save",
            )
            .unwrap();
        let called = Cell::new(false);
        let result =
            DesktopStateStore::open_existing_startup_workspace(&path, "owner", 100, |_| {
                called.set(true);
                Err("independent owner evidence mismatch".into())
            });
        assert!(called.get());
        assert!(result.is_err());
        let (_, head) =
            DesktopStateStore::open_existing_startup_workspace(&path, "owner", 100, |_| Ok(()))
                .unwrap()
                .unwrap();
        assert!(head.owner_initialized);
        assert!(head.latest.is_some());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture(label: &str) -> (PathBuf, DesktopStateStore) {
        let root = std::env::temp_dir().join(format!(
            "startup-head-{label}-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        fs::create_dir_all(&root).unwrap();
        let store = DesktopStateStore::open(root.join("state.sqlite3")).unwrap();
        (root, store)
    }

    #[test]
    fn startup_readonly_head_preserves_exact_owner_and_sync_pair() {
        let (root, store) = fixture("owner-pair");
        let state = app_data::default_app_data_json(100);
        let expected = store
            .record_with_sync_state("owner-a", &state, b"owner-a-sync", 100, "local_save")
            .unwrap();
        store
            .record_with_sync_state("owner-b", &state, b"foreign-sync", 110, "local_save")
            .unwrap();
        let evidence = store.journal_evidence().unwrap();
        SAVE_AUDIT_COUNTS.with(|counts| counts.set(SaveAuditCounts::default()));
        let (_, head) = DesktopStateStore::open_existing_startup_workspace(
            store.database_path(),
            "owner-a",
            200,
            |actual| {
                assert_eq!(*actual, evidence);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        assert!(head.owner_initialized);
        assert_eq!(head.latest, Some(expected));
        let checks = SAVE_AUDIT_COUNTS.with(Cell::get);
        assert!(
            checks.integrity > 0,
            "startup must execute full journal integrity"
        );
        assert!(
            checks.quick > 0,
            "startup must execute post-audit structure validation"
        );
        assert!(
            checks.history > 0,
            "startup must verify persisted recovery rows"
        );
        assert!(head.privacy_policy.is_empty());
        assert_eq!(store.journal_evidence().unwrap(), evidence);
        let (_, unknown) = DesktopStateStore::open_existing_startup_workspace(
            store.database_path(),
            "unknown-owner",
            200,
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!(!unknown.owner_initialized);
        assert!(unknown.latest.is_none());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_quarantine_does_not_reuse_head_changed_by_a_trigger() {
        let (root, store) = fixture("quarantine-write");
        let state = app_data::default_app_data_json(100);
        store.record("owner-a", &state, 100, "local_save").unwrap();
        store.record("owner-b", &state, 110, "local_save").unwrap();
        let connection = store.open_connection(false).unwrap();
        connection.execute_batch("UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='owner-b';
            CREATE TRIGGER damage_visited_head AFTER DELETE ON desktop_state_snapshots WHEN OLD.owner='owner-b'
            BEGIN UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='owner-a'; END;").unwrap();
        drop(connection);
        let (_, head) = DesktopStateStore::open_existing_startup_workspace(
            store.database_path(),
            "owner-a",
            200,
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!(head.owner_initialized);
        assert!(
            head.latest.is_none(),
            "a pre-quarantine copy cannot authorize a damaged row"
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_privacy_write_rechecks_the_returned_head() {
        let (root, store) = fixture("privacy-write");
        let plain = app_data::upsert_note_app_data_json(
            &app_data::default_app_data_json(100),
            r#"{"id":"private","title":"Private","content":"MUST_NOT_RESTORE","updatedAtEpochMillis":100}"#,
            100
        ).unwrap();
        let deleted =
            app_data::delete_note_permanently_app_data_json(&plain, "private", 200).unwrap();
        store.record("owner", &deleted, 200, "local_save").unwrap();
        let connection = store.open_connection(false).unwrap();
        connection.execute_batch("DELETE FROM desktop_state_privacy_barriers;
            CREATE TRIGGER damage_head_on_privacy_repair AFTER INSERT ON desktop_state_privacy_barriers
            BEGIN UPDATE desktop_state_snapshots SET app_data_json='{}' WHERE owner='owner'; END;").unwrap();
        drop(connection);
        let (_, head) = DesktopStateStore::open_existing_startup_workspace(
            store.database_path(),
            "owner",
            300,
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!(head.owner_initialized);
        assert!(
            head.latest.is_none(),
            "privacy writes require a freshly verified head"
        );
        assert!(!head.privacy_policy.is_empty());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
